//! Actions on messages (the marked ones, or the selected one) and composing. Each change goes
//! to the server per account, since a list can mix accounts (All Inboxes).

use std::collections::{BTreeMap, HashSet};

use rune::{TextEdit, Tx};

use crate::app::{App, Compose, Sheet};
use crate::jmap::{Addr, Body, Outgoing, Summary, format_addrs, parse_addrs};
use crate::worker::Cmd;

impl App {
    /// What an action applies to, by uid: the marked messages in list order, else the
    /// selected one.
    pub fn targets(&self) -> Vec<String> {
        if self.marked.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.messages.iter().filter(|m| self.marked.contains(&m.uid)).map(|m| m.uid.clone()).collect()
        }
    }

    pub fn has_targets(&self) -> bool {
        !self.marked.is_empty() || self.selected.is_some()
    }

    /// Every message of the conversations of rows `uids`.
    fn members_of(&self, uids: &[String]) -> Vec<Summary> {
        let uids = set(uids);
        self.messages
            .iter()
            .filter(|m| uids.contains(m.uid.as_str()))
            .flat_map(|row| self.thread(row).into_iter().cloned())
            .collect()
    }

    /// Marks the target conversations read, or unread when they're all read already.
    pub fn toggle_read(&mut self) {
        self.toggle_read_rows(self.targets());
    }

    /// Marks rows' conversations read, or unread when they're all read already.
    pub fn toggle_read_rows(&mut self, rows: Vec<String>) {
        let members = self.members_of(&rows);
        let before: Vec<(String, bool)> = members.iter().map(|m| (m.uid.clone(), m.seen)).collect();
        let to_read = members.iter().any(|m| !m.seen);
        self.pending_undo = Some((
            if to_read { "Marked as read" } else { "Marked as unread" }.to_owned(),
            crate::undo::UndoKind::Keywords { keyword: "$seen", before },
        ));
        let seen = members.iter().any(|m| !m.seen);
        let uids: Vec<String> = if seen {
            members.iter().map(|m| m.uid.clone()).collect()
        } else {
            // Unread marks the newest message of each, as Mail does.
            rows
        };
        self.set_seen(&uids, seen);
    }

    /// Flags the target conversations' newest messages, or unflags every flagged message in
    /// them when they're all flagged already.
    pub fn toggle_flag(&mut self) {
        self.toggle_flag_rows(self.targets());
    }

    /// Flags rows' conversations, or unflags them when they're all flagged already.
    pub fn toggle_flag_rows(&mut self, rows: Vec<String>) {
        let members = self.members_of(&rows);
        let before: Vec<(String, bool)> = members.iter().map(|m| (m.uid.clone(), m.flagged)).collect();
        let row_set = set(&rows);
        let flag = self.messages.iter().filter(|m| row_set.contains(m.uid.as_str())).any(|row| !self.thread(row).iter().any(|m| m.flagged));
        self.pending_undo = Some((
            if flag { "Flagged" } else { "Unflagged" }.to_owned(),
            crate::undo::UndoKind::Keywords { keyword: "$flagged", before },
        ));
        let uids: Vec<String> = if flag {
            rows
        } else {
            members.iter().filter(|m| m.flagged).map(|m| m.uid.clone()).collect()
        };
        let mut by_account: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for m in self.update_messages(&uids, |m| m.flagged = flag) {
            by_account.entry(m.account).or_default().push(m.id);
        }
        for (account, ids) in by_account {
            self.send(Cmd::SetKeyword { account, ids, keyword: "$flagged", on: flag });
        }
    }

    /// Which messages a move of `rows` to `dest` takes: per (account, from, to), the ids of
    /// each conversation's messages that are in the mailbox shown. A mailbox of another
    /// account takes nothing: mail moves within its own account.
    pub(crate) fn plan_moves(&self, rows: &[String], dest: Dest) -> Plan {
        let mut plan = Plan::default();
        let rows = set(rows);
        for row in self.messages.iter().filter(|m| rows.contains(m.uid.as_str())) {
            let to = match dest {
                Dest::Role(role) => self.role_in(&row.account, role),
                Dest::Mailbox { account, id } => (account == row.account).then(|| id.to_owned()),
            };
            let (Some(from), Some(to)) = (self.source_mailbox(row), to) else { continue };
            if from == to {
                continue;
            }
            for m in self.thread(row).into_iter().filter(|m| m.mailbox_ids.contains(&from)) {
                plan.moves.entry((m.account.clone(), from.clone(), to.clone())).or_default().push(m.id.clone());
                if !m.seen {
                    plan.unread.push((m.account.clone(), from.clone()));
                }
            }
            plan.rows.push(row.uid.clone());
        }
        plan
    }

    /// Moves the target conversations to their account's mailbox with `role` and selects
    /// the row after them. Only messages in the mailbox shown move (a reply in Sent stays).
    pub fn move_targets(&mut self, role: &str, tx: &mut Tx) {
        self.move_rows(self.targets(), Dest::Role(role), tx);
    }

    /// Moves rows' conversations to `dest`. When the selection goes, the row after them is
    /// selected; otherwise the selection stays.
    pub fn move_rows(&mut self, rows: Vec<String>, dest: Dest, tx: &mut Tx) {
        if rows.is_empty() {
            return;
        }
        let Plan { moves, rows: moved_rows, unread } = self.plan_moves(&rows, dest);
        if moved_rows.is_empty() {
            match dest {
                Dest::Role(role) if !self.messages.iter().any(|m| rows.contains(&m.uid) && self.role_in(&m.account, role).is_some()) => {
                    self.say(format!("This account has no {role} mailbox"), tx);
                }
                Dest::Mailbox { account, .. } if self.messages.iter().any(|m| rows.contains(&m.uid) && m.account != account) => {
                    self.say("Mail can only move within its own account", tx);
                }
                _ => {}
            }
            return;
        }
        // Sets, not lists: select all and archive moves thousands, and each is looked up per row.
        let moved_rows: HashSet<String> = moved_rows.into_iter().collect();
        let selection_moves = self.selected.as_ref().is_some_and(|s| moved_rows.contains(s));
        let last = self.messages.iter().rposition(|m| moved_rows.contains(&m.uid)).unwrap_or(0);
        let next = self.messages[last + 1..].iter().find(|m| !moved_rows.contains(&m.uid)).map(|m| m.uid.clone()).or_else(|| {
            self.messages[..last].iter().rev().find(|m| !moved_rows.contains(&m.uid)).map(|m| m.uid.clone())
        });
        let mut counts = Vec::new();
        for ((account, from, _), ids) in &moves {
            let lost_unread = u32::try_from(unread.iter().filter(|(a, f)| a == account && f == from).count()).unwrap_or(u32::MAX);
            let lost = u32::try_from(ids.len()).unwrap_or(u32::MAX);
            if let Some(a) = self.accounts.iter_mut().find(|a| &a.key == account)
                && let Some(mb) = a.mailboxes.iter_mut().find(|mb| &mb.id == from)
            {
                counts.push((account.clone(), from.clone(), mb.total.min(lost), mb.unread.min(lost_unread)));
                mb.total = mb.total.saturating_sub(lost);
                mb.unread = mb.unread.saturating_sub(lost_unread);
            }
        }
        let keys: HashSet<String> =
            self.messages.iter().filter(|m| moved_rows.contains(&m.uid)).map(|m| m.thread_key.clone()).collect();
        let removed: Vec<(usize, Summary)> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| moved_rows.contains(&m.uid))
            .map(|(i, m)| (i, m.clone()))
            .collect();
        let removed_threads: crate::worker::Threads =
            self.threads.iter().filter(|(k, _)| keys.contains(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        self.messages.retain(|m| !moved_rows.contains(&m.uid));
        self.threads.retain(|k, _| !keys.contains(k));
        self.marked.retain(|m| !moved_rows.contains(m));
        self.list_version += 1;
        if selection_moves {
            self.selected = None;
        }
        let n = moved_rows.len();
        let noun = match (n, self.settings.conversations) {
            (1, true) => "conversation",
            (1, false) => "message",
            (_, true) => "conversations",
            (_, false) => "messages",
        };
        let what = if n == 1 { String::new() } else { format!(" {n} {noun}") };
        let label = match dest {
            Dest::Role("archive") => format!("Archived{what}"),
            Dest::Role("trash") => format!("Deleted{what}"),
            Dest::Role("junk") => format!("Moved{what} to Junk"),
            Dest::Role(role) => format!("Moved{what} to {role}"),
            Dest::Mailbox { account, id } => {
                let name = self.account(account).and_then(|a| a.mailboxes.iter().find(|m| m.id == id)).map_or(id.to_owned(), |m| m.name.clone());
                format!("Moved{what} to {name}")
            }
        };
        let undo_moves: Vec<(String, Vec<String>, String, String)> =
            moves.iter().map(|((a, f, t), ids)| (a.clone(), ids.clone(), f.clone(), t.clone())).collect();
        self.offer_undo(label, crate::undo::UndoKind::Moves { moves: undo_moves, rows: removed, threads: removed_threads, counts }, tx);
        if let Some(uid) = next.filter(|_| selection_moves) {
            self.select_message(uid);
        }
        for ((account, from, to), ids) in moves {
            self.send(Cmd::Move { account, ids, from, to });
        }
    }

    /// The message Unsubscribe acts on: the highlighted one in a conversation, else the
    /// newest, if its list says how to leave.
    pub fn unsubscribe_target(&self) -> Option<(Summary, crate::privacy::Unsubscribe)> {
        let m = self.answered(None)?;
        let u = self.bodies.get(&m.uid)?.unsubscribe.clone()?;
        Some((m, u))
    }

    /// Unsubscribe asks first; asked again (the key twice, or the banner's button), it goes
    /// ahead: one click if the list allows, else an unsubscribe email, else the list's page.
    pub fn unsubscribe(&mut self, tx: &mut Tx) {
        let Some((m, u)) = self.unsubscribe_target() else {
            self.say("This message doesn't say how to unsubscribe", tx);
            return;
        };
        if self.unsubscribed.contains(&m.uid) {
            return;
        }
        if self.unsubscribe_confirm.as_ref() != Some(&m.uid) {
            self.unsubscribe_confirm = Some(m.uid.clone());
            return;
        }
        self.unsubscribe_confirm = None;
        self.unsubscribed.insert(m.uid.clone());
        if let Some(url) = u.one_click {
            self.send(Cmd::Unsubscribe { uid: m.uid, url });
        } else if let Some(url) = u.mailto {
            let (to, subject, body) = crate::privacy::mailto(&url);
            let mail = Outgoing { to: parse_addrs(&to), subject, body, ..Outgoing::default() };
            self.send(Cmd::Send { id: 0, account: m.account, mail });
            self.say(format!("Unsubscribe email sent to {to}"), tx);
        } else if let Some(url) = u.web {
            // The list's own page; its link isn't cleaned, since the parameters say who you are.
            if let Err(e) = tx.open_url(&url) {
                self.unsubscribed.remove(&m.uid);
                self.say(format!("Couldn't open the unsubscribe page: {e:?}"), tx);
            } else {
                self.say("Opened the unsubscribe page", tx);
            }
        }
    }

    pub fn cancel_unsubscribe(&mut self) -> bool {
        self.unsubscribe_confirm.take().is_some()
    }

    // Compose

    fn open_compose(&mut self, c: Compose) {
        self.compose = c;
        self.sheet = Sheet::Compose;
    }

    fn new_compose(&self) -> Compose {
        Compose::empty(self.current_account().unwrap_or_default())
    }

    pub fn compose_new(&mut self) {
        let mut c = self.new_compose();
        c.to.focused = true;
        self.open_compose(c);
    }

    pub fn compose_to(&mut self, to: &str) {
        let mut c = self.new_compose();
        c.to = TextEdit::new(to);
        c.subject.focused = true;
        self.open_compose(c);
    }

    /// The message replies and forwards answer: the one asked for, else the highlighted
    /// message of the open conversation, else its newest.
    fn answered(&self, uid: Option<&str>) -> Option<Summary> {
        match uid.map(str::to_owned).or_else(|| self.focused_uid()) {
            Some(uid) => self.find_message(&uid).cloned(),
            None => self.selected_message().and_then(|row| self.thread(row).last().map(|m| (*m).clone())),
        }
    }

    /// Flags or unflags just the highlighted message.
    pub fn toggle_flag_focused(&mut self) {
        let Some(m) = self.focused_uid().and_then(|u| self.find_message(&u).cloned()) else { return };
        let flag = !m.flagged;
        self.update_message(&m.uid, |x| x.flagged = flag);
        self.send(Cmd::SetKeyword { account: m.account, ids: vec![m.id], keyword: "$flagged", on: flag });
    }

    /// Marks just the highlighted message read or unread.
    pub fn toggle_read_focused(&mut self) {
        let Some(m) = self.focused_uid().and_then(|u| self.find_message(&u).cloned()) else { return };
        self.set_seen(&[m.uid], !m.seen);
    }

    /// Loads remote images for the highlighted message.
    pub fn allow_remote_focused(&mut self) {
        if let Some(uid) = self.focused_uid() {
            self.allow_remote(uid);
        }
    }

    pub fn reply(&mut self, all: bool) {
        self.reply_to(None, all);
    }

    pub fn reply_to(&mut self, uid: Option<&str>, all: bool) {
        let Some(m) = self.answered(uid) else { return };
        let body = self.bodies.get(&m.uid).cloned();
        let mut to: Vec<Addr> =
            body.as_ref().map(|b| b.reply_to.clone()).filter(|r| !r.is_empty()).unwrap_or(m.from.clone());
        let mut cc = Vec::new();
        if all {
            let me = m.account.to_lowercase();
            to.extend(m.to.iter().filter(|a| a.email.to_lowercase() != me).cloned());
            cc = body.as_ref().map(|b| b.cc.clone()).unwrap_or_default();
            cc.retain(|a| a.email.to_lowercase() != me);
        }
        let mut c = Compose::empty(m.account.clone());
        c.to = TextEdit::new(format_addrs(&to));
        c.cc = TextEdit::new(format_addrs(&cc));
        c.subject = TextEdit::new(prefixed("Re:", &m.subject));
        c.body = TextEdit::new(format!("\n\n{}", quote(&m, body.as_ref())));
        // Ready to type above the quote (a new field's caret sits at the end, under it).
        c.body.set_caret(0);
        c.body.focused = true;
        if let Some(b) = body {
            c.in_reply_to = b.message_id.clone();
            c.references = b.references.iter().chain(&b.message_id).cloned().collect();
        }
        self.open_compose(c);
    }

    pub fn forward(&mut self) {
        self.forward_message(None);
    }

    pub fn forward_message(&mut self, uid: Option<&str>) {
        let Some(m) = self.answered(uid) else { return };
        let body = self.bodies.get(&m.uid).map_or_else(|| m.preview.clone(), |b| b.text.clone());
        let mut c = Compose::empty(m.account.clone());
        c.subject = TextEdit::new(prefixed("Fwd:", &m.subject));
        c.body = TextEdit::new(format!(
            "\n\nBegin forwarded message:\n\nFrom: {}\nSubject: {}\nDate: {}\nTo: {}\n\n{body}",
            format_addrs(&m.from),
            m.subject,
            crate::ui::dates::long(&m.received_at),
            format_addrs(&m.to),
        ));
        c.to.focused = true;
        self.open_compose(c);
    }

    pub fn close_sheet(&mut self) {
        self.from_open = false;
        match self.sheet {
            Sheet::Compose if self.compose.sending => {}
            Sheet::Compose => {
                self.compose = Compose::empty(String::new());
                self.sheet = Sheet::None;
            }
            _ => self.sheet = Sheet::None,
        }
        self.calendar_edit = None;
    }

    pub fn send_compose(&mut self, tx: &mut Tx) {
        if self.compose.sending {
            return;
        }
        let to = parse_addrs(&self.compose.to.text);
        if to.is_empty() {
            self.say("Add at least one recipient", tx);
            return;
        }
        if self.account(&self.compose.account).is_none() {
            self.say("Choose an account to send from", tx);
            return;
        }
        let (html, attachments) = self.outgoing_attachments();
        let total: usize = attachments.iter().map(|a| crate::attach::sent_size(a.bytes.len())).sum();
        if total > crate::attach::MAX_BYTES {
            self.say("Attachments are too big to send together: mail takes them as a third larger, over 25 MB", tx);
            return;
        }
        let c = &self.compose;
        let mail = Outgoing {
            html,
            attachments,
            to,
            cc: parse_addrs(&c.cc.text),
            subject: c.subject.text.clone(),
            body: c.body.text.clone(),
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            from_name: None,
        };
        let account = c.account.clone();
        self.queue_send(account, mail, tx);
    }
}

