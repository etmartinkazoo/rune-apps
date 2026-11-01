//! App state, worker events and navigation. Message and compose actions are in `actions.rs`.
//!
//! Accounts are keyed by username. Messages carry their account and a `uid`
//! (`account:id`) that selection, marks and cached bodies use.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use chrono::{Local, NaiveDate};
use rune::{Image, ImageCache, TextEdit, Tx, Waker};

use crate::accounts::{ImapConfig, Saved};
use crate::backend::Kind;
use crate::caldav::Event as CalEvent;
use crate::contacts::Contact;
use crate::jmap::{Body, Mailbox, Summary};
use crate::settings::Settings;
use crate::worker::{self, Cmd, Event, Threads, View};

const STATUS_SECS: f64 = 6.0;
const MAX_BODIES: usize = 300;
/// How many of a list's newest conversations have their newest message fetched ahead.
const PREFETCH: usize = 12;
const EVENTS_EVERY_SECS: f64 = 300.0;
const CONTACTS_EVERY_SECS: f64 = 1800.0;
const MAX_SUGGESTIONS: usize = 6;

pub const FAVORITES: [&str; 6] = ["inbox", "drafts", "sent", "archive", "junk", "trash"];

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    SignIn,
    Connecting,
    Ready,
}

/// Which side the keyboard is in: the message list, or the open conversation (`l` goes in,
/// `h` comes back; `j`/`k` move in whichever is active).
#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    List,
    Thread,
}

/// The sheet over the main window, if any.
#[derive(Clone, Copy, PartialEq)]
pub enum Sheet {
    None,
    Compose,
    Settings,
    Shortcuts,
    AddAccount,
}

/// The page Settings shows, picked in its sidebar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsPane {
    General,
    Viewing,
    Accounts,
    Privacy,
    Storage,
    Ai,
    Keyboard,
}

pub struct Account {
    pub key: String,
    pub kind: Kind,
    /// Your name for it in the sidebar, if any.
    pub label: Option<String>,
    /// Settings' field for the label.
    pub label_edit: TextEdit,
    /// Settings' field for the name on mail it sends (empty: the server's or the sign-in's).
    pub from_name_edit: TextEdit,
    /// Its place in the saved order, which is the sidebar's.
    pub position: usize,
    /// Its password opens a calendar (an IMAP account on Fastmail or Gmail).
    pub own_calendar: bool,
    /// Connected this run (rather than shown from the store while it connects).
    pub live: bool,
    pub mailboxes: Vec<Mailbox>,
    pub calendar: bool,
    pub events: Vec<CalEvent>,
    pub contacts: Vec<Contact>,
}

/// An address offered while typing To or Cc.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub email: String,
    /// From an address book, rather than only seen in mail.
    pub contact: bool,
}

/// Which compose field suggestions are for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AddressField {
    To,
    Cc,
}

impl Account {
    /// What the sidebar calls it: its label, else its address.
    pub fn name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.key)
    }
}

/// One row of the sidebar.
pub struct Item {
    pub view: View,
    pub name: String,
    pub role: Option<String>,
    pub depth: usize,
    pub unread: u32,
}

/// What a list of suggestions was worked out for: the text, the list's version, and how many
/// contacts there were.
type SuggestedFor = (String, u64, usize);

pub struct Section {
    /// What folding it is saved under: "favorites", or the account's address.
    pub id: String,
    pub title: String,
    pub items: Vec<Item>,
}

/// Which kind of account the Add Account form is for.
#[derive(Clone, Copy, PartialEq)]
pub enum AddKind {
    Fastmail,
    Gmail,
    Other,
}

/// The Add Account form. Fastmail uses `App::token`.
pub struct AddForm {
    pub kind: AddKind,
    pub name: TextEdit,
    pub email: TextEdit,
    pub password: TextEdit,
    pub login: TextEdit,
    pub imap_host: TextEdit,
    pub imap_port: TextEdit,
    pub smtp_host: TextEdit,
    pub smtp_port: TextEdit,
    pub starttls: bool,
    pub save_sent: bool,
}

impl AddForm {
    pub fn new(kind: AddKind) -> AddForm {
        let mut f = AddForm {
            kind,
            name: TextEdit::new(""),
            email: TextEdit::new(""),
            password: TextEdit::new("").secure(),
            login: TextEdit::new(""),
            imap_host: TextEdit::new(""),
            imap_port: TextEdit::new("993"),
            smtp_host: TextEdit::new(""),
            smtp_port: TextEdit::new("465"),
            starttls: false,
            save_sent: true,
        };
        f.email.focused = kind != AddKind::Fastmail;
        f
    }

    /// The account this form describes, or what's missing.
    pub fn account(&self) -> Result<Saved, String> {
        let email = self.email.text.trim().to_owned();
        let password = self.password.text.trim().to_owned();
        let name = Some(self.name.text.trim().to_owned()).filter(|n| !n.is_empty());
        if !email.contains('@') {
            return Err("Enter the email address.".into());
        }
        if password.is_empty() {
            return Err(if self.kind == AddKind::Gmail { "Paste the app password." } else { "Enter the password." }.into());
        }
        let cfg = match self.kind {
            AddKind::Gmail => ImapConfig::gmail(&email, name, &password),
            _ => {
                let port = |t: &TextEdit, what: &str| t.text.trim().parse::<u16>().map_err(|_| format!("{what} port must be a number."));
                let (imap_host, smtp_host) = (self.imap_host.text.trim().to_owned(), self.smtp_host.text.trim().to_owned());
                if imap_host.is_empty() || smtp_host.is_empty() {
                    return Err("Enter the IMAP and SMTP servers.".into());
                }
                let login = Some(self.login.text.trim().to_owned()).filter(|l| !l.is_empty()).unwrap_or(email.clone());
                ImapConfig {
                    gmail: false,
                    email,
                    name,
                    login,
                    password,
                    imap_host,
                    imap_port: port(&self.imap_port, "IMAP")?,
                    smtp_host,
                    smtp_port: port(&self.smtp_port, "SMTP")?,
                    smtp_starttls: self.starttls,
                    save_sent: self.save_sent,
                }
            }
        };
        Ok(Saved { imap: Some(cfg), ..Saved::default() })
    }
}

pub struct Compose {
    pub account: String,
    pub attachments: Vec<crate::attach::Attachment>,
    /// Files are being dragged over the sheet.
    pub drop_over: bool,
    pub sending: bool,
    pub to: TextEdit,
    pub cc: TextEdit,
    pub subject: TextEdit,
    pub body: TextEdit,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
}

impl Compose {
    pub fn empty(account: String) -> Compose {
        Compose {
            account,
            attachments: Vec::new(),
            drop_over: false,
            sending: false,
            to: TextEdit::new(""),
            cc: TextEdit::new(""),
            subject: TextEdit::new(""),
            body: TextEdit::new(""),
            in_reply_to: Vec::new(),
            references: Vec::new(),
        }
    }
}

pub struct App {
    pub phase: Phase,
    pub settings: Settings,
    pub sheet: Sheet,
    pub token: TextEdit,
    pub add: AddForm,
    pub signin_error: Option<String>,
    pub accounts: Vec<Account>,
    connecting: usize,
    pub view: Option<View>,
    /// One row per conversation: its newest message in the view.
    pub messages: Vec<Summary>,
    /// Every row's conversation, oldest first, by thread key. A message appears in both
    /// lists; `update_message` changes both copies.
    pub threads: Threads,
    /// Messages open in the reader (the rest of a conversation shows one line each).
    pub expanded: HashSet<String>,
    /// The highlighted message of the open conversation: `h`/`l` move it, Space opens or
    /// closes it, and replies answer it.
    pub thread_focus: Option<String>,
    pub pane: Pane,
    pub list_version: u64,
    pub list_loading: bool,
    /// The message the reader shows (Apple Mail's selection, vim's cursor), by uid.
    pub selected: Option<String>,
    /// Messages marked with `x`, Cmd-click or Shift-click; actions apply to these when any.
    pub marked: HashSet<String>,
    pub bodies: HashMap<String, Body>,
    /// Bodies in the order they arrived, oldest first, for evicting.
    body_order: VecDeque<String>,
    /// Bodies asked for and not yet back.
    /// Bodies asked for, and whether as opened (true) or ahead (false).
    requested_bodies: HashMap<String, bool>,
    /// Bodies that failed to load, with why.
    pub body_failed: HashMap<String, String>,
    /// A check for new mail is running.
    pub refreshing: bool,
    /// Mail HTML laid out and painted off this thread (`page.rs`).
    pub pages: crate::page::Pages,
    /// The reader column's width in points, as last drawn: what pages are laid out at.
    pub page_width: f32,
    /// Device pixels per point and whether the system is dark, as the last view ran: the
    /// view knows them, and pages are asked for outside it.
    pub view_env: std::cell::Cell<(f32, bool)>,
    /// Messages whose remote images the user chose to load.
    pub remote_allowed: HashSet<String>,
    /// Messages shown on a white page while the app is dark.
    pub on_white: HashSet<String>,
    pub search: TextEdit,
    pub searched: String,
    /// The store's answers to the current search.
    local_hits: Vec<Summary>,
    /// Mail kept on this computer (None when Settings turn it off or it can't open).
    pub store: Option<crate::store::Shared>,
    /// What the store holds, for Settings' meter, and when it was measured.
    pub storage_usage: Option<(crate::store::Usage, f64)>,
    pub settings_pane: SettingsPane,
    /// Settings asked to turn storage off: the second press confirms.
    pub confirm_storage_off: bool,
    pub compose: Compose,
    pub status: Option<(String, f64)>,
    /// A vim `g` waiting for its second key.
    pub pending_g: bool,
    /// Sends waiting out the undo delay, and sends with the server awaiting its answer.
    pub waiting_sends: Vec<crate::undo::PendingSend>,
    pub in_flight_sends: Vec<crate::undo::PendingSend>,
    pub(crate) next_send: u64,
    thumbs_waiting: bool,
    /// The last action, while its Undo is on offer, and one recorded without a Tx to hand
    /// (offered on the next poll).
    pub undo: Option<crate::undo::Undo>,
    pub(crate) pending_undo: Option<(String, crate::undo::UndoKind)>,
    /// Messages Copy for AI is waiting on before it copies.
    pub ai_pending: Option<Vec<String>>,
    /// Settings' field for the Copy for AI prompt.
    pub ai_prompt_edit: TextEdit,
    /// When the settings, changed by typing, are saved: a moment after the last key, rather
    /// than a file written per key.
    save_settings_at: Option<f64>,
    /// A row slid by a two-finger swipe, a row being dragged, and where things are drawn.
    pub swipe: Option<crate::gestures::Swipe>,
    pub dragging: Option<crate::gestures::Dragging>,
    pub rects: crate::gestures::Rects,
    /// The compose From select's list is open (where there's no native pop-up menu).
    pub from_open: bool,
    /// The highlighted address suggestion, and whether Esc put the list away.
    pub suggest_index: usize,
    pub suggest_hidden: bool,
    /// The last suggestions worked out, and what they were for (the text, the list's version
    /// and how many contacts there were): the view asks every frame, and working them out
    /// goes through every contact and message.
    suggested: std::cell::RefCell<Option<(SuggestedFor, Vec<Suggestion>)>>,
    contacts_at: f64,
    /// The account whose calendar password Settings is asking for, and its field.
    pub calendar_edit: Option<String>,
    pub calendar_field: TextEdit,
    /// The message whose Unsubscribe was asked for once and now asks to confirm.
    pub unsubscribe_confirm: Option<String>,
    /// Messages whose lists were unsubscribed from (or are being).
    pub unsubscribed: HashSet<String>,
    /// The account whose Remove button was pressed once and now asks to confirm.
    pub confirm_remove: Option<String>,
    requested_images: HashSet<String>,
    image_cache: Option<ImageCache>,
    clock: f64,
    next_refresh: Option<f64>,
    events_day: NaiveDate,
    events_at: f64,
    cmds: Sender<Cmd>,
    events: Receiver<Event>,
    worker: Option<(Receiver<Cmd>, Sender<Event>)>,
}

