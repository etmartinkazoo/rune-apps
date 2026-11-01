//! Undo: a short wait before sent mail leaves, and a pill after moves, flags and read changes.
//!
//! Send closes the compose sheet at once but holds the message for the delay set in Settings
//! (5 s by default); Undo in the pill (or Ctrl+Z) brings the sheet back as it was. Should the
//! send fail, the sheet comes back too, with the error, so a message is never lost. One still
//! waiting when the app quits isn't sent.
//!
//! After archiving, deleting, moving, flagging or marking read, the pill offers Undo for
//! `UNDO_SECS`: moves go back on the server and their rows come back where they were; flags and
//! read states return to what each message had.

use std::time::Duration;

use rune::{TextEdit, Tx};

use crate::app::{App, Compose, Sheet};
use crate::jmap::{Outgoing, Summary};
use crate::worker::{Cmd, Threads};

const UNDO_SECS: f64 = 8.0;

/// A message waiting out the undo delay, or handed to the server and not yet confirmed.
pub struct PendingSend {
    pub id: u64,
    pub account: String,
    pub mail: Outgoing,
    /// The sheet as it was, to bring back on Undo or failure.
    pub draft: Draft,
    pub due: f64,
}

#[derive(Clone)]
pub struct Draft {
    account: String,
    attachments: Vec<crate::attach::Attachment>,
    to: String,
    cc: String,
    subject: String,
    /// Whole, not just its text: pasted images are pills in it, and their attachments go
    /// with the message only while their pill is there.
    body: TextEdit,
    in_reply_to: Vec<String>,
    references: Vec<String>,
}

impl Draft {
    pub fn of(c: &Compose) -> Draft {
        Draft {
            account: c.account.clone(),
            attachments: c.attachments.clone(),
            to: c.to.text.clone(),
            cc: c.cc.text.clone(),
            subject: c.subject.text.clone(),
            body: c.body.clone(),
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
        }
    }

    fn compose(&self) -> Compose {
        let mut c = Compose::empty(self.account.clone());
        c.attachments = self.attachments.clone();
        c.to = TextEdit::new(&self.to);
        c.cc = TextEdit::new(&self.cc);
        c.subject = TextEdit::new(&self.subject);
        c.body = self.body.clone();
        c.body.focused = true;
        c.in_reply_to = self.in_reply_to.clone();
        c.references = self.references.clone();
        c
    }
}

/// What the pill's Undo puts back.
pub enum UndoKind {
    /// Each (account, ids, from, to) moved, the rows that left the list with their places, and
    /// what each (account, mailbox) count went down by: (total, unread).
    Moves {
        moves: Vec<(String, Vec<String>, String, String)>,
        rows: Vec<(usize, Summary)>,
        threads: Threads,
        counts: Vec<(String, String, u32, u32)>,
    },
    /// A keyword ($seen or $flagged) and what each message had before.
    Keywords { keyword: &'static str, before: Vec<(String, bool)> },
}

pub struct Undo {
    pub label: String,
    pub until: f64,
    pub kind: UndoKind,
}

impl App {
    /// Holds the compose sheet's message for the undo delay, or sends it now when that's off.
    pub(crate) fn queue_send(&mut self, account: String, mail: Outgoing, tx: &mut Tx) {
        let delay = self.settings.undo_send_secs as f64;
        self.next_send += 1;
        let pending = PendingSend { id: self.next_send, account, mail, draft: Draft::of(&self.compose), due: self.now() + delay };
        self.compose = Compose::empty(String::new());
        if self.sheet == Sheet::Compose {
            self.sheet = Sheet::None;
        }
        if delay <= 0.0 {
            self.dispatch_send(pending);
        } else {
            self.waiting_sends.push(pending);
            tx.wake_after(Duration::from_secs(1));
        }
    }

    fn dispatch_send(&mut self, p: PendingSend) {
        self.send(Cmd::Send { id: p.id, account: p.account.clone(), mail: p.mail.clone() });
        self.in_flight_sends.push(p);
    }

    /// Sends what's waited long enough; keeps the countdown ticking. True when the pill changed.
    pub fn tick_sends(&mut self, tx: &mut Tx) -> bool {
        if self.waiting_sends.is_empty() {
            return false;
        }
        let now = self.now();
        let (due, wait): (Vec<PendingSend>, Vec<PendingSend>) = std::mem::take(&mut self.waiting_sends).into_iter().partition(|p| p.due <= now);
        self.waiting_sends = wait;
        for p in due {
            self.dispatch_send(p);
        }
        if !self.waiting_sends.is_empty() {
            tx.wake_after(Duration::from_millis(250));
        }
        true
    }

