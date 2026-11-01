//! The app's state and what the views and keys do to it. The phone thread (`worker.rs`) does
//! the talking; this keeps what's shown.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rune::{TextEdit, Tx, Waker};
use tokio::sync::mpsc::UnboundedSender;

use crate::bluez::Device;
use crate::calls::{LiveCall, State};
use crate::number;
use crate::settings::Settings;
use crate::vcard::{Call, Contact};
use crate::worker::{Cmd, Event, Link, Message};

const STATUS_SECS: f64 = 4.0;
/// Pinned conversations: two rows of three at the top of Messages.
pub const MAX_PINS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Messages,
    Calls,
    Contacts,
}

/// What the right side shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pane {
    /// A conversation, by its number's key.
    Thread(String),
    /// A new message: the To field and the text.
    Compose,
    Keypad,
    /// A contact, by index.
    Contact(usize),
    Nothing,
}

/// Dragging a pinned conversation to a new place in the grid of them, three to a row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PinDrag {
    /// The tile being dragged, by its place before the drag.
    pub from: Option<usize>,
    /// Where it lands if dropped now.
    pub to: usize,
    /// How far the pointer has gone since the press.
    pub dx: f32,
    pub dy: f32,
    /// A tile's width and height, with the gap after it.
    step: (f32, f32),
    /// The press that ended this drag mustn't also open the conversation.
    dropped: bool,
}

/// Points the pointer travels before a press on a tile becomes a drag.
const DRAG_SLOP: f32 = 4.0;
pub const PIN_COLUMNS: usize = 3;

impl PinDrag {
    /// A new press: whatever the last one was, it can't swallow this one's press.
    pub fn pressed(&mut self) {
        self.dropped = false;
    }

    /// Where tile `index` sits now, from its own place: the dragged one with the pointer, and
    /// those between its place and where it'd land a step toward the gap it left.
    pub fn shift(&self, index: usize) -> (f32, f32) {
        let Some(from) = self.from else { return (0.0, 0.0) };
        if index == from {
            return (self.dx, self.dy);
        }
        let to = self.to;
        let slot = if from < index && index <= to {
            index - 1
        } else if to <= index && index < from {
            index + 1
        } else {
            index
        };
        let cell = |i: usize| ((i % PIN_COLUMNS) as f32, (i / PIN_COLUMNS) as f32);
        let ((x0, y0), (x1, y1)) = (cell(index), cell(slot));
        ((x1 - x0) * self.step.0, (y1 - y0) * self.step.1)
    }
}

/// One conversation: the messages with one number.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub key: String,
    pub number: String,
    pub name: String,
    /// Oldest first.
    pub messages: Vec<Message>,
    pub unread: usize,
}

impl Thread {
    pub fn last(&self) -> Option<&Message> {
        self.messages.last()
    }
}

pub struct App {
    pub settings: Settings,
    pub link: Link,
    pub devices: Vec<Device>,
    pub discovering: bool,
    /// Pairing: the phone's name and the code both screens show.
    pub pair_code: Option<(String, u32)>,
    pub pairing: Option<String>,
    /// A phone's address, asking whether to really remove it.
    pub removing: Option<String>,
    pub pin_drag: PinDrag,
    /// The phone thread is still on its first try at reaching the phone.
    pub connecting: bool,
    /// Where the keys act, and the cursor in each place.
    pub focus: crate::actions::Focus,
    pub cursor: Option<String>,
    pub msg_cursor: Option<String>,
    pub picked: crate::actions::Picked,
    pub swipe: crate::actions::Swipe,
    /// A swipe just let go: the click that may follow it is swallowed.
    pub swiped: bool,
    /// Deleting, waiting for a yes.
    pub confirm: Option<crate::actions::Confirm>,
    /// The first key of a two-key command (gg, dd).
    pub pending_key: Option<char>,
    /// The list of keys is showing.
    pub show_keys: bool,
    pub tab: Tab,
    pub pane: Pane,
    pub messages: Vec<Message>,
    pub threads: Vec<Thread>,
    pub contacts: Vec<Contact>,
    names: HashMap<String, String>,
    pub recents: Vec<Call>,
    pub calls: Vec<LiveCall>,
    /// When the current call was answered, on the app's clock.
    pub call_started: Option<f64>,
    pub muted: bool,
    pub call_keypad: bool,
    pub search: TextEdit,
    pub draft: TextEdit,
    pub to: TextEdit,
    pub dial: TextEdit,
    pub sending: bool,
    pub status: Option<(String, f64)>,
    pub clock: f64,
    /// The setup screen is open over a connected phone (to switch or pair).
    pub setup_open: bool,
    /// A headless demo scene: no phone thread, so nothing touches the real Bluetooth.
    pub demo: bool,
    cmds: Option<UnboundedSender<Cmd>>,
    events: Option<Receiver<Event>>,
}

