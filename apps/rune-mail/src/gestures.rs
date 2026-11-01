//! Two-finger swipes on list rows, and dragging rows onto sidebar mailboxes.
//!
//! A horizontal trackpad swipe over a row slides it: left reveals Flag, Archive and Delete,
//! right reveals Read/Unread. Let go past half-way and the row snaps open on its buttons;
//! swipe far and the action happens (left: Archive, or Delete in Settings; right: read or
//! unread). Vertical swipes still scroll the list: a row takes a gesture only when it starts
//! mostly sideways. Some touchpads never say when the fingers lift, so a swipe also settles
//! once its deltas stop for a moment.
//!
//! Dragging a row (all marked rows, if it's marked) shows a label under the pointer and lights
//! the mailbox it's over; dropping moves the conversations there. Rows and mailboxes report
//! where they're drawn, so the drop can be found without the view.

use std::collections::HashMap;
use std::time::Duration;

use rune::{Drag, GesturePhase, Phase, Tx, WheelEvent};

use crate::actions::Dest;
use crate::app::App;
use crate::settings::SwipeLeft;
use crate::worker::View;

/// Width of each revealed button, and how far a row slides to show them.
pub const BUTTON: f32 = 68.0;
pub const OPEN_LEFT: f32 = BUTTON * 3.0;
pub const OPEN_RIGHT: f32 = BUTTON;
/// Past this, letting go snaps the row open on its buttons.
const OPEN_AT: f32 = 44.0;
/// Past this, letting go does the full-swipe action.
const FULL_LEFT: f32 = 236.0;
const FULL_RIGHT: f32 = 150.0;
/// With no lift reported, a swipe settles after this long without deltas.
const QUIET_SECS: f64 = 0.22;
/// How far the pointer travels before a press becomes a drag.
const DRAG_START: f32 = 6.0;

pub struct Swipe {
    pub uid: String,
    /// How far the row has slid: negative left, positive right.
    pub offset: f32,
    /// The fingers are still on it.
    pub live: bool,
    last: f64,
}

pub struct Dragging {
    pub rows: Vec<String>,
    pub label: String,
    /// The pointer, in window points.
    pub x: f32,
    pub y: f32,
    /// The mailbox under the pointer, if it's one that can take the drop.
    pub over: Option<View>,
}

/// Where things are drawn, for hit testing drags: list rows by uid, sidebar mailboxes by view.
#[derive(Default)]
pub struct Rects {
    pub rows: HashMap<String, [f32; 4]>,
    pub mailboxes: HashMap<String, (View, [f32; 4])>,
}

fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

impl App {
    /// A wheel or trackpad delta over row `uid`; true when the row takes the gesture.
    pub fn swipe_wheel(&mut self, uid: &str, e: WheelEvent, tx: &mut Tx) -> bool {
        let active = self.swipe.as_ref().is_some_and(|w| w.uid == uid && w.live);
        if e.momentum {
            return active; // the coast after a swipe this row took
        }
        if !active {
            // Only a gesture that starts mostly sideways is a swipe; the rest scroll the list.
            if e.phase == GesturePhase::Ended || e.dx.abs() <= e.dy.abs() * 1.5 || e.dx == 0.0 {
                return false;
            }
            let base = self.swipe.as_ref().filter(|w| w.uid == uid).map_or(0.0, |w| w.offset);
            self.swipe = Some(Swipe { uid: uid.to_owned(), offset: base, live: true, last: self.now() });
        }
        if e.phase == GesturePhase::Ended {
            self.settle_swipe(tx);
            return true;
        }
        let now = self.now();
        let w = self.swipe.as_mut().expect("a live swipe");
        w.offset = (w.offset + e.dx).clamp(-(FULL_LEFT + 60.0), FULL_RIGHT + 50.0);
        w.last = now;
        tx.instant();
        tx.wake_after(Duration::from_secs_f64(QUIET_SECS));
        true
    }

    /// Settles a swipe whose fingers seem to have lifted (no deltas for a moment); `poll`
    /// calls it each frame.
    pub fn settle_quiet_swipe(&mut self, tx: &mut Tx) -> bool {
        let quiet = self.swipe.as_ref().is_some_and(|w| w.live && self.now() - w.last >= QUIET_SECS);
        if quiet {
            self.settle_swipe(tx);
        }
        quiet
    }