    /// The newest message still waiting to send, and the seconds left.
    pub fn waiting_send(&self) -> Option<(&PendingSend, u64)> {
        self.waiting_sends.last().map(|p| (p, (p.due - self.now()).max(0.0).ceil() as u64))
    }

    /// Undo on a waiting send: the compose sheet comes back as it was.
    pub fn undo_send(&mut self) -> bool {
        let Some(p) = self.waiting_sends.pop() else { return false };
        self.compose = p.draft.compose();
        self.sheet = Sheet::Compose;
        true
    }

    /// The server took send `id`. False if it wasn't a compose send (an unsubscribe email).
    pub fn sent(&mut self, id: u64) -> bool {
        let before = self.in_flight_sends.len();
        self.in_flight_sends.retain(|p| p.id != id);
        before != self.in_flight_sends.len()
    }

    /// Send `id` failed: its sheet comes back, so nothing typed is lost.
    pub fn send_failed(&mut self, id: u64) -> bool {
        let Some(i) = self.in_flight_sends.iter().position(|p| p.id == id) else { return false };
        let p = self.in_flight_sends.remove(i);
        self.compose = p.draft.compose();
        self.sheet = Sheet::Compose;
        true
    }

    /// Offers Undo for what just happened.
    pub(crate) fn offer_undo(&mut self, label: String, kind: UndoKind, tx: &mut Tx) {
        self.undo = Some(Undo { label, until: self.now() + UNDO_SECS, kind });
        tx.wake_after(Duration::from_secs_f64(UNDO_SECS));
    }

    /// Ends an expired offer. True when the pill went away.
    pub fn expire_undo(&mut self) -> bool {
        let expired = self.undo.as_ref().is_some_and(|u| self.now() >= u.until);
        if expired {
            self.undo = None;
        }
        expired
    }

    /// Ctrl+Z, or the pill's Undo: a waiting send first, else the last action.
    pub fn undo_last(&mut self, tx: &mut Tx) -> bool {
        if self.undo_send() {
            return true;
        }
        let Some(u) = self.undo.take() else { return false };
        match u.kind {
            UndoKind::Moves { moves, rows, threads, counts } => {
                for (account, ids, from, to) in moves {
                    self.send(Cmd::Move { account, ids, from: to, to: from });
                }
                // Until the server's counts come back with the move.
                for (account, id, total, unread) in counts {
                    if let Some(a) = self.accounts.iter_mut().find(|a| a.key == account)
                        && let Some(mb) = a.mailboxes.iter_mut().find(|mb| mb.id == id)
                    {
                        mb.total = mb.total.saturating_add(total);
                        mb.unread = mb.unread.saturating_add(unread);
                    }
                }
                // The rows come back where they were, if the list is still the one they left.
                for (i, row) in rows {
                    if self.messages.iter().all(|m| m.uid != row.uid) {
                        let at = i.min(self.messages.len());
                        self.messages.insert(at, row);
                    }
                }
                self.threads.extend(threads);
                self.list_version += 1;
            }
            UndoKind::Keywords { keyword, before } => {
                if keyword == "$seen" {
                    let (was_read, was_unread): (Vec<_>, Vec<_>) = before.into_iter().partition(|(_, v)| *v);
                    self.set_seen(&was_read.into_iter().map(|(u, _)| u).collect::<Vec<_>>(), true);
                    self.set_seen(&was_unread.into_iter().map(|(u, _)| u).collect::<Vec<_>>(), false);
                } else {
                    let mut by: std::collections::BTreeMap<(String, bool), Vec<String>> = Default::default();
                    let (were_on, were_off): (Vec<_>, Vec<_>) = before.into_iter().partition(|(_, v)| *v);
                    for (was, uids) in [(true, were_on), (false, were_off)] {
                        let uids: Vec<String> = uids.into_iter().map(|(u, _)| u).collect();
                        for m in self.update_messages(&uids, |m| m.flagged = was) {
                            by.entry((m.account, was)).or_default().push(m.id);
                        }
                    }
                    for ((account, on), ids) in by {
                        self.send(Cmd::SetKeyword { account, ids, keyword, on });
                    }
                }
            }
        }
        self.say(format!("Undid: {}", u.label), tx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undone_send_keeps_its_pasted_images() {
        let _one = crate::demo::test_app();
        let mut s = crate::app::init();
        s.compose_new();
        s.paste_image(vec![0x89, b'P', b'N', b'G']);
        let back = Draft::of(&s.compose).compose();
        assert_eq!(back.body.pills().count(), 1, "the image's pill came back");
        s.compose = back;
        assert_eq!(s.outgoing_attachments().1.len(), 1, "so the image still goes with the message");
    }
}