/// The window has no close hook here, but the state goes when it closes: typing saved a moment
/// later mustn't be lost by quitting within that moment.
impl Drop for App {
    fn drop(&mut self) {
        if self.save_settings_at.take().is_some() {
            let _ = self.settings.save();
        }
    }
}

pub fn init() -> App {
    let (cmds, cmd_rx) = channel();
    let (event_tx, events) = channel();
    let tokens = worker::saved_accounts();
    for account in &tokens {
        let _ = cmds.send(Cmd::Connect { account: account.clone(), save: false });
    }
    let mut s = App {
        phase: if tokens.is_empty() { Phase::SignIn } else { Phase::Connecting },
        settings: Settings::load(),
        sheet: Sheet::None,
        token: token_field(),
        add: AddForm::new(AddKind::Fastmail),
        signin_error: crate::accounts::load_problem(),
        accounts: Vec::new(),
        connecting: tokens.len(),
        view: None,
        messages: Vec::new(),
        threads: Threads::new(),
        expanded: HashSet::new(),
        thread_focus: None,
        pane: Pane::List,
        list_version: 0,
        list_loading: false,
        selected: None,
        marked: HashSet::new(),
        bodies: HashMap::new(),
        body_order: VecDeque::new(),
        requested_bodies: HashMap::new(),
        body_failed: HashMap::new(),
        refreshing: false,
        pages: crate::page::Pages::new(),
        page_width: 640.0,
        view_env: std::cell::Cell::new((1.0, false)),
        remote_allowed: HashSet::new(),
        on_white: HashSet::new(),
        search: TextEdit::new(""),
        searched: String::new(),
        local_hits: Vec::new(),
        store: None,
        storage_usage: None,
        settings_pane: SettingsPane::General,
        confirm_storage_off: false,
        compose: Compose::empty(String::new()),
        status: None,
        pending_g: false,
        from_open: false,
        waiting_sends: Vec::new(),
        in_flight_sends: Vec::new(),
        next_send: 0,
        thumbs_waiting: false,
        undo: None,
        pending_undo: None,
        ai_pending: None,
        ai_prompt_edit: TextEdit::new(""),
        save_settings_at: None,
        swipe: None,
        dragging: None,
        rects: Default::default(),
        suggest_index: 0,
        suggest_hidden: false,
        suggested: Default::default(),
        contacts_at: 0.0,
        calendar_edit: None,
        calendar_field: TextEdit::new("").secure(),
        confirm_remove: None,
        unsubscribe_confirm: None,
        unsubscribed: HashSet::new(),
        requested_images: HashSet::new(),
        image_cache: None,
        clock: 0.0,
        next_refresh: None,
        events_day: Local::now().date_naive(),
        events_at: 0.0,
        cmds,
        events,
        worker: Some((cmd_rx, event_tx)),
    };
    let settings = s.settings.clone();
    s.ai_prompt_edit = TextEdit::new(&settings.ai_prompt);
    s.store = crate::store::Store::open();
    if let Some(store) = &s.store {
        store.set_enabled(settings.storage.on);
        if !settings.storage.on && settings.unreadable.is_none() {
            s.send(Cmd::ClearStore { on: false });
        }
    }
    if settings.storage.on {
        s.restore(&tokens);
    }
    s
}

fn token_field() -> TextEdit {
    let mut token = TextEdit::new("").secure();
    token.focused = true;
    token
}

pub fn setup(s: &mut App, wake: Waker) {
    s.image_cache = Some(ImageCache::new(wake.clone()));
    s.pages.set_waker(wake.clone());
    // A scene (applied before setup) may already show a message whose images need the cache.
    if let Some(uid) = s.selected.clone() {
        s.request_images(&uid);
    }
    if let Some((cmds, events)) = s.worker.take() {
        worker::spawn(cmds, s.cmds.clone(), events, wake, s.store.clone());
        s.send(Cmd::Storage(s.settings.storage.clone()));
    }
}

pub fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = false;
    while let Ok(event) = s.events.try_recv() {
        s.apply(event, tx);
        changed = true;
    }
    changed |= s.poll_pages(tx);
    // Attachment thumbnails decode in the background too; redraw the strip when they land.
    let thumbs = s.compose.attachments.iter().any(|a| a.thumb.as_ref().is_some_and(|t| !t.is_ready() && t.error().is_none()));
    if thumbs {
        tx.wake_after(Duration::from_millis(100));
    }
    changed |= std::mem::replace(&mut s.thumbs_waiting, thumbs) && !thumbs;
    changed |= s.settle_quiet_swipe(tx);
    changed |= s.tick_sends(tx);
    changed |= s.expire_undo();
    changed |= s.tick_storage(tx);
    if let Some((label, kind)) = s.pending_undo.take() {
        s.offer_undo(label, kind, tx);
        changed = true;
    }
    s.save_typed_settings(tx);
    if let Some(why) = s.settings.unreadable.take() {
        s.say(format!("Settings couldn't be read, so they're the defaults: {why}"), tx);
        changed = true;
    }
    if s.status.as_ref().is_some_and(|(_, until)| clock >= *until) {
        s.status = None;
        changed = true;
    }
    if s.phase == Phase::Ready && s.next_refresh.is_some_and(|at| clock >= at) {
        s.refresh();
        s.schedule_refresh(tx);
    }
    // A new day, or a while since the last look: fetch today's events again.
    let today = Local::now().date_naive();
    if s.phase == Phase::Ready
        && s.accounts.iter().any(|a| a.calendar)
        && (today != s.events_day || clock - s.events_at >= EVENTS_EVERY_SECS)
    {
        s.events_day = today;
        s.events_at = clock;
        s.send(Cmd::LoadEvents);
        tx.wake_after(Duration::from_secs_f64(EVENTS_EVERY_SECS));
        changed = true;
    }
    // Accounts load their contacts on connecting; after that, every half hour.
    if s.phase == Phase::Ready && s.contacts_at == 0.0 {
        s.contacts_at = clock.max(f64::MIN_POSITIVE);
        tx.wake_after(Duration::from_secs_f64(CONTACTS_EVERY_SECS));
    } else if s.phase == Phase::Ready && clock - s.contacts_at >= CONTACTS_EVERY_SECS {
        s.contacts_at = clock;
        s.send(Cmd::LoadContacts);
        tx.wake_after(Duration::from_secs_f64(CONTACTS_EVERY_SECS));
    }
    changed
}

impl App {
    /// Decodes an image (for attachment thumbnails).
    pub fn load_image(&self, bytes: Vec<u8>) -> Option<Image> {
        self.image_cache.as_ref().map(|c| c.load(bytes))
    }