    /// Let go: do the full-swipe action, snap open on the buttons, or slide back.
    fn settle_swipe(&mut self, tx: &mut Tx) {
        let Some(w) = self.swipe.as_mut() else { return };
        w.live = false;
        let (uid, offset) = (w.uid.clone(), w.offset);
        if offset <= -FULL_LEFT {
            self.swipe = None;
            let role = match self.settings.swipe_left {
                SwipeLeft::Archive => "archive",
                SwipeLeft::Delete => "trash",
            };
            self.move_rows(vec![uid], Dest::Role(role), tx);
        } else if offset >= FULL_RIGHT {
            self.swipe = None;
            self.toggle_read_rows(vec![uid]);
        } else if offset <= -OPEN_AT {
            w.offset = -OPEN_LEFT;
        } else if offset >= OPEN_AT {
            w.offset = OPEN_RIGHT;
        } else {
            self.swipe = None;
        }
    }

    /// How far row `uid` is slid.
    pub fn swipe_offset(&self, uid: &str) -> f32 {
        self.swipe.as_ref().filter(|w| w.uid == uid).map_or(0.0, |w| w.offset)
    }

    /// Closes an open swipe; true when there was one (a click then only closes it).
    pub fn close_swipe(&mut self) -> bool {
        self.swipe.take().is_some()
    }

    /// A revealed button: does its action on that row alone.
    pub fn swipe_action(&mut self, uid: &str, action: &str, tx: &mut Tx) {
        self.swipe = None;
        let rows = vec![uid.to_owned()];
        match action {
            "archive" | "trash" => self.move_rows(rows, Dest::Role(action), tx),
            "flag" => self.toggle_flag_rows(rows),
            "read" => self.toggle_read_rows(rows),
            _ => {}
        }
    }

    /// A press-and-drag on row `i`: past a few points it becomes a drag of that row (or every
    /// marked row, if it's marked); dropping on a mailbox moves them there.
    pub fn row_drag(&mut self, i: usize, d: &mut Drag, tx: &mut Tx) {
        let Some(uid) = self.messages.get(i).map(|m| m.uid.clone()) else { return };
        match d.phase {
            Phase::Down => {}
            Phase::Move => {
                if self.dragging.is_none() {
                    if d.dx.hypot(d.dy) < DRAG_START || self.swipe.is_some() {
                        return;
                    }
                    let rows = if self.marked.contains(&uid) { self.targets() } else { vec![uid.clone()] };
                    let label = match rows.as_slice() {
                        [one] => self.messages.iter().find(|m| &m.uid == one).map_or_else(String::new, |m| {
                            if m.subject.is_empty() { "(no subject)".to_owned() } else { m.subject.clone() }
                        }),
                        many => format!("{} {}", many.len(), if self.settings.conversations { "conversations" } else { "messages" }),
                    };
                    self.dragging = Some(Dragging { rows, label, x: 0.0, y: 0.0, over: None });
                }
                let origin = self.rects.rows.get(&uid).copied().unwrap_or_default();
                let (x, y) = (origin[0] + d.x, origin[1] + d.y);
                // Only mailboxes on show: a folded section's rows keep their last rects.
                let shown: Vec<View> = self
                    .sidebar()
                    .into_iter()
                    .filter(|sec| !self.is_collapsed(&sec.id))
                    .flat_map(|sec| sec.items.into_iter().map(|i| i.view))
                    .collect();
                let over = self
                    .rects
                    .mailboxes
                    .values()
                    .find(|(v, r)| shown.contains(v) && inside(*r, x, y))
                    .map(|(v, _)| v.clone());
                let over = over.filter(|v| matches!(v, View::Mailbox { .. }));
                if let Some(dr) = self.dragging.as_mut() {
                    (dr.x, dr.y, dr.over) = (x, y, over);
                }
                tx.instant();
            }
            Phase::Up => {
                let Some(dr) = self.dragging.take() else { return };
                if d.cancelled() {
                    return;
                }
                if let Some(View::Mailbox { account, id }) = dr.over {
                    self.move_rows(dr.rows, Dest::Mailbox { account: &account, id: &id }, tx);
                }
            }
        }
    }

    /// A drag is over this mailbox.
    pub fn drop_over(&self, view: &View) -> bool {
        self.dragging.as_ref().and_then(|d| d.over.as_ref()) == Some(view)
    }
}
