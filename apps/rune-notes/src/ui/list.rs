//! The notes of the selected folder (or the search's, from every folder): title, when, and the
//! first line of the body, newest first. A note drags onto a folder in the sidebar to move.

use rune::{Color, El, PointerPress, ScrollSpec, Tx, col, el, icon, row, text};

use super::style::{self, BODY, BODY_BOLD, Palette, SMALL, TITLE};
use super::tool;
use crate::app::{App, Menu, Place, Target};
use crate::store::Note;

pub const WIDTH: f32 = 300.0;

pub fn view(s: &App, p: Palette) -> El {
    let notes = s.visible();
    let n = notes.len();
    let searching = !s.query().is_empty();
    // Notes from other folders say which folder they're in.
    let say_folder = searching || s.place == Place::All;
    let mut rows = col().key("notes").w_full().px(8.0).pb(12.0);
    if s.open.as_ref().is_some_and(|o| o.path.is_none()) {
        // A new note, not written yet, tops the list.
        let title = crate::store::title_of(&s.editor.text, std::path::Path::new("New Note"));
        rows = rows.child(card("new-note-row", &title, "No additional text", "Now", None, true, p)).child(between(false, p));
    }
    for (i, note) in notes.iter().enumerate() {
        let next_selected = notes.get(i + 1).is_some_and(|m| s.is_open(&m.path));
        rows = rows.child(note_row(s, note, say_folder, i + 1 < n && !s.is_open(&note.path) && !next_selected, p));
    }
    if n == 0 && s.open.as_ref().is_none_or(|o| o.path.is_some()) {
        let empty = if searching { "No notes match" } else { "No notes" };
        rows = rows.child(col().w_full().pt(40.0).center().child(text(empty).font(BODY).color(p.tertiary)));
    }
    let count = if n == 1 { "1 note".to_owned() } else { format!("{n} notes") };
    let head = row().key("list-head").w_full().px(18.0).pt(4.0).pb(8.0).gap(8.0).center_y()
        .child(text(s.place_title()).font(TITLE).color(p.text).truncate().min_w(0.0))
        .child(text(count).font(SMALL).color(p.secondary));
    col().key("list").w(WIDTH).h_full().fill(p.content)
        .child(toolbar(s, p))
        .child(head)
        .child(col().key("list-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(rows))
}

fn toolbar(s: &App, p: Palette) -> El {
    let input = super::field::input(&s.search, BODY, p).key("search").single_line().grow(1.0)
        .placeholder("Search all notes", p.tertiary)
        .on_edit(|s: &mut App| &mut s.search)
        .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut Tx| super::field::platform_key(&mut s.search, e))
        // Return opens the first match.
        .on_submit(|s: &mut App, _| {
            if let Some(p) = s.visible().first().map(|n| n.path.clone()) {
                s.open_note(&p);
                s.search.focused = false;
            }
        });
    let mut field = row().w_full().h(28.0).pl(8.0).pr(4.0).gap(6.0).center_y().radius(7.0).fill(p.field).cursor(rune::Cursor::Text)
        .on_press(|s: &mut App| s.search.focused = true)
        .child(icon(&style::SEARCH, 13.0).color(p.secondary))
        .child(input);
    if !s.search.text.is_empty() {
        field = field.child(el().key("clear-search").size(20.0, 20.0).radius(10.0).center().hover(p.hover).help("Clear search")
            .on_press(|s: &mut App| s.search.replace_text(""))
            .child(icon(&style::X, 12.0).color(p.secondary)));
    }
    row().key("list-toolbar").w_full().h(52.0).px(12.0).gap(8.0).center_y()
        .child(row().grow(1.0).min_w(0.0).child(field))
        .child(tool(&style::NEW_NOTE, "New note (Ctrl+N)", p).key("new-note").on_press(|s: &mut App| s.new_note()))
}

fn note_row(s: &App, note: &Note, say_folder: bool, rule: bool, p: Palette) -> El {
    let path = note.path.clone();
    let folder = say_folder.then(|| s.folder_name(note.folder()));
    let selected = s.is_open(&path);
    let dragged = s.dragging.as_ref().is_some_and(|d| d.note == path);
    let snippet = if note.snippet.is_empty() { "No additional text" } else { &note.snippet };
    let (open, drag, menu, rect) = (path.clone(), path.clone(), path.clone(), path.clone());
    let c = card(&format!("note-{}", path.to_string_lossy()), &note.title, snippet, &super::when(note.modified), folder, selected, p)
        .opacity(if dragged { 0.45 } else { 1.0 })
        .on_press(move |s: &mut App| s.open_note(&open))
        .on_drag(move |s: &mut App, d, tx| s.note_drag(&drag, d, tx))
        .on_secondary_press(move |s: &mut App, at: PointerPress, _: &mut Tx| {
            s.menu = Some(Menu { target: Target::Note(menu.clone()), x: at.x, y: at.y });
        })
        .on_drawn_layout_tx(move |s: &mut App, r, tx| {
            s.rects.notes.insert(rect.clone(), r);
            tx.live_only();
        });
    col().w_full().child(c).child(between(rule, p))
}

/// The space between two notes, so one's hover never touches the next: with a hairline, as
/// Apple Notes draws, except beside the selected one.
fn between(rule: bool, p: Palette) -> El {
    el().w_full().py(2.0).px(10.0).child(el().w_full().h(1.0).fill(if rule { p.separator } else { Color::TRANSPARENT }))
}

fn card(key: &str, title: &str, snippet: &str, when: &str, folder: Option<String>, selected: bool, p: Palette) -> El {
    let mut c = col().key(key.to_owned()).role("listitem").w_full().px(10.0).py(8.0).gap(2.0).radius(8.0)
        .fill(if selected { p.sidebar_selection } else { Color::TRANSPARENT })
        .hover(if selected { Color::TRANSPARENT } else { p.hover })
        .child(text(title.to_owned()).font(BODY_BOLD).color(p.text).truncate().min_w(0.0))
        .child(row().w_full().gap(8.0)
            .child(text(when.to_owned()).font(SMALL).color(p.text))
            .child(text(snippet.to_owned()).font(SMALL).color(p.secondary).truncate().grow(1.0).min_w(0.0)));
    if let Some(f) = folder {
        c = c.child(row().gap(4.0).center_y().pt(1.0)
            .child(icon(&style::FOLDER, 11.0).color(p.secondary))
            .child(text(f).font(SMALL).color(p.secondary).truncate().min_w(0.0)));
    }
    c
}
