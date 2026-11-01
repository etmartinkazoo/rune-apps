//! The controls every app draws the same way: buttons, a segmented control, a switch, the rows
//! and sections of a settings sheet, and the sheet itself.
//!
//! They're generic over the app's state `S`, so one definition serves every app; handlers
//! take `&mut S` and the frame's `Tx`.

use rune::{Color, El, Icon, ScrollSpec, Tx, col, el, icon, row, text};

use super::style::{BODY, BODY_BOLD, Base, SECTION, SMALL};

/// A push button: `ink` on `fill`. Attach `on_press` to it.
pub fn button(label: &str, ink: Color, fill: Color, p: &Base) -> El {
    row().h(26.0).px(12.0).radius(6.0).center().fill(fill).hover(p.hover).child(text(label.to_owned()).font(BODY).color(ink))
}

/// A square toolbar button with an icon.
pub fn tool(i: &'static Icon, help: &str, p: &Base) -> El {
    el().size(28.0, 26.0).radius(6.0).center().hover(p.hover).help(help.to_owned()).child(icon(i, 15.0).color(p.secondary))
}

/// A segmented control: one of `options`, `current` highlighted; pressing one calls `set`.
pub fn segmented<S: 'static, T: Copy + PartialEq + 'static>(
    key: &str,
    options: &[(&str, T)],
    current: T,
    set: impl Fn(&mut S, T, &mut Tx) + Copy + 'static,
    p: &Base,
) -> El {
    let mut r = row().key(key.to_owned()).role("radiogroup").h(26.0).padding(2.0).gap(2.0).radius(7.0).fill(p.field);
    for (label, value) in options {
        let (label, value) = (*label, *value);
        let on = value == current;
        r = r.child(
            row().key(format!("{key}-{}", label.to_lowercase().replace(' ', "-"))).h(22.0).px(12.0).center().radius(5.0)
                .fill(if on { p.content } else { Color::TRANSPARENT })
                .hover(p.hover)
                .on_press_tx(move |s: &mut S, tx: &mut Tx| set(s, value, tx))
                .child(text(label).font(SMALL).color(if on { p.text } else { p.secondary })),
        );
    }
    r
}

/// An on/off switch; pressing it calls `set` with the new value.
pub fn switch<S: 'static>(key: &str, on: bool, set: impl Fn(&mut S, bool, &mut Tx) + 'static, p: &Base) -> El {
    let knob = el().size(16.0, 16.0).radius(8.0).fill(Color::WHITE).shadow(1.0, 2.0, Color::BLACK.alpha(0.25))
        .absolute(if on { 16.0 } else { 2.0 }, 2.0).motion(rune::tok::motion::SNAPPY);
    el().key(key.to_owned()).role("switch").focusable(true).size(34.0, 20.0).radius(10.0)
        .fill(if on { p.accent } else { p.field.alpha(0.25).mix(p.secondary, 0.5) })
        .motion(rune::tok::motion::SNAPPY)
        .on_press_tx(move |s: &mut S, tx: &mut Tx| set(s, !on, tx))
        .child(knob)
}

/// A section's heading in a settings sheet.
pub fn section(title: &str, p: &Base) -> El {
    row().w_full().pt(14.0).pb(6.0).child(text(title.to_uppercase()).font(SECTION).color(p.secondary))
}

/// A row of a settings sheet: what it is (and a line on what it does), its control on the right.
pub fn setting(label: &str, detail: Option<&str>, control: El, p: &Base) -> El {
    let mut words = col().grow(1.0).min_w(0.0).gap(2.0).child(text(label.to_owned()).font(BODY).color(p.text));
    if let Some(d) = detail {
        words = words.child(text(d.to_owned()).font(SMALL).color(p.secondary).wrap());
    }
    row().w_full().py(8.0).gap(16.0).center_y().child(words).child(control)
}

/// A sheet over the window: a title bar with Done, `body` scrolling under it, the window dimmed
/// behind. Done, Escape and a press outside call `close`.
pub fn sheet<S: 'static>(key: &str, title: &str, w: f32, body: El, close: fn(&mut S), p: &Base) -> El {
    let bar = row().w_full().h(44.0).px(16.0).center_y()
        .child(text(title.to_owned()).font(BODY_BOLD).color(p.text).grow(1.0))
        .child(button("Done", p.on_selection, p.accent, p).default_button().on_press(move |s: &mut S| close(s)));
    let card = col().key(key.to_owned()).w(w).max_h(620.0).radius(12.0).clip_rounded(12.0)
        .fill(p.content).border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, move |s: &mut S| close(s))
        .child(bar)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(col().w_full().shrink().min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(body));
    el().key(format!("{key}-scrim")).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}
