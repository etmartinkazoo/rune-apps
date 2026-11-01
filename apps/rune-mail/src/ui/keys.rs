//! Keyboard: Apple Mail's shortcuts always, and vim keys while nothing has focus.
//! `SHORTCUTS` and `VIM` list them for the help sheet.

use rune::{Key, KeyEvent, Tx};

use crate::app::{App, Phase, Sheet};

/// (keys, action), in groups for the help sheet. "Ctrl" is Cmd on macOS.
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    ("Messages", &[
        ("↑ / ↓", "Previous / next message"),
        ("Shift ↑ / ↓", "Extend marks up / down"),
        ("Ctrl+Click", "Mark or unmark a message"),
        ("Shift+Click", "Mark a range"),
        ("Ctrl+Shift+A", "Mark all"),
        ("Esc", "Clear marks"),
        ("Delete", "Move to Trash"),
        ("Ctrl+Z", "Undo the last action, or a send still waiting"),
        ("Ctrl+E", "Archive"),
        ("Ctrl+Shift+J", "Move to Junk"),
        ("Ctrl+Shift+U", "Mark as read / unread"),
        ("Ctrl+Shift+L", "Flag / unflag"),
        ("Ctrl+Shift+X", "Unsubscribe (press again to confirm)"),
        ("Ctrl+Shift+C", "Copy for AI: the conversation (or marked ones) as a prompt"),
    ]),
    ("Compose", &[
        ("Ctrl+N", "New message"),
        ("Ctrl+R", "Reply"),
        ("Ctrl+Shift+R", "Reply all"),
        ("Ctrl+Shift+F", "Forward"),
        ("Tab", "Next field, through To, Cc, Subject and the message"),
        ("Enter / Tab in To or Cc", "Take the suggestion and stay / move on"),
        ("Ctrl+Enter", "Send"),
        ("Esc", "Close the sheet"),
    ]),
    ("Conversations", &[
        ("→ / ←", "Into the conversation / back to the list"),
        ("↑ / ↓", "Previous / next message, in the conversation"),
        ("Space", "Open / close the highlighted message"),
        ("Esc", "Back to the list"),
        ("Ctrl+R / Ctrl+Shift+R", "Reply / reply all to the highlighted message"),
        ("Ctrl+Shift+F", "Forward the highlighted message"),
    ]),
    ("Mailboxes", &[
        ("Ctrl+1 … 6", "Inbox (all accounts), Drafts, Sent, Archive, Junk, Trash"),
        ("Ctrl+Shift+N", "Get new mail (also the toolbar's first button)"),
        ("Ctrl+F", "Search"),
        ("Space / Shift+Space", "Page a single message down / up"),
        ("Ctrl+,", "Settings"),
        ("Ctrl+/", "This list"),
    ]),
];

pub const VIM: &[(&str, &str)] = &[
    ("j / k", "Down / up: the list, or the conversation once in it"),
    ("l / h", "Into the conversation / back to the list"),
    ("gg / G", "First / last, in the list or the conversation"),
    ("J / K", "Next / previous mailbox"),
    ("gi gd gs ga gj gt", "Go to Inbox, Drafts, Sent, Archive, Junk, Trash"),
    ("gr", "Get new mail"),
    ("x", "Mark or unmark"),
    ("Space", "Open / close the highlighted message"),
    ("o", "Open / close every message in the conversation"),
    ("S / U", "Flag / mark unread just the highlighted message"),
    ("i", "Load remote content in the highlighted message"),
    ("X", "Unsubscribe (press again to confirm)"),
    ("y", "Copy for AI (yank the conversation as a prompt)"),
    ("d", "Delete (move to Trash)"),
    ("a  or  e", "Archive"),
    ("!", "Move to Junk"),
    ("u", "Mark as read / unread"),
    ("s", "Flag / unflag"),
    ("c", "Compose"),
    ("r / R / f", "Reply / reply all / forward (the highlighted message)"),
    ("Ctrl+D / Ctrl+U", "Page the message down / up"),
    ("/", "Search"),
    ("?", "Show shortcuts"),
];

/// Keys no focused control took (the whole order is in Rune's README, "Key routing").
pub fn on_key(s: &mut App, e: &KeyEvent, tx: &mut Tx) -> bool {
    if s.phase != Phase::Ready {
        return false;
    }
    match s.sheet {
        Sheet::None => {}
        Sheet::Compose => {
            return match e.key {
                Key::Escape if s.from_open => {
                    s.from_open = false;
                    true
                }
                Key::Escape => {
                    s.close_sheet();
                    true
                }
                Key::Enter if e.mods.cmd => {
                    s.send_compose(tx);
                    true
                }
                _ if e.is_command('d') && e.mods.shift => {
                    s.send_compose(tx);
                    true
                }
                _ => false,
            };
        }
        Sheet::Settings | Sheet::Shortcuts | Sheet::AddAccount => {
            let close = matches!(e.key, Key::Escape) || e.is_command(',') || e.is_command('/')
                || matches!(&e.key, Key::Text(t) if t == "?" && s.sheet == Sheet::Shortcuts);
            if close {
                s.close_sheet();
            }
            return close;
        }
    }
    if mail_key(s, e, tx) {
        s.pending_g = false;
        return true;
    }
    if s.settings.vim && !e.mods.cmd && !e.mods.alt && let Key::Text(t) = &e.key {
        return vim_key(s, t, tx);
    }
    false
}

