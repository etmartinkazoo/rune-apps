//! The thread that talks to the phone: BlueZ for the link and pairing, PipeWire for calls,
//! obexd for messages and contacts. It runs them on a tokio runtime of its own and hands the
//! window what it needs to show.
// On macOS the phone thread is `macworker`; this is for Linux (and its tests).
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use rune::Waker;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::{Connection, MatchRule, MessageStream};

use crate::bluez::{self, Confirmation, Device};
use crate::calls::{self, Change, LiveCall, State};
use crate::obex::{self, Listed};
use crate::vcard::{self, Call, Contact};

/// How many of the newest messages each folder lists.
const MESSAGES: u16 = 400;
/// While the phone is linked without calls, its call gateway is looked for this often: PipeWire
/// brings Hands-Free up a while after the link, and may drop and redo it once or twice.
const GATEWAY_RETRY: Duration = Duration::from_secs(3);
/// Messages are listed again this often, in case a notification was missed.
const RELIST: Duration = Duration::from_secs(120);

pub enum Cmd {
    /// Looks for phones nearby, or stops.
    Discover(bool),
    /// Pairs with the device at this path, then uses it.
    Pair(String),
    /// A pairing attempt finished: the worker uses the phone if it worked.
    PairDone { path: String, result: Result<(), String> },
    /// The person's answer to the pairing code.
    Confirm(bool),
    /// Uses this (paired) phone.
    Use(String),
    /// Unpairs this device (by path), letting go of it first if it's the phone in use.
    Remove(String),
    Dial(String),
    Answer,
    /// Hangs up the active call, or declines the ringing one.
    HangUp,
    Tones(String),
    Mute(bool),
    Send { number: String, text: String },
    /// Fetches the whole text of these messages (the listing has at most 255 characters).
    Fetch(Vec<String>),
    MarkRead(Vec<String>),
    MarkUnread(Vec<String>),
    /// Deletes these messages on the phone.
    Delete(Vec<String>),
}

/// A message, for the window.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub id: String,
    pub number: String,
    pub name: String,
    pub text: String,
    pub at: String,
    pub incoming: bool,
    pub read: bool,
    pub mms: bool,
    /// Its text may be longer than listed.
    pub partial: bool,
}

impl From<&Listed> for Message {
    fn from(l: &Listed) -> Message {
        Message {
            id: l.path.clone(),
            number: l.number.clone(),
            name: l.name.clone(),
            text: l.preview.clone(),
            at: l.at.clone(),
            incoming: l.incoming,
            read: l.read,
            // Until it's read whole, a listed MMS with text (an RCS chat message, often) isn't
            // shown as a photo.
            mms: l.mms && l.preview.is_empty(),
            partial: l.mms || l.preview.chars().count() >= 250,
        }
    }
}

/// What reaches the phone, and how.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub phone: Option<Device>,
    pub calls: bool,
    pub messages: bool,
    pub contacts: bool,
}

pub enum Event {
    Devices(Vec<Device>),
    Link(Link),
    /// Pairing: both screens show this code; the answer goes back as `Cmd::Confirm`.
    PairCode { name: String, passkey: u32 },
    Paired(Device),
    /// This device is unpaired now.
    Removed(Device),
    /// The pairing attempt failed, so a code it showed is stale.
    PairFailed,
    Calls(Vec<LiveCall>),
    Messages(Vec<Message>),
    /// Messages fetched whole: (id, text, has an attachment).
    Fetched(Vec<(String, String, bool)>),
    Sent,
    Contacts(Vec<Contact>),
    Recents(Vec<Call>),
    Error(String),
}

/// The buses the worker uses; tests point them at a private daemon.
pub struct Buses {
    pub system: Option<Connection>,
    pub session: Connection,
}

