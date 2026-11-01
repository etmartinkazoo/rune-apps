//! The network threads. The UI sends `Cmd`s and never blocks; results come back as `Event`s,
//! and the waker asks Rune for a frame so `poll` can apply them. Accounts are keyed by their
//! username. Images download on a small pool of their own so a newsletter's pictures don't
//! hold up the next message.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use rune::Waker;

use crate::accounts::{self, Saved};
use crate::caldav::{CalDav, Event as CalEvent};
use crate::contacts::{CardDav, Contact};
use crate::backend::{Backend, Kind};
use crate::jmap::{Body, Downloads, Mailbox, Outgoing, Summary};
use crate::settings::Storage;
use crate::store::Shared;

const PAGE: usize = 200;
const IMAGE_THREADS: usize = 4;

/// Conversations by thread key, each oldest first.
pub type Threads = HashMap<String, Vec<Summary>>;

/// What the message list shows.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum View {
    Mailbox { account: String, id: String },
    /// Every account's inbox, newest first.
    AllInboxes,
}

pub enum Cmd {
    /// Adds an account; `save` keeps it for next time.
    Connect { account: Saved, save: bool },
    Remove { account: String },
    /// Sets or clears an account's calendar app password, checking it first.
    SetCalendar { account: String, password: Option<String> },
    LoadEvents,
    LoadContacts,
    /// Names an account for the sidebar; empty clears it.
    SetLabel { account: String, label: String },
    /// Sets the name on an account's sent mail; empty goes back to the default.
    SetFromName { account: String, name: String },
    /// Puts accounts in this order (the sidebar's).
    Reorder { accounts: Vec<String> },
    /// Shows or hides the calendar of an IMAP account whose password opens one.
    SetCalendarEnabled { account: String, on: bool },
    /// `threads` groups the list into conversations.
    Refresh { view: Option<View>, search: String, threads: bool },
    LoadList { view: View, search: String, threads: bool },
    /// `opened`: you're waiting for it, rather than it being fetched ahead.
    LoadBody { account: String, id: String, opened: bool },
    /// `uid` is the message showing it (its date decides when a kept copy goes).
    FetchImage { account: String, uid: String, src: String },
    SetKeyword { account: String, ids: Vec<String>, keyword: &'static str, on: bool },
    Move { account: String, ids: Vec<String>, from: String, to: String },
    /// `id` ties the outcome back to the compose send that asked (0 for others).
    Send { id: u64, account: String, mail: Outgoing },
    /// RFC 8058 one-click unsubscribe: a POST to the list's https URL.
    Unsubscribe { uid: String, url: String },
    /// Local storage settings: the prefetcher follows them.
    Storage(Storage),
    /// Keeps messages as they are now (a flag or read change), off the UI thread.
    Keep(Vec<Summary>),
    /// Keeps a small value in the store (the last view), off the UI thread.
    Remember { key: String, value: String },
    /// Empties the store (the UI has already turned it off), then turns it `on` again or not.
    /// Actions it held for the network move to memory, or back in.
    ClearStore { on: bool },
    /// Opens attachment `file` of a message with the system's app, or writes it to `save`.
    Attachment { account: String, id: String, file: crate::jmap::FileInfo, save: Option<PathBuf> },
    /// An account finished connecting on its own thread (internal).
    Attach { saved: Saved, save: bool, result: Result<Backend, String> },
}

/// An action kept in the store's outbox while its account can't be reached.
#[derive(serde::Serialize, serde::Deserialize)]
enum Op {
    Keyword { ids: Vec<String>, keyword: String, on: bool },
    Move { ids: Vec<String>, from: String, to: String },
}

/// Every this many refreshes, reload everything even if nothing seems to have changed (an
/// IMAP server without CONDSTORE doesn't report flag changes made elsewhere).
const FULL_REFRESH_EVERY: u32 = 10;
/// How many attempts a queued action gets before the server's version wins.
const MAX_TRIES: i64 = 5;
/// The first page an IMAP list shows while the rest loads.
const QUICK_PAGE: usize = 50;

pub enum Event {
    /// `position` is the account's place in the saved order; `own_calendar` says its password
    /// opens a calendar (so Settings offers a switch rather than asking for a password).
    Connected { account: String, calendar: bool, kind: Kind, label: Option<String>, from_name: Option<String>, position: usize, own_calendar: bool },
    ConnectFailed { error: String },
    Mailboxes { account: String, list: Vec<Mailbox> },
    /// One row per conversation (its newest message in the view), and the conversations.
    List { view: View, search: String, messages: Vec<Summary>, threads: Threads },
    /// Boxed, as a body is larger than every other event.
    Body { uid: String, body: Box<Body> },
    BodyFailed { uid: String, error: String },
    /// A refresh finished (or failed), so the toolbar can stop spinning.
    Refreshed,
    Image { src: String, bytes: Vec<u8> },
    /// Today's events for one account (replacing what it had).
    Events { account: String, events: Vec<CalEvent> },
    CalendarSet { account: String, on: bool },
    /// Every contact of one account's address books (replacing what it had).
    Contacts { account: String, contacts: Vec<Contact> },
    Sent { id: u64 },
    SendFailed { id: u64, error: String },
    Unsubscribed { uid: String },
    Error(String),
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-mail")
}

/// The saved accounts, for the UI to connect at launch.
pub fn saved_accounts() -> Vec<Saved> {
    accounts::load()
}

struct Account {
    client: Backend,
    mailboxes: Vec<Mailbox>,
    calendar: Option<CalDav>,
    contacts: Option<CardDav>,
}

/// Starts image threads; returns where to send (downloader, source) pairs.
/// An image to fetch: its downloader, source, message, and whether to keep it in the store.
type ImageJob = (Downloads, String, String, bool);

fn image_pool(events: Sender<Event>, wake: Waker, store: Option<Shared>) -> Sender<ImageJob> {
    let (tx, rx) = channel::<ImageJob>();
    let rx = Arc::new(Mutex::new(rx));
    for i in 0..IMAGE_THREADS {
        let (rx, events, wake, store) = (rx.clone(), events.clone(), wake.clone(), store.clone());
        std::thread::Builder::new()
            .name(format!("images-{i}"))
            .spawn(move || {
                loop {
                    let Ok((downloads, src, uid, keep)) = rx.lock().unwrap().recv() else { return };
                    if let Some(bytes) = store.as_ref().and_then(|s| s.image(&src)) {
                        let _ = events.send(Event::Image { src, bytes });
                        wake();
                        continue;
                    }
                    // A failed image just stays blank.
                    if let Ok(bytes) = downloads.fetch(&src) {
                        if keep && let Some(s) = &store {
                            s.save_image(&src, &bytes, &uid);
                        }
                        let _ = events.send(Event::Image { src, bytes });
                        wake();
                    }
                }
            })
            .expect("spawn an image thread");
    }
    tx
}

/// One message body to fetch, with what's needed to sign in to its account, and whether
/// you're waiting for it (opened) rather than it being fetched ahead. An empty id just
/// signs in, so the first message opened doesn't wait for that.
type BodyJob = (Saved, String, String, bool);

/// A mailbox list to load: the account, its mailboxes as known, the view, search, threads.
type ListJob = (Saved, Vec<Mailbox>, View, String, bool);

/// List loads, on a thread per account with its own connection: a click on one account's
/// mailbox doesn't wait behind another's, nor behind the worker's refreshes and actions.
fn list_pool(events: Sender<Event>, wake: Waker, store: Option<Shared>) -> Sender<ListJob> {
    let (tx, rx) = channel::<ListJob>();
    std::thread::Builder::new()
        .name("lists".into())
        .spawn(move || {
            let mut lanes: HashMap<String, Sender<ListJob>> = HashMap::new();
            for job in rx {
                let key = job.0.username.clone().unwrap_or_default();
                let lane = lanes.entry(key).or_insert_with(|| list_lane(events.clone(), wake.clone(), store.clone()));
                let _ = lane.send(job);
            }
        })
        .expect("spawn the list thread");
    tx
}

/// One account's lane. Only the newest request runs: clicking through mailboxes asks for each.
fn list_lane(events: Sender<Event>, wake: Waker, store: Option<Shared>) -> Sender<ListJob> {
    let (tx, rx) = channel::<ListJob>();
    std::thread::Builder::new()
        .name("list-lane".into())
        .spawn(move || {
            let mut conn: Option<Backend> = None;
            while let Ok(first) = rx.recv() {
                let (saved, mailboxes, view, search, threads) = rx.try_iter().last().unwrap_or(first);
                // All Inboxes loads on the worker; asked for here, it signs the lane in ahead
                // of the first click.
                let View::Mailbox { id, .. } = &view else {
                    if conn.is_none() {
                        conn = Backend::connect(&saved).ok();
                    }
                    continue;
                };
                // A connection gone stale while idle gets one more try on a fresh one.
                let load = |page: usize, conn: &mut Option<Backend>| -> Result<(Vec<Summary>, Threads), String> {
                    let mut last = String::new();
                    for _ in 0..2 {
                        let c = match conn {
                            Some(c) => c,
                            None => conn.insert(Backend::connect(&saved)?),
                        };
                        match list_from(c, &mailboxes, id, &search, threads, page) {
                            Ok(found) => return Ok(found),
                            Err(e) => {
                                last = e;
                                *conn = None;
                            }
                        }
                    }
                    Err(last)
                };
                let send = |e: Event| {
                    let _ = events.send(e);
                    wake();
                };
                // IMAP fetches a page in a round trip per few dozen messages; the first ones
                // show while the rest come.
                if saved.imap.is_some() {
                    match load(QUICK_PAGE, &mut conn) {
                        Ok((messages, found)) => send(Event::List { view: view.clone(), search: search.clone(), messages, threads: found }),
                        Err(e) => {
                            send(Event::Error(e));
                            continue;
                        }
                    }
                }
                match load(PAGE, &mut conn) {
                    Ok((messages, found)) => {
                        if let Some(s) = store.as_ref().filter(|_| search.is_empty()) {
                            s.save_listing(&view, threads, &messages, &found);
                        }
                        send(Event::List { view, search, messages, threads: found });
                    }
                    Err(e) => send(Event::Error(e)),
                }
            }
        })
        .expect("spawn a list lane");
    tx
}

/// The body threads: a lane per account for messages you open and another for fetching
/// ahead, each with its own connection, so opening a message never waits behind a
/// prefetch, a slow account, list loads or refreshes.
fn body_pool(events: Sender<Event>, wake: Waker, store: Option<Shared>) -> Sender<BodyJob> {
    let (tx, rx) = channel::<BodyJob>();
    std::thread::Builder::new()
        .name("bodies".into())
        .spawn(move || {
            let mut lanes: HashMap<(String, bool), Sender<BodyJob>> = HashMap::new();
            for job in rx {
                let lane = lanes
                    .entry((job.1.clone(), job.3))
                    .or_insert_with(|| body_lane(events.clone(), wake.clone(), store.clone(), job.3));
                let _ = lane.send(job);
            }
        })
        .expect("spawn the body thread");
    tx
}

/// One lane: the newest request goes first, so the message just opened jumps ahead of ones
/// scrolled past.
fn body_lane(events: Sender<Event>, wake: Waker, store: Option<Shared>, opened: bool) -> Sender<BodyJob> {
    let (tx, rx) = channel::<BodyJob>();
    std::thread::Builder::new()
        .name(if opened { "bodies-open" } else { "bodies-ahead" }.into())
        .spawn(move || {
            let mut conn: Option<Backend> = None;
            let mut stack: Vec<BodyJob> = Vec::new();
            loop {
                stack.extend(rx.try_iter());
                let Some((saved, account, id, _)) = stack.pop() else {
                    match rx.recv() {
                        Ok(job) => stack.push(job),
                        Err(_) => return,
                    }
                    continue;
                };
                if id.is_empty() {
                    if conn.is_none() {
                        conn = Backend::connect(&saved).ok();
                    }
                    continue;
                }
                let uid = crate::jmap::uid(&account, &id);
                if stack.iter().any(|(_, a, i, _)| *a == account && *i == id) {
                    continue; // asked for again; the newer request will do it
                }
                // Read before: straight from the store, no network.
                if let Some(body) = store.as_ref().and_then(|s| s.body(&uid)) {
                    let _ = events.send(Event::Body { uid, body: Box::new(body) });
                    wake();
                    continue;
                }
                if conn.is_none() {
                    conn = Backend::connect(&saved).ok();
                }
                let mut result = match conn.as_mut() {
                    Some(c) => c.body(&id),
                    None => Err("Couldn't connect".to_owned()),
                };
                // A connection gone stale while idle: once more on a fresh one.
                if result.is_err() {
                    conn = Backend::connect(&saved).ok();
                    if let Some(c) = conn.as_mut() {
                        result = c.body(&id);
                    }
                }
                let event = match result {
                    Ok(body) => {
                        if let Some(s) = &store {
                            s.save_body(&uid, &body, opened);
                        }
                        Event::Body { uid, body: Box::new(body) }
                    }
                    Err(error) => {
                        conn = None; // a fresh connection next time
                        Event::BodyFailed { uid, error }
                    }
                };
                let _ = events.send(event);
                wake();
            }
        })
        .expect("spawn a body lane");
    tx
}

/// `me` is the sending side of `cmds`, for connections that finish on their own threads.
pub fn spawn(cmds: Receiver<Cmd>, me: Sender<Cmd>, events: Sender<Event>, wake: Waker, store: Option<Shared>) {
    std::thread::Builder::new()
        .name("jmap".into())
        .spawn(move || {
            let images = image_pool(events.clone(), wake.clone(), store.clone());
            let prefetch = store.clone().map(crate::prefetch::spawn);
            let bodies = body_pool(events.clone(), wake.clone(), store.clone());
            let lists = list_pool(events.clone(), wake.clone(), store.clone());
            let saved = accounts::load();
            let worker = Worker {
                events,
                wake,
                accounts: Vec::new(),
                saved,
                images,
                bodies,
                lists,
                prefetch,
                storage: Storage::default(),
                stale_mailboxes: Vec::new(),
                outbox: Vec::new(),
                me,
                connecting: HashSet::new(),
                deferred: Vec::new(),
                store,
                fingerprints: HashMap::new(),
                refreshes: 0,
            };
            // Secrets still in accounts.json move to the keyring.
            if worker.saved.iter().any(|s| !s.in_keyring && s.username.is_some()) {
                worker.persist();
            }
            worker.run(cmds)
        })
        .expect("spawn the JMAP thread");
}

struct Worker {
    events: Sender<Event>,
    wake: Waker,
    accounts: Vec<Account>,
    saved: Vec<Saved>,
    images: Sender<ImageJob>,
    bodies: Sender<BodyJob>,
    lists: Sender<ListJob>,
    prefetch: Option<Sender<crate::prefetch::Job>>,
    storage: Storage,
    /// Accounts shown with their kept mailboxes, to check once what waited for them has run.
    stale_mailboxes: Vec<String>,
    /// Actions waiting for the network while storage is off: (account, op, tries).
    outbox: Vec<(String, Op, i64)>,
    me: Sender<Cmd>,
    /// Accounts still connecting, and commands waiting for them.
    connecting: HashSet<String>,
    deferred: Vec<Cmd>,
    store: Option<Shared>,
    /// What each account looked like at the last refresh (JMAP state, IMAP STATUS), to skip
    /// reloading what hasn't changed.
    fingerprints: HashMap<String, String>,
    refreshes: u32,
}

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn account(&mut self, key: &str) -> Result<&mut Account, String> {
        self.accounts.iter_mut().find(|a| a.client.username() == key).ok_or_else(|| format!("No account {key}"))
    }

