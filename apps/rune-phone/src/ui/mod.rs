//! macOS Messages and Phone in one window: conversations, calls and contacts down the side, the
//! one chosen beside them, and a call's controls over it all.

pub mod call;
pub mod field;
pub mod pane;
pub mod setup;
pub mod sidebar;
pub mod style;

use rune::{Anim, Color, Cx, El, Icon, Key, KeyEvent, Tx, col, el, icon, row, text};

use crate::actions::{KEYS, SWIPE_ACTS, Target};
use crate::app::{App, Pane};
use style::{BODY, Palette};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let p = Palette::new(s.settings.dark(cx.system_dark()));
    let mut root = el().key("root").w_full().h_full().fill(p.window);
    root = if s.needs_setup() {
        root.child(setup::view(s, p))
    } else {
        root.child(
            row().w_full().h_full()
                .child(sidebar::view(s, p))
                .child(el().w(1.0).h_full().fill(p.separator))
                .child(pane::view(s, p)),
        )
    };
    if let Some(c) = s.ringing() {
        root = root.child(call::incoming(s, c, p));
    }
    if let Some(c) = &s.confirm {
        root = root.child(confirm(&c.what, p));
    }
    if s.show_keys {
        root = root.child(keys(p));
    }
    if let Some((status, _)) = &s.status {
        root = root.child(row().key("status").pin(None, None, Some(16.0), Some(16.0)).px(14.0).h(32.0).center_y()
            .radius(16.0).fill(p.card).border(1.0, p.separator).shadow(4.0, 14.0, Color::BLACK.alpha(if p.dark { 0.4 } else { 0.15 }))
            .child(text(status.clone()).font(BODY).color(p.text)));
    }
    root
}

/// Asking before deleting on the phone.
fn confirm(what: &str, p: Palette) -> El {
    let card = col().key("confirm").w(360.0).padding(22.0).gap(10.0).radius(16.0).fill(p.card).border(1.0, p.separator)
        .shadow(18.0, 48.0, Color::BLACK.alpha(0.35))
        .child(text(format!("Delete {what}?")).font(style::NAME).color(p.text).wrap())
        .child(text("They're deleted on the phone too. This can't be undone.").font(style::SMALL).color(p.secondary).wrap())
        .child(row().w_full().gap(8.0).pt(6.0)
            .child(el().grow(1.0))
            .child(button("Cancel", p.text, p.field, p).key("confirm-no").help("n or Escape")
                .on_press_tx(|s: &mut App, tx: &mut Tx| s.answer_delete(false, tx)))
            .child(button("Delete", Color::WHITE, *style::RED, p).key("confirm-yes").help("y or Return")
                .on_press_tx(|s: &mut App, tx: &mut Tx| s.answer_delete(true, tx))));
    col().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).center().fill(Color::BLACK.alpha(0.3)).child(card)
}

/// The keys Messages answers to, from `?`.
fn keys(p: Palette) -> El {
    let mut list = col().gap(5.0);
    for (k, what) in KEYS {
        list = list.child(row().gap(12.0)
            .child(text((*k).to_owned()).font(style::BODY_BOLD).color(p.text).w(180.0))
            .child(text((*what).to_owned()).font(BODY).color(p.secondary)));
    }
    let card = col().key("keys").padding(22.0).gap(12.0).radius(16.0).fill(p.card).border(1.0, p.separator)
        .shadow(18.0, 48.0, Color::BLACK.alpha(0.35))
        .child(row().w_full().center_y()
            .child(text("Keyboard").font(style::NAME).color(p.text).grow(1.0))
            .child(tool(&style::CLOSE, "Close (? or Escape)", p).key("keys-close").on_press(|s: &mut App| s.show_keys = false)))
        .child(list);
    col().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).center().fill(Color::BLACK.alpha(0.3))
        .on_press(|s: &mut App| s.show_keys = false).child(card)
}