/// macOS has no BlueZ, obexd or PipeWire: its phone thread is `macworker`, with the same
/// commands and events.
#[cfg(target_os = "macos")]
pub fn spawn(wake: Waker, phone: Option<String>) -> (UnboundedSender<Cmd>, std::sync::mpsc::Receiver<Event>) {
    crate::macworker::spawn(wake, phone)
}

#[cfg(not(target_os = "macos"))]
pub fn spawn(wake: Waker, phone: Option<String>) -> (UnboundedSender<Cmd>, std::sync::mpsc::Receiver<Event>) {
    let (cmds, rx) = unbounded_channel();
    let (tx, events) = std::sync::mpsc::channel();
    let me = cmds.clone();
    std::thread::Builder::new()
        .name("phone".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
            rt.block_on(async move {
                let session = match Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Event::Error(format!("No session bus: {e}")));
                        wake();
                        return;
                    }
                };
                let system = Connection::system().await.ok();
                run(Buses { system, session }, phone, rx, me, tx, wake).await;
            });
        })
        .expect("spawn the phone thread");
    (cmds, events)
}

struct Worker {
    buses: Buses,
    events: std::sync::mpsc::Sender<Event>,
    wake: Waker,
    /// Sends the worker its own commands (a pairing attempt finishing, for one).
    me: UnboundedSender<Cmd>,
    link: Link,
    gateway: Option<String>,
    calls: Vec<LiveCall>,
    map: Option<String>,
    messages: Vec<Message>,
    /// The pairing code waiting for an answer.
    confirm: Option<tokio::sync::oneshot::Sender<bool>>,
    expecting: Arc<Mutex<Option<String>>>,
    /// The microphone level before muting.
    mic: u8,
    /// Desktop notifications sent, by id: what answering one does.
    notified: HashMap<u32, String>,
}

pub async fn run(
    buses: Buses,
    phone: Option<String>,
    mut cmds: UnboundedReceiver<Cmd>,
    me: UnboundedSender<Cmd>,
    events: std::sync::mpsc::Sender<Event>,
    wake: Waker,
) {
    let (requests, mut confirmations) = unbounded_channel::<Confirmation>();
    let expecting = Arc::new(Mutex::new(None));
    // Kept alive for as long as the worker runs: it's the agent's connection.
    let _agent = match &buses.system {
        Some(_) => bluez::serve_agent(bluez::Agent { requests, expecting: expecting.clone() }).await.ok(),
        None => None,
    };
    let mut w = Worker {
        buses,
        events,
        wake,
        me,
        link: Link::default(),
        gateway: None,
        calls: Vec::new(),
        map: None,
        messages: Vec::new(),
        confirm: None,
        expecting,
        mic: 15,
        notified: HashMap::new(),
    };
    let mut telephony = stream(&w.buses.session, &calls::rule()).await;
    let mut obex_events = stream(&w.buses.session, &obex::new_message_rule()).await;
    let mut actions = stream(&w.buses.session, "type='signal',interface='org.freedesktop.Notifications',member='ActionInvoked'").await;
    let mut tick = tokio::time::interval(RELIST);
    tick.tick().await;
    let mut retry = tokio::time::interval(GATEWAY_RETRY);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    w.devices().await;
    if let Some(addr) = phone {
        w.use_phone(&addr).await;
    }
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(c) => w.handle(c).await,
                None => return,
            },
            Some(msg) = next(&mut telephony) => {
                if let Some(change) = calls::change(&msg) {
                    w.on_call_change(change).await;
                }
            }
            Some(msg) = next(&mut obex_events) => {
                // Listing announces every message it lists too: only one not yet known is new.
                // Signals that piled up meanwhile are taken together, for one listing at most.
                let mut fresh = w.is_new(&msg);
                if let Some(rx) = obex_events.as_mut() {
                    while let Ok(m) = rx.try_recv() {
                        fresh |= w.is_new(&m);
                    }
                }
                if fresh {
                    w.relist(20).await;
                }
            }
            Some(msg) = next(&mut actions) => {
                if let Ok((id, action)) = msg.body().deserialize::<(u32, String)>() {
                    w.on_action(id, &action).await;
                }
            }
            Some(c) = confirmations.recv() => {
                // A code for a pairing that has since failed or been replaced: turn it away.
                if w.expecting.lock().unwrap_or_else(|e| e.into_inner()).as_deref() != Some(c.device.as_str()) {
                    let _ = c.answer.send(false);
                    continue;
                }
                let name = w.name_of(&c.device).await;
                w.confirm = Some(c.answer);
                w.send(Event::PairCode { name, passkey: c.passkey });
            }
            _ = retry.tick(), if w.link.phone.is_some() && w.gateway.is_none() => {
                w.find_gateway().await;
                if w.gateway.is_some() {
                    w.send(Event::Link(w.link.clone()));
                }
            }
            _ = tick.tick() => {
                if w.map.is_some() {
                    w.relist(50).await;
                }
            }
        }
    }
}

