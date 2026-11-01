//! Two-finger swipes on list rows, as in Rune Mail: a sideways trackpad swipe slides a row left
//! to show its buttons (Copy User Name, Copy Password and Delete; Recover and Delete Now in
//! Deleted). Let go past a little and the row snaps open on them; swipe far and it asks to
//! delete. Vertical swipes still scroll the list: a row takes a gesture only when it starts
//! mostly sideways. Some touchpads never say when the fingers lift, so a swipe also settles
//! once its deltas stop for a moment.

use std::time::{Duration, Instant};

use rune::{GesturePhase, Tx, WheelEvent};

use crate::app::{App, can_delete};
use crate::vault::Item;

/// Width of each revealed button.
pub const BUTTON: f32 = 72.0;
/// Past this, letting go snaps the row open on its buttons.
const OPEN_AT: f32 = 44.0;
/// Past this, letting go asks to delete.
pub const FULL: f32 = 250.0;
/// With no lift reported, a swipe settles after this long without deltas.
const QUIET: Duration = Duration::from_millis(220);

pub struct Swipe {
    pub id: String,
    /// How far the row has slid left (negative).
    pub offset: f32,
    /// The fingers are still on it.
    pub live: bool,
    last: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    CopyUser,
    CopyPassword,
    Delete,
    Recover,
    DeleteNow,
}

/// The buttons a swipe shows on `item`, left to right; the last is the full swipe's.
pub fn actions(item: &Item) -> Vec<Action> {
    if item.deleted.is_some() {
        return if can_delete(item) { vec![Action::Recover, Action::DeleteNow] } else { Vec::new() };
    }
    let mut a = Vec::new();
    if !item.username.is_empty() {
        a.push(Action::CopyUser);
    }
    if !item.password.is_empty() {
        a.push(Action::CopyPassword);
    }
    if can_delete(item) {
        a.push(Action::Delete);
    }
    a
}

#[derive(Debug, PartialEq)]
enum Settle {
    Close,
    Open(f32),
    Full,
}

/// Where a row let go at `offset` goes, with `buttons` to show.
fn settle(offset: f32, buttons: &[Action]) -> Settle {
    let destructive = buttons.last().is_some_and(|a| matches!(a, Action::Delete | Action::DeleteNow));
    if buttons.is_empty() || offset > -OPEN_AT {
        Settle::Close
    } else if offset <= -FULL && destructive {
        Settle::Full
    } else {
        Settle::Open(-(BUTTON * buttons.len() as f32))
    }
}

impl Swipe {
    /// Row `id` open on its buttons: demo scenes.
    pub fn open(id: &str, buttons: usize) -> Swipe {
        Swipe { id: id.to_owned(), offset: -(BUTTON * buttons as f32), live: false, last: Instant::now() }
    }
}

impl App {
    fn swipe_item(&self, id: &str) -> Option<&Item> {
        self.vault.as_ref()?.items.iter().find(|i| i.id == id)
    }

    /// A wheel or trackpad delta over row `id`; true when the row takes the gesture.
    pub fn swipe_wheel(&mut self, id: &str, e: WheelEvent, tx: &mut Tx) -> bool {
        let active = self.swipe.as_ref().is_some_and(|w| w.id == id && w.live);
        if e.momentum {
            return active; // the coast after a swipe this row took
        }
        if !active {
            // Only a gesture that starts mostly sideways is a swipe; the rest scroll the list.
            if e.phase == GesturePhase::Ended || e.dx.abs() <= e.dy.abs() * 1.5 || e.dx == 0.0 {
                return false;
            }
            if self.swipe_item(id).is_none_or(|i| actions(i).is_empty()) {
                return false;
            }
            let base = self.swipe.as_ref().filter(|w| w.id == id).map_or(0.0, |w| w.offset);
            self.swipe = Some(Swipe { id: id.to_owned(), offset: base, live: true, last: Instant::now() });
        }
        if e.phase == GesturePhase::Ended {
            self.settle_swipe(tx);
            return true;
        }
        let w = self.swipe.as_mut().expect("a live swipe");
        w.offset = (w.offset + e.dx).clamp(-(FULL + 60.0), 0.0);
        w.last = Instant::now();
        tx.instant();
        tx.wake_after(QUIET);
        true
    }

    /// Settles a swipe whose fingers seem to have lifted (no deltas for a moment); `poll`
    /// calls it each time it runs.
    pub fn settle_quiet_swipe(&mut self, tx: &mut Tx) -> bool {
        let quiet = self.swipe.as_ref().is_some_and(|w| w.live && w.last.elapsed() >= QUIET);
        if quiet {
            self.settle_swipe(tx);
        }
        quiet
    }

    fn settle_swipe(&mut self, tx: &mut Tx) {
        let Some(w) = &self.swipe else { return };
        let (id, offset) = (w.id.clone(), w.offset);
        let buttons = self.swipe_item(&id).map(actions).unwrap_or_default();
        match settle(offset, &buttons) {
            Settle::Close => self.swipe = None,
            Settle::Open(at) => {
                let w = self.swipe.as_mut().expect("checked above");
                w.offset = at;
                w.live = false;
            }
            Settle::Full => {
                self.swipe = None;
                self.ask_delete(&id);
            }
        }
        tx.instant();
    }

    /// How far row `id` is slid.
    pub fn swipe_offset(&self, id: &str) -> f32 {
        self.swipe.as_ref().filter(|w| w.id == id).map_or(0.0, |w| w.offset)
    }

    /// Closes an open swipe; true when there was one (a click then only closes it).
    pub fn close_swipe(&mut self) -> bool {
        self.swipe.take().is_some()
    }

    /// A revealed button: its action on that row's item.
    pub fn swipe_action(&mut self, id: &str, action: Action, tx: &mut Tx) {
        self.swipe = None;
        let Some(item) = self.swipe_item(id) else { return };
        let (username, password) = (item.username.clone(), item.password.clone());
        match action {
            Action::CopyUser => self.copy("user name", username, tx),
            Action::CopyPassword => self.copy("password", password, tx),
            Action::Delete | Action::DeleteNow => self.ask_delete(id),
            Action::Recover => self.restore_item(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Kind;

    #[test]
    fn buttons_follow_the_item() {
        let mut i = Item::new_login(None);
        i.username = "me".into();
        i.password = "pw".into();
        assert_eq!(actions(&i), [Action::CopyUser, Action::CopyPassword, Action::Delete]);
        i.editable = false;
        assert_eq!(actions(&i), [Action::CopyUser, Action::CopyPassword], "a read-only shared item can't be deleted");
        let mut note = Item::new_login(None);
        note.kind = Kind::Note;
        assert_eq!(actions(&note), [Action::Delete]);
        note.deleted = Some("2026-10-01T00:00:00Z".into());
        assert_eq!(actions(&note), [Action::Recover, Action::DeleteNow]);
    }

    #[test]
    fn letting_go_closes_opens_or_asks() {
        let all = [Action::CopyUser, Action::CopyPassword, Action::Delete];
        assert_eq!(settle(-20.0, &all), Settle::Close);
        assert_eq!(settle(-60.0, &all), Settle::Open(-BUTTON * 3.0));
        assert_eq!(settle(-FULL - 5.0, &all), Settle::Full);
        let copies = [Action::CopyUser, Action::CopyPassword];
        assert_eq!(settle(-FULL - 5.0, &copies), Settle::Open(-BUTTON * 2.0), "only a delete happens on a full swipe");
        assert_eq!(settle(-FULL - 5.0, &[]), Settle::Close);
    }
}
