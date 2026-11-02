//! Apple Passwords' layout: categories down the side, the list beside them, and the selected
//! item's details.

pub mod detail;
pub mod field;
pub mod list;
pub mod sidebar;
pub mod signin;
pub mod style;

use rune::{Color, Cx, El, Icon, Key, KeyEvent, Tx, col, el, icon, row, text};

use crate::app::{App, Confirm, Phase};
use style::{BODY, Palette, SMALL};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let p = Palette::new(s.settings.dark(cx.system_dark()));
    let content = match s.phase {
        Phase::SignIn => signin::sign_in(s, p),
        Phase::Unlock => signin::unlock(s, p),
        Phase::Ready => row().w_full().h_full().fill(p.window)
            .child(sidebar::view(s, p))
            .child(el().w(1.0).h_full().fill(p.separator))
            .child(list::view(s, p))
            .child(el().w(1.0).h_full().fill(p.separator))
            .child(detail::view(s, p)),
    };
    let mut root = el().key("root").w_full().h_full().fill(p.window).child(content);
    if s.accounts_open && s.phase == Phase::Ready {
        root = root.child(sidebar::account_menu(s, p));
    }
    if s.settings_open {
        root = root.child(sidebar::settings(s, p));
    }
    if let Some(r) = s.copy_tip.as_ref().and_then(|t| s.field_rects.get(&t.key))
        && s.confirm.is_none()
        && !s.settings_open
    {
        root = root.child(copy_tip(*r, p));
    }
    if let Some(c) = &s.confirm {
        root = root.child(confirm(c, p));
    }
    if let Some((status, _)) = &s.status {
        root = root.child(row().key("status").pin(None, None, Some(16.0), Some(16.0)).px(14.0).h(32.0).center_y()
            .radius(16.0).fill(p.card).border(1.0, p.separator).shadow(4.0, 14.0, Color::BLACK.alpha(if p.dark { 0.4 } else { 0.15 }))
            .child(text(status.clone()).font(BODY).color(p.text)));
    }
    root
}

/// "Click to copy", centred just above the value at `r` (window points).
fn copy_tip(r: [f32; 4], p: Palette) -> El {
    const W: f32 = 92.0;
    const H: f32 = 22.0;
    let x = r[0] + r[2] / 2.0 - W / 2.0;
    row().key("copy-tip").pin(Some(x.max(4.0)), Some((r[1] - H - 4.0).max(4.0)), None, None).size(W, H).center()
        .radius(5.0).fill(if p.dark { style::c("#2b2b2b") } else { style::c("#fafafa") }).border(1.0, p.separator)
        .shadow(2.0, 8.0, Color::BLACK.alpha(if p.dark { 0.35 } else { 0.12 }))
        .child(text("Click to copy").font(SMALL).color(p.text))
}

/// Asks before a delete: Cancel, or Delete. Escape and a click outside cancel.
fn confirm(c: &Confirm, p: Palette) -> El {
    let name = if c.name.is_empty() { "this item".to_owned() } else { format!("\u{201c}{}\u{201d}", c.name) };
    let (title, says) = if c.permanent {
        (format!("Delete {name} permanently?"), "This can't be undone.")
    } else {
        (format!("Delete {name}?"), "It moves to Deleted, where you can recover it.")
    };
    let card = col().key("confirm").w(320.0).padding(20.0).gap(8.0).items_center().radius(14.0).fill(p.content)
        .border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .role("alertdialog")
        .modal(true, |s: &mut App| s.confirm = None)
        .child(icon(&style::TRASH, 28.0).color(p.danger))
        .child(text(title).font(style::BODY_BOLD).color(p.text).wrap().align(rune::Align::Center))
        .child(text(says).font(SMALL).color(p.secondary).wrap().align(rune::Align::Center))
        .child(row().w_full().pt(10.0).gap(8.0)
            .child(button("Cancel", p.text, p.field, p).key("confirm-cancel").grow(1.0).on_press(|s: &mut App| s.confirm = None))
            .child(button("Delete", Color::WHITE, p.danger, p).key("confirm-delete").grow(1.0).on_press(|s: &mut App| s.confirm_delete())));
    el().w_full().h_full().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

pub fn button(label: &str, ink: Color, fill: Color, p: Palette) -> El {
    row().h(26.0).px(12.0).radius(6.0).center().fill(fill).hover(p.hover).child(text(label).font(BODY).color(ink))
}

/// A square toolbar button with an icon.
pub fn tool(i: &'static Icon, help: &str, p: Palette) -> El {
    el().size(28.0, 26.0).radius(6.0).center().hover(p.hover).help(help.to_owned()).child(icon(i, 15.0).color(p.secondary))
}

pub fn segmented<T: Copy + PartialEq + 'static>(key: &str, options: &[(&str, T)], current: T, set: fn(&mut App, T, &mut Tx), p: Palette) -> El {
    let mut r = row().key(key.to_owned()).role("radiogroup").h(26.0).padding(2.0).gap(2.0).radius(7.0).fill(p.field);
    for (label, value) in options {
        let (label, value) = (*label, *value);
        let on = value == current;
        r = r.child(row().key(format!("{key}-{}", label.to_lowercase().replace(' ', "-"))).h(22.0).px(12.0).center().radius(5.0)
            .fill(if on { p.content } else { Color::TRANSPARENT })
            .hover(p.hover)
            .on_press_tx(move |s: &mut App, tx: &mut Tx| set(s, value, tx))
            .child(text(label).font(SMALL).color(if on { p.text } else { p.secondary })));
    }
    r
}

/// Keys that aren't a focused field's: ↑ and ↓ through the list, ⌘N a new password, ⌘F to
/// search, ⌘E to edit, ⌘L to lock, ⌘U / ⌘P / ⌘T to copy the selected item's user name, password
/// or verification code, Escape out of editing, Delete (or ⌘⌫) to delete.
pub fn on_key(s: &mut App, e: &KeyEvent, tx: &mut Tx) -> bool {
    if s.phase != Phase::Ready {
        return false;
    }
    match &e.key {
        _ if s.confirm.is_some() => return false,
        Key::Escape if s.settings_open => s.settings_open = false,
        Key::Escape if s.accounts_open => s.accounts_open = false,
        Key::Escape if s.swipe.is_some() => {
            s.close_swipe();
        }
        Key::Escape => return s.cancel_edit(),
        _ if e.is_command('l') => s.lock(),
        _ if e.is_command('n') => s.new_item(),
        _ if e.is_command('f') => s.search.focused = true,
        _ if e.is_command('e') && s.draft.is_none() => s.edit(),
        _ if e.is_command('s') && s.draft.is_some() => s.save(tx),
        _ if e.is_command('r') => s.sync(),
        _ if e.is_command('u') && s.draft.is_none() => s.copy_selected_user(tx),
        _ if e.is_command('p') && s.draft.is_none() => s.copy_selected_password(tx),
        _ if e.is_command('t') && s.draft.is_none() => s.copy_selected_code(tx),
        Key::Delete if s.draft.is_none() => s.ask_delete_selected(),
        Key::Backspace if e.mods.cmd && s.draft.is_none() => s.ask_delete_selected(),
        Key::Up if s.draft.is_none() => s.step(-1),
        Key::Down if s.draft.is_none() => s.step(1),
        _ => return false,
    }
    true
}