/// The messages matching `rule`, read off the bus as they come by a task of their own. zbus stops
/// reading a connection while any stream on it is full, so a bounded stream the worker isn't
/// polling (it's awaiting a reply on that same connection) would hold the reply back forever:
/// listing messages makes obexd announce every one it lists, more than a bounded stream holds.
async fn stream(conn: &Connection, rule: &str) -> Option<UnboundedReceiver<zbus::message::Message>> {
    let rule = MatchRule::try_from(rule).ok()?;
    let mut s = MessageStream::for_match_rule(rule, conn, None).await.ok()?;
    let (tx, rx) = unbounded_channel();
    tokio::spawn(async move {
        while let Some(m) = s.next().await {
            if let Ok(m) = m
                && tx.send(m).is_err()
            {
                return;
            }
        }
    });
    Some(rx)
}

/// The next message of a stream that may not exist (then never ready).
async fn next(s: &mut Option<UnboundedReceiver<zbus::message::Message>>) -> Option<zbus::message::Message> {
    match s {
        Some(s) => s.recv().await,
        None => std::future::pending().await,
    }
}

impl Worker {
    /// Whether a signal from obexd announces a message this worker hasn't listed yet.
    fn is_new(&self, msg: &zbus::message::Message) -> bool {
        let Some(map) = self.map.as_deref() else { return false };
        let Ok((path, _)) = msg.body().deserialize::<(zbus::OwnedObjectPath, HashMap<String, HashMap<String, zbus::OwnedVariant>>)>() else {
            return false;
        };
        obex::is_new_message(msg, map) && !self.messages.iter().any(|k| k.id == path.as_str())
    }

    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn error(&self, e: impl Into<String>) {
        self.send(Event::Error(e.into()));
    }