    fn persist(&self) {
        if let Err(e) = accounts::save(&self.saved) {
            self.send(Event::Error(format!("Couldn't save accounts: {e}")));
        }
    }

    fn reload_mailboxes(&mut self, key: &str) -> Result<(), String> {
        let a = self.account(key)?;
        let list = a.client.mailboxes()?;
        a.mailboxes = list.clone();
        if let Some(s) = &self.store {
            s.save_mailboxes(key, &list);
        }
        self.send(Event::Mailboxes { account: key.to_owned(), list });
        Ok(())
    }

    /// Today's events, fetched on a thread of their own: a slow calendar (Google's takes a
    /// few round trips) holds up nothing else, and they're kept for the next launch.
    fn events_for(&self, a: &Account) {
        let Some(cal) = a.calendar.clone() else { return };
        let (account, events, wake, store) = (a.client.username().to_owned(), self.events.clone(), self.wake.clone(), self.store.clone());
        std::thread::Builder::new()
            .name("calendar".into())
            .spawn(move || {
                let event = match cal.today() {
                    Ok(list) => {
                        if let Some(s) = &store {
                            s.save_events(&account, &chrono::Local::now().date_naive().to_string(), &list);
                        }
                        Event::Events { account, events: list }
                    }
                    Err(e) => Event::Error(e),
                };
                let _ = events.send(event);
                wake();
            })
            .expect("spawn a calendar thread");
    }

