//! The open note: its Markdown source in a text field, headings set larger, or the Markdown
//! rendered (Ctrl+E switches). Return carries a list on; Tab indents.

use rune::{Cx, El, Key, KeyEvent, ScrollSpec, Span, TextStyle, Tx, col, el, row, text};
use rune_kit::{Markdown, MarkdownStyle, PaletteSeed};

use super::style::{self, HEADINGS, NOTE, Palette, SMALL};
use super::tool;
use crate::app::{App, Sheet};
use crate::store;

/// The widest a line of the note runs, for reading.
const MEASURE: f32 = 720.0;

pub fn view(s: &App, cx: &mut Cx<App>, p: Palette) -> El {
    let Some(open) = &s.open else {
        let hint = if s.tree.notes.is_empty() { "No notes yet: Ctrl+N starts one" } else { "No note selected" };
        return col().key("editor").grow(1.0).min_w(0.0).h_full().fill(p.content)
            .child(el().w_full().h(52.0))
            .child(col().w_full().grow(1.0).center().child(text(hint).font(style::BODY).color(p.tertiary)));
    };
    let viewport = cx.layout_size("editor-scroll").map_or(600.0, |[_, h]| h);
    let body = if s.preview { preview(s, p) } else { source(s, viewport, p) };
    let when = match open.disk {
        Some(t) => super::when_long(t),
        None => "New note".into(),
    };
    col().key("editor").grow(1.0).min_w(0.0).h_full().fill(p.content)
        .child(toolbar(s, p))
        .child(col().key("editor-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false })
            .child(row().w_full().justify_center().px(36.0)
                .child(col().w_full().max_w(MEASURE).gap(10.0)
                    .child(row().w_full().justify_center().pt(2.0).child(text(when).font(SMALL).color(p.tertiary)))
                    .child(body))))
}

fn toolbar(s: &App, p: Palette) -> El {
    let path = s.open.as_ref().and_then(|o| o.path.clone());
    let mut bar = row().key("editor-toolbar").w_full().h(52.0).px(16.0).gap(4.0).center_y().justify_end();
    if let Some(path) = path {
        bar = bar.child(tool(&style::FOLDER_INPUT, "Move to folder…", p).key("move-note")
            .on_press(move |s: &mut App| s.sheet = Sheet::MoveTo(path.clone())));
    }
    let (i, help) = if s.preview { (&style::PENCIL, "Edit the Markdown (Ctrl+E)") } else { (&style::EYE, "Preview (Ctrl+E)") };
    bar.child(tool(i, help, p).key("toggle-preview").on_press(|s: &mut App| s.toggle_preview()))
        .child(tool(&style::TRASH, "Delete note", p).key("delete-note").on_press(|s: &mut App| s.trash_open()))
}

fn source(s: &App, viewport: f32, p: Palette) -> El {
    super::field::input(&s.editor, NOTE, p).key("note-source").w_full().min_h((viewport - 60.0).max(200.0)).pb(40.0)
        .placeholder("Start with a # Title", p.tertiary)
        .decorations(headings(&s.editor.text, p))
        .on_edit(|s: &mut App| &mut s.editor)
        .on_input_key(|s: &mut App, e: &KeyEvent, _: &mut Tx| {
            let took = key(s, e);
            if took {
                s.edited();
            }
            took
        })
        .on_input_change(|s: &mut App, _| s.edited())
        // A field submits on Return; a note wants the line break.
        .on_submit(|s: &mut App, _| {
            s.editor.insert("\n");
            s.edited();
        })
}

/// The editor's own keys: platform editing keys, Return carrying a list on, Tab indenting.
fn key(s: &mut App, e: &KeyEvent) -> bool {
    if super::field::platform_key(&mut s.editor, e) {
        return true;
    }
    let plain = !e.mods.cmd && !e.mods.ctrl && !e.mods.alt;
    match e.key {
        Key::Enter if plain && !e.mods.shift => {
            let ed = &mut s.editor;
            let caret = ed.focus.max(ed.anchor);
            let start = ed.text[..ed.focus.min(ed.anchor)].rfind('\n').map_or(0, |i| i + 1);
            match store::continue_list(&ed.text[start..ed.focus.min(ed.anchor)]) {
                // Return on an empty item ends the list: the marker goes.
                Some(next) if next.is_empty() => {
                    ed.set_selection(start, caret);
                    ed.insert("");
                }
                Some(next) => ed.insert(&format!("\n{next}")),
                None => ed.insert("\n"),
            }
            true
        }
        Key::Tab if plain && !e.mods.shift => {
            s.editor.insert("    ");
            true
        }
        _ => false,
    }
}

/// Headings drawn larger and bold, with their `#`s faint, so the source reads like the note.
fn headings(t: &str, p: Palette) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut at = 0;
    let mut fenced = false;
    for line in t.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && let Some((level, _)) = store::heading(bare) {
            let size = HEADINGS[level.min(3) - 1];
            let hashes = bare.len() - bare.trim_start().len() + level;
            spans.push(Span::new(at, at + hashes).size(size).weight(700.0).color(p.tertiary));
            if hashes < bare.len() {
                spans.push(Span::new(at + hashes, at + bare.len()).size(size).weight(700.0));
            }
        }
        at += line.len();
    }
    spans
}

fn preview(s: &App, p: Palette) -> El {
    let heading = |i: usize, lh: f32| TextStyle::new(HEADINGS[i], 700.0).lh(lh);
    let style = MarkdownStyle::from_seed(PaletteSeed { bg: p.content, fg: p.text, accent: p.accent })
        .body_font(NOTE)
        .heading_font(1, heading(0, 34.0))
        .heading_font(2, heading(1, 28.0))
        .heading_font(3, heading(2, 24.0))
        .selection(p.text_selection);
    let body = if s.editor.text.trim().is_empty() {
        text("Nothing here yet").font(NOTE).color(p.tertiary)
    } else {
        Markdown::new(s.editor.text.clone())
            .selectable(true)
            .on_task(|s: &mut App, _, at, done| s.set_task(at, done))
            .render(&style)
    };
    col().key("note-preview").w_full().pb(40.0).child(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_outside_code_are_set_larger() {
        let p = Palette::new(false);
        let spans = headings("# Title\nbody\n## Sub\n```\n# not a heading\n```\n", p);
        let ranges: Vec<(u32, u32, Option<f32>)> = spans.iter().map(|s| (s.start, s.end, s.size)).collect();
        assert_eq!(ranges, [(0, 1, Some(26.0)), (1, 7, Some(26.0)), (13, 15, Some(21.0)), (15, 19, Some(21.0))]);
    }
}