pub fn init() -> App {
    App {
        settings: Settings::load(),
        link: Link::default(),
        devices: Vec::new(),
        discovering: false,
        pair_code: None,
        pairing: None,
        removing: None,
        pin_drag: PinDrag::default(),
        connecting: false,
        focus: crate::actions::Focus::List,
        cursor: None,
        msg_cursor: None,
        picked: crate::actions::Picked::default(),
        swipe: crate::actions::Swipe::default(),
        swiped: false,
        confirm: None,
        pending_key: None,
        show_keys: false,
        tab: Tab::Messages,
        pane: Pane::Nothing,
        messages: Vec::new(),
        threads: Vec::new(),
        contacts: Vec::new(),
        names: HashMap::new(),
        recents: Vec::new(),
        calls: Vec::new(),
        call_started: None,
        muted: false,
        call_keypad: false,
        search: TextEdit::new(""),
        draft: TextEdit::new(""),
        to: TextEdit::new(""),
        dial: TextEdit::new(""),
        sending: false,
        status: None,
        clock: 0.0,
        setup_open: false,
        demo: false,
        cmds: None,
        events: None,
    }
}

pub fn setup(s: &mut App, wake: Waker) {
    if s.demo {
        return;
    }
    // What the phone showed last time, at once; the phone thread brings it up to date.
    if let Some(address) = s.settings.phone.clone() {
        s.connecting = true;
        if let Some(c) = crate::cache::load(&address) {
            s.link.phone = c.phone.map(|p| Device { connected: false, ..p });
            s.recents = c.recents;
            s.take_contacts(c.contacts);
            s.take_messages(c.messages);
        }
    }
    let (cmds, events) = crate::worker::spawn(wake, s.settings.phone.clone());
    s.cmds = Some(cmds);
    s.events = Some(events);
}

pub fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = false;
    let events: Vec<Event> = s.events.as_ref().map(|e| e.try_iter().collect()).unwrap_or_default();
    for e in events {
        s.apply(e, tx);
        changed = true;
    }
    if s.swipe_tick(tx) {
        changed = true;
    }
    if s.status.as_ref().is_some_and(|(_, until)| clock >= *until) {
        s.status = None;
        changed = true;
    }
    // A call's timer ticks.
    if s.call_started.is_some() {
        tx.wake_after(Duration::from_secs(1));
        changed = true;
    }
    changed
}

impl App {
    pub fn send_cmd(&self, cmd: Cmd) {
        if let Some(c) = &self.cmds {
            let _ = c.send(cmd);
        }
    }

    pub fn say(&mut self, text: impl Into<String>, tx: &mut Tx) {
        self.status = Some((text.into(), self.clock + STATUS_SECS));
        tx.wake_after(Duration::from_secs_f64(STATUS_SECS));
    }

    fn apply(&mut self, e: Event, tx: &mut Tx) {
        let keep = matches!(e, Event::Messages(_) | Event::Fetched(_) | Event::Contacts(_) | Event::Recents(_) | Event::Link(_));
        self.apply_event(e, tx);
        if keep {
            self.keep();
        }
    }