    async fn handle(&mut self, cmd: Cmd) {
        let r = match cmd {
            Cmd::Discover(on) => match &self.buses.system {
                Some(sys) => {
                    let r = bluez::discover(sys, on).await;
                    if on {
                        // What turns up arrives over the next seconds.
                        for _ in 0..8 {
                            tokio::time::sleep(Duration::from_millis(1500)).await;
                            self.devices().await;
                        }
                    }
                    r
                }
                None => Err("Bluetooth isn't available".into()),
            },
            Cmd::Pair(path) => self.pair(&path).await,
            Cmd::PairDone { path, result } => self.pair_done(&path, result).await,
            Cmd::Confirm(yes) => {
                if let Some(c) = self.confirm.take() {
                    let _ = c.send(yes);
                }
                Ok(())
            }
            Cmd::Use(address) => {
                self.use_phone(&address).await;
                Ok(())
            }
            Cmd::Remove(path) => self.remove(&path).await,
            Cmd::Dial(number) => match self.gateway_now().await {
                Some(g) => calls::dial(&self.buses.session, &g, &number).await,
                None => Err("Calls need the phone connected over Bluetooth".into()),
            },
            Cmd::Answer => match self.calls.iter().find(|c| matches!(c.state, State::Incoming | State::Waiting)) {
                Some(c) => calls::answer(&self.buses.session, &c.path.clone()).await,
                None => Ok(()),
            },
            Cmd::HangUp => {
                let call = self
                    .calls
                    .iter()
                    .find(|c| matches!(c.state, State::Active | State::Dialing | State::Alerting))
                    .or_else(|| self.calls.iter().find(|c| c.state != State::Disconnected))
                    .map(|c| c.path.clone());
                let ended = match &call {
                    Some(c) => calls::hang_up(&self.buses.session, c).await,
                    None => Err("No call to end".into()),
                };
                match (ended, self.gateway_now().await) {
                    (Ok(()), _) => Ok(()),
                    (Err(_), Some(g)) => calls::hang_up_all(&self.buses.session, &g).await,
                    (Err(e), None) => if call.is_some() { Err(e) } else { Ok(()) },
                }
            }
            Cmd::Tones(t) => match &self.gateway {
                Some(g) => calls::tones(&self.buses.session, g, &t).await,
                None => Ok(()),
            },
            Cmd::Mute(on) => match &self.gateway {
                Some(g) => calls::mute(&self.buses.session, g, on, self.mic).await,
                None => Ok(()),
            },
            Cmd::Send { number, text } => self.send_sms(&number, &text).await,
            Cmd::Fetch(ids) => {
                let mut got = Vec::new();
                for id in ids {
                    if let Ok(p) = obex::get(&self.buses.session, &id).await {
                        got.push((id, p.text, p.attachment));
                    }
                }
                self.send(Event::Fetched(got));
                Ok(())
            }
            Cmd::MarkRead(ids) => self.set_read(&ids, true).await,
            Cmd::MarkUnread(ids) => self.set_read(&ids, false).await,
            Cmd::Delete(ids) => self.delete(&ids).await,
        };
        if let Err(e) = r {
            self.error(e);
        }
    }

    async fn devices(&mut self) {
        let Some(sys) = &self.buses.system else { return };
        if let Ok(list) = bluez::devices(sys).await {
            self.send(Event::Devices(list));
        }
    }

    async fn remove(&mut self, path: &str) -> Result<(), String> {
        let sys = self.buses.system.clone().ok_or("Bluetooth isn't available")?;
        let device = bluez::devices(&sys).await?.into_iter().find(|d| d.path == path).ok_or("That phone is already gone")?;
        if self.link.phone.as_ref().is_some_and(|p| p.address.eq_ignore_ascii_case(&device.address)) {
            if let Some(map) = self.map.take() {
                obex::close(&self.buses.session, &map).await;
            }
            self.gateway = None;
            self.calls.clear();
            self.messages.clear();
            self.link = Link::default();
            self.send(Event::Calls(Vec::new()));
            self.send(Event::Link(self.link.clone()));
        }
        let result = bluez::remove(&sys, path).await;
        self.devices().await;
        result?;
        self.send(Event::Removed(device));
        Ok(())
    }

    async fn name_of(&self, path: &str) -> String {
        let Some(sys) = &self.buses.system else { return String::new() };
        bluez::devices(sys).await.ok().and_then(|l| l.into_iter().find(|d| d.path == path)).map(|d| d.name).unwrap_or_default()
    }