fn mail_key(s: &mut App, e: &KeyEvent, tx: &mut Tx) -> bool {
    let shift = e.mods.shift;
    match &e.key {
        Key::Up if shift => extend(s, -1, tx),
        Key::Down if shift => extend(s, 1, tx),
        Key::Up => s.step(-1, tx),
        Key::Down => s.step(1, tx),
        Key::Right if s.selected.is_some() => s.enter_thread(tx),
        Key::Left if s.leave_thread() => {}
        Key::Escape => {
            if !s.cancel_unsubscribe() && !s.leave_thread() && !s.clear_marks() {
                return false;
            }
        }
        _ if e.is_command('x') && shift => s.unsubscribe(tx),
        _ if e.is_command('c') && shift => s.copy_for_ai(tx),
        // Text selected in a message's HTML is the page thread's to give.
        _ if e.is_command('c') && s.pages.copy() => {}
        _ if e.is_command('z') && !shift => {
            if !s.undo_last(tx) {
                return false;
            }
        }
        Key::Backspace | Key::Delete => s.move_targets("trash", tx),
        // In the conversation Space opens or closes the highlighted message; else it pages.
        Key::Text(t) if t == " " && !e.mods.cmd && !shift && s.focused_uid().is_some() && s.conversation().len() > 1 => {
            s.toggle_focused(tx)
        }
        Key::Text(t) if t == " " && !e.mods.cmd => page(s, if shift { -1 } else { 1 }, tx),
        _ if e.is_command('n') && shift => s.refresh(),
        _ if e.is_command('n') => s.compose_new(),
        _ if e.is_command('r') => s.reply(shift),
        _ if e.is_command('f') && shift => s.forward(),
        _ if e.is_command('f') => s.search.focused = true,
        _ if e.is_command('e') => s.move_targets("archive", tx),
        _ if e.is_command('j') && shift => s.move_targets("junk", tx),
        _ if e.is_command('u') && shift => s.toggle_read(),
        _ if e.is_command('l') && shift => s.toggle_flag(),
        _ if e.is_command('a') && shift => s.mark_all(),
        _ if e.is_command(',') => s.sheet = Sheet::Settings,
        _ if e.is_command('/') => s.sheet = Sheet::Shortcuts,
        _ if s.settings.vim && e.is_command('d') => page(s, 1, tx),
        _ if s.settings.vim && e.is_command('u') => page(s, -1, tx),
        Key::Command(c @ '1'..='6') => s.select_role(crate::app::FAVORITES[(*c as u8 - b'1') as usize]),
        _ => return false,
    }
    true
}

fn vim_key(s: &mut App, t: &str, tx: &mut Tx) -> bool {
    if std::mem::take(&mut s.pending_g) {
        let role = match t {
            "g" => {
                s.jump(false, tx);
                return true;
            }
            "i" => "inbox",
            "d" => "drafts",
            "s" => "sent",
            "a" => "archive",
            "j" => "junk",
            "t" => "trash",
            "r" => {
                s.refresh();
                return true;
            }
            _ => return true,
        };
        s.select_role(role);
        return true;
    }
    match t {
        "j" => s.step(1, tx),
        "k" => s.step(-1, tx),
        "l" if s.selected.is_some() => s.enter_thread(tx),
        "h" => {
            s.leave_thread();
        }
        "g" => s.pending_g = true,
        "G" => s.jump(true, tx),
        "J" => s.step_mailbox(1),
        "K" => s.step_mailbox(-1),
        "x" => s.toggle_mark_selected(),
        "o" => s.toggle_expand_all(),
        "S" => s.toggle_flag_focused(),
        "U" => s.toggle_read_focused(),
        "i" => s.allow_remote_focused(),
        "X" => s.unsubscribe(tx),
        "y" => s.copy_for_ai(tx),
        "d" => s.move_targets("trash", tx),
        "a" | "e" => s.move_targets("archive", tx),
        "!" => s.move_targets("junk", tx),
        "u" => s.toggle_read(),
        "s" => s.toggle_flag(),
        "c" => s.compose_new(),
        "r" => s.reply(false),
        "R" => s.reply(true),
        "f" => s.forward(),
        "/" => s.search.focused = true,
        "?" => s.sheet = Sheet::Shortcuts,
        _ => return false,
    }
    true
}

/// Shift-arrows: mark the selection and move it, marking as it goes.
fn extend(s: &mut App, delta: isize, tx: &mut Tx) {
    if let Some(id) = s.selected.clone() {
        s.marked.insert(id);
    }
    s.step_selection(delta, tx);
    if let Some(id) = s.selected.clone() {
        s.marked.insert(id);
    }
    s.list_version += 1;
}

fn page(s: &App, pages: i32, tx: &mut Tx) {
    if let Some(key) = s.reader_key() {
        tx.scroll_page(key, pages);
    }
}