    /// Address books, also on a thread of their own.
    fn contacts_for_account(&self, a: &Account) {
        let Some(book) = a.contacts.clone() else { return };
        let (account, events, wake, store) = (a.client.username().to_owned(), self.events.clone(), self.wake.clone(), self.store.clone());
        std::thread::Builder::new()
            .name("contacts".into())
            .spawn(move || {
                let event = match book.contacts() {
                    Ok(contacts) => {
                        if let Some(s) = &store {
                            s.save_contacts(&account, &contacts);
                        }
                        Event::Contacts { account, contacts }
                    }
                    Err(e) => Event::Error(e),
                };
                let _ = events.send(event);
                wake();
            })
            .expect("spawn a contacts thread");
    }

    fn list(&mut self, view: &View, search: &str, threads: bool) -> Result<(Vec<Summary>, Threads), String> {
        self.list_page(view, search, threads, PAGE)
    }

    fn list_page(&mut self, view: &View, search: &str, threads: bool, page: usize) -> Result<(Vec<Summary>, Threads), String> {
        match view {
            View::Mailbox { account, id } => listing(self.account(account)?, id, search, threads, page),
            View::AllInboxes => {
                let (mut rows, mut all) = (Vec::new(), Threads::new());
                for a in &mut self.accounts {
                    if let Some(inbox) = role(&a.mailboxes, "inbox") {
                        let (r, t) = listing(a, &inbox, search, threads, page)?;
                        rows.extend(r);
                        all.extend(t);
                    }
                }
                // RFC 3339 UTC timestamps sort as strings.
                rows.sort_by(|a, b| b.received_at.cmp(&a.received_at));
                rows.truncate(page);
                all.retain(|k, _| rows.iter().any(|m| &m.thread_key == k));
                Ok((rows, all))
            }
        }
    }

