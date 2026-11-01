//! Text fields that follow the platform's editing conventions.
//!
//! Rune's fields edit the macOS way: ⌘ jumps or deletes to the line's edge, ⌥ works by word,
//! Home and End go to the ends of the text. Off macOS Rune reports Ctrl as ⌘, so Ctrl+Backspace
//! there would delete to the start of the line. `platform_key` remaps the keys whose meaning
//! differs on Linux and Windows before the field sees them:
//!
//! | Keys | Linux and Windows |
//! | --- | --- |
//! | Ctrl+Backspace / Ctrl+Delete | Delete a word back / forward |
//! | Ctrl+← / Ctrl+→ (Shift selects) | Move a word |
//! | Home / End (Shift selects) | Start / end of the line |
//! | Ctrl+Home / Ctrl+End | Start / end of the text |
//! | Ctrl+Y | Redo |
//!
//! Double- and triple-click select a word and a line, Ctrl+A selects all, Ctrl+Z and
//! Ctrl+Shift+Z undo and redo: Rune does those on every platform.

use rune::{El, Key, KeyEvent, Mods, TextEdit, TextStyle, text_input};

use super::style::Base;

/// A text field with a visible caret and selection in the accent colour.
pub fn input(edit: &TextEdit, style: TextStyle, p: &Base) -> El {
    text_input(edit, style).color(p.text).caret(p.accent, p.accent.alpha(if p.dark { 0.4 } else { 0.25 }))
}

/// The rounded box around a single-line field. A press anywhere in it, padding included,
/// focuses the field, as clicking a native text box's border does.
pub fn boxed<S: 'static>(input: El, h: f32, px: f32, pick: impl Fn(&mut S) -> &mut TextEdit + 'static, p: &Base) -> El {
    rune::row().w_full().h(h).px(px).center_y().radius(7.0).fill(p.field).cursor(rune::Cursor::Text)
        .on_press(move |s: &mut S| pick(s).focused = true)
        .child(input)
}

/// Applies a key whose meaning differs off macOS; true when it took it.
pub fn platform_key(edit: &mut TextEdit, e: &KeyEvent) -> bool {
    if cfg!(target_os = "macos") {
        return false;
    }
    let m = e.mods;
    let by_word = Mods { shift: m.shift, alt: true, ..Mods::default() };
    let by_line = Mods { shift: m.shift, cmd: true, ..Mods::default() };
    let plain = Mods { shift: m.shift, ..Mods::default() };
    let (key, mods) = match (&e.key, m.ctrl) {
        (Key::Backspace, true) => (Key::Backspace, by_word),
        (Key::Delete, true) => (Key::Delete, by_word),
        (Key::Left, true) => (Key::Left, by_word),
        (Key::Right, true) => (Key::Right, by_word),
        (Key::Home, false) => (Key::Left, by_line),
        (Key::End, false) => (Key::Right, by_line),
        (Key::Home, true) => (Key::Home, plain),
        (Key::End, true) => (Key::End, plain),
        _ if e.is_command('y') && !m.shift => {
            edit.redo();
            return true;
        }
        _ => return false,
    };
    edit.key(&KeyEvent { key, mods }, None);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(edit: &mut TextEdit, key: Key, ctrl: bool, shift: bool) -> bool {
        // Off macOS, Rune reports Ctrl as both `ctrl` and `cmd`.
        platform_key(edit, &KeyEvent { key, mods: Mods { ctrl, cmd: ctrl, shift, ..Mods::default() } })
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn ctrl_works_by_word_and_home_end_by_line() {
        let mut e = TextEdit::new("first line\nhello brave world");
        let end = e.text.len();
        e.anchor = end;
        e.focus = end;
        assert!(press(&mut e, Key::Backspace, true, false));
        assert_eq!(e.text, "first line\nhello brave ", "Ctrl+Backspace deletes one word, not the line");
        assert!(press(&mut e, Key::Left, true, false));
        assert_eq!(&e.text[e.focus..], "brave ");
        assert!(press(&mut e, Key::Home, false, true));
        assert_eq!((e.focus, e.anchor), (11, 17), "Shift+Home selects to the start of this line");
        assert!(press(&mut e, Key::Home, true, false));
        assert_eq!(e.focus, 0, "Ctrl+Home goes to the start of the text");
        assert!(!press(&mut e, Key::Text("a".into()), false, false));
    }
}
