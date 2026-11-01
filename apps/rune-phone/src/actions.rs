//! Acting on conversations and messages: picking many (Ctrl+click, Shift+click, or the keys),
//! moving through them with vim's keys, and swiping one: left to delete, right to mark it read
//! or unread. Deleting asks first; it deletes on the phone.

use std::collections::BTreeSet;

use rune::{Key, KeyEvent, Tx};

use crate::app::{App, MAX_PINS, Pane, Tab};
use crate::worker::{Cmd, Message};

/// Where the keys act: the conversations down the side, or the messages of the open one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    List,
    Thread,
}

/// A conversation (by its key) or a message (by its id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Thread(String),
    Message(String),
}

/// A row being swiped sideways, by a drag or two fingers on a trackpad.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Swipe {
    pub target: Option<Target>,
    /// How far it's gone: left negative (delete), right positive (read or unread).
    pub dx: f32,
    /// The trackpad's horizontal travel so far, before it's sure to be a swipe.
    pending: f32,
    /// When the trackpad last moved it, on the app's clock: not every system says when the
    /// fingers lift, so a pause ends it.
    wheel_at: Option<f64>,
}

/// Seconds without a trackpad move that end a two-finger swipe.
const WHEEL_REST: f64 = 0.35;

/// Past this far, letting go acts.
pub const SWIPE_ACTS: f32 = 90.0;
/// A row doesn't go further than this.
const SWIPE_MAX: f32 = 150.0;
/// Points sideways before a drag or two-finger move is a swipe and not a click or a scroll.
const SWIPE_SLOP: f32 = 8.0;

/// What's picked: conversations in the list, messages in the open one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    pub threads: BTreeSet<String>,
    pub messages: BTreeSet<String>,
}

/// Deleting, waiting for a yes: the messages, and what to call them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub ids: Vec<String>,
    pub what: String,
}