    /// Connects on a thread of its own, so accounts come up side by side; commands for the
    /// account wait until it's here.
    fn connect(&mut self, saved: Saved, save: bool) {
        if let Some(u) = &saved.username {
            self.connecting.insert(u.clone());
        }
        let me = self.me.clone();
        std::thread::Builder::new()
            .name("connect".into())
            .spawn(move || {
                let result = Backend::connect(&saved);
                let _ = me.send(Cmd::Attach { saved, save, result });
            })
            .expect("spawn a connect thread");
    }

    fn attach(&mut self, saved: Saved, save: bool, result: Result<Backend, String>) {
        if let Some(u) = &saved.username {
            self.connecting.remove(u);
        }
        let client = match result {
            Ok(c) => c,
            Err(error) => return self.send(Event::ConnectFailed { error }),
        };
        let key = client.username().to_owned();
        if self.accounts.iter().any(|a| a.client.username() == key) {
            return self.send(Event::ConnectFailed { error: format!("{key} is already added.") });
        }
        let same = |s: &Saved| match (&s.imap, &saved.imap) {
            (Some(a), Some(b)) => a.email == b.email,
            (None, None) => s.token == saved.token,
            _ => false,
        };
        let i = match self.saved.iter().position(same) {
            Some(i) => {
                if save {
                    // Re-adding replaces the old password or token.
                    let calendar = self.saved[i].calendar_password.take();
                    self.saved[i] = Saved { calendar_password: calendar, ..saved.clone() };
                }
                i
            }
            None => {
                self.saved.push(saved.clone());
                self.saved.len() - 1
            }
        };
        let changed = self.saved[i].username.as_deref() != Some(&key);
        self.saved[i].username = Some(key.clone());
        let kind = client.kind(&self.saved[i]);
        let own_calendar = self.saved[i].imap.as_ref().and_then(|c| c.calendar()).is_some();
        let calendar = calendar_for(&self.saved[i], kind);
        if save || changed {
            self.persist();
        }
        let (label, from_name, position) = (self.saved[i].label.clone(), self.saved[i].sender_name(), i);
        self.send(Event::Connected { account: key.clone(), calendar: calendar.is_some(), kind, label, from_name, position, own_calendar });
        let contacts = contacts_for(&self.saved[i]);
        // The kept mailboxes let its list load straight away; they're checked after that.
        let kept = self.store.as_ref().map(|s| s.mailboxes(&key)).unwrap_or_default();
        let stale = !kept.is_empty();
        self.accounts.push(Account { client, mailboxes: kept, calendar, contacts });
        let _ = self.lists.send((self.saved[i].clone(), Vec::new(), View::AllInboxes, String::new(), false));
        self.events_for(self.accounts.last().unwrap());
        self.contacts_for_account(self.accounts.last().unwrap());
        if stale {
            self.stale_mailboxes.push(key);
        } else if let Err(e) = self.reload_mailboxes(&key) {
            self.send(Event::Error(e));
        }
    }