    /// Seconds on the app clock, as of the last frame.
    /// Shows what the store kept of the saved accounts, before any of them connects: their
    /// mailboxes and contacts, and the list last looked at.
    fn restore(&mut self, saved: &[Saved]) {
        let Some(store) = self.store.clone() else { return };
        for (i, sv) in saved.iter().enumerate() {
            let Some(key) = sv.username.clone() else { continue };
            let kind = match &sv.imap {
                Some(c) if c.gmail => Kind::Gmail,
                Some(_) => Kind::Imap,
                None => Kind::Fastmail,
            };
            let own_calendar = sv.imap.as_ref().and_then(|c| c.calendar()).is_some();
            let calendar = match &sv.imap {
                Some(c) => c.calendar().is_some() && !sv.calendar_off,
                None => sv.calendar_password.is_some(),
            };
            self.accounts.push(Account {
                label_edit: TextEdit::new(sv.label.as_deref().unwrap_or("")),
                from_name_edit: TextEdit::new(sv.sender_name().unwrap_or_default()),
                label: sv.label.clone(),
                kind,
                position: i,
                own_calendar,
                live: false,
                mailboxes: store.mailboxes(&key),
                events: store.events(&key, &Local::now().date_naive().to_string()),
                calendar,
                contacts: store.contacts(&key),
                key,
            });
        }
        if self.accounts.is_empty() {
            return;
        }
        self.phase = Phase::Ready;
        let last = store.get("view").and_then(|v| serde_json::from_str::<View>(&v).ok()).filter(|v| match v {
            View::AllInboxes => self.accounts.len() > 1,
            View::Mailbox { account, .. } => self.account(account).is_some(),
        });
        match last {
            Some(v) => self.select_view(v),
            None => self.select_role("inbox"),
        }
    }

    pub fn now(&self) -> f64 {
        self.clock
    }