    /// Saves what the window shows of the phone, for the next start.
    /// Only what came from the phone replaces what was kept: a part it didn't give this time
    /// (messages it didn't allow, say) stays as it was.
    fn keep(&self) {
        let Some(phone) = self.link.phone.clone().filter(|_| !self.demo) else { return };
        crate::cache::save(crate::cache::Update {
            phone,
            messages: self.link.messages.then(|| self.messages.clone()),
            contacts: self.link.contacts.then(|| self.contacts.clone()),
            recents: self.link.contacts.then(|| self.recents.clone()),
        });
    }

    fn apply_event(&mut self, e: Event, tx: &mut Tx) {
        match e {
            Event::Devices(d) => self.devices = d,
            Event::Link(l) => {
                self.connecting = false;
                if let Some(p) = &l.phone
                    && self.settings.phone.as_ref() != Some(&p.address)
                {
                    self.settings.phone = Some(p.address.clone());
                    let _ = self.settings.save();
                }
                if l.calls || l.messages {
                    self.setup_open = false;
                }
                self.link = l;
            }
            Event::PairCode { name, passkey } => self.pair_code = Some((name, passkey)),
            Event::Paired(d) => {
                self.pairing = None;
                self.pair_code = None;
                self.discovering = false;
                self.say(format!("Paired with {}", d.name), tx);
            }
            Event::Removed(d) => {
                self.removing = None;
                if self.settings.phone.as_ref().is_some_and(|a| a.eq_ignore_ascii_case(&d.address)) {
                    self.settings.phone = None;
                    let _ = self.settings.save();
                    crate::cache::clear();
                    self.messages.clear();
                    self.threads.clear();
                    self.take_contacts(Vec::new());
                    self.recents.clear();
                    self.pane = Pane::Nothing;
                }
                self.say(format!("Removed {}", if d.name.is_empty() { &d.address } else { &d.name }), tx);
            }
            Event::PairFailed => {
                self.pairing = None;
                self.pair_code = None;
            }
            Event::Calls(c) => {
                let live = c.iter().any(|c| c.state == State::Active);
                match (live, self.call_started) {
                    (true, None) => self.call_started = Some(self.clock),
                    (false, _) if c.is_empty() => {
                        self.call_started = None;
                        self.muted = false;
                        self.call_keypad = false;
                    }
                    _ => {}
                }
                self.calls = c;
            }
            Event::Messages(m) => self.take_messages(m),
            Event::Fetched(got) => {
                for (id, text, attachment) in got {
                    if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
                        if !text.is_empty() {
                            m.text = text;
                        }
                        m.partial = false;
                        m.mms |= attachment;
                    }
                }
                self.rethread();
            }
            Event::Sent => {
                self.sending = false;
                self.draft = TextEdit::new("");
                self.draft.focused = true;
            }
            Event::Contacts(c) => self.take_contacts(c),
            Event::Recents(r) => self.recents = r,
            Event::Error(e) => {
                self.sending = false;
                self.pairing = None;
                self.say(e, tx);
            }
        }
    }

    /// Groups the messages into conversations, newest first.
    pub(crate) fn rethread(&mut self) {
        let mut by: HashMap<String, Thread> = HashMap::new();
        for m in &self.messages {
            let key = if m.number.is_empty() { m.name.clone() } else { number::key(&m.number) };
            let t = by.entry(key.clone()).or_insert_with(|| Thread {
                key: key.clone(),
                number: m.number.clone(),
                name: String::new(),
                messages: Vec::new(),
                unread: 0,
            });
            t.unread += usize::from(m.incoming && !m.read);
            if t.name.is_empty() && !m.name.is_empty() {
                t.name = m.name.clone();
            }
            t.messages.push(m.clone());
        }
        let mut threads: Vec<Thread> = by.into_values().collect();
        for t in &mut threads {
            t.messages.sort_by(|a, b| a.at.cmp(&b.at));
            if let Some(n) = self.names.get(&t.key) {
                t.name = n.clone();
            }
        }
        threads.sort_by(|a, b| b.last().map(|m| m.at.as_str()).cmp(&a.last().map(|m| m.at.as_str())));
        self.threads = threads;
    }

    /// A contact's name for a number, else the number laid out.
    pub fn who(&self, n: &str) -> String {
        self.names.get(&number::key(n)).cloned().unwrap_or_else(|| number::pretty(n))
    }

    pub fn thread(&self, key: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.key == key)
    }

    /// The conversations in the list: all that match a search, or else all but the pinned.
    pub fn visible_threads(&self) -> Vec<&Thread> {
        let q = self.search.text.trim().to_lowercase();
        self.threads
            .iter()
            .filter(|t| if q.is_empty() { !self.is_pinned(&t.key) } else { t.name.to_lowercase().contains(&q) || t.number.contains(&q) || t.messages.iter().any(|m| m.text.to_lowercase().contains(&q)) })
            .collect()
    }

    /// The pinned conversations there are messages for, in the order pinned.
    pub fn pinned_threads(&self) -> Vec<&Thread> {
        self.settings.pinned.iter().filter_map(|k| self.thread(k)).collect()
    }

    pub fn is_pinned(&self, key: &str) -> bool {
        self.settings.pinned.iter().any(|k| k == key)
    }

    /// A press on pinned tile `index` (of `count`, each `w` by `h` with `gap` between) moved to
    /// `dx, dy`: past a few points it's a drag, and the tile it's over is where it would land.
    pub fn drag_pin(&mut self, index: usize, count: usize, dx: f32, dy: f32, size: (f32, f32), gap: f32) {
        let d = &mut self.pin_drag;
        if d.from.is_none() {
            if dx.hypot(dy) < DRAG_SLOP {
                return;
            }
            *d = PinDrag { from: Some(index), to: index, step: (size.0 + gap, size.1 + gap), ..PinDrag::default() };
        }
        (d.dx, d.dy) = (dx, dy);
        let col = (index % PIN_COLUMNS) as f32 + (dx / d.step.0).round();
        let row = (index / PIN_COLUMNS) as f32 + (dy / d.step.1).round();
        let rows = count.div_ceil(PIN_COLUMNS);
        let (col, row) = (col.clamp(0.0, (PIN_COLUMNS - 1) as f32) as usize, row.clamp(0.0, rows.saturating_sub(1) as f32) as usize);
        d.to = (row * PIN_COLUMNS + col).min(count.saturating_sub(1));
    }

    /// The drag let go: the tile moves to where it was over, unless the drag was called off.
    pub fn drop_pin(&mut self, cancelled: bool) {
        let Some(from) = self.pin_drag.from.take() else { return };
        let to = self.pin_drag.to;
        self.pin_drag = PinDrag { dropped: true, ..PinDrag::default() };
        if cancelled || from == to {
            return;
        }
        // The order shown is of the pins with messages; any others keep their place after them.
        let mut shown: Vec<String> = self.pinned_threads().iter().map(|t| t.key.clone()).collect();
        if from >= shown.len() || to >= shown.len() {
            return;
        }
        let key = shown.remove(from);
        shown.insert(to, key);
        let rest: Vec<String> = self.settings.pinned.iter().filter(|k| !shown.contains(k)).cloned().collect();
        self.settings.pinned = shown.into_iter().chain(rest).collect();
        if !self.demo {
            let _ = self.settings.save();
        }
    }

    /// A pinned tile was pressed: it opens, unless the press is the end of a drag.
    pub fn press_pin(&mut self, key: &str) {
        if std::mem::take(&mut self.pin_drag.dropped) {
            return;
        }
        self.focus = crate::actions::Focus::List;
        self.cursor = Some(key.to_owned());
        self.clear_picks();
        self.open_thread(key);
    }

    /// Pins the conversation at the top of Messages, or unpins it.
    pub fn toggle_pin(&mut self, key: &str, tx: &mut Tx) {
        if self.is_pinned(key) {
            self.settings.pinned.retain(|k| k != key);
        } else {
            // A pin whose conversation is gone (deleted on the phone) gives up its place.
            if !self.threads.is_empty() {
                let threads = &self.threads;
                self.settings.pinned.retain(|k| threads.iter().any(|t| &t.key == k));
            }
            if self.settings.pinned.len() >= MAX_PINS {
                self.say(format!("{MAX_PINS} conversations fit at the top: unpin one first"), tx);
                return;
            }
            self.settings.pinned.push(key.to_owned());
        }
        if !self.demo {
            let _ = self.settings.save();
        }
    }

    pub fn visible_contacts(&self) -> Vec<(usize, &Contact)> {
        let q = self.search.text.trim().to_lowercase();
        self.contacts
            .iter()
            .enumerate()
            .filter(|(_, c)| q.is_empty() || c.name.to_lowercase().contains(&q) || c.numbers.iter().any(|(_, n)| n.contains(&q)))
            .collect()
    }

    /// Takes contacts as the phone thread would hand them over: demo scenes.
    pub fn take_contacts(&mut self, c: Vec<Contact>) {
        self.names = c.iter().flat_map(|c| c.numbers.iter().map(|(_, n)| (number::key(n), c.name.clone()))).collect();
        self.contacts = c;
        self.contacts.sort_by_key(|c| c.name.to_lowercase());
        self.rethread();
    }

    pub fn take_messages(&mut self, mut m: Vec<Message>) {
        // A text fetched whole stays whole when a listing (which cuts texts short) comes again.
        // Messages are matched by the phone's handle: the session part of the id changes with
        // each connection, so the cached ones still match.
        let handle = |id: &str| id.rsplit('/').next().unwrap_or(id).to_owned();
        let whole: HashMap<String, String> =
            self.messages.iter().filter(|k| !k.partial).map(|k| (handle(&k.id), k.text.clone())).collect();
        for n in m.iter_mut().filter(|n| n.partial) {
            if let Some(text) = whole.get(&handle(&n.id)).filter(|t| t.len() >= n.text.len()) {
                n.text = text.clone();
                n.partial = false;
            }
        }
        self.messages = m;
        self.rethread();
    }

    // Messages

    pub fn open_thread(&mut self, key: &str) {
        self.tab = Tab::Messages;
        self.pane = Pane::Thread(key.to_owned());
        self.draft.focused = true;
        // Kept messages from before the phone connected can't be fetched or marked read yet.
        if !self.link.messages {
            return;
        }
        let Some(t) = self.thread(key) else { return };
        let partial: Vec<String> = t.messages.iter().filter(|m| m.partial).map(|m| m.id.clone()).collect();
        let unread: Vec<String> = t.messages.iter().filter(|m| m.incoming && !m.read).map(|m| m.id.clone()).collect();
        if !partial.is_empty() {
            self.send_cmd(Cmd::Fetch(partial));
        }
        if !unread.is_empty() {
            for m in self.messages.iter_mut().filter(|m| unread.contains(&m.id)) {
                m.read = true;
            }
            self.send_cmd(Cmd::MarkRead(unread));
            self.rethread();
        }
    }

    /// Opens the conversation with `number`, or a new one to it.
    pub fn message(&mut self, n: &str) {
        let key = number::key(n);
        if self.thread(&key).is_some() {
            self.open_thread(&key);
        } else {
            self.tab = Tab::Messages;
            self.pane = Pane::Compose;
            self.to = TextEdit::new(n);
            self.draft.focused = true;
        }
    }

    pub fn compose(&mut self) {
        self.tab = Tab::Messages;
        self.pane = Pane::Compose;
        self.to = TextEdit::new("");
        self.to.focused = true;
    }

    /// Sends the draft to the open conversation's number, or to the To field's.
    pub fn send_draft(&mut self, tx: &mut Tx) {
        let text = self.draft.text.trim().to_owned();
        if text.is_empty() || self.sending {
            return;
        }
        let n = match &self.pane {
            Pane::Thread(key) => self.thread(key).map(|t| t.number.clone()).unwrap_or_default(),
            Pane::Compose => self.resolve_to(),
            _ => return,
        };
        if number::dialable(&n).is_none() {
            self.say("Who to? Type a number or a contact's name", tx);
            return;
        }
        self.sending = true;
        if self.pane == Pane::Compose {
            self.pane = Pane::Thread(number::key(&n));
        }
        self.send_cmd(Cmd::Send { number: n, text });
    }

    /// The To field as a number: as typed, or the first number of the contact it names.
    pub fn resolve_to(&self) -> String {
        let typed = self.to.text.trim();
        if number::dialable(typed).is_some_and(|d| d.len() >= 3) && typed.chars().all(|c| !c.is_alphabetic()) {
            return typed.to_owned();
        }
        let q = typed.to_lowercase();
        self.contacts.iter().find(|c| c.name.to_lowercase().starts_with(&q)).and_then(|c| c.numbers.first()).map(|(_, n)| n.clone()).unwrap_or_default()
    }

    // Calls

    pub fn call(&mut self, n: &str, tx: &mut Tx) {
        if !self.link.calls {
            self.say("Calls need the phone connected over Bluetooth", tx);
            return;
        }
        self.send_cmd(Cmd::Dial(n.to_owned()));
    }

    pub fn ringing(&self) -> Option<&LiveCall> {
        self.calls.iter().find(|c| matches!(c.state, State::Incoming | State::Waiting))
    }

    /// The call in progress: active, being dialled, or ringing at the other end.
    pub fn current(&self) -> Option<&LiveCall> {
        self.calls
            .iter()
            .find(|c| matches!(c.state, State::Active | State::Dialing | State::Alerting | State::Held))
    }

    pub fn press_key(&mut self, k: &str) {
        if self.current().is_some() {
            self.send_cmd(Cmd::Tones(k.to_owned()));
        } else {
            self.dial.insert(k);
        }
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.send_cmd(Cmd::Mute(self.muted));
    }

    // Setup

    pub fn open_setup(&mut self) {
        self.setup_open = true;
        self.send_cmd(Cmd::Discover(false));
    }

    pub fn discover(&mut self) {
        self.discovering = true;
        self.send_cmd(Cmd::Discover(true));
    }

    pub fn pair(&mut self, d: &Device) {
        self.pairing = Some(d.path.clone());
        self.send_cmd(Cmd::Pair(d.path.clone()));
    }

    pub fn use_phone(&mut self, d: &Device) {
        self.send_cmd(Cmd::Use(d.address.clone()));
        self.setup_open = false;
    }

    /// Unpairs the phone (the setup screen asks first, with `removing`).
    pub fn remove_phone(&mut self, d: &Device) {
        self.removing = None;
        self.send_cmd(Cmd::Remove(d.path.clone()));
    }

    pub fn confirm(&mut self, yes: bool) {
        self.pair_code = None;
        if !yes {
            self.pairing = None;
        }
        self.send_cmd(Cmd::Confirm(yes));
    }

    /// The setup screen shows: no phone yet, or it was asked for.
    pub fn needs_setup(&self) -> bool {
        self.setup_open || self.settings.phone.is_none()
    }
}