    fn set_calendar(&mut self, key: &str, password: Option<String>) -> Result<(), String> {
        let cal = password.as_deref().map(|p| CalDav::new(key, p));
        let events = match &cal {
            Some(c) => Some(c.today()?),
            None => None,
        };
        let mut book = None;
        if let Some(entry) = self.saved.iter_mut().find(|s| s.username.as_deref() == Some(key)) {
            entry.calendar_password = password;
            book = contacts_for(entry);
        }
        self.persist();
        if let Some(a) = self.accounts.iter_mut().find(|a| a.client.username() == key) {
            a.calendar = cal;
            a.contacts = book;
        }
        if let Some(a) = self.accounts.iter().find(|a| a.client.username() == key) {
            self.contacts_for_account(a);
        }
        self.send(Event::CalendarSet { account: key.to_owned(), on: events.is_some() });
        self.send(Event::Events { account: key.to_owned(), events: events.unwrap_or_default() });
        Ok(())
    }

    fn set_calendar_enabled(&mut self, key: &str, on: bool) -> Result<(), String> {
        let Some(i) = self.saved.iter().position(|s| s.username.as_deref() == Some(key)) else { return Ok(()) };
        self.saved[i].calendar_off = !on;
        self.persist();
        let saved = self.saved[i].clone();
        let kind = self.account(key)?.client.kind(&saved);
        let cal = calendar_for(&saved, kind);
        let events = match &cal {
            Some(c) => c.today()?,
            None => Vec::new(),
        };
        let has = cal.is_some();
        self.account(key)?.calendar = cal;
        self.send(Event::CalendarSet { account: key.to_owned(), on: has });
        self.send(Event::Events { account: key.to_owned(), events });
        Ok(())
    }

    /// Checks for new mail. Each account first says whether anything changed (JMAP's state
    /// string, or IMAP STATUS of the inbox and the mailbox shown); only those that changed
    /// reload their mailboxes, and the list reloads only if one of its accounts changed.
    /// Checks for changes and reloads what changed. True when anything did, or this was a
    /// full refresh.
    fn refresh(&mut self, view: Option<View>, search: String, threads: bool) -> Result<bool, String> {
        self.replay_outbox();
        self.refreshes += 1;
        let full = self.refreshes.is_multiple_of(FULL_REFRESH_EVERY);
        let keys: Vec<String> = self.accounts.iter().map(|a| a.client.username().to_owned()).collect();
        let mut changed: Vec<String> = Vec::new();
        for key in keys {
            let watch: Vec<String> = {
                let a = self.account(&key)?;
                let mut w: Vec<String> = role(&a.mailboxes, "inbox").into_iter().collect();
                if let Some(View::Mailbox { account, id }) = &view
                    && *account == key
                    && !w.contains(id)
                {
                    w.push(id.clone());
                }
                w
            };
            let print = self.account(&key)?.client.fingerprint(&watch).ok();
            let same = print.is_some() && self.fingerprints.get(&key) == print.as_ref();
            if let Some(p) = print {
                self.fingerprints.insert(key.clone(), p);
            }
            if !same || full {
                self.reload_mailboxes(&key)?;
                changed.push(key);
            }
        }
        if let Some(view) = view {
            let affected = match &view {
                View::Mailbox { account, .. } => changed.contains(account),
                View::AllInboxes => !changed.is_empty(),
            };
            if affected || !search.is_empty() {
                let (messages, threads_found) = self.list(&view, &search, threads)?;
                self.keep_listing(&view, &search, threads, &messages, &threads_found);
                self.send(Event::List { view, search, messages, threads: threads_found });
            }
        }
        Ok(full || !changed.is_empty())
    }

    fn keep_listing(&self, view: &View, search: &str, threads: bool, rows: &[Summary], found: &Threads) {
        if let Some(s) = self.store.as_ref().filter(|_| search.is_empty()) {
            s.save_listing(view, threads, rows, found);
        }
    }

    /// Tries the actions queued while offline; the server's version wins once one has failed
    /// `MAX_TRIES` times.
    fn replay_outbox(&mut self) {
        for (account, op, tries) in std::mem::take(&mut self.outbox) {
            if self.connecting.contains(&account) {
                self.outbox.push((account, op, tries));
                continue;
            }
            let result = self.account(&account).and_then(|a| match &op {
                Op::Keyword { ids, keyword, on } => a.client.set_keyword(ids, keyword, *on),
                Op::Move { ids, from, to } => a.client.move_to(ids, from, to),
            });
            if result.is_err() && tries + 1 < MAX_TRIES {
                self.outbox.push((account, op, tries + 1));
            }
        }
        let Some(store) = self.store.clone() else { return };
        for (seq, account, op) in store.queued() {
            if self.connecting.contains(&account) {
                continue;
            }
            let Some(op) = serde_json::from_str::<Op>(&op).ok() else {
                store.done(seq);
                continue;
            };
            let result = self.account(&account).and_then(|a| match &op {
                Op::Keyword { ids, keyword, on } => a.client.set_keyword(ids, keyword, *on),
                Op::Move { ids, from, to } => a.client.move_to(ids, from, to),
            });
            if result.is_ok() || store.tried(seq).is_some_and(|n| n >= MAX_TRIES) {
                store.done(seq);
            }
        }
    }