    pub(crate) fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }

    /// Drops the network thread before it starts, for headless demo scenes: they must never
    /// reach the accounts behind saved tokens.
    pub fn offline(&mut self) {
        self.worker = None;
        self.store = None;
        self.cmds = channel().0;
        self.connecting = 0;
    }

    pub fn schedule_refresh(&mut self, tx: &mut Tx) {
        let minutes = self.settings.refresh_minutes;
        self.next_refresh = (minutes > 0).then_some(self.clock + minutes as f64 * 60.0);
        if minutes > 0 {
            tx.wake_after(Duration::from_secs(minutes as u64 * 60));
        }
    }

    /// Checks every account for new mail and reloads the list.
    pub fn refresh(&mut self) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.send(Cmd::Refresh {
            view: self.view.clone(),
            search: self.searched.clone(),
            threads: self.settings.conversations,
        });
    }

    pub fn say(&mut self, text: impl Into<String>, tx: &mut Tx) {
        self.status = Some((text.into(), self.clock + STATUS_SECS));
        tx.wake_after(Duration::from_secs_f64(STATUS_SECS));
    }

    fn apply(&mut self, event: Event, tx: &mut Tx) {
        match event {
            Event::Connected { account, calendar, kind, label, from_name, position, own_calendar } => {
                self.connecting = self.connecting.saturating_sub(1);
                let first = !self.accounts.iter().any(|a| a.live);
                // Shown from the store already? Then it just comes alive.
                let restored = match self.accounts.iter_mut().find(|a| a.key == account) {
                    Some(a) => {
                        (a.kind, a.calendar, a.own_calendar, a.live) = (kind, calendar, own_calendar, true);
                        true
                    }
                    None => {
                        let label_edit = TextEdit::new(label.as_deref().unwrap_or(""));
                        let from_name_edit = TextEdit::new(from_name.as_deref().unwrap_or(""));
                        let at = self.accounts.iter().position(|a| a.position > position).unwrap_or(self.accounts.len());
                        self.accounts.insert(at, Account {
                            key: account,
                            kind,
                            label,
                            label_edit,
                            from_name_edit,
                            position,
                            own_calendar,
                            live: true,
                            mailboxes: Vec::new(),
                            calendar,
                            events: Vec::new(),
                            contacts: Vec::new(),
                        });
                        false
                    }
                };
                self.phase = Phase::Ready;
                self.signin_error = None;
                self.token = token_field();
                self.add = AddForm::new(AddKind::Fastmail);
                if self.sheet == Sheet::AddAccount {
                    self.sheet = Sheet::None;
                }
                if first {
                    self.schedule_refresh(tx);
                }
                if restored {
                    // Its list was asked for at launch and comes once it's connected.
                } else if self.view == Some(View::AllInboxes) {
                    self.reload_list();
                } else if self.current_mailbox().is_some_and(|m| m.role.as_deref() == Some("inbox")) {
                    // A second account arrived while showing the first's inbox: show them together.
                    self.select_view(View::AllInboxes);
                }
            }
            Event::ConnectFailed { error } => {
                self.connecting = self.connecting.saturating_sub(1);
                if self.accounts.is_empty() && self.connecting == 0 {
                    self.phase = Phase::SignIn;
                }
                self.signin_error = Some(error);
                self.token.focused = true;
            }
            Event::Mailboxes { account, list } => {
                let Some(a) = self.accounts.iter_mut().find(|a| a.key == account) else { return };
                a.mailboxes = list;
                if self.view.is_none() {
                    self.select_role("inbox");
                }
            }
            Event::List { view, search, mut messages, threads } => {
                if self.view.as_ref() == Some(&view) && search == self.searched {
                    if !search.is_empty() {
                        // Kept mail the server's search didn't return (it may not index it all).
                        let extra: Vec<Summary> =
                            self.local_hits.iter().filter(|h| !messages.iter().any(|m| m.uid == h.uid)).cloned().collect();
                        messages.extend(extra);
                        messages.sort_by(|a, b| b.received_at.cmp(&a.received_at));
                    }
                    let uids: HashSet<&str> = messages.iter().map(|m| m.uid.as_str()).collect();
                    self.marked.retain(|id| uids.contains(id.as_str()));
                    self.rects.rows.retain(|id, _| uids.contains(id.as_str()));
                    // A refresh can make another message the row for the open conversation.
                    if let Some(key) = self.selected_message().map(|m| m.thread_key.clone())
                        && let Some(row) = messages.iter().find(|m| m.thread_key == key)
                    {
                        self.selected = Some(row.uid.clone());
                    } else if self.selected.as_ref().is_some_and(|s| !uids.contains(s.as_str())) {
                        // Gone elsewhere (archived on another device): nothing stays open that
                        // keys and actions would still act on.
                        self.selected = None;
                        self.thread_focus = None;
                        self.expanded.clear();
                    }
                    self.messages = messages;
                    self.threads = threads;
                    self.prefetch();
                    self.list_version += 1;
                    self.list_loading = false;
                }
            }
            Event::Body { uid, body } => {
                self.requested_bodies.remove(&uid);
                self.body_failed.remove(&uid);
                self.evict_bodies();
                self.bodies.insert(uid.clone(), *body);
                self.body_order.push_back(uid.clone());
                // Images load only for a message on screen: a prefetched one stays unopened
                // as far as its sender can tell.
                if self.expanded.contains(&uid) || self.selected.as_ref() == Some(&uid) {
                    self.request_images(&uid);
                }
                self.finish_ai_copy(tx);
            }
            Event::BodyFailed { uid, error } => {
                self.requested_bodies.remove(&uid);
                self.body_failed.insert(uid, error);
                self.finish_ai_copy(tx);
            }
            Event::Refreshed => self.refreshing = false,
            Event::Image { src, bytes } => self.pages.image(src, bytes.into()),
            Event::Events { account, events } => {
                if let Some(a) = self.accounts.iter_mut().find(|a| a.key == account) {
                    a.events = events;
                }
            }
            Event::Contacts { account, contacts } => {
                if let Some(a) = self.accounts.iter_mut().find(|a| a.key == account) {
                    a.contacts = contacts;
                }
            }
            Event::CalendarSet { account, on } => {
                if let Some(a) = self.accounts.iter_mut().find(|a| a.key == account) {
                    a.calendar = on;
                }
                if self.calendar_edit.as_ref() == Some(&account) {
                    self.calendar_edit = None;
                    self.calendar_field = TextEdit::new("").secure();
                }
                let name = self.account(&account).map_or(account.clone(), |a| a.name().to_owned());
                self.say(if on { format!("Showing {name}'s calendar") } else { format!("{name}'s calendar is off") }, tx);
            }
            Event::Unsubscribed { uid } => {
                let list = self.bodies.get(&uid).and_then(|b| b.unsubscribe.as_ref()).and_then(|u| u.list.clone());
                self.say(match list {
                    Some(l) => format!("Unsubscribed from {l}"),
                    None => "Unsubscribed".to_owned(),
                }, tx);
            }
            Event::Sent { id } => {
                // Otherwise an unsubscribe email, which says so itself.
                if self.sent(id) {
                    self.say("Message sent", tx);
                }
            }
            Event::SendFailed { id, error } => {
                if self.send_failed(id) {
                    self.say(format!("Not sent: {error}"), tx);
                } else {
                    self.say(error, tx);
                }
            }
            Event::Error(e) => {
                if e.starts_with("Unsubscribing failed") {
                    self.unsubscribed.clear();
                }
                self.compose.sending = false;
                self.list_loading = false;
                self.say(e, tx);
            }
        }
    }

    /// Asks for the images of message `uid` that may load: its own inline ones always, remote
    /// ones when settings or the reader's banner allow. `data:` images need no asking.
    pub fn request_images(&mut self, uid: &str) {
        let Some(doc) = self.bodies.get(uid).and_then(|b| b.html.as_ref()) else { return };
        let srcs: Vec<String> = doc.images().into_iter().map(str::to_owned).collect();
        self.fetch_images(uid, srcs);
    }

    /// Fetches what of `srcs` (images message `uid` shows) its privacy rules let load.
    fn fetch_images(&mut self, uid: &str, srcs: Vec<String>) {
        if !self.settings.render_html {
            return;
        }
        let Some(body) = self.bodies.get(uid) else { return };
        let inline = body.inline.clone();
        let own = |src: &str| inline.as_ref().is_none_or(|own| own.iter().any(|s| s == src));
        let account = uid.split_once(':').map_or("", |(a, _)| a).to_owned();
        let remote_ok = self.settings.remote_images || self.remote_allowed.contains(uid);
        for src in srcs {
            if self.requested_images.contains(&src) {
                continue;
            }
            let remote = src.starts_with("http://") || src.starts_with("https://");
            let allowed = (src.starts_with("blob:") && own(&src))
                || (remote && remote_ok && crate::privacy::image_tracker(&src, None, None).is_none() && !font_or_style(&src));
            if allowed {
                self.send(Cmd::FetchImage { account: account.clone(), uid: uid.to_owned(), src: src.clone() });
                self.requested_images.insert(src);
            }
        }
    }

    /// Lays out the pages the reader shows or is likely to show next, at the reader's width and
    /// in its colours, and lets the rest go. Called every frame; asks only for what changed.
    fn ensure_pages(&mut self) {
        let (scale, system_dark) = self.view_env.get();
        let dark = self.settings.dark(system_dark);
        let palette = crate::ui::style::Palette::new(dark);
        let width = self.page_width.round().max(200.0) as u32;
        // What the reader shows: a lone message always, a conversation's open ones.
        let conversation = self.conversation();
        let single = conversation.len() == 1;
        let mut wanted: Vec<String> =
            conversation.iter().filter(|m| single || self.expanded.contains(&m.uid)).map(|m| m.uid.clone()).collect();
        // The rows either side, laid out ahead so opening them shows at once.
        if let Some(i) = self.selected.as_ref().and_then(|s| self.messages.iter().position(|m| &m.uid == s)) {
            for j in [i + 1, i.wrapping_sub(1)] {
                if let Some(m) = self.messages.get(j).and_then(|row| self.thread(row).last().map(|m| m.uid.clone())) {
                    wanted.push(m);
                }
            }
        }
        if !self.settings.render_html {
            wanted.clear();
        }
        for uid in &wanted {
            let Some(body) = self.bodies.get(uid) else { continue };
            let (Some(doc), Some(src)) = (body.html.as_ref(), body.html_src.as_ref()) else { continue };
            let white = self.on_white.contains(uid);
            let ink = (dark && !doc.colored && !white).then(|| (rgba8(palette.text), rgba8(palette.accent)));
            let look = crate::page::Look { width, scale, ink };
            self.pages.show(uid, src, look);
        }
        let keep: HashSet<&str> = wanted.iter().map(String::as_str).collect();
        self.pages.keep(&keep);
    }

    /// Takes in what the page thread finished. True when the view should run again.
    fn poll_pages(&mut self, tx: &mut Tx) -> bool {
        self.ensure_pages();
        let (changed, done) = self.pages.poll();
        for d in done {
            match d {
                crate::page::Done::Click { href } => self.open_link(&href, tx),
                crate::page::Done::Copied(text) => tx.copy(text),
                crate::page::Done::Wants { uid, srcs } => self.fetch_images(&uid, srcs),
                crate::page::Done::Laid { .. } | crate::page::Done::Hover { .. } | crate::page::Done::Failed { .. } => {}
            }
        }
        changed
    }

    /// A link in a message: mail to its address, anything else in the browser, without its
    /// tracking parameters.
    pub fn open_link(&mut self, href: &str, tx: &mut Tx) {
        if let Some(to) = href.strip_prefix("mailto:") {
            let to = to.split('?').next().unwrap_or_default();
            self.compose_to(&crate::ui::html_view::percent_decode(to));
        } else if let Err(e) = tx.open_url(&crate::privacy::clean_link(href)) {
            self.say(format!("Couldn't open the link: {e:?}"), tx);
        }
    }

    pub fn allow_remote(&mut self, uid: String) {
        self.remote_allowed.insert(uid.clone());
        self.request_images(&uid);
    }

    /// Shows a message on a white page in dark mode, or back in the app's colours.
    pub fn toggle_white(&mut self, uid: &str) {
        if !self.on_white.remove(uid) {
            self.on_white.insert(uid.to_owned());
        }
    }

    // Address suggestions

    /// The compose address field being typed in, if any.
    pub fn address_field(&self) -> Option<AddressField> {
        if self.sheet != Sheet::Compose {
            return None;
        }
        if self.compose.to.focused {
            Some(AddressField::To)
        } else if self.compose.cc.focused {
            Some(AddressField::Cc)
        } else {
            None
        }
    }

    pub fn address_edit(&mut self, field: AddressField) -> &mut TextEdit {
        match field {
            AddressField::To => &mut self.compose.to,
            AddressField::Cc => &mut self.compose.cc,
        }
    }

    /// Addresses matching the last, unfinished entry of `text`: address-book contacts
    /// first, then people from mail, best matches first.
    pub fn suggestions(&self, text: &str) -> Vec<Suggestion> {
        let contacts = self.accounts.iter().map(|a| a.contacts.len()).sum();
        let key = (text.to_owned(), self.list_version, contacts);
        if let Some((k, list)) = self.suggested.borrow().as_ref()
            && *k == key
        {
            return list.clone();
        }
        let list = self.work_out_suggestions(text);
        *self.suggested.borrow_mut() = Some((key, list.clone()));
        list
    }

    fn work_out_suggestions(&self, text: &str) -> Vec<Suggestion> {
        let (done, query) = split_entry(text);
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let have: HashSet<String> = crate::jmap::parse_addrs(done).into_iter().map(|a| a.email.to_lowercase()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut scored: Vec<(u8, Suggestion)> = Vec::new();
        let mut offer = |name: &str, email: &str, contact: bool, seen: &mut HashSet<String>| {
            let key = email.to_lowercase();
            if seen.contains(&key) || have.contains(&key) {
                return;
            }
            let name_l = name.to_lowercase();
            let score = if name_l.starts_with(&q) {
                0
            } else if name_l.split_whitespace().any(|w| w.starts_with(&q)) {
                1
            } else if key.starts_with(&q) {
                2
            } else if key.split(['@', '.', '_', '-']).any(|w| w.starts_with(&q)) {
                3
            } else {
                return;
            };
            seen.insert(key);
            scored.push((score + if contact { 0 } else { 4 }, Suggestion { name: name.to_owned(), email: email.to_owned(), contact }));
        };
        for c in self.accounts.iter().flat_map(|a| &a.contacts) {
            for e in &c.emails {
                offer(&c.name, e, true, &mut seen);
            }
        }
        let mine: HashSet<String> = self.accounts.iter().map(|a| a.key.to_lowercase()).collect();
        for m in self.messages.iter().chain(self.threads.values().flatten()) {
            for a in m.from.iter().chain(&m.to) {
                if !mine.contains(&a.email.to_lowercase()) {
                    offer(a.name.as_deref().unwrap_or(""), &a.email, false, &mut seen);
                }
            }
        }
        scored.sort_by_key(|(s, _)| *s);
        scored.into_iter().take(MAX_SUGGESTIONS).map(|(_, s)| s).collect()
    }

    /// The suggestions for the focused address field, unless Esc put them away.
    pub fn open_suggestions(&self) -> Option<(AddressField, Vec<Suggestion>)> {
        let field = self.address_field()?;
        if self.suggest_hidden {
            return None;
        }
        let text = match field {
            AddressField::To => &self.compose.to.text,
            AddressField::Cc => &self.compose.cc.text,
        };
        let list = self.suggestions(text);
        (!list.is_empty()).then_some((field, list))
    }

    /// Replaces the unfinished entry with `s`, ready for the next one.
    pub fn accept_suggestion(&mut self, field: AddressField, s: &Suggestion) {
        let addr = crate::jmap::format_addrs(&[crate::jmap::Addr {
            name: (!s.name.is_empty()).then(|| s.name.clone()),
            email: s.email.clone(),
        }]);
        let edit = self.address_edit(field);
        let (done, _) = split_entry(&edit.text);
        let text = format!("{}{addr}, ", if done.trim().is_empty() { String::new() } else { format!("{} ", done.trim_end()) });
        let caret = text.len();
        edit.replace_text(text);
        edit.set_caret(caret);
        edit.focused = true;
        self.suggest_index = 0;
    }

    // Lookups

    pub fn account(&self, key: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.key == key)
    }

    pub fn role_in(&self, account: &str, role: &str) -> Option<String> {
        self.account(account)?.mailboxes.iter().find(|m| m.role.as_deref() == Some(role)).map(|m| m.id.clone())
    }

    /// The account new mail and per-account mailbox keys go to: the shown mailbox's, else
    /// the selected message's, else the first.
    pub fn current_account(&self) -> Option<String> {
        match &self.view {
            Some(View::Mailbox { account, .. }) => Some(account.clone()),
            _ => self.selected_message().map(|m| m.account.clone()).or_else(|| self.accounts.first().map(|a| a.key.clone())),
        }
    }

    pub fn current_mailbox(&self) -> Option<&Mailbox> {
        let Some(View::Mailbox { account, id }) = &self.view else { return None };
        self.account(account)?.mailboxes.iter().find(|m| &m.id == id)
    }

    /// The list's title and its total and unread counts.
    pub fn view_summary(&self) -> (String, u32, u32) {
        match &self.view {
            Some(View::AllInboxes) => {
                let inboxes = self.accounts.iter().flat_map(|a| a.mailboxes.iter().filter(|m| m.role.as_deref() == Some("inbox")));
                let (total, unread) = inboxes.fold((0, 0), |(t, u), m| (t + m.total, u + m.unread));
                ("All Inboxes".to_owned(), total, unread)
            }
            _ => self.current_mailbox().map_or((String::new(), 0, 0), |m| (m.name.clone(), m.total, m.unread)),
        }
    }

    /// The mailbox a message leaves when it moves out of the current view.
    pub fn source_mailbox(&self, m: &Summary) -> Option<String> {
        match &self.view {
            Some(View::Mailbox { id, .. }) => Some(id.clone()),
            _ => self.role_in(&m.account, "inbox"),
        }
    }

    pub fn selected_message(&self) -> Option<&Summary> {
        self.messages.iter().find(|m| Some(&m.uid) == self.selected.as_ref())
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.messages.iter().position(|m| Some(&m.uid) == self.selected.as_ref())
    }

    /// The scroll view key of the reader for the shown message.
    pub fn reader_key(&self) -> Option<String> {
        self.selected.as_ref().map(|uid| format!("read-{uid}"))
    }

    /// Every account's events today, all-day first.
    pub fn today_events(&self) -> Vec<&CalEvent> {
        let mut all: Vec<&CalEvent> = self.accounts.iter().flat_map(|a| &a.events).collect();
        all.sort_by(|a, b| a.start.is_some().cmp(&b.start.is_some()).then(a.start.cmp(&b.start)));
        all
    }

    pub fn has_calendar(&self) -> bool {
        self.accounts.iter().any(|a| a.calendar)
    }

    // Accounts

    pub fn connect(&mut self) {
        let account = if self.add.kind == AddKind::Fastmail {
            let token = self.token.text.trim().to_owned();
            if token.is_empty() {
                return;
            }
            Saved { token, ..Saved::default() }
        } else {
            match self.add.account() {
                Ok(a) => a,
                Err(e) => {
                    self.signin_error = Some(e);
                    return;
                }
            }
        };
        if self.accounts.is_empty() {
            self.phase = Phase::Connecting;
        }
        self.connecting += 1;
        self.signin_error = None;
        self.send(Cmd::Connect { account, save: true });
    }

    /// An account is being added or reconnected.
    pub fn is_connecting(&self) -> bool {
        self.connecting > 0
    }

    pub fn choose_add_kind(&mut self, kind: AddKind) {
        self.add = AddForm::new(kind);
        self.token = token_field();
        self.token.focused = kind == AddKind::Fastmail;
        self.signin_error = None;
    }

    pub fn add_account(&mut self) {
        self.add = AddForm::new(AddKind::Fastmail);
        self.token = token_field();
        self.signin_error = None;
        self.sheet = Sheet::AddAccount;
    }

    pub fn remove_account(&mut self, key: &str) {
        self.confirm_remove = None;
        self.send(Cmd::Remove { account: key.to_owned() });
        self.accounts.retain(|a| a.key != key);
        let prefix = format!("{key}:");
        self.bodies.retain(|uid, _| !uid.starts_with(&prefix));
        self.marked.retain(|uid| !uid.starts_with(&prefix));
        self.remote_allowed.retain(|uid| !uid.starts_with(&prefix));
        self.on_white.retain(|uid| !uid.starts_with(&prefix));
        if self.selected.as_ref().is_some_and(|uid| uid.starts_with(&prefix)) {
            self.selected = None;
        }
        self.messages.retain(|m| m.account != key);
        self.threads.retain(|k, _| !k.starts_with(&prefix));
        self.list_version += 1;
        match &self.view {
            Some(View::Mailbox { account, .. }) if account == key => {
                self.view = None;
                self.select_role("inbox");
            }
            Some(View::AllInboxes) if self.accounts.len() < 2 => {
                self.view = None;
                self.select_role("inbox");
            }
            _ => {}
        }
        if self.accounts.is_empty() {
            self.phase = Phase::SignIn;
            self.sheet = Sheet::None;
            self.token = token_field();
            self.next_refresh = None;
        }
    }

    /// Settings' label field for account `key` (a spare one if it's just been removed).
    pub fn label_field(&mut self, key: &str) -> &mut TextEdit {
        match self.accounts.iter().position(|a| a.key == key) {
            Some(i) => &mut self.accounts[i].label_edit,
            None => &mut self.calendar_field,
        }
    }

    pub fn sender_name_field(&mut self, key: &str) -> &mut TextEdit {
        match self.accounts.iter().position(|a| a.key == key) {
            Some(i) => &mut self.accounts[i].from_name_edit,
            None => &mut self.calendar_field,
        }
    }

    /// Saves the name typed for account `key`'s sent mail.
    pub fn save_from_name(&mut self, key: &str) {
        let Some(a) = self.accounts.iter().find(|a| a.key == key) else { return };
        let name = a.from_name_edit.text.trim().to_owned();
        self.send(Cmd::SetFromName { account: key.to_owned(), name });
    }

    /// Keeps Settings' storage meter current while it's showing. True when it changed.
    pub fn tick_storage(&mut self, tx: &mut Tx) -> bool {
        if self.sheet != Sheet::Settings || self.settings_pane != SettingsPane::Storage {
            self.confirm_storage_off = false;
            return false;
        }
        let Some(store) = self.store.clone().filter(|s| s.is_enabled()) else { return false };
        let stale = self.storage_usage.as_ref().is_none_or(|(_, at)| self.now() - at >= 5.0);
        if stale {
            let usage = store.usage();
            let changed = self.storage_usage.as_ref().is_none_or(|(u, _)| *u != usage);
            self.storage_usage = Some((usage, self.now()));
            tx.wake_after(Duration::from_secs(5));
            return changed;
        }
        false
    }

    /// Opens attachment `index` of message `uid` in the system's app for it.
    pub fn open_attachment(&mut self, uid: &str, index: usize) {
        let Some((account, id, file)) = self.attachment_of(uid, index) else { return };
        self.send(Cmd::Attachment { account, id, file, save: None });
    }

    /// Asks where to save attachment `index` of message `uid`, then saves it there.
    pub fn save_attachment(&mut self, uid: &str, index: usize, tx: &mut Tx) {
        let Some((account, id, file)) = self.attachment_of(uid, index) else { return };
        let options = rune::FileDialogOptions {
            title: "Save Attachment".into(),
            directory: dirs::download_dir(),
            file_name: Some(file.name.clone()),
            ..Default::default()
        };
        tx.save_file(options, move |s: &mut App, result, tx| match result {
            Ok(Some(path)) => s.send(Cmd::Attachment { account, id, file, save: Some(path) }),
            Ok(None) => {}
            Err(e) => s.say(format!("Couldn't save it: {e}"), tx),
        });
    }

    fn attachment_of(&self, uid: &str, index: usize) -> Option<(String, String, crate::jmap::FileInfo)> {
        let m = self.find_message(uid)?;
        let file = self.bodies.get(uid)?.files.get(index)?.clone();
        Some((m.account.clone(), m.id.clone(), file))
    }

    /// Deletes what's kept and starts filling it again.
    pub fn clear_storage(&mut self) {
        if let Some(store) = &self.store {
            store.set_enabled(false);
            self.send(Cmd::ClearStore { on: self.settings.storage.on });
        }
        self.send(Cmd::Storage(self.settings.storage.clone()));
        self.storage_usage = None;
    }

    /// Saves the label typed for account `key`.
    pub fn save_label(&mut self, key: &str) {
        let Some(a) = self.accounts.iter_mut().find(|a| a.key == key) else { return };
        let label = a.label_edit.text.trim().to_owned();
        a.label = (!label.is_empty()).then_some(label.clone());
        self.send(Cmd::SetLabel { account: key.to_owned(), label });
    }

    /// Moves account `key` up (-1) or down (1) the sidebar.
    pub fn move_account(&mut self, key: &str, delta: isize) {
        let Some(i) = self.accounts.iter().position(|a| a.key == key) else { return };
        let j = (i as isize + delta).clamp(0, self.accounts.len() as isize - 1) as usize;
        if i == j {
            return;
        }
        self.accounts.swap(i, j);
        for (n, a) in self.accounts.iter_mut().enumerate() {
            a.position = n;
        }
        self.send(Cmd::Reorder { accounts: self.accounts.iter().map(|a| a.key.clone()).collect() });
    }

    pub fn set_calendar_enabled(&mut self, key: &str, on: bool) {
        self.send(Cmd::SetCalendarEnabled { account: key.to_owned(), on });
    }

    pub fn edit_calendar(&mut self, key: &str) {
        self.calendar_field = TextEdit::new("").secure();
        self.calendar_field.focused = true;
        self.calendar_edit = Some(key.to_owned());
    }

    pub fn save_calendar(&mut self) {
        let Some(key) = &self.calendar_edit else { return };
        let password = self.calendar_field.text.trim().to_owned();
        if !password.is_empty() {
            self.send(Cmd::SetCalendar { account: key.clone(), password: Some(password) });
        }
    }

    pub fn remove_calendar(&mut self, key: &str) {
        self.send(Cmd::SetCalendar { account: key.to_owned(), password: None });
    }

    /// The Copy for AI prompt was typed in: it takes effect now, and is saved once typing stops.
    pub fn edit_ai_prompt(&mut self, tx: &mut Tx) {
        self.settings.ai_prompt = self.ai_prompt_edit.text.clone();
        self.save_settings_at = Some(self.clock + 1.0);
        tx.wake_after(Duration::from_secs(1));
    }

    fn save_typed_settings(&mut self, tx: &mut Tx) {
        if self.save_settings_at.is_some_and(|at| self.clock >= at) {
            self.save_settings_at = None;
            if let Err(e) = self.settings.save() {
                self.say(format!("Couldn't save settings: {e}"), tx);
            }
        }
    }

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings), tx: &mut Tx) {
        let before = self.settings.clone();
        f(&mut self.settings);
        if self.settings.refresh_minutes != before.refresh_minutes {
            self.schedule_refresh(tx);
        }
        if self.settings.conversations != before.conversations {
            self.selected = None;
            self.marked.clear();
            self.reload_list();
        }
        if self.settings.storage != before.storage
            && let Some(store) = &self.store
        {
            store.set_enabled(self.settings.storage.on);
            if before.storage.on && !self.settings.storage.on {
                self.send(Cmd::ClearStore { on: false });
            }
            self.send(Cmd::Storage(self.settings.storage.clone()));
            self.storage_usage = None;
        }
        let more_images = (self.settings.remote_images && !before.remote_images)
            || (self.settings.render_html && !before.render_html);
        if more_images && let Some(uid) = self.selected.clone() {
            self.request_images(&uid);
        }
        if let Err(e) = self.settings.save() {
            self.say(format!("Couldn't save settings: {e}"), tx);
        }
    }

    // Navigation

    /// The sidebar. One account: Favorites, then its folders. Several: All Inboxes and each
    /// inbox under Favorites, then a section per account with its other mailboxes.
    pub fn sidebar(&self) -> Vec<Section> {
        // Built every frame, so children are found through a map: hundreds of Gmail labels
        // each scanning the whole list would be quadratic.
        fn folders(a: &Account, out: &mut Vec<Item>) {
            let mut kids: HashMap<Option<&str>, Vec<&Mailbox>> = HashMap::new();
            for m in &a.mailboxes {
                kids.entry(m.parent_id.as_deref()).or_default().push(m);
            }
            for list in kids.values_mut() {
                list.sort_by(|x, y| x.sort_order.cmp(&y.sort_order).then_with(|| x.name.cmp(&y.name)));
            }
            fn walk(a: &Account, kids: &HashMap<Option<&str>, Vec<&Mailbox>>, parent: Option<&str>, depth: usize, out: &mut Vec<Item>) {
                // A server that names a folder its own ancestor mustn't recurse forever.
                if depth > 32 {
                    return;
                }
                for m in kids.get(&parent).into_iter().flatten() {
                    if m.role.is_none() {
                        out.push(item(a, m, m.name.clone(), depth));
                    }
                    walk(a, kids, Some(&m.id), depth + 1, out);
                }
            }
            walk(a, &kids, None, 0, out);
        }
        fn item(a: &Account, m: &Mailbox, name: String, depth: usize) -> Item {
            let view = View::Mailbox { account: a.key.clone(), id: m.id.clone() };
            Item { view, name, role: m.role.clone(), depth, unread: m.unread }
        }
        let find = |a: &'_ Account, role: &str| a.mailboxes.iter().find(|m| m.role.as_deref() == Some(role)).cloned();
        let mut sections = Vec::new();
        if let [a] = self.accounts.as_slice() {
            let favorites = FAVORITES.iter().filter_map(|r| find(a, r)).map(|m| item(a, &m, m.name.clone(), 0)).collect();
            sections.push(Section { id: "favorites".into(), title: "Favorites".into(), items: favorites });
            let mut items = Vec::new();
            folders(a, &mut items);
            sections.push(Section { id: a.key.clone(), title: a.name().to_owned(), items });
            return sections;
        }
        let inboxes: Vec<(Mailbox, &Account)> = self.accounts.iter().filter_map(|a| find(a, "inbox").map(|m| (m, a))).collect();
        let unread = inboxes.iter().map(|(m, _)| m.unread).sum();
        let mut favorites =
            vec![Item { view: View::AllInboxes, name: "All Inboxes".into(), role: Some("inbox".into()), depth: 0, unread }];
        favorites.extend(inboxes.iter().map(|(m, a)| item(a, m, a.name().to_owned(), 1)));
        sections.push(Section { id: "favorites".into(), title: "Favorites".into(), items: favorites });
        for a in &self.accounts {
            let mut items: Vec<Item> =
                FAVORITES[1..].iter().filter_map(|r| find(a, r)).map(|m| item(a, &m, m.name.clone(), 0)).collect();
            folders(a, &mut items);
            sections.push(Section { id: a.key.clone(), title: a.name().to_owned(), items });
        }
        sections
    }

    pub fn select_view(&mut self, view: View) {
        if self.view.as_ref() == Some(&view) {
            return;
        }
        self.view = Some(view.clone());
        self.pane = Pane::List;
        self.selected = None;
        self.marked.clear();
        self.messages.clear();
        self.threads.clear();
        self.list_version += 1;
        // What the store kept shows at once; the server's list replaces it when it comes.
        if let Some(store) = &self.store {
            // Written by the worker: the store's write lock may be busy with the prefetcher.
            self.send(Cmd::Remember { key: "view".into(), value: serde_json::to_string(&view).unwrap_or_default() });
            if let Some((rows, threads)) = store.listing(&view, self.settings.conversations) {
                self.messages = rows;
                self.threads = threads;
            }
        }
        self.reload_list();
        if !self.messages.is_empty() {
            self.list_loading = false;
        }
    }

    /// The mailboxes the list shows: one, or every account's inbox.
    fn view_mailboxes(&self) -> Vec<(String, String)> {
        match &self.view {
            Some(View::Mailbox { account, id }) => vec![(account.clone(), id.clone())],
            Some(View::AllInboxes) => {
                self.accounts.iter().filter_map(|a| self.role_in(&a.key, "inbox").map(|id| (a.key.clone(), id))).collect()
            }
            None => Vec::new(),
        }
    }

    fn reload_list(&mut self) {
        let Some(view) = self.view.clone() else { return };
        self.list_loading = true;
        self.send(Cmd::LoadList { view, search: self.searched.clone(), threads: self.settings.conversations });
    }

    /// Goes to a standard mailbox: the inbox across accounts when there are several, else
    /// the current account's.
    pub fn select_role(&mut self, role: &str) {
        if role == "inbox" && self.accounts.len() > 1 {
            return self.select_view(View::AllInboxes);
        }
        let Some(account) = self.current_account() else { return };
        if let Some(id) = self.role_in(&account, role) {
            self.select_view(View::Mailbox { account, id });
        }
    }

    pub fn is_collapsed(&self, section: &str) -> bool {
        self.settings.collapsed.iter().any(|c| c == section)
    }

    /// Folds or unfolds a sidebar section, remembered across launches.
    pub fn toggle_section(&mut self, section: &str, tx: &mut Tx) {
        let id = section.to_owned();
        self.update_settings(
            |st| {
                if let Some(i) = st.collapsed.iter().position(|c| *c == id) {
                    st.collapsed.remove(i);
                } else {
                    st.collapsed.push(id);
                }
            },
            tx,
        );
    }

    /// `J`/`K`: the next or previous mailbox among the sections that are open.
    pub fn step_mailbox(&mut self, delta: isize) {
        let order: Vec<View> = self
            .sidebar()
            .into_iter()
            .filter(|s| !self.is_collapsed(&s.id))
            .flat_map(|s| s.items)
            .map(|i| i.view)
            .collect();
        if order.is_empty() {
            return;
        }
        let i = self.view.as_ref().and_then(|v| order.iter().position(|o| o == v)).unwrap_or(0) as isize;
        let next = (i + delta).clamp(0, order.len() as isize - 1) as usize;
        self.select_view(order[next].clone());
    }

    pub fn run_search(&mut self) {
        let query = self.search.text.trim().to_owned();
        if query == self.searched {
            return;
        }
        self.searched = query;
        self.selected = None;
        self.marked.clear();
        // Kept mail answers at once; the server's results (older mail too) join them.
        self.local_hits = match (&self.store, self.searched.is_empty()) {
            (Some(store), false) => store.search(&self.searched, &self.view_mailboxes(), 200),
            _ => Vec::new(),
        };
        if !self.searched.is_empty() {
            self.messages = self.local_hits.clone();
            self.threads.clear();
            self.list_version += 1;
        }
        self.reload_list();
    }

    /// Opens row `uid`'s conversation: its newest message and any unread ones expand, and
    /// unread ones are marked read when that setting is on.
    pub fn select_message(&mut self, uid: String) {
        let Some(row) = self.messages.iter().find(|m| m.uid == uid) else { return };
        let members: Vec<Summary> = self.thread(row).into_iter().cloned().collect();
        self.selected = Some(uid.clone());
        self.expanded.clear();
        self.expanded.insert(uid.clone());
        if let Some(newest) = members.last() {
            self.expanded.insert(newest.uid.clone());
            self.thread_focus = Some(newest.uid.clone());
        }
        self.expanded.extend(members.iter().filter(|m| !m.seen).map(|m| m.uid.clone()));
        // Opening a conversation again retries what failed last time.
        let open: Vec<Summary> = members.iter().filter(|m| self.expanded.contains(&m.uid)).cloned().collect();
        for m in &open {
            self.body_failed.remove(&m.uid);
            self.load_body(m);
        }
        // The rows either side are likely next (j, k, arrows): their bodies come ahead. Asked
        // for in reverse, as the newest request goes first: the next row, then the one after,
        // then the one before.
        if let Some(i) = self.messages.iter().position(|m| m.uid == uid) {
            let near: Vec<Summary> = [i + 1, i + 2, i.wrapping_sub(1)]
                .into_iter()
                .filter_map(|j| self.messages.get(j))
                .filter_map(|row| self.thread(row).last().map(|m| (*m).clone()))
                .collect();
            for m in near.iter().rev() {
                self.ask_body(m, false);
            }
        }
        if self.settings.mark_read_on_open {
            let unread: Vec<String> = members.iter().filter(|m| !m.seen).map(|m| m.uid.clone()).collect();
            self.set_seen(&unread, true);
        }
    }

    /// Asks for a message's body unless it's here, on its way, or failed (Try Again retries).
    pub(crate) fn load_body(&mut self, m: &Summary) {
        self.ask_body(m, true);
    }

    /// `opened`: you're waiting for it. One asked for ahead is asked again as opened, so it
    /// doesn't wait in the prefetch queue.
    fn ask_body(&mut self, m: &Summary, opened: bool) {
        if self.bodies.contains_key(&m.uid) {
            if self.expanded.contains(&m.uid) {
                self.request_images(&m.uid);
            }
            return;
        }
        if self.body_failed.contains_key(&m.uid) {
            return;
        }
        match self.requested_bodies.get(&m.uid) {
            Some(true) => return,
            Some(false) if !opened => return,
            _ => {}
        }
        self.requested_bodies.insert(m.uid.clone(), opened);
        self.send(Cmd::LoadBody { account: m.account.clone(), id: m.id.clone(), opened });
    }

    pub fn retry_body(&mut self, uid: &str) {
        self.body_failed.remove(uid);
        if let Some(m) = self.find_message(uid).cloned() {
            self.load_body(&m);
        }
    }

    /// Fetches the newest message of the list's first conversations ahead of opening them.
    fn prefetch(&mut self) {
        let newest: Vec<Summary> = self
            .messages
            .iter()
            .take(PREFETCH)
            .filter_map(|row| self.thread(row).last().map(|m| (*m).clone()))
            .collect();
        // Oldest first, so the body thread (newest request first) starts at the top of the list.
        for m in newest.iter().rev() {
            self.ask_body(m, false);
        }
    }

    /// Drops the oldest bodies past the limit, never those of the open conversation.
    fn evict_bodies(&mut self) {
        if self.bodies.len() < MAX_BODIES {
            return;
        }
        let open: HashSet<String> = self.conversation().iter().map(|m| m.uid.clone()).collect();
        let mut kept = VecDeque::new();
        while self.bodies.len() >= MAX_BODIES * 3 / 4 {
            let Some(uid) = self.body_order.pop_front() else { break };
            if open.contains(&uid) {
                kept.push_back(uid);
            } else {
                self.bodies.remove(&uid);
            }
        }
        kept.append(&mut self.body_order);
        self.body_order = kept;
    }

    /// A message's conversation, oldest first (just the message when it has none).
    pub fn thread<'a>(&'a self, row: &'a Summary) -> Vec<&'a Summary> {
        match self.threads.get(&row.thread_key) {
            Some(list) if !list.is_empty() => list.iter().collect(),
            _ => vec![row],
        }
    }

    /// Finds a message by uid in a conversation or the list.
    pub fn find_message(&self, uid: &str) -> Option<&Summary> {
        self.threads.values().flatten().find(|m| m.uid == uid).or_else(|| self.messages.iter().find(|m| m.uid == uid))
    }

    /// Applies `f` to both copies of message `uid` (its row and its place in a conversation).
    pub(crate) fn update_message(&mut self, uid: &str, f: impl Fn(&mut Summary)) {
        for m in self.messages.iter_mut().chain(self.threads.values_mut().flatten()).filter(|m| m.uid == uid) {
            f(m);
        }
        self.list_version += 1;
        if let (Some(_), Some(m)) = (&self.store, self.find_message(uid)) {
            self.send(Cmd::Keep(vec![m.clone()]));
        }
    }

    /// [`App::update_message`] for many at once, in one pass over the list; returns each as
    /// it is now.
    pub(crate) fn update_messages(&mut self, uids: &[String], f: impl Fn(&mut Summary)) -> Vec<Summary> {
        let wanted: HashSet<&str> = uids.iter().map(String::as_str).collect();
        let mut changed: HashMap<String, Summary> = HashMap::new();
        for m in self.messages.iter_mut().chain(self.threads.values_mut().flatten()) {
            if wanted.contains(m.uid.as_str()) {
                f(m);
                changed.entry(m.uid.clone()).or_insert_with(|| m.clone());
            }
        }
        self.list_version += 1;
        let changed: Vec<Summary> = changed.into_values().collect();
        if self.store.is_some() && !changed.is_empty() {
            self.send(Cmd::Keep(changed.clone()));
        }
        changed
    }

    /// Marks messages read or unread, here and on the server, keeping unread counts right.
    /// One pass over the list however many there are: marking thousands at once is common.
    pub(crate) fn set_seen(&mut self, uids: &[String], seen: bool) {
        let wanted: HashSet<&str> = uids.iter().map(String::as_str).collect();
        let mut changed: HashMap<String, Summary> = HashMap::new();
        for m in self.messages.iter_mut().chain(self.threads.values_mut().flatten()) {
            if m.seen != seen && wanted.contains(m.uid.as_str()) {
                m.seen = seen;
                changed.entry(m.uid.clone()).or_insert_with(|| m.clone());
            }
        }
        if changed.is_empty() {
            return;
        }
        self.list_version += 1;
        let delta: i64 = if seen { -1 } else { 1 };
        let mut by_account: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for m in changed.values() {
            if let Some(a) = self.accounts.iter_mut().find(|a| a.key == m.account) {
                for mb in a.mailboxes.iter_mut().filter(|mb| m.mailbox_ids.contains(&mb.id)) {
                    mb.unread = u32::try_from((i64::from(mb.unread) + delta).max(0)).unwrap_or(u32::MAX);
                }
            }
            by_account.entry(m.account.clone()).or_default().push(m.id.clone());
        }
        if self.store.is_some() {
            self.send(Cmd::Keep(changed.into_values().collect()));
        }
        for (account, ids) in by_account {
            self.send(Cmd::SetKeyword { account, ids, keyword: "$seen", on: seen });
        }
    }

    /// The open conversation in reader order (newest first unless Settings say otherwise).
    pub fn conversation(&self) -> Vec<&Summary> {
        let Some(row) = self.selected_message() else { return Vec::new() };
        let mut list = self.thread(row);
        if self.settings.newest_first {
            list.reverse();
        }
        list
    }

    /// The reader's key for one message of the conversation, for scrolling to it.
    pub fn message_key(uid: &str) -> String {
        format!("tm-{uid}")
    }

    /// Highlights a message of the open conversation and scrolls to it; the keyboard moves
    /// into the conversation.
    pub fn focus_message(&mut self, uid: &str, tx: &mut Tx) {
        self.pane = Pane::Thread;
        self.thread_focus = Some(uid.to_owned());
        tx.scroll_into_view(Self::message_key(uid));
    }

    /// Moves the highlight `delta` messages through the conversation, in reader order.
    pub fn step_thread(&mut self, delta: isize, tx: &mut Tx) {
        let order: Vec<String> = self.conversation().iter().map(|m| m.uid.clone()).collect();
        if order.is_empty() {
            return;
        }
        let i = self.thread_focus.as_ref().and_then(|f| order.iter().position(|u| u == f)).map_or(0, |i| i as isize + delta);
        let uid = order[i.clamp(0, order.len() as isize - 1) as usize].clone();
        self.open_only(&uid, tx);
    }

    /// Like an accordion: opens `uid`, closes the conversation's other messages, highlights it,
    /// and scrolls the reader so the message starts at the top.
    pub fn open_only(&mut self, uid: &str, tx: &mut Tx) {
        self.expanded.clear();
        self.expanded.insert(uid.to_owned());
        if let Some(m) = self.find_message(uid).cloned() {
            self.load_body(&m);
        }
        self.pane = Pane::Thread;
        self.thread_focus = Some(uid.to_owned());
        // Placed from the layout after the others close, so it lands at the top exactly.
        tx.scroll_key_to(Self::message_key(uid), crate::ui::reader::CARD_GAP);
    }

    /// Opens or closes the highlighted message.
    pub fn toggle_focused(&mut self, tx: &mut Tx) {
        if let Some(uid) = self.focused_uid() {
            self.toggle_expanded(&uid);
            self.focus_message(&uid, tx);
        }
    }

    /// Moves the keyboard into the open conversation, onto its highlighted message.
    pub fn enter_thread(&mut self, tx: &mut Tx) {
        let order: Vec<String> = self.conversation().iter().map(|m| m.uid.clone()).collect();
        let Some(first) = order.first() else { return };
        let uid = self.thread_focus.clone().filter(|f| order.contains(f)).unwrap_or_else(|| first.clone());
        self.focus_message(&uid, tx);
    }

    /// Moves the keyboard back to the message list.
    pub fn leave_thread(&mut self) -> bool {
        std::mem::replace(&mut self.pane, Pane::List) == Pane::Thread
    }

    /// `j`/`k` and the arrows: through the list, or through the conversation when it's active.
    pub fn step(&mut self, delta: isize, tx: &mut Tx) {
        match self.pane {
            Pane::Thread if self.selected.is_some() => self.step_thread(delta, tx),
            _ => self.step_selection(delta, tx),
        }
    }

    /// `gg` and `G`: the first or last row of the list, or message of the conversation.
    pub fn jump(&mut self, last: bool, tx: &mut Tx) {
        if self.pane == Pane::Thread && self.selected.is_some() {
            let order: Vec<String> = self.conversation().iter().map(|m| m.uid.clone()).collect();
            if let Some(uid) = if last { order.last() } else { order.first() }.cloned() {
                self.open_only(&uid, tx);
            }
        } else {
            let i = if last { self.messages.len().saturating_sub(1) } else { 0 };
            self.select_index(i, tx);
        }
    }

    /// The highlighted message, while the keyboard is in the open conversation.
    pub fn focused_uid(&self) -> Option<String> {
        if self.pane != Pane::Thread {
            return None;
        }
        let f = self.thread_focus.as_ref()?;
        self.conversation().iter().any(|m| &m.uid == f).then(|| f.clone())
    }

    /// Opens or closes one message of the open conversation.
    pub fn toggle_expanded(&mut self, uid: &str) {
        if !self.expanded.remove(uid)
            && let Some(m) = self.find_message(uid).cloned()
        {
            self.expanded.insert(uid.to_owned());
            self.load_body(&m);
        }
    }

    /// Opens every message of the open conversation, or closes all but the newest.
    pub fn toggle_expand_all(&mut self) {
        let Some(row) = self.selected_message() else { return };
        let members: Vec<Summary> = self.thread(row).into_iter().cloned().collect();
        if members.iter().all(|m| self.expanded.contains(&m.uid)) {
            self.expanded.clear();
            if let Some(newest) = members.last() {
                self.expanded.insert(newest.uid.clone());
            }
        } else {
            for m in &members {
                self.expanded.insert(m.uid.clone());
                self.load_body(m);
            }
        }
    }

    pub fn select_index(&mut self, i: usize, tx: &mut Tx) {
        let Some(m) = self.messages.get(i) else { return };
        let uid = m.uid.clone();
        tx.scroll_into_view(format!("msg-{uid}"));
        self.select_message(uid);
    }

    pub fn step_selection(&mut self, delta: isize, tx: &mut Tx) {
        self.pane = Pane::List;
        let n = self.messages.len() as isize;
        if n == 0 {
            return;
        }
        let i = match self.selected_index() {
            Some(i) => (i as isize + delta).clamp(0, n - 1),
            None => 0,
        };
        self.select_index(i as usize, tx);
    }

    /// A click on row `i`: Cmd toggles its mark, Shift marks the range from the selection,
    /// a plain click selects it and clears the marks.
    pub fn click_row(&mut self, i: usize, tx: &mut Tx) {
        let Some(uid) = self.messages.get(i).map(|m| m.uid.clone()) else { return };
        if self.close_swipe() {
            return; // a click on a swiped-open list first closes it
        }
        self.pane = Pane::List;
        let mods = tx.mods();
        if mods.cmd {
            if let Some(cur) = self.selected.clone()
                && self.marked.is_empty()
                && cur != uid
            {
                self.marked.insert(cur);
            }
            self.toggle_mark(&uid);
        } else if mods.shift {
            let from = self.selected_index().unwrap_or(i);
            let (a, b) = (from.min(i), from.max(i));
            self.marked.extend(self.messages[a..=b].iter().map(|m| m.uid.clone()));
            self.list_version += 1;
        } else {
            if !self.marked.is_empty() {
                self.marked.clear();
                self.list_version += 1;
            }
            self.select_message(uid);
        }
    }

    pub fn toggle_mark(&mut self, uid: &str) {
        if !self.marked.remove(uid) {
            self.marked.insert(uid.to_owned());
        }
        self.list_version += 1;
    }

    pub fn toggle_mark_selected(&mut self) {
        if let Some(uid) = self.selected.clone() {
            self.toggle_mark(&uid);
        }
    }

    /// Marks every message, or clears the marks when all are marked.
    pub fn mark_all(&mut self) {
        if self.marked.len() == self.messages.len() {
            self.marked.clear();
        } else {
            self.marked = self.messages.iter().map(|m| m.uid.clone()).collect();
        }
        self.list_version += 1;
    }

    pub fn clear_marks(&mut self) -> bool {
        let had = !self.marked.is_empty();
        self.marked.clear();
        self.list_version += 1;
        had
    }
}

