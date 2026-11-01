//! Sheets over the window: Settings, Move to Folder, and the question before a folder goes.

use std::path::Path;

use rune::{Color, El, ScrollSpec, col, el, icon, row, text};

use super::style::{self, BODY, BODY_BOLD, Palette, SMALL};
use super::{button, segmented};
use crate::app::App;
use crate::settings::Appearance;

fn scrim(key: &str, card: El, p: Palette) -> El {
    el().key(format!("{key}-scrim")).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

fn card(key: &str, w: f32, p: Palette) -> El {
    col().key(key.to_owned()).w(w).padding(22.0).gap(12.0).radius(14.0).fill(p.content)
        .border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.close_sheet())
}

fn setting(label: &str, detail: Option<&str>, control: El, p: Palette) -> El {
    let mut words = col().grow(1.0).min_w(0.0).gap(2.0).child(text(label).font(BODY).color(p.text));
    if let Some(d) = detail {
        words = words.child(text(d).font(SMALL).color(p.secondary).wrap());
    }
    row().w_full().py(8.0).gap(16.0).center_y().child(words).child(control)
}

pub fn settings(s: &App, p: Palette) -> El {
    let input = super::field::input(&s.root_edit, BODY, p).key("root-field").single_line().grow(1.0)
        .placeholder("~/Notes", p.tertiary)
        .on_edit(|s: &mut App| &mut s.root_edit)
        .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(&mut s.root_edit, e))
        .on_submit(|s: &mut App, _| s.use_root());
    let changed = crate::settings::expand(&s.root_edit.text) != s.root;
    let use_it = if changed {
        button("Use This Folder", p.on_selection, p.accent, p).key("use-root").on_press(|s: &mut App| s.use_root())
    } else {
        button("In Use", p.tertiary, p.field, p)
    };
    let folder = col().w_full().gap(6.0)
        .child(text("Notes folder").font(BODY).color(p.text))
        .child(row().w_full().gap(8.0).center_y()
            .child(row().grow(1.0).min_w(0.0).child(super::field::boxed(input, 30.0, 10.0, |s| &mut s.root_edit, p)))
            .child(use_it))
        .child(text("Each note is a Markdown (.md) file here and each folder a folder, so any editor can open them and \
            git, Dropbox or Syncthing can sync them. Deleted notes go to .trash inside it.")
            .font(SMALL).color(p.secondary).wrap());
    let appearance = setting("Appearance", None, segmented("set-appearance",
        &[("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)],
        s.settings.appearance, |s, v| s.update_settings(|st| st.appearance = v), p), p);
    let c = card("settings", 520.0, p)
        .child(row().w_full().center_y()
            .child(text("Settings").font(style::BIG).color(p.text).grow(1.0))
            .child(button("Done", p.on_selection, p.accent, p).key("settings-done").default_button().on_press(|s: &mut App| s.close_sheet())))
        .child(folder)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(appearance);
    scrim("settings", c, p)
}

/// Every folder, to move a note into; its own is greyed.
pub fn move_to(s: &App, note: &Path, p: Palette) -> El {
    let title = s.tree.note(note).map_or_else(String::new, |n| n.title.clone());
    let here = note.parent().map(Path::to_owned);
    let mut list = col().w_full().gap(1.0);
    for (f, depth) in s.tree.root.walk() {
        let own = Some(&f.path) == here.as_ref();
        let (to, from) = (f.path.clone(), note.to_owned());
        let mut r = row().key(format!("move-{}", f.path.to_string_lossy())).w_full().h(28.0).pl(8.0 + depth as f32 * 16.0).pr(8.0)
            .gap(8.0).center_y().radius(6.0).opacity(if own { 0.4 } else { 1.0 })
            .child(icon(&style::FOLDER, 15.0).color(p.accent))
            .child(text(s.folder_name(&f.path)).font(BODY).color(p.text).truncate().grow(1.0).min_w(0.0));
        if !own {
            r = r.hover(p.hover).on_press(move |s: &mut App| s.move_note(&from, &to));
        }
        list = list.child(r);
    }
    let c = card("move-to", 380.0, p)
        .child(col().gap(2.0)
            .child(text("Move to Folder").font(style::TITLE).color(p.text))
            .child(text(title).font(SMALL).color(p.secondary).truncate()))
        .child(col().w_full().max_h(360.0).scroll(ScrollSpec { stick_bottom: false }).child(list))
        .child(row().w_full().justify_end()
            .child(button("Cancel", p.text, p.field, p).key("move-cancel").on_press(|s: &mut App| s.close_sheet())));
    scrim("move-to", c, p)
}

pub fn delete_folder(s: &App, f: &Path, p: Palette) -> El {
    let name = s.folder_name(f);
    let n = s.notes_under(f);
    let what = match n {
        0 => "The folder".to_owned(),
        1 => "The folder and the note in it".to_owned(),
        n => format!("The folder and the {n} notes in it"),
    };
    let target = f.to_owned();
    let c = card("delete-folder", 400.0, p)
        .child(text(format!("Delete “{name}”?")).font(style::TITLE).color(p.text))
        .child(text(format!("{what} will move to .trash in your notes folder, where you can get them back.")).font(BODY).color(p.secondary).wrap())
        .child(row().w_full().pt(4.0).gap(8.0).justify_end()
            .child(button("Cancel", p.text, p.field, p).key("delete-cancel").on_press(|s: &mut App| s.close_sheet()))
            .child(row().key("delete-confirm").h(26.0).px(12.0).radius(6.0).center().fill(p.danger).default_button()
                .on_press(move |s: &mut App| s.trash_folder(&target))
                .child(text("Delete").font(BODY_BOLD).color(Color::WHITE))));
    scrim("delete-folder", c, p)
}