    /// Keeps a failed action to try again on the next refresh.
    /// In the store when it's on (so a restart keeps it), else in memory.
    fn queue(&mut self, account: &str, op: Op) {
        match self.store.as_ref().filter(|s| s.is_enabled()) {
            Some(s) if s.queue(account, &serde_json::to_string(&op).unwrap_or_default()) => {}
            _ => self.outbox.push((account.to_owned(), op, 0)),
        }
    }

    /// Tells the prefetcher mail may have changed.
    fn kick(&self) {
        if let Some(p) = self.prefetch.as_ref().filter(|_| self.storage.on) {
            let _ = p.send(crate::prefetch::Job::Kick(self.saved.clone()));
        }
    }

    /// An attachment's bytes: kept ones from the store, else the server (kept after, if on).
    fn attachment(&mut self, account: &str, id: &str, file: &crate::jmap::FileInfo) -> Result<Vec<u8>, String> {
        let uid = crate::jmap::uid(account, id);
        if let Some(data) = self.store.as_ref().and_then(|s| s.file(&uid, &file.part)) {
            return Ok(data);
        }
        let data = self.account(account)?.client.attachment(id, file)?;
        if let Some(s) = &self.store {
            s.save_file(&uid, &file.part, &data);
        }
        Ok(data)
    }

    /// Whether `cmd` must wait for an account that's still connecting.
    fn waits(&self, cmd: &Cmd) -> bool {
        let busy = |a: &str| self.connecting.contains(a);
        match cmd {
            Cmd::LoadList { view: View::Mailbox { account, .. }, .. }
            | Cmd::SetKeyword { account, .. }
            | Cmd::Move { account, .. }
            | Cmd::Send { account, .. }
            | Cmd::FetchImage { account, .. }
            | Cmd::Attachment { account, .. }
            | Cmd::SetCalendar { account, .. }
            | Cmd::SetCalendarEnabled { account, .. } => busy(account),
            Cmd::LoadList { view: View::AllInboxes, .. } | Cmd::Refresh { .. } | Cmd::LoadEvents | Cmd::LoadContacts => {
                !self.connecting.is_empty()
            }
            _ => false,
        }
    }