/// A colour as sRGB bytes, for the page thread's CSS.
fn rgba8(c: rune::Color) -> [u8; 4] {
    let hex = c.hex();
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    [byte(1), byte(3), byte(5), byte(7)]
}

/// A URL that looks like a stylesheet or a font rather than an image: never loaded, as they
/// can track as well as an image and change little worth having.
fn font_or_style(src: &str) -> bool {
    let path = src.split(['?', '#']).next().unwrap_or(src).to_ascii_lowercase();
    // Google Fonts' stylesheets have no extension: `fonts.googleapis.com/css?family=...`.
    [".css", "/css", "/css2", ".woff", ".woff2", ".ttf", ".otf", ".eot"].iter().any(|ext| path.ends_with(ext))
}

/// Splits an address field into its finished entries (through the last comma or semicolon)
/// and the one being typed.
fn split_entry(text: &str) -> (&str, &str) {
    match text.rfind([',', ';']) {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("", text),
    }
}

#[cfg(test)]
mod suggestion_tests {
    use super::split_entry;

    #[test]
    fn splits_the_entry_being_typed() {
        assert_eq!(split_entry("Ada <ada@x.com>, gra"), ("Ada <ada@x.com>,", " gra"));
        assert_eq!(split_entry("gra"), ("", "gra"));
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;

    #[test]
    fn eviction_drops_the_oldest_bodies_but_never_the_open_conversation() {
        crate::demo::test_config();
        let _one = crate::demo::test_app();
        let mut s = init();
        crate::demo::thread_scene(&mut s);
        let open: Vec<String> = s.conversation().iter().map(|m| m.uid.clone()).collect();
        let template = s.bodies.values().next().cloned().unwrap();
        // The open conversation's bodies arrived first, so they're the oldest.
        s.body_order = open.iter().cloned().collect();
        for i in 0..MAX_BODIES {
            let uid = format!("x:{i}");
            s.bodies.insert(uid.clone(), template.clone());
            s.body_order.push_back(uid);
        }
        s.evict_bodies();
        assert!(s.bodies.len() < MAX_BODIES);
        for uid in &open {
            assert!(s.bodies.contains_key(uid) || !s.body_order.contains(uid), "{uid} was evicted");
        }
        assert!(!s.bodies.contains_key("x:0"), "the oldest other body goes first");
        assert!(s.bodies.contains_key(&format!("x:{}", MAX_BODIES - 1)), "the newest stays");
    }

    #[test]
    fn marking_read_changes_every_copy_and_each_count_once() {
        let _one = crate::demo::test_app();
        let mut s = init();
        crate::demo::thread_scene(&mut s);
        let unread: Vec<Summary> = s.threads.values().flatten().filter(|m| !m.seen).cloned().collect();
        assert!(!unread.is_empty(), "the scene has unread mail");
        let count = |s: &App, m: &Summary| {
            s.accounts.iter().find(|a| a.key == m.account).and_then(|a| a.mailboxes.iter().find(|b| m.mailbox_ids.contains(&b.id))).map(|b| b.unread)
        };
        let before = count(&s, &unread[0]);
        let in_box = unread.iter().filter(|m| m.mailbox_ids == unread[0].mailbox_ids && m.account == unread[0].account).count();
        let uids: Vec<String> = unread.iter().map(|m| m.uid.clone()).collect();
        s.set_seen(&uids, true);
        assert!(s.messages.iter().chain(s.threads.values().flatten()).filter(|m| uids.contains(&m.uid)).all(|m| m.seen));
        assert_eq!(count(&s, &unread[0]), before.map(|n| n.saturating_sub(u32::try_from(in_box).unwrap())));
    }

    #[test]
    fn a_prompt_typed_just_before_quitting_is_saved() {
        let _one = crate::demo::test_app();
        let mut s = init();
        s.settings.ai_prompt = "Typed then quit".into();
        s.save_settings_at = Some(f64::MAX);
        drop(s);
        assert_eq!(Settings::load().ai_prompt, "Typed then quit");
        Settings::default().save().unwrap();
    }

    #[test]
    fn only_a_message_s_own_inline_images_load_without_asking() {
        let _one = crate::demo::test_app();
        let mut s = init();
        s.settings.render_html = true;
        s.settings.remote_images = false;
        let html = r#"<img src="blob:mine:image/png"><img src="blob:theirs:image/png">"#;
        let body = Body {
            html: Some(crate::html::parse(html, &|_| None)),
            inline: Some(vec!["blob:mine:image/png".into()]),
            ..Body::default()
        };
        s.bodies.insert("a:1".into(), body);
        s.request_images("a:1");
        assert!(s.requested_images.contains("blob:mine:image/png"));
        assert!(!s.requested_images.contains("blob:theirs:image/png"), "a blob the mail named itself waits");
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;

    #[test]
    fn starts_on_what_the_store_kept_before_any_account_connects() {
        let _one = crate::demo::test_app();
        let saved = vec![Saved {
            token: "fmu1-test".into(),
            username: Some("restore@example.com".into()),
            label: Some("Home".into()),
            ..Saved::default()
        }];
        crate::accounts::save(&saved).unwrap();
        let mut settings = Settings::default();
        settings.storage.on = true;
        settings.save().unwrap();
        let store = crate::store::Store::open().unwrap();
        store.set_enabled(true);
        let inbox = Mailbox {
            id: "in".into(), name: "Inbox".into(), role: Some("inbox".into()), parent_id: None,
            sort_order: 0, unread: 1, total: 1,
        };
        store.save_mailboxes("restore@example.com", &[inbox]);
        let m = Summary {
            id: "m1".into(), account: "restore@example.com".into(), uid: crate::jmap::uid("restore@example.com", "m1"),
            thread_id: "t".into(), thread_key: crate::jmap::uid("restore@example.com", "t"),
            from: vec![], to: vec![], subject: "Kept".into(), preview: String::new(),
            received_at: "2026-10-30T09:00:00Z".into(), seen: false, flagged: false, has_attachment: false,
            mailbox_ids: vec!["in".into()],
        };
        let threads: Threads = [(m.thread_key.clone(), vec![m.clone()])].into_iter().collect();
        let view = View::Mailbox { account: "restore@example.com".into(), id: "in".into() };
        store.save_listing(&view, true, std::slice::from_ref(&m), &threads);

        let s = init();
        assert!(s.phase == Phase::Ready, "no waiting for the network");
        assert_eq!(s.accounts.len(), 1);
        assert_eq!(s.accounts[0].name(), "Home");
        assert!(!s.accounts[0].live);
        assert_eq!(s.view, Some(view));
        assert_eq!(s.messages.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Kept"]);
        let _ = std::fs::remove_file(crate::accounts::path_for_tests());
        Settings::default().save().unwrap();
    }
}
