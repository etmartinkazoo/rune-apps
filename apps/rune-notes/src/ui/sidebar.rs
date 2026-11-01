//! The sidebar: All Notes, then the folder tree under the notes folder, each with its count
//! and, when it has folders inside, a disclosure triangle.

use std::path::Path;

use rune::{Color, El, PointerPress, ScrollSpec, Tx, col, el, icon, row, text};

use super::style::{self, BODY, COUNT, Palette, SECTION};
use crate::app::{App, Menu, Place, Target};
use crate::store::Folder;

pub const WIDTH: f32 = 220.0;
const INDENT: f32 = 14.0;

pub fn view(s: &App, p: Palette) -> El {
    let mut list = col().key("folders").w_full().px(10.0).pt(4.0).pb(12.0).gap(1.0)
        .child(item("all", &style::FILES, "All Notes", s.tree.notes.len(), s.place == Place::All, 0, p)
            .on_press(|s: &mut App| s.select_place(Place::All)))
        .child(row().w_full().pt(14.0).pb(4.0).px(6.0).child(text("FOLDERS").font(SECTION).color(p.secondary)));
    for (f, depth) in shown(s) {
        let renaming = s.naming.as_ref().is_some_and(|n| n.renaming.as_deref() == Some(f.path.as_path()));
        list = list.child(if renaming { naming_row(s, depth, p) } else { folder_row(s, f, depth, p) });
        if s.naming.as_ref().is_some_and(|n| n.renaming.is_none() && n.parent == f.path) {
            list = list.child(naming_row(s, depth + 1, p));
        }
    }
    let new_folder = row().key("new-folder").h(28.0).px(8.0).gap(6.0).center_y().radius(6.0).hover(p.hover)
        .help("New folder inside the selected one (Ctrl+Shift+N)")
        .on_press(|s: &mut App| s.new_folder())
        .child(icon(&style::FOLDER_PLUS, 15.0).color(p.secondary))
        .child(text("New Folder").font(BODY).color(p.secondary));
    let settings = el().key("open-settings").size(28.0, 28.0).radius(6.0).center().hover(p.hover).help("Settings (Ctrl+,)")
        .on_press(|s: &mut App| s.open_settings())
        .child(icon(&style::GEAR, 15.0).color(p.secondary));
    col().key("sidebar").w(WIDTH).h_full().fill(p.sidebar)
        .child(el().w_full().h(52.0))
        .child(col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list))
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(row().w_full().h(44.0).px(8.0).center_y().justify_between().child(new_folder).child(settings))
}

/// The folders on show, in order, each with its depth: those inside a folded one are hidden.
fn shown(s: &App) -> Vec<(&Folder, usize)> {
    let mut out = Vec::new();
    fn go<'a>(s: &App, f: &'a Folder, depth: usize, out: &mut Vec<(&'a Folder, usize)>) {
        out.push((f, depth));
        if !s.is_collapsed(&f.path) {
            for c in &f.children {
                go(s, c, depth + 1, out);
            }
        }
    }
    go(s, &s.tree.root, 0, &mut out);
    out
}

pub fn row_key(path: &Path) -> String {
    format!("folder-{}", path.to_string_lossy())
}

fn item(key: &str, i: &'static rune::Icon, name: &str, count: usize, selected: bool, depth: usize, p: Palette) -> El {
    row().key(key.to_owned()).role("listitem").w_full().h(28.0).pl(6.0 + depth as f32 * INDENT).pr(8.0).gap(6.0).center_y()
        .radius(6.0).fill(if selected { p.sidebar_selection } else { Color::TRANSPARENT }).hover(p.hover)
        .child(icon(i, 15.0).color(p.accent))
        .child(text(name.to_owned()).font(BODY).color(p.text).truncate().grow(1.0).min_w(0.0))
        .child(text(count.to_string()).font(COUNT).color(p.secondary))
}

fn folder_row(s: &App, f: &Folder, depth: usize, p: Palette) -> El {
    let path = f.path.clone();
    let selected = s.place == Place::Folder(path.clone()) && s.query().is_empty();
    let target = s.drop_over(&path);
    let open = !s.is_collapsed(&path);
    let disclosure = if f.children.is_empty() {
        el().size(14.0, 14.0)
    } else {
        let t = path.clone();
        el().key(format!("disclose-{}", path.to_string_lossy())).size(14.0, 14.0).center().radius(3.0).hover(p.hover)
            .help(if open { "Collapse" } else { "Expand" })
            .on_press(move |s: &mut App| s.toggle_folder(&t))
            .child(icon(if open { &style::CHEVRON_DOWN } else { &style::CHEVRON_RIGHT }, 11.0).color(p.secondary))
    };
    let name = s.folder_name(&path);
    let (sel, ren, menu, rect) = (path.clone(), path.clone(), path.clone(), path.clone());
    row().key(row_key(&path)).role("treeitem").expanded(open).w_full().h(28.0).pl(depth as f32 * INDENT).pr(8.0).gap(4.0).center_y()
        .radius(6.0)
        .fill(if target { p.accent.alpha(0.22) } else if selected { p.sidebar_selection } else { Color::TRANSPARENT })
        .border(1.0, if target { p.accent } else { Color::TRANSPARENT })
        .hover(p.hover)
        .on_press(move |s: &mut App| s.select_place(Place::Folder(sel.clone())))
        .on_double_press(move |s: &mut App| s.start_rename(&ren))
        .on_secondary_press(move |s: &mut App, at: PointerPress, _: &mut Tx| {
            s.menu = Some(Menu { target: Target::Folder(menu.clone()), x: at.x, y: at.y });
        })
        // Where it's drawn, so a dragged note can be dropped on it.
        .on_drawn_layout_tx(move |s: &mut App, r, tx| {
            s.rects.folders.insert(rect.clone(), r);
            tx.live_only();
        })
        .child(disclosure)
        .child(icon(&style::FOLDER, 15.0).color(p.accent))
        .child(text(name).font(BODY).color(p.text).truncate().grow(1.0).min_w(0.0))
        .child(text(f.count.to_string()).font(COUNT).color(p.secondary))
}

/// The name field of a folder being made or renamed: Return keeps it, Escape drops it.
fn naming_row(s: &App, depth: usize, p: Palette) -> El {
    let Some(n) = &s.naming else { return el() };
    let input = super::field::input(&n.edit, BODY, p).key("folder-name").single_line().grow(1.0)
        .placeholder("Folder name", p.tertiary)
        .on_edit(|s: &mut App| &mut s.naming.as_mut().expect("naming").edit)
        .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut Tx| match &mut s.naming {
            Some(n) => super::field::platform_key(&mut n.edit, e),
            None => false,
        })
        .on_submit(|s: &mut App, _| s.commit_naming());
    row().key("naming").w_full().h(28.0).pl(depth as f32 * INDENT).pr(4.0).gap(4.0).center_y()
        .child(el().size(14.0, 14.0))
        .child(icon(&style::FOLDER, 15.0).color(p.accent))
        .child(row().grow(1.0).min_w(0.0).h(24.0).px(6.0).center_y().radius(5.0).fill(p.content).border(1.0, p.accent)
            .cursor(rune::Cursor::Text)
            .child(input))
}