    async fn pair(&mut self, path: &str) -> Result<(), String> {
        let sys = self.buses.system.clone().ok_or("Bluetooth isn't available")?;
        let _ = bluez::discover(&sys, false).await;
        *self.expecting.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_owned());
        // Off the worker's own loop: the pairing agent's code has to reach the window (and the
        // person's answer come back) while BlueZ waits for it.
        let path = path.to_owned();
        let me = self.me.clone();
        tokio::spawn(async move {
            let result = bluez::pair(&sys, &path).await;
            let _ = me.send(Cmd::PairDone { path, result });
        });
        Ok(())
    }

    async fn pair_done(&mut self, path: &str, result: Result<(), String>) -> Result<(), String> {
        {
            let mut expecting = self.expecting.lock().unwrap_or_else(|e| e.into_inner());
            if expecting.as_deref() == Some(path) {
                *expecting = None;
            }
        }
        if let Err(e) = result {
            self.send(Event::PairFailed);
            return Err(e);
        }
        let sys = self.buses.system.clone().ok_or("Bluetooth isn't available")?;
        let device = bluez::devices(&sys).await?.into_iter().find(|d| d.path == path).ok_or("The phone went away")?;
        self.send(Event::Paired(device.clone()));
        self.use_phone(&device.address).await;
        Ok(())
    }

    /// Connects the phone at `address` and opens what it offers: calls, messages, contacts.
    async fn use_phone(&mut self, address: &str) {
        let mut phone = None;
        if let Some(sys) = self.buses.system.clone()
            && let Ok(list) = bluez::devices(&sys).await
        {
            phone = list.into_iter().find(|d| d.address.eq_ignore_ascii_case(address));
            if let Some(p) = phone.as_mut().filter(|p| !p.connected) {
                match bluez::connect(&sys, &p.path).await {
                    Ok(()) => p.connected = true,
                    Err(e) => self.error(format!("Couldn't connect to {}: {e}", p.name)),
                }
                // Hands-Free comes up a moment after the link.
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        let phone = phone.unwrap_or(Device {
            path: String::new(),
            address: address.to_owned(),
            name: "Phone".into(),
            paired: true,
            connected: true,
            phone: true,
        });
        self.link = Link { phone: Some(phone), ..Link::default() };
        self.find_gateway().await;
        // The window shows the phone connected as soon as calls are, not once every message is in.
        self.send(Event::Link(self.link.clone()));
        self.open_messages(address).await;
        self.send(Event::Link(self.link.clone()));
        self.load_contacts(address).await;
        self.send(Event::Link(self.link.clone()));
    }

    /// The call gateway, looked for again if there's none yet (it may have just come up).
    async fn gateway_now(&mut self) -> Option<String> {
        if self.gateway.is_none() && self.link.phone.is_some() {
            self.find_gateway().await;
            if self.gateway.is_some() {
                self.send(Event::Link(self.link.clone()));
            }
        }
        self.gateway.clone()
    }

    async fn find_gateway(&mut self) {
        let Some(address) = self.link.phone.as_ref().map(|p| p.address.clone()) else {
            self.gateway = None;
            self.link.calls = false;
            return;
        };
        self.gateway = calls::gateways(&self.buses.session)
            .await
            .ok()
            .and_then(|gs| gs.into_iter().find(|(_, a)| a.eq_ignore_ascii_case(&address) || a.is_empty()).map(|(p, _)| p));
        self.link.calls = self.gateway.is_some();
        if let Some(g) = &self.gateway {
            self.calls = calls::calls(&self.buses.session, g).await.unwrap_or_default();
            self.send(Event::Calls(self.calls.clone()));
        }
    }

    async fn open_messages(&mut self, address: &str) {
        if let Some(old) = self.map.take() {
            obex::close(&self.buses.session, &old).await;
        }
        let opened = async {
            let map = obex::session(&self.buses.session, address, "map").await?;
            obex::open_messages(&self.buses.session, &map).await?;
            Ok::<_, String>(map)
        }
        .await;
        match opened {
            Ok(map) => {
                self.map = Some(map);
                self.link.messages = true;
                self.send(Event::Link(self.link.clone()));
                self.relist(MESSAGES).await;
            }
            Err(e) => self.error(format!("Messages: {e}")),
        }
    }

    /// Lists the newest `count` of the inbox and sent folders, merged into what's known.
    async fn relist(&mut self, count: u16) {
        let Some(map) = self.map.clone() else { return };
        let mut fresh: Vec<Message> = Vec::new();
        for folder in ["inbox", "sent"] {
            match obex::list(&self.buses.session, &map, folder, count).await {
                Ok(l) => fresh.extend(l.iter().map(Message::from)),
                Err(e) => {
                    // The session ended (the phone went out of range): open another next time.
                    self.map = None;
                    self.link.messages = false;
                    self.send(Event::Link(self.link.clone()));
                    self.error(format!("Messages: {e}"));
                    return;
                }
            }
        }
        let known: std::collections::HashSet<String> = self.messages.iter().map(|m| m.id.clone()).collect();
        let first = self.messages.is_empty();
        for m in fresh.iter().filter(|m| m.incoming && !m.read && !known.contains(&m.id)) {
            if !first {
                let who = if m.name.is_empty() { crate::number::pretty(&m.number) } else { m.name.clone() };
                self.notify(&who, &m.text, &[], &m.id).await;
            }
        }
        for m in fresh {
            match self.messages.iter_mut().find(|k| k.id == m.id) {
                Some(k) => {
                    // A fetched text stays, unless the listing has more.
                    let text = if k.text.len() > m.text.len() { k.text.clone() } else { m.text.clone() };
                    *k = Message { text, partial: k.partial && m.partial, ..m };
                }
                None => self.messages.push(m),
            }
        }
        self.messages.sort_by(|a, b| a.at.cmp(&b.at));
        self.send(Event::Messages(self.messages.clone()));
    }

    async fn set_read(&mut self, ids: &[String], read: bool) -> Result<(), String> {
        let mut failed = 0;
        for id in ids {
            match obex::set_read(&self.buses.session, id, read).await {
                Ok(()) => self.messages.iter_mut().filter(|m| &m.id == id).for_each(|m| m.read = read),
                Err(_) => failed += 1,
            }
        }
        if failed > 0 {
            self.send(Event::Messages(self.messages.clone()));
            return Err(format!("The phone didn't mark {failed} of them {}", if read { "read" } else { "unread" }));
        }
        Ok(())
    }

    /// Deletes on the phone, and tells the window what's left (it took them out already, so a
    /// failure puts those back).
    async fn delete(&mut self, ids: &[String]) -> Result<(), String> {
        if self.map.is_none() {
            return Err("Messages need the phone connected over Bluetooth".into());
        }
        let mut failed = 0;
        for id in ids {
            match obex::delete(&self.buses.session, id).await {
                Ok(()) => self.messages.retain(|m| &m.id != id),
                Err(_) => failed += 1,
            }
        }
        self.send(Event::Messages(self.messages.clone()));
        if failed > 0 {
            return Err(format!("The phone didn't delete {failed} of {}", ids.len()));
        }
        Ok(())
    }

    async fn send_sms(&mut self, number: &str, text: &str) -> Result<(), String> {
        let map = self.map.clone().ok_or("Messages need the phone connected over Bluetooth")?;
        obex::send(&self.buses.session, &map, number, text).await?;
        self.send(Event::Sent);
        self.relist(20).await;
        Ok(())
    }

    async fn load_contacts(&mut self, address: &str) {
        let pbap = match obex::session(&self.buses.session, address, "pbap").await {
            Ok(p) => p,
            Err(e) => return self.error(format!("Contacts: {e}")),
        };
        if let Ok(text) = obex::phonebook(&self.buses.session, &pbap, "pb").await {
            self.link.contacts = true;
            self.send(Event::Contacts(vcard::contacts(&text)));
        }
        if let Ok(text) = obex::phonebook(&self.buses.session, &pbap, "cch").await {
            self.send(Event::Recents(vcard::calls(&text)));
        }
        obex::close(&self.buses.session, &pbap).await;
    }

    async fn on_call_change(&mut self, change: Change) {
        match change {
            Change::Gateway { .. } | Change::GatewayGone(_) => {
                self.find_gateway().await;
                self.send(Event::Link(self.link.clone()));
                return;
            }
            Change::Call(c) => {
                if c.state == State::Incoming {
                    let who = if c.name.is_empty() { crate::number::pretty(&c.number) } else { c.name.clone() };
                    self.notify("Incoming call", &who, &["answer", "Answer", "decline", "Decline"], &c.path).await;
                }
                self.calls.retain(|k| k.path != c.path);
                self.calls.push(c);
            }
            Change::CallGone(path) => {
                self.calls.retain(|c| c.path != path);
                // The call is in the history now.
                if let Some(addr) = self.link.phone.as_ref().map(|p| p.address.clone()) {
                    self.send(Event::Calls(self.calls.clone()));
                    self.load_contacts(&addr).await;
                    return;
                }
            }
            Change::CallProperty { path, name, value } => calls::apply(&mut self.calls, &path, &name, &value),
        }
        self.send(Event::Calls(self.calls.clone()));
    }

    /// A desktop notification, so a call or text is seen with the window in the background.
    async fn notify(&mut self, summary: &str, body: &str, actions: &[&str], about: &str) {
        let hints: HashMap<&str, zbus::Variant> = HashMap::new();
        let reply = self
            .buses
            .session
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &("Rune Phone", 0u32, "phone", summary, body, actions, hints, -1i32),
            )
            .await;
        if let Ok(r) = reply
            && let Ok(id) = r.body().deserialize::<u32>()
        {
            self.notified.insert(id, about.to_owned());
        }
    }

    async fn on_action(&mut self, id: u32, action: &str) {
        let Some(call) = self.notified.remove(&id) else { return };
        let r = match action {
            "answer" => calls::answer(&self.buses.session, &call).await,
            "decline" => calls::hang_up(&self.buses.session, &call).await,
            _ => Ok(()),
        };
        if let Err(e) = r {
            self.error(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{self, PHONE};

    /// The worker on a thread of its own, its session bus the tests' private one.
    fn start(address: String) -> (UnboundedSender<Cmd>, std::sync::mpsc::Receiver<Event>) {
        let (cmds, rx) = unbounded_channel();
        let (tx, events) = std::sync::mpsc::channel();
        let me = cmds.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let session = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                run(Buses { system: None, session }, Some(PHONE.into()), rx, me, tx, Arc::new(|| {})).await;
            });
        });
        (cmds, events)
    }

    /// Waits (serving the fakes meanwhile) for an event `f` picks.
    async fn wait<T>(events: &std::sync::mpsc::Receiver<Event>, mut f: impl FnMut(Event) -> Option<T>) -> T {
        for _ in 0..500 {
            while let Ok(e) = events.try_recv() {
                match &e {
                    Event::Error(e) => eprintln!("worker error: {e}"),
                    Event::Link(l) => eprintln!("link {l:?}"),
                    Event::Messages(m) => eprintln!("messages {}", m.len()),
                    Event::Calls(c) => eprintln!("calls {c:?}"),
                    Event::Contacts(c) => eprintln!("contacts {}", c.len()),
                    _ => eprintln!("other event"),
                }
                if let Some(t) = f(e) {
                    return t;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the event never came");
    }

    #[tokio::test]
    async fn calls_texts_and_contacts_through_the_phones_services() {
        let bus = fakes::Bus::start();
        let fake = bus.connect().await;
        let seen = fakes::serve(&fake).await;
        fakes::add_message(&fake, &seen, 1, "inbox", "+15550100", "Are we still on for lunch?", "20261031T120000").await;
        fakes::add_message(&fake, &seen, 2, "sent", "+15550100", "Yes, noon", "20261031T120500").await;
        let (cmds, events) = start(bus.address.clone());

        let messages = wait(&events, |e| if let Event::Messages(m) = e { Some(m) } else { None }).await;
        let link = wait(&events, |e| if let Event::Link(l) = e { l.contacts.then_some(l) } else { None }).await;
        assert!(link.calls && link.messages && link.contacts);
        assert_eq!(messages.iter().map(|m| (m.text.as_str(), m.incoming)).collect::<Vec<_>>(), [("Are we still on for lunch?", true), ("Yes, noon", false)]);

        // A text sent goes to the phone's outbox as a bMessage.
        cmds.send(Cmd::Send { number: "+15550100".into(), text: "See you there".into() }).unwrap();
        wait(&events, |e| matches!(e, Event::Sent).then_some(())).await;
        let pushed = seen.lock().unwrap().pushed.clone();
        assert!(pushed[0].contains("TEL:+15550100") && pushed[0].contains("See you there"));

        // A new text: obexd announces it, the worker lists it and notifies.
        fakes::add_message(&fake, &seen, 3, "inbox", "+15550100", "Running late", "20261031T121000").await;
        let messages = wait(&events, |e| if let Event::Messages(m) = e { m.iter().any(|m| m.text == "Running late").then_some(m) } else { None }).await;
        assert_eq!(messages.len(), 3);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(seen.lock().unwrap().notified.iter().any(|(_, body)| body == "Running late"));

        // Dialling, then hanging up.
        cmds.send(Cmd::Dial("+1 (555) 010-0123".into())).unwrap();
        let calls = wait(&events, |e| if let Event::Calls(c) = e { (!c.is_empty()).then_some(c) } else { None }).await;
        assert_eq!(seen.lock().unwrap().dialed, ["+15550100123"], "dialled as digits");
        assert_eq!(calls[0].state, State::Dialing);
        cmds.send(Cmd::Tones("1#".into())).unwrap();
        cmds.send(Cmd::HangUp).unwrap();
        wait(&events, |e| if let Event::Calls(c) = e { c.is_empty().then_some(()) } else { None }).await;
        assert_eq!(seen.lock().unwrap().tones, ["1#"]);

        // A call coming in: it rings, a notification offers to answer, and answering takes it.
        fakes::add_call(&fake, &seen, "/org/pipewire/Telephony/ag0/call9", "+15550100", "incoming").await;
        wait(&events, |e| if let Event::Calls(c) = e { c.iter().any(|c| c.state == State::Incoming).then_some(()) } else { None }).await;
        cmds.send(Cmd::Answer).unwrap();
        wait(&events, |e| if let Event::Calls(c) = e { c.iter().any(|c| c.state == State::Active).then_some(()) } else { None }).await;
        assert_eq!(seen.lock().unwrap().answered, ["/org/pipewire/Telephony/ag0/call9"]);
        assert!(seen.lock().unwrap().notified.iter().any(|(s, _)| s == "Incoming call"));

        // Whole messages fetched as bMessages.
        cmds.send(Cmd::Fetch(vec!["/org/bluez/obex/client/session0/message1".into()])).unwrap();
        let fetched = wait(&events, |e| if let Event::Fetched(f) = e { Some(f) } else { None }).await;
        assert_eq!(fetched[0].1, "Are we still on for lunch?");

        // Marked unread, then deleted, on the phone.
        let first = "/org/bluez/obex/client/session0/message1".to_owned();
        cmds.send(Cmd::MarkUnread(vec![first.clone()])).unwrap();
        cmds.send(Cmd::Delete(vec![first.clone()])).unwrap();
        let left = wait(&events, |e| if let Event::Messages(m) = e { (!m.iter().any(|m| m.id == first)).then_some(m) } else { None }).await;
        assert_eq!(left.len(), 2);
        assert_eq!(seen.lock().unwrap().read, [(first.clone(), false)]);
        assert!(!seen.lock().unwrap().messages.iter().any(|(p, ..)| *p == first), "gone from the phone");
    }
}