    fn run(mut self, cmds: Receiver<Cmd>) {
        let mut queue = std::collections::VecDeque::new();
        loop {
            queue.extend(cmds.try_iter());
            let cmd = match queue.pop_front() {
                Some(c) => c,
                None => match cmds.recv() {
                    Ok(c) => c,
                    Err(_) => return,
                },
            };
            // Clicking through mailboxes asks for each one's list; only the last is shown.
            if matches!(cmd, Cmd::LoadList { .. }) {
                queue.extend(cmds.try_iter());
                if queue.iter().any(|c| matches!(c, Cmd::LoadList { .. })) {
                    continue;
                }
            }
            self.handle(cmd);
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        if self.waits(&cmd) {
            self.deferred.push(cmd);
            return;
        }
        let result = match cmd {
                Cmd::Attach { saved, save, result } => {
                    // Sign in the lane for opened messages now, not on the first click.
                    let key = saved.username.clone().unwrap_or_default();
                    let warm = result.is_ok().then(|| saved.clone());
                    self.attach(saved, save, result);
                    if let Some(saved) = warm {
                        let _ = self.bodies.send((saved, key, String::new(), true));
                    }
                    self.replay_outbox();
                    if self.connecting.is_empty() {
                        self.kick();
                    }
                    // What waited for it goes now (or waits on for others still connecting).
                    for c in std::mem::take(&mut self.deferred) {
                        self.handle(c);
                    }
                    for key in std::mem::take(&mut self.stale_mailboxes) {
                        if let Err(e) = self.reload_mailboxes(&key) {
                            self.send(Event::Error(e));
                        }
                    }
                    Ok(())
                }
                Cmd::Connect { account, save } => {
                    self.connect(account, save);
                    Ok(())
                }
                Cmd::Remove { account } => {
                    if let Some(s) = &self.store {
                        s.forget(&account);
                    }
                    self.accounts.retain(|a| a.client.username() != account);
                    for s in self.saved.iter().filter(|s| s.username.as_deref() == Some(&account)) {
                        s.forget_secrets();
                    }
                    self.saved.retain(|s| s.username.as_deref() != Some(&account));
                    self.persist();
                    Ok(())
                }
                Cmd::SetCalendar { account, password } => self.set_calendar(&account, password),
                Cmd::SetLabel { account, label } => {
                    if let Some(s) = self.saved.iter_mut().find(|s| s.username.as_deref() == Some(&account)) {
                        s.label = Some(label.trim().to_owned()).filter(|l| !l.is_empty());
                    }
                    self.persist();
                    Ok(())
                }
                Cmd::SetFromName { account, name } => {
                    if let Some(s) = self.saved.iter_mut().find(|s| s.username.as_deref() == Some(&account)) {
                        s.from_name = Some(name.trim().to_owned()).filter(|n| !n.is_empty());
                    }
                    self.persist();
                    Ok(())
                }
                Cmd::Reorder { accounts } => {
                    let at = |s: &Saved| s.username.as_ref().and_then(|u| accounts.iter().position(|a| a == u)).unwrap_or(usize::MAX);
                    self.saved.sort_by_key(at);
                    self.persist();
                    Ok(())
                }
                Cmd::SetCalendarEnabled { account, on } => self.set_calendar_enabled(&account, on),
                Cmd::LoadEvents => {
                    for a in &self.accounts {
                        self.events_for(a);
                    }
                    Ok(())
                }
                Cmd::LoadContacts => {
                    for a in &self.accounts {
                        self.contacts_for_account(a);
                    }
                    Ok(())
                }
                Cmd::Refresh { view, search, threads } => {
                    let r = self.refresh(view, search, threads);
                    self.send(Event::Refreshed);
                    // The prefetcher lists every mailbox of every account: only when something
                    // changed, or on the full refresh that catches mail filed into folders the
                    // quick check doesn't watch.
                    if r.as_ref().is_ok_and(|changed| *changed) {
                        self.kick();
                    }
                    r.map(drop)
                }
                Cmd::LoadList { view: view @ View::Mailbox { .. }, search, threads } => {
                    let View::Mailbox { account, .. } = &view else { unreachable!("matched above") };
                    let saved = self.saved.iter().find(|s| s.username.as_deref() == Some(account)).cloned();
                    match (saved, self.account(account).map(|a| a.mailboxes.clone())) {
                        (Some(saved), Ok(mailboxes)) => {
                            let _ = self.lists.send((saved, mailboxes, view, search, threads));
                            Ok(())
                        }
                        (_, Err(e)) => Err(e),
                        (None, _) => Err(format!("No account {account}")),
                    }
                }
                Cmd::LoadList { view, search, threads } => (|| {
                    // IMAP fetches a page in a round trip per few dozen messages; show the first
                    // ones while the rest come.
                    let imap = match &view {
                        View::Mailbox { account, .. } => self.account(account).is_ok_and(|a| a.client.is_imap()),
                        View::AllInboxes => self.accounts.iter().any(|a| a.client.is_imap()),
                    };
                    if imap {
                        let (messages, found) = self.list_page(&view, &search, threads, QUICK_PAGE)?;
                        self.send(Event::List { view: view.clone(), search: search.clone(), messages, threads: found });
                    }
                    let (messages, found) = self.list(&view, &search, threads)?;
                    self.keep_listing(&view, &search, threads, &messages, &found);
                    self.send(Event::List { view, search, messages, threads: found });
                    Ok(())
                })(),
                Cmd::LoadBody { account, id, opened } => {
                    match self.saved.iter().find(|s| s.username.as_deref() == Some(&account)) {
                        Some(saved) => {
                            let _ = self.bodies.send((saved.clone(), account, id, opened));
                        }
                        None => self.send(Event::BodyFailed {
                            uid: crate::jmap::uid(&account, &id),
                            error: format!("No account {account}"),
                        }),
                    }
                    Ok(())
                }
                Cmd::FetchImage { account, uid, src } => {
                    // Inline images count as the message; remote ones are kept only if asked.
                    let keep = src.starts_with("blob:") || self.storage.remote_images;
                    self.account(&account).map(|a| a.client.downloads()).map(|d| {
                        let _ = self.images.send((d, src, uid, keep));
                    })
                }
                Cmd::Remember { key, value } => {
                    if let Some(store) = &self.store {
                        store.set(&key, &value);
                    }
                    Ok(())
                }
                Cmd::Keep(messages) => {
                    if let Some(store) = &self.store {
                        store.put_messages(&messages);
                    }
                    Ok(())
                }
                Cmd::ClearStore { on } => {
                    if let Some(store) = self.store.clone() {
                        store.set_enabled(false);
                        let queued = store.clear();
                        store.set_enabled(on);
                        for (account, op) in queued {
                            if let Ok(op) = serde_json::from_str::<Op>(&op) {
                                self.queue(&account, op);
                            }
                        }
                    }
                    Ok(())
                }
                Cmd::Storage(storage) => {
                    let was_on = self.storage.on;
                    self.storage = storage.clone();
                    if let Some(p) = &self.prefetch {
                        let _ = p.send(crate::prefetch::Job::Settings(storage));
                    }
                    let _ = was_on;
                    if self.storage.on && self.connecting.is_empty() {
                        self.kick();
                    }
                    Ok(())
                }
                Cmd::Attachment { account, id, file, save } => self.attachment(&account, &id, &file).and_then(|data| {
                    // Opened when it wasn't asked to be saved somewhere.
                    let open = save.is_none();
                    let path = match save {
                        Some(p) => p,
                        None => {
                            // A folder per attachment, so two "invoice.pdf"s, in one message
                            // or two, don't clash.
                            let dir = dirs::cache_dir()
                                .unwrap_or_else(std::env::temp_dir)
                                .join("rune-mail/open")
                                .join(format!("{:016x}", fxhash(&format!("{}/{}", crate::jmap::uid(&account, &id), file.part))));
                            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                            dir.join(safe_name(&file.name))
                        }
                    };
                    std::fs::write(&path, &data).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
                    if open {
                        std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| format!("Couldn't open it: {e}"))?;
                    }
                    Ok(())
                }),
                // Unread counts change with $seen and with moves.
                Cmd::SetKeyword { account, ids, keyword, on } => {
                    match self.account(&account).and_then(|a| a.client.set_keyword(&ids, keyword, on)) {
                        Ok(()) => self.reload_mailboxes(&account),
                        Err(e) => {
                            // Kept, and tried again when the network's back.
                            self.queue(&account, Op::Keyword { ids, keyword: keyword.to_owned(), on });
                            Err(format!("{e} (will retry)"))
                        }
                    }
                }
                Cmd::Move { account, ids, from, to } => {
                    match self.account(&account).and_then(|a| a.client.move_to(&ids, &from, &to)) {
                        Ok(()) => self.reload_mailboxes(&account),
                        Err(e) => {
                            self.queue(&account, Op::Move { ids, from, to });
                            Err(format!("{e} (will retry)"))
                        }
                    }
                }
                Cmd::Send { id, account, mut mail } => {
                    if mail.from_name.is_none() {
                        mail.from_name = self.saved.iter().find(|s| s.username.as_deref() == Some(&account)).and_then(Saved::sender_name);
                    }
                    match self.account(&account).and_then(|a| a.client.send(&mail, &a.mailboxes)) {
                        Ok(note) => {
                            self.send(Event::Sent { id });
                            if let Some(note) = note {
                                self.send(Event::Error(note));
                            }
                        }
                        Err(error) => self.send(Event::SendFailed { id, error }),
                    }
                    Ok(())
                }
                Cmd::Unsubscribe { uid, url } => {
                    crate::privacy::one_click(&url).map(|()| self.send(Event::Unsubscribed { uid }))
                }
        };
        if let Err(e) = result {
            self.send(Event::Error(e));
        }
    }
}