/// Where a move goes: an account's standard mailbox, or a particular mailbox.
#[derive(Clone, Copy, Debug)]
pub enum Dest<'a> {
    Role(&'a str),
    Mailbox { account: &'a str, id: &'a str },
}

#[derive(Default)]
pub(crate) struct Plan {
    pub moves: BTreeMap<(String, String, String), Vec<String>>,
    /// The rows that leave the list.
    pub rows: Vec<String>,
    /// (account, mailbox) once per unread message leaving it.
    pub unread: Vec<(String, String)>,
}

fn prefixed(prefix: &str, subject: &str) -> String {
    if subject.to_lowercase().starts_with(&prefix.to_lowercase()) {
        subject.to_owned()
    } else {
        format!("{prefix} {subject}")
    }
}

fn quote(m: &Summary, body: Option<&Body>) -> String {
    let who = m.from.first().map_or("someone", Addr::display);
    let text = body.map_or(m.preview.as_str(), |b| b.text.as_str());
    let quoted: Vec<String> = text.lines().map(|l| format!("> {l}")).collect();
    format!("On {}, {who} wrote:\n\n{}", crate::ui::dates::long(&m.received_at), quoted.join("\n"))
}

fn set(uids: &[String]) -> HashSet<&str> {
    uids.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use crate::jmap::uid;

    fn thread_app() -> crate::app::App {
        crate::demo::test_config();
        let _one = crate::demo::test_app();
        let mut s = crate::app::init();
        crate::demo::thread_scene(&mut s);
        s
    }

    #[test]
    fn archiving_a_conversation_moves_only_what_is_in_the_inbox() {
        let s = thread_app();
        let plan = s.plan_moves(&[uid("ted@fastmail.com", "m3")], super::Dest::Role("archive"));
        let ids: Vec<&String> = plan.moves.values().flatten().collect();
        assert_eq!(ids, ["m3x", "m3a", "m3"], "Ted's replies stay in Sent: {:?}", plan.moves);
        let (account, from, to) = plan.moves.keys().next().unwrap();
        assert_eq!((account.as_str(), from.as_str(), to.as_str()), ("ted@fastmail.com", "in", "ar"));
        assert_eq!(plan.rows, [uid("ted@fastmail.com", "m3")]);
    }

    #[test]
    fn replies_answer_the_newest_message_and_opening_expands_it() {
        let mut s = thread_app();
        assert!(s.expanded.contains(&uid("ted@fastmail.com", "m3")));
        assert!(!s.expanded.contains(&uid("ted@fastmail.com", "m3a")));
        s.reply(false);
        assert_eq!(s.compose.to.text, "Grace Hopper <grace@example.com>");
        assert_eq!(s.compose.account, "ted@fastmail.com");
        assert!(s.compose.body.focused);
        assert_eq!((s.compose.body.anchor, s.compose.body.focus), (0, 0), "typing starts above the quote");
    }
}