/// A row that can be swiped: `content` moves with the swipe, and what letting go would do shows
/// in the space it leaves (Delete to the left, read or unread to the right). `inset` is the
/// content's own vertical margin, which the action's background keeps to.
pub fn swipeable(s: &App, target: Target, content: El, radius: f32, inset: f32, p: Palette) -> El {
    let dx = s.swiped_by(&target);
    let mut w = col().w_full();
    let mut content = content.offset(dx, 0.0);
    if dx != 0.0 {
        let left = dx < 0.0;
        let unread = match &target {
            Target::Thread(k) => s.thread(k).is_some_and(|t| t.unread > 0),
            Target::Message(id) => s.messages.iter().any(|m| &m.id == id && m.incoming && !m.read),
        };
        let (fill, glyph, label) =
            if left { (*style::RED, &*style::TRASH, "Delete") } else { (p.accent, &*style::MAIL, if unread { "Read" } else { "Unread" }) };
        let ready = dx.abs() >= SWIPE_ACTS;
        // The label once letting go would act; before that the icon alone, which fits sooner.
        let mut action = row().gap(6.0).center_y().opacity(if ready { 1.0 } else { 0.55 }).child(icon(glyph, 15.0).color(Color::WHITE));
        if ready {
            action = action.child(text(label).font(style::BODY_BOLD).color(Color::WHITE));
        }
        let mut bg = row().pin(Some(0.0), Some(inset), Some(0.0), Some(inset)).px(18.0).center_y().radius(radius)
            .fill(fill.alpha(if ready { 1.0 } else { 0.7 }));
        bg = if left { bg.child(el().grow(1.0)).child(action) } else { bg.child(action) };
        w = w.child(bg);
        // With the finger or pointer exactly; it springs back once let go.
        content = content.offset_motion(Anim::INSTANT);
    }
    w.child(content)
}

/// A square toolbar button with an icon.
pub fn tool(i: &'static Icon, help: &str, p: Palette) -> El {
    el().size(28.0, 26.0).radius(6.0).center().hover(p.hover).help(help.to_owned()).child(icon(i, 15.0).color(p.secondary))
}

pub fn button(label: &str, ink: Color, fill: Color, p: Palette) -> El {
    row().h(28.0).px(14.0).radius(7.0).center().fill(fill).hover(p.hover).child(text(label.to_owned()).font(BODY).color(ink))
}

/// A round button, as the call controls are.
pub fn round(i: &'static Icon, fill: Color, ink: Color, size: f32) -> El {
    el().size(size, size).radius(size / 2.0).center().fill(fill).press_scale(0.9).child(icon(i, size * 0.42).color(ink))
}

/// A person's initials on their colour; a number, with no name, gets a silhouette.
pub fn avatar(name: &str, size: f32) -> El {
    let letters: String = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphabetic())).take(2).collect::<String>().to_uppercase();
    let circle = el().size(size, size).radius(size / 2.0).center();
    if letters.is_empty() {
        return circle.fill(style::avatar_colour("")).child(icon(&style::PERSON, size * 0.5).color(Color::WHITE));
    }
    circle.fill(style::avatar_colour(name)).child(text(letters).font(rune::TextStyle::new(size * 0.4, 600.0)).color(Color::WHITE))
}

/// ⌘N a new message, ⌘F to search, ⌘1-3 the tabs; on the keypad, digits dial.
pub fn on_key(s: &mut App, e: &KeyEvent, tx: &mut Tx) -> bool {
    match &e.key {
        _ if e.is_command('n') => s.compose(),
        _ if e.is_command('f') => s.search.focused = true,
        _ if e.is_command('1') => s.tab = crate::app::Tab::Messages,
        _ if e.is_command('2') => s.tab = crate::app::Tab::Calls,
        _ if e.is_command('3') => s.tab = crate::app::Tab::Contacts,
        Key::Escape if s.setup_open => s.setup_open = false,
        Key::Text(t) if s.pane == Pane::Keypad && t.len() == 1 && "0123456789*#+".contains(t.as_str()) && !e.mods.cmd => s.press_key(t),
        Key::Enter if s.pane == Pane::Keypad && !s.dial.text.is_empty() => {
            let n = s.dial.text.clone();
            s.call(&n, tx);
        }
        Key::Backspace if s.pane == Pane::Keypad => {
            let mut t = s.dial.text.clone();
            t.pop();
            s.dial = rune::TextEdit::new(&t);
        }
        _ => return s.vim_key(e, tx),
    }
    true
}