/// The calendar an account shows: a Fastmail JMAP account's separate app password, or an
/// IMAP account's own password where it opens one (Fastmail, Gmail), unless switched off.
fn calendar_for(saved: &Saved, kind: Kind) -> Option<CalDav> {
    let user = saved.username.as_deref()?;
    match &saved.imap {
        Some(cfg) if !saved.calendar_off => cfg.calendar().map(|p| CalDav::with(p, &cfg.login, &cfg.password)),
        Some(_) => None,
        None => saved.calendar_password.as_deref().filter(|_| kind == Kind::Fastmail).map(|p| CalDav::new(user, p)),
    }
}

/// The address book an account can read: Fastmail's CardDAV, with an IMAP account's own
/// app password or a Fastmail JMAP account's calendar app password.
fn contacts_for(saved: &Saved) -> Option<CardDav> {
    match &saved.imap {
        Some(cfg) if cfg.calendar() == Some(crate::caldav::Provider::Fastmail) => Some(CardDav::new(&cfg.login, &cfg.password)),
        Some(_) => None,
        None => Some(CardDav::new(saved.username.as_deref()?, saved.calendar_password.as_deref()?)),
    }
}

/// A file name without folders or anything a file system won't take.
fn safe_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    let n = n.trim_start_matches('.').trim();
    if n.is_empty() { "attachment".into() } else { n.into() }
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn role(mailboxes: &[Mailbox], role: &str) -> Option<String> {
    mailboxes.iter().find(|m| m.role.as_deref() == Some(role)).map(|m| m.id.clone())
}

/// A mailbox's rows and conversations. Conversations leave out messages in Trash and Junk
/// unless that's the mailbox shown.
fn listing(a: &mut Account, mailbox: &str, search: &str, threads: bool, page: usize) -> Result<(Vec<Summary>, Threads), String> {
    list_from(&mut a.client, &a.mailboxes, mailbox, search, threads, page)
}

/// A mailbox's newest `page` rows, and their conversations when `threads`.
fn list_from(
    client: &mut Backend,
    mailboxes: &[Mailbox],
    mailbox: &str,
    search: &str,
    threads: bool,
    page: usize,
) -> Result<(Vec<Summary>, Threads), String> {
    if !threads {
        let rows = client.messages(mailbox, search, page)?;
        let threads = rows.iter().map(|m| (m.thread_key.clone(), vec![m.clone()])).collect();
        return Ok((rows, threads));
    }
    let (newest, all) = client.conversations(mailbox, search, page)?;
    let hidden: Vec<String> =
        ["trash", "junk"].iter().filter_map(|r| role(mailboxes, r)).filter(|id| id != mailbox).collect();
    let mut by_thread = Threads::new();
    for mut m in all {
        m.thread_key = crate::jmap::uid(client.username(), &m.thread_id);
        let shown = newest.contains(&m.id) || !m.mailbox_ids.iter().any(|b| hidden.contains(b));
        if shown {
            by_thread.entry(m.thread_key.clone()).or_default().push(m);
        }
    }
    for list in by_thread.values_mut() {
        list.sort_by(|x, y| x.received_at.cmp(&y.received_at));
    }
    let by_id: HashMap<&str, &Summary> = by_thread.values().flatten().map(|m| (m.id.as_str(), m)).collect();
    let rows = newest.iter().filter_map(|id| by_id.get(id.as_str()).map(|m| (*m).clone())).collect();
    Ok((rows, by_thread))
}