/// A message's id is its session's path; its handle (the last part) is what a key can use.
pub fn handle(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

impl App {
    // Where the keys go

    /// The conversations in the order the keys move through them: the pinned, then the list.
    pub fn nav_threads(&self) -> Vec<String> {
        let mut keys: Vec<String> =
            if self.search.text.trim().is_empty() { self.pinned_threads().iter().map(|t| t.key.clone()).collect() } else { Vec::new() };
        keys.extend(self.visible_threads().iter().map(|t| t.key.clone()));
        keys
    }

    fn open_key(&self) -> Option<String> {
        if let Pane::Thread(k) = &self.pane { Some(k.clone()) } else { None }
    }

    /// The messages of the open conversation, oldest first.
    pub fn open_messages(&self) -> Vec<&Message> {
        self.open_key().and_then(|k| self.thread(&k)).map(|t| t.messages.iter().collect()).unwrap_or_default()
    }

    fn reveal(&self, tx: &mut Tx) {
        match self.focus {
            Focus::List => {
                if let Some(k) = &self.cursor {
                    tx.scroll_into_view(format!("thread-{k}"));
                }
            }
            Focus::Thread => {
                if let Some(id) = &self.msg_cursor {
                    tx.scroll_into_view(format!("msg-{}", handle(id)));
                }
            }
        }
    }

    /// Moves the cursor `by` rows (clamped), extending the pick on the way when `pick`.
    fn step(&mut self, by: isize, pick: bool, tx: &mut Tx) {
        match self.focus {
            Focus::List => {
                let keys = self.nav_threads();
                let at = self.cursor.as_ref().and_then(|c| keys.iter().position(|k| k == c));
                let next = moved(at, by, keys.len());
                if pick && let Some(i) = at {
                    self.picked.threads.insert(keys[i].clone());
                }
                self.cursor = next.map(|i| keys[i].clone());
                if pick && let Some(c) = &self.cursor {
                    self.picked.threads.insert(c.clone());
                }
            }
            Focus::Thread => {
                let ids: Vec<String> = self.open_messages().iter().map(|m| m.id.clone()).collect();
                // From nothing, up starts at the newest: the bottom, where the thread opens.
                let at = self.msg_cursor.as_ref().and_then(|c| ids.iter().position(|k| k == c));
                let next = if at.is_none() && by < 0 { ids.len().checked_sub(1) } else { moved(at, by, ids.len()) };
                if pick && let Some(i) = at {
                    self.picked.messages.insert(ids[i].clone());
                }
                self.msg_cursor = next.map(|i| ids[i].clone());
                if pick && let Some(c) = &self.msg_cursor {
                    self.picked.messages.insert(c.clone());
                }
            }
        }
        self.reveal(tx);
    }

    fn step_to_end(&mut self, last: bool, tx: &mut Tx) {
        let big = if last { isize::MAX / 2 } else { isize::MIN / 2 };
        if self.focus == Focus::Thread && self.msg_cursor.is_none() {
            self.step(-1, false, tx);
        }
        self.step(big, false, tx);
    }

    /// Opens the conversation under the cursor, for the keys to move through its messages.
    fn open_cursor(&mut self) {
        let Some(k) = self.cursor.clone() else { return };
        self.open_thread(&k);
        self.draft.focused = false;
        self.focus = Focus::Thread;
        self.msg_cursor = None;
        self.picked.messages.clear();
    }

    // Picking

    pub fn clear_picks(&mut self) {
        self.picked = Picked::default();
    }

    fn toggle(set: &mut BTreeSet<String>, item: &str) {
        if !set.remove(item) {
            set.insert(item.to_owned());
        }
    }

    /// A click on a conversation: Ctrl picks or unpicks it, Shift picks from the cursor to it,
    /// a plain click opens it (and lets go of the picked).
    pub fn click_thread(&mut self, key: &str, tx: &mut Tx) {
        if std::mem::take(&mut self.swiped) {
            return;
        }
        let m = tx.mods();
        self.focus = Focus::List;
        if m.cmd || m.ctrl {
            Self::toggle(&mut self.picked.threads, key);
        } else if m.shift {
            let keys = self.nav_threads();
            let from = self.cursor.as_ref().and_then(|c| keys.iter().position(|k| k == c)).unwrap_or(0);
            if let Some(to) = keys.iter().position(|k| k == key) {
                self.picked.threads.extend(keys[from.min(to)..=from.max(to)].iter().cloned());
            }
            return;
        } else {
            self.clear_picks();
            self.open_thread(key);
        }
        self.cursor = Some(key.to_owned());
    }

    /// A click on a message: Ctrl picks or unpicks it, Shift picks from the cursor to it, and a
    /// plain click moves the cursor there (letting go of the picked).
    pub fn click_message(&mut self, id: &str, tx: &mut Tx) {
        if std::mem::take(&mut self.swiped) {
            return;
        }
        let m = tx.mods();
        self.focus = Focus::Thread;
        if m.cmd || m.ctrl {
            Self::toggle(&mut self.picked.messages, id);
        } else if m.shift {
            let ids: Vec<String> = self.open_messages().iter().map(|m| m.id.clone()).collect();
            let from = self.msg_cursor.as_ref().and_then(|c| ids.iter().position(|k| k == c)).unwrap_or(ids.len().saturating_sub(1));
            if let Some(to) = ids.iter().position(|k| k == id) {
                self.picked.messages.extend(ids[from.min(to)..=from.max(to)].iter().cloned());
            }
            return;
        } else {
            self.picked.messages.clear();
        }
        self.msg_cursor = Some(id.to_owned());
    }

    fn pick_all(&mut self) {
        match self.focus {
            Focus::List => self.picked.threads = self.nav_threads().into_iter().collect(),
            Focus::Thread => self.picked.messages = self.open_messages().iter().map(|m| m.id.clone()).collect(),
        }
    }

    /// The picked conversations, for the list's bar.
    pub fn picked_threads(&self) -> Vec<Target> {
        self.picked.threads.iter().cloned().map(Target::Thread).collect()
    }

    /// The picked messages, for the conversation's bar.
    pub fn picked_messages(&self) -> Vec<Target> {
        self.picked.messages.iter().cloned().map(Target::Message).collect()
    }

    /// What an action applies to: what's picked where the keys are, or else the cursor's.
    pub fn chosen(&self) -> Vec<Target> {
        match self.focus {
            Focus::List if !self.picked.threads.is_empty() => self.picked.threads.iter().cloned().map(Target::Thread).collect(),
            Focus::Thread if !self.picked.messages.is_empty() => self.picked.messages.iter().cloned().map(Target::Message).collect(),
            Focus::List => self.cursor.iter().cloned().map(Target::Thread).collect(),
            Focus::Thread => self.msg_cursor.iter().cloned().map(Target::Message).collect(),
        }
    }

    /// The messages behind some targets.
    fn messages_of(&self, targets: &[Target]) -> Vec<&Message> {
        let mut out = Vec::new();
        for t in targets {
            match t {
                Target::Thread(k) => out.extend(self.thread(k).map(|t| t.messages.iter()).into_iter().flatten()),
                Target::Message(id) => out.extend(self.messages.iter().find(|m| &m.id == id)),
            }
        }
        out
    }

    // Acting

    fn connected(&mut self, tx: &mut Tx) -> bool {
        if !self.link.messages {
            self.say("Messages need the phone connected", tx);
        }
        self.link.messages
    }

    /// Marks them read if any is unread, else unread: a conversation, by its newest text in.
    pub fn toggle_read(&mut self, targets: &[Target], tx: &mut Tx) {
        if targets.is_empty() || !self.connected(tx) {
            return;
        }
        let incoming: Vec<&Message> = self.messages_of(targets).into_iter().filter(|m| m.incoming).collect();
        let unread: Vec<String> = incoming.iter().filter(|m| !m.read).map(|m| m.id.clone()).collect();
        let (ids, read) = if !unread.is_empty() {
            (unread, true)
        } else {
            // All read: a conversation goes unread by its newest text in, a message by itself.
            let mut ids = Vec::new();
            for t in targets {
                match t {
                    Target::Message(id) => ids.extend(incoming.iter().filter(|m| &m.id == id).map(|m| m.id.clone())),
                    Target::Thread(k) => ids.extend(
                        self.thread(k).and_then(|t| t.messages.iter().rev().find(|m| m.incoming)).map(|m| m.id.clone()),
                    ),
                }
            }
            (ids, false)
        };
        if ids.is_empty() {
            return;
        }
        for m in self.messages.iter_mut().filter(|m| ids.contains(&m.id)) {
            m.read = read;
        }
        self.rethread();
        let n = ids.len();
        self.send_cmd(if read { Cmd::MarkRead(ids) } else { Cmd::MarkUnread(ids) });
        self.say(format!("Marked {n} {} {}", if n == 1 { "message" } else { "messages" }, if read { "read" } else { "unread" }), tx);
    }

    /// Pins the conversations if any isn't pinned (as many as fit), else unpins them all.
    pub fn toggle_pins(&mut self, targets: &[Target], tx: &mut Tx) {
        let keys: Vec<String> = targets.iter().filter_map(|t| if let Target::Thread(k) = t { Some(k.clone()) } else { None }).collect();
        if keys.is_empty() {
            return;
        }
        if keys.iter().all(|k| self.is_pinned(k)) {
            keys.iter().for_each(|k| self.toggle_pin(k, tx));
            return;
        }
        let room = MAX_PINS.saturating_sub(self.pinned_threads().len());
        let to_pin: Vec<&String> = keys.iter().filter(|k| !self.is_pinned(k)).collect();
        if to_pin.len() > room {
            self.say(format!("{MAX_PINS} conversations fit at the top: there's room for {room} more"), tx);
            return;
        }
        for k in to_pin {
            self.toggle_pin(k, tx);
        }
    }

    /// Asks whether to delete them (on the phone).
    pub fn ask_delete(&mut self, targets: &[Target], tx: &mut Tx) {
        if targets.is_empty() || !self.connected(tx) {
            return;
        }
        let ids: Vec<String> = self.messages_of(targets).iter().map(|m| m.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let threads = targets.iter().filter(|t| matches!(t, Target::Thread(_))).count();
        let what = if threads > 0 {
            format!("{} ({})", plural(threads, "conversation", "conversations"), plural(ids.len(), "message", "messages"))
        } else {
            plural(ids.len(), "message", "messages")
        };
        self.confirm = Some(Confirm { ids, what });
    }

    /// The answer to deleting: yes deletes them on the phone, and here at once.
    pub fn answer_delete(&mut self, yes: bool, tx: &mut Tx) {
        let Some(c) = self.confirm.take() else { return };
        if !yes {
            return;
        }
        self.messages.retain(|m| !c.ids.contains(&m.id));
        self.rethread();
        self.picked.messages.retain(|id| !c.ids.contains(id));
        let gone: Vec<String> = self.picked.threads.iter().filter(|k| self.thread(k).is_none()).cloned().collect();
        self.picked.threads.retain(|k| !gone.contains(k));
        if self.cursor.as_ref().is_some_and(|k| self.thread(k).is_none()) {
            self.cursor = None;
        }
        if self.msg_cursor.as_ref().is_some_and(|id| c.ids.contains(id)) {
            self.msg_cursor = None;
        }
        if let Pane::Thread(k) = &self.pane
            && self.thread(k).is_none()
        {
            self.pane = Pane::Nothing;
            self.focus = Focus::List;
        }
        self.say(format!("Deleted {}", c.what), tx);
        self.send_cmd(Cmd::Delete(c.ids));
    }

    // Swiping

    /// A drag on a row moved by `dx, dy`; past a little sideways it's a swipe.
    pub fn drag_swipe(&mut self, target: Target, dx: f32, dy: f32) {
        if self.swipe.target.as_ref() != Some(&target) {
            if dx.abs() < SWIPE_SLOP || dx.abs() < dy.abs() {
                return;
            }
            self.swipe = Swipe { target: Some(target), ..Swipe::default() };
        }
        self.swipe.dx = dx.clamp(-SWIPE_MAX, SWIPE_MAX);
    }

    /// Two fingers on a trackpad moved by `dx, dy` over a row; true when that's a swipe (else
    /// the list scrolls as usual).
    pub fn wheel_swipe(&mut self, target: Target, e: &rune::WheelEvent, tx: &mut Tx) -> bool {
        use rune::GesturePhase;
        let mine = self.swipe.target.as_ref() == Some(&target);
        if e.phase == GesturePhase::Ended || e.momentum {
            if mine && e.phase == GesturePhase::Ended {
                self.end_swipe(false, tx);
            }
            return mine;
        }
        if !mine {
            if e.dy.abs() > e.dx.abs() {
                self.swipe.pending = 0.0;
                return false;
            }
            self.swipe.pending += e.dx;
            if self.swipe.pending.abs() < SWIPE_SLOP {
                return true;
            }
            self.swipe = Swipe { target: Some(target), dx: self.swipe.pending, pending: 0.0, wheel_at: Some(self.clock) };
            tx.wake_after(std::time::Duration::from_secs_f64(WHEEL_REST));
            return true;
        }
        tx.animated();
        self.swipe.dx = (self.swipe.dx + e.dx).clamp(-SWIPE_MAX, SWIPE_MAX);
        self.swipe.wheel_at = Some(self.clock);
        tx.wake_after(std::time::Duration::from_secs_f64(WHEEL_REST));
        true
    }

    /// From the app's tick: a two-finger swipe that's rested ends.
    pub fn swipe_tick(&mut self, tx: &mut Tx) -> bool {
        match self.swipe.wheel_at {
            Some(t) if self.clock - t >= WHEEL_REST => {
                self.end_swipe(false, tx);
                true
            }
            Some(_) => {
                tx.wake_after(std::time::Duration::from_secs_f64(WHEEL_REST / 2.0));
                false
            }
            None => false,
        }
    }

    /// The swipe let go: far enough left asks to delete, far enough right marks read or unread.
    pub fn end_swipe(&mut self, cancelled: bool, tx: &mut Tx) {
        let Swipe { target, dx, .. } = std::mem::take(&mut self.swipe);
        let Some(target) = target else { return };
        // The click that may follow the drag mustn't also open or pick the row.
        self.swiped = true;
        tx.animated();
        if cancelled {
            return;
        }
        if dx <= -SWIPE_ACTS {
            self.ask_delete(&[target], tx);
        } else if dx >= SWIPE_ACTS {
            self.toggle_read(&[target], tx);
        }
    }

    /// How far a row is swiped now.
    pub fn swiped_by(&self, target: &Target) -> f32 {
        if self.swipe.target.as_ref() == Some(target) { self.swipe.dx } else { 0.0 }
    }

    /// A press is starting: a swipe that ended earlier no longer swallows it.
    pub fn press_down(&mut self) {
        self.swiped = false;
    }

    // Keys

    /// Vim's keys for Messages, where no field is taking them. j/k move, gg/G the ends, l or
    /// Return opens, h or Escape goes back, x or Space picks, J/K pick while moving, u read or
    /// unread, p pin, dd delete, i write, / search, c a new message, ? the list of them.
    pub fn vim_key(&mut self, e: &KeyEvent, tx: &mut Tx) -> bool {
        // Deleting waits for an answer: y or Return, n or Escape.
        if self.confirm.is_some() {
            match &e.key {
                Key::Text(t) if t == "y" => self.answer_delete(true, tx),
                Key::Enter => self.answer_delete(true, tx),
                Key::Text(t) if t == "n" => self.answer_delete(false, tx),
                Key::Escape => self.answer_delete(false, tx),
                _ => {}
            }
            return true;
        }
        let typing = self.search.focused || self.draft.focused || self.to.focused;
        if e.key == Key::Escape && typing {
            // Out of the field and back to the keys, as Escape leaves insert mode.
            self.search.focused = false;
            self.to.focused = false;
            if std::mem::take(&mut self.draft.focused) {
                self.focus = Focus::Thread;
            }
            return true;
        }
        if typing || self.tab != Tab::Messages || self.pane == Pane::Keypad {
            return false;
        }
        if self.show_keys && matches!(&e.key, Key::Escape) || matches!(&e.key, Key::Text(t) if t == "?") {
            self.show_keys = !self.show_keys;
            return true;
        }
        if self.focus == Focus::Thread && self.open_key().is_none() {
            self.focus = Focus::List;
        }
        let pending = self.pending_key.take();
        let half = 10;
        match (&e.key, pending) {
            (Key::Text(t), Some('g')) if t == "g" => self.step_to_end(false, tx),
            (Key::Text(t), Some('d')) if t == "d" => {
                let chosen = self.chosen();
                self.ask_delete(&chosen, tx);
            }
            (Key::Text(t), _) if t == "g" || t == "d" => self.pending_key = t.chars().next(),
            (Key::Text(t), _) if t == "j" => self.step(1, false, tx),
            (Key::Text(t), _) if t == "k" => self.step(-1, false, tx),
            (Key::Down, _) => self.step(1, e.mods.shift, tx),
            (Key::Up, _) => self.step(-1, e.mods.shift, tx),
            (Key::Text(t), _) if t == "J" => self.step(1, true, tx),
            (Key::Text(t), _) if t == "K" => self.step(-1, true, tx),
            (Key::Text(t), _) if t == "G" => self.step_to_end(true, tx),
            (Key::Home, _) => self.step_to_end(false, tx),
            (Key::End, _) => self.step_to_end(true, tx),
            (Key::PageDown, _) => self.step(half, false, tx),
            (Key::PageUp, _) => self.step(-half, false, tx),
            _ if e.is_command('d') => self.step(half, false, tx),
            _ if e.is_command('u') => self.step(-half, false, tx),
            _ if e.is_command('a') => self.pick_all(),
            (Key::Text(t), _) if t == "x" || t == " " => match self.focus {
                Focus::List => {
                    if let Some(c) = self.cursor.clone() {
                        Self::toggle(&mut self.picked.threads, &c);
                    }
                }
                Focus::Thread => {
                    if let Some(c) = self.msg_cursor.clone() {
                        Self::toggle(&mut self.picked.messages, &c);
                    }
                }
            },
            (Key::Text(t), _) if t == "u" => {
                let chosen = self.chosen();
                self.toggle_read(&chosen, tx);
            }
            (Key::Text(t), _) if t == "p" && self.focus == Focus::List => {
                let chosen = self.chosen();
                self.toggle_pins(&chosen, tx);
            }
            (Key::Delete, _) => {
                let chosen = self.chosen();
                self.ask_delete(&chosen, tx);
            }
            (Key::Text(t), _) if (t == "l" || t == "o") && self.focus == Focus::List => self.open_cursor(),
            (Key::Enter, _) if self.focus == Focus::List => self.open_cursor(),
            (Key::Text(t), _) if t == "i" || t == "a" => {
                if self.open_key().is_some() {
                    self.draft.focused = true;
                } else {
                    self.compose();
                }
            }
            (Key::Enter, _) => self.draft.focused = true,
            (Key::Text(t), _) if t == "h" && self.focus == Focus::Thread => {
                self.focus = Focus::List;
                self.reveal(tx);
            }
            (Key::Escape, _) => {
                if self.picked != Picked::default() {
                    self.clear_picks();
                } else if self.focus == Focus::Thread {
                    self.focus = Focus::List;
                } else {
                    self.cursor = None;
                }
            }
            (Key::Text(t), _) if t == "/" => {
                self.focus = Focus::List;
                self.search.focused = true;
            }
            (Key::Text(t), _) if t == "c" => self.compose(),
            _ => return false,
        }
        true
    }
}

/// `at` moved `by`, kept in `0..len`; from nowhere, down starts at the top.
fn moved(at: Option<usize>, by: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(at) = at else { return Some(if by >= 0 { 0 } else { len - 1 }) };
    Some((at as isize).saturating_add(by).clamp(0, len as isize - 1) as usize)
}

/// The keys, for the list `?` shows.
pub const KEYS: &[(&str, &str)] = &[
    ("j / k", "Down / up"),
    ("gg / G", "First / last"),
    ("Ctrl+d / Ctrl+u", "Down / up ten"),
    ("l, o or Return", "Open the conversation"),
    ("h or Escape", "Back to the list"),
    ("x or Space", "Pick or unpick"),
    ("J / K", "Pick while moving"),
    ("Ctrl+a", "Pick all"),
    ("u", "Mark read / unread"),
    ("p", "Pin / unpin"),
    ("dd or Delete", "Delete (asks first)"),
    ("i or Return", "Write a reply"),
    ("Escape", "Leave the field, or let go of the picked"),
    ("/", "Search"),
    ("c", "New message"),
    ("Ctrl+click / Shift+click", "Pick one / a run"),
    ("Swipe left / right", "Delete / read or unread"),
    ("?", "Show or hide these"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods;

    fn press(s: &mut App, keys: &str) {
        let mut tx = Tx::default();
        for c in keys.chars() {
            let key = match c {
                '\n' => Key::Enter,
                '\x1b' => Key::Escape,
                c => Key::Text(c.to_string()),
            };
            s.vim_key(&KeyEvent { key, mods: Mods::default() }, &mut tx);
        }
    }

    fn scene() -> App {
        let mut s = crate::app::init();
        crate::demo::pinned(&mut s);
        s.pane = Pane::Nothing;
        s.draft.focused = false;
        s
    }

    #[test]
    fn the_keys_move_open_and_go_back() {
        let mut s = scene();
        let keys = s.nav_threads();
        press(&mut s, "j");
        assert_eq!(s.cursor.as_ref(), Some(&keys[0]), "down from nothing starts at the top: the first pinned");
        press(&mut s, "G");
        assert_eq!(s.cursor.as_ref(), keys.last());
        press(&mut s, "gg");
        assert_eq!(s.cursor.as_ref(), Some(&keys[0]));
        press(&mut s, "jl");
        assert_eq!((s.pane.clone(), s.focus), (Pane::Thread(keys[1].clone()), Focus::Thread));
        assert!(!s.draft.focused, "opened from the keys, the keys keep going");
        press(&mut s, "k");
        assert_eq!(s.msg_cursor.as_deref(), s.open_messages().last().map(|m| m.id.as_str()), "up from nothing: the newest");
        press(&mut s, "i");
        assert!(s.draft.focused);
        press(&mut s, "j");
        assert!(s.draft.focused, "while writing, the field has the keys");
        press(&mut s, "\x1b");
        assert!(!s.draft.focused && s.focus == Focus::Thread, "Escape leaves the field");
        press(&mut s, "h");
        assert_eq!(s.focus, Focus::List);
    }

    #[test]
    fn picking_and_acting_on_many() {
        let mut s = scene();
        let keys = s.nav_threads();
        press(&mut s, "jxjj ");
        assert_eq!(s.picked.threads, [keys[0].clone(), keys[2].clone()].into_iter().collect());
        press(&mut s, "J");
        assert!(s.picked.threads.contains(&keys[3]) && s.picked.threads.len() == 3, "J picks where it goes too");
        press(&mut s, "\x1b");
        assert!(s.picked.threads.is_empty());

        // Katherine's text is unread: u reads it, u again marks it unread.
        let kj = crate::number::key("+15550100300");
        s.cursor = Some(kj.clone());
        press(&mut s, "u");
        assert_eq!(s.thread(&kj).unwrap().unread, 0);
        press(&mut s, "u");
        assert_eq!(s.thread(&kj).unwrap().unread, 1);

        // dd asks; n keeps them, y deletes them.
        let before = s.messages.len();
        press(&mut s, "dd");
        assert_eq!(s.confirm.as_ref().map(|c| c.what.as_str()), Some("1 conversation (1 message)"));
        press(&mut s, "n");
        assert!(s.confirm.is_none() && s.messages.len() == before);
        press(&mut s, "ddy");
        assert_eq!(s.messages.len(), before - 1);
        assert!(s.thread(&kj).is_none() && s.cursor.is_none());
    }

    #[test]
    fn a_swipe_left_asks_to_delete_and_swallows_its_click() {
        let mut s = scene();
        let mut tx = Tx::default();
        let ada = crate::number::key("+15550100100");
        let t = Target::Thread(ada.clone());
        s.drag_swipe(t.clone(), -4.0, 0.0);
        assert_eq!(s.swiped_by(&t), 0.0, "a little is still a click");
        s.drag_swipe(t.clone(), -20.0, 30.0);
        assert_eq!(s.swiped_by(&t), 0.0, "more down than across is a scroll");
        s.drag_swipe(t.clone(), -120.0, 4.0);
        assert_eq!(s.swiped_by(&t), -120.0);
        s.end_swipe(false, &mut tx);
        assert_eq!(s.confirm.as_ref().map(|c| c.ids.len()), Some(5));
        s.click_thread(&ada, &mut tx);
        assert_eq!(s.pane, Pane::Nothing, "the click ending the swipe doesn't open it");
        s.answer_delete(false, &mut tx);

        // Right, on a message: read or unread.
        let id = s.messages.iter().find(|m| m.incoming).unwrap().id.clone();
        let m = Target::Message(id.clone());
        s.drag_swipe(m.clone(), 100.0, 0.0);
        s.end_swipe(false, &mut tx);
        assert!(!s.messages.iter().find(|x| x.id == id).unwrap().read);
    }

    #[test]
    fn moving_stays_in_the_list() {
        assert_eq!(moved(None, 1, 3), Some(0));
        assert_eq!(moved(None, -1, 3), Some(2));
        assert_eq!(moved(Some(1), 10, 3), Some(2));
        assert_eq!(moved(Some(1), -10, 3), Some(0));
        assert_eq!(moved(Some(0), 1, 0), None);
    }
}
