//! Apple Notes' layout: folders in the sidebar, the selected folder's notes beside it, the
//! open note on the right.

pub mod editor;
pub mod field;
pub mod list;
pub mod sheets;
pub mod sidebar;
pub mod style;

use rune::{Color, Cx, El, Key, KeyEvent, Tx, col, el, icon, row, text};

use crate::app::{App, Menu, Sheet, Target};
use style::{BODY, Palette, SMALL};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let p = Palette::new(s.settings.dark(cx.system_dark()));
    let main = row().w_full().h_full().fill(p.window)
        .child(sidebar::view(s, p))
        .child(el().w(1.0).h_full().fill(p.separator))
        .child(list::view(s, p))
        .child(el().w(1.0).h_full().fill(p.separator))
        .child(editor::view(s, cx, p));
    let mut root = el().key("root").w_full().h_full().child(main)
        .on_window_focus(|s: &mut App, focused, _: &mut Tx| {
            // Back in the window: catch up with changes made elsewhere. Leaving it: save.
            if focused {
                s.rescan();
            } else {
                s.save();
            }
        });
    if let Some(m) = &s.menu {
        root = root.child(menu(s, m, p));
    }
    if let Some(d) = &s.dragging {
        // The label that follows the pointer while a note is dragged to a folder.
        root = root.child(row().key("drag-label").pin(Some(d.x + 14.0), Some(d.y + 10.0), None, None)
            .h(26.0).px(10.0).gap(6.0).center_y().radius(13.0).max_w(320.0)
            .fill(if d.over.is_some() { p.accent } else { p.secondary })
            .shadow(4.0, 14.0, Color::BLACK.alpha(0.25))
            .inert()
            .child(icon(&style::FILE_TEXT, 13.0).color(Color::WHITE))
            .child(text(d.label.clone()).font(SMALL).color(Color::WHITE).truncate().min_w(0.0)));
    }
    if let Some((status, _)) = &s.status {
        root = root.child(row().key("status").pin(None, None, Some(16.0), Some(16.0)).px(14.0).h(32.0).center_y()
            .radius(16.0).fill(p.card).border(1.0, p.separator).shadow(4.0, 14.0, Color::BLACK.alpha(if p.dark { 0.4 } else { 0.15 }))
            .child(text(status.clone()).font(BODY).color(p.text)));
    }
    match &s.sheet {
        Sheet::None => root,
        Sheet::Settings => root.child(sheets::settings(s, p)),
        Sheet::MoveTo(note) => root.child(sheets::move_to(s, note, p)),
        Sheet::DeleteFolder(f) => root.child(sheets::delete_folder(s, f, p)),
    }
}

pub fn tool(i: &'static rune::Icon, help: &str, p: Palette) -> El {
    el().size(28.0, 26.0).radius(6.0).center().hover(p.hover).help(help).child(icon(i, 15.0).color(p.secondary))
}

pub fn button(label: &str, ink: Color, fill: Color, p: Palette) -> El {
    row().h(26.0).px(12.0).radius(6.0).center().fill(fill).hover(p.hover).child(text(label).font(BODY).color(ink))
}

pub fn segmented<T: Copy + PartialEq + 'static>(key: &str, options: &[(&str, T)], current: T, set: fn(&mut App, T), p: Palette) -> El {
    let mut r = row().key(key.to_owned()).role("radiogroup").h(26.0).padding(2.0).gap(2.0).radius(7.0).fill(p.field);
    for (label, value) in options {
        let (label, value) = (*label, *value);
        let on = value == current;
        r = r.child(row().key(format!("{key}-{}", label.to_lowercase().replace(' ', "-"))).h(22.0).px(12.0).center().radius(5.0)
            .fill(if on { p.content } else { Color::TRANSPARENT })
            .hover(p.hover)
            .on_press(move |s: &mut App| set(s, value))
            .child(text(label).font(SMALL).color(if on { p.text } else { p.secondary })));
    }
    r
}