/// "9:41 AM" today, "Yesterday", a weekday this week, else "31 Oct" (or with the year).
pub fn when(at: &str) -> String {
    let Ok(t) = chrono::NaiveDateTime::parse_from_str(at.get(..15).unwrap_or(at), "%Y%m%dT%H%M%S") else { return String::new() };
    let today = chrono::Local::now().date_naive();
    let days = (today - t.date()).num_days();
    match days {
        0 => t.format("%-I:%M %p").to_string(),
        1 => "Yesterday".into(),
        2..=6 => t.format("%A").to_string(),
        _ if t.date().format("%Y").to_string() == today.format("%Y").to_string() => t.format("%-d %b").to_string(),
        _ => t.format("%-d %b %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, number: &str, text: &str, at: &str, incoming: bool) -> Message {
        Message { id: id.into(), number: number.into(), name: String::new(), text: text.into(), at: at.into(), incoming, read: !incoming, mms: false, partial: false }
    }

    #[test]
    fn threads_group_by_line_and_take_contact_names() {
        crate::settings::use_scratch_dirs();
        let mut s = init();
        s.contacts = vec![Contact { name: "Ada".into(), numbers: vec![("mobile".into(), "+1 555 010 0100".into())] }];
        s.names = s.contacts.iter().flat_map(|c| c.numbers.iter().map(|(_, n)| (number::key(n), c.name.clone()))).collect();
        s.messages = vec![
            msg("1", "5550100100", "hi", "20261031T100000", true),
            msg("2", "+15550100100", "hello", "20261031T100100", false),
            msg("3", "5559999999", "other", "20261030T090000", true),
        ];
        s.rethread();
        assert_eq!(s.threads.len(), 2);
        assert_eq!(s.threads[0].name, "Ada", "the newest first, by its contact's name");
        assert_eq!(s.threads[0].messages.len(), 2);
        assert_eq!(s.threads[0].unread, 1);
        s.to = TextEdit::new("ad");
        assert_eq!(s.resolve_to(), "+1 555 010 0100");
        s.to = TextEdit::new("555 1234");
        assert_eq!(s.resolve_to(), "555 1234");
    }

    #[test]
    fn pinned_tiles_drag_into_a_new_order() {
        let mut s = init();
        crate::demo::pinned(&mut s);
        let order = |s: &App| s.pinned_threads().iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        let before = order(&s);
        assert_eq!(before.len(), 5);
        let size = (100.0, 80.0);

        s.drag_pin(0, 5, 2.0, 1.0, size, 2.0);
        assert_eq!(s.pin_drag.from, None, "inside the slop it's still a click");

        // The first tile, dragged a row down and one across: over the fifth.
        s.drag_pin(0, 5, 103.0, 85.0, size, 2.0);
        assert_eq!((s.pin_drag.from, s.pin_drag.to), (Some(0), 4));
        assert_eq!(s.pin_drag.shift(0), (103.0, 85.0), "the dragged one follows the pointer");
        assert_eq!(s.pin_drag.shift(3), (204.0, -82.0), "the fourth wraps back to the end of the first row");
        assert_eq!(s.pin_drag.shift(1), (-102.0, 0.0));
        s.drop_pin(false);
        let after = order(&s);
        assert_eq!(after, [before[1].clone(), before[2].clone(), before[3].clone(), before[4].clone(), before[0].clone()]);

        s.press_pin(&s.pinned_threads()[0].key.clone());
        assert_eq!(s.pane, Pane::Thread(crate::number::key("+15550100100")), "the press ending a drag doesn't open the tile under it");

        s.pin_drag.pressed();
        let key = s.pinned_threads()[0].key.clone();
        s.press_pin(&key);
        assert_eq!(s.pane, Pane::Thread(key), "the next click does");

        s.drag_pin(4, 5, -300.0, -200.0, size, 2.0);
        assert_eq!(s.pin_drag.to, 0, "clamped to the grid");
        s.drop_pin(true);
        assert_eq!(order(&s), after, "a cancel moves nothing");
    }

    #[test]
    fn a_whole_text_outlasts_a_new_session_listing_it_short() {
        let mut s = init();
        s.settings = Settings::default();
        let mut whole = msg("/org/bluez/obex/client/session1/message77", "+15550100100", "The whole of a long text", "20260101T100000", true);
        whole.partial = false;
        s.take_messages(vec![whole]);
        let mut short = msg("/org/bluez/obex/client/session9/message77", "+15550100100", "The whole", "20260101T100000", true);
        short.partial = true;
        s.take_messages(vec![short]);
        assert_eq!((s.messages[0].text.as_str(), s.messages[0].partial), ("The whole of a long text", false));
        assert_eq!(s.messages[0].id, "/org/bluez/obex/client/session9/message77", "the id is the new session's");
    }
}