/// The right-click menu of a folder or note, where it was clicked; a press outside closes it.
fn menu(s: &App, m: &Menu, p: Palette) -> El {
    let item = |label: &str, ink: Color| row().w_full().h(26.0).px(10.0).center_y().radius(5.0).hover(p.hover)
        .child(text(label).font(BODY).color(ink));
    let mut c = col().key("menu").w(190.0).padding(5.0).radius(8.0).fill(p.card).border(1.0, p.separator)
        .shadow(6.0, 20.0, Color::BLACK.alpha(0.25))
        .pin(Some(m.x), Some(m.y), None, None)
        .modal(true, |s: &mut App| s.menu = None);
    match &m.target {
        Target::Folder(f) => {
            let (a, b, d) = (f.clone(), f.clone(), f.clone());
            c = c.child(item("New Folder Inside", p.text).key("menu-new-folder").on_press(move |s: &mut App| {
                s.select_place(crate::app::Place::Folder(a.clone()));
                s.new_folder();
            }));
            if *f != s.root {
                c = c.child(item("Rename", p.text).key("menu-rename").on_press(move |s: &mut App| s.start_rename(&b)))
                    .child(item("Delete Folder…", p.danger).key("menu-delete").on_press(move |s: &mut App| s.ask_delete_folder(&d)));
            }
        }
        Target::Note(n) => {
            let (a, b) = (n.clone(), n.clone());
            c = c.child(item("Move to…", p.text).key("menu-move").on_press(move |s: &mut App| {
                s.menu = None;
                s.sheet = Sheet::MoveTo(a.clone());
            }))
            .child(item("Delete", p.danger).key("menu-delete").on_press(move |s: &mut App| s.trash_note(&b)));
        }
    }
    c
}

/// "9:41 AM" today, "Yesterday", a weekday this week, else the date.
pub fn when(t: std::time::SystemTime) -> String {
    use chrono::{DateTime, Datelike, Local};
    let t: DateTime<Local> = t.into();
    let today = Local::now().date_naive();
    let d = t.date_naive();
    let days = (today - d).num_days();
    match days {
        0 => clock(t.time()),
        1 => "Yesterday".into(),
        2..=6 => t.format("%A").to_string(),
        _ if d.year() == today.year() => t.format("%-d %B").to_string(),
        _ => t.format("%-d %b %Y").to_string(),
    }
}

/// "30 October 2026 at 9:41 AM", over the note.
pub fn when_long(t: std::time::SystemTime) -> String {
    let t: chrono::DateTime<chrono::Local> = t.into();
    format!("{} at {}", t.format("%-d %B %Y"), clock(t.time()))
}

fn clock(t: chrono::NaiveTime) -> String {
    use chrono::Timelike;
    let (h, m) = (t.hour(), t.minute());
    let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    format!("{h12}:{m:02} {half}")
}

pub fn on_key(s: &mut App, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        if s.menu.take().is_some() {
            return true;
        }
        if s.sheet != Sheet::None {
            s.close_sheet();
            return true;
        }
        if s.naming.is_some() {
            s.cancel_naming();
            return true;
        }
        if s.search.focused || !s.query().is_empty() {
            s.search.replace_text("");
            s.search.focused = false;
            return true;
        }
        return false;
    }
    if e.is_command(',') {
        s.open_settings();
        return true;
    }
    if s.sheet != Sheet::None {
        return false;
    }
    if e.is_command('n') {
        if e.mods.shift { s.new_folder() } else { s.new_note() }
        return true;
    }
    if e.is_command('f') {
        s.editor.focused = false;
        s.search.focused = true;
        return true;
    }
    if e.is_command('e') {
        s.toggle_preview();
        return true;
    }
    if e.is_command('s') {
        s.save();
        return true;
    }
    if e.is_command('r') {
        s.rescan();
        return true;
    }
    // The rest are plain keys for the list, not for a field.
    if tx.focused().is_some() || s.editor.focused || s.search.focused || s.naming.is_some() || e.mods.cmd || e.mods.ctrl || e.mods.alt {
        return false;
    }
    match e.key {
        Key::Up => s.step(-1),
        Key::Down => s.step(1),
        Key::Delete | Key::Backspace => s.trash_open(),
        Key::Enter if s.open.is_some() && !s.preview => s.editor.focused = true,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn dates_read_like_apple_notes() {
        let now = SystemTime::now();
        assert!(when(now).ends_with('M'), "today shows the time");
        let long_ago = now - Duration::from_secs(400 * 86_400);
        assert!(when(long_ago).chars().last().unwrap().is_ascii_digit(), "another year shows it");
        assert!(when_long(now).contains(" at "));
    }
}
