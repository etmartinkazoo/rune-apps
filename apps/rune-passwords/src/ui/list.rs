//! The list: the category's items by name, each with its site's icon, name and username. A
//! two-finger swipe left slides a row open on its buttons (`swipe.rs`).

use rune::{Color, El, GrowthSpec, ScrollSpec, Tx, col, el, icon, row, text};

use super::field::{input, platform_key};
use super::style::{self, BIG, BODY, BODY_BOLD, Palette, SMALL};
use super::tool;
use crate::app::App;
use crate::swipe::{Action, BUTTON, FULL, actions};
use crate::vault::{Item, Kind};

pub const WIDTH: f32 = 300.0;
const ROW: f32 = 52.0;

pub fn view(s: &App, p: Palette) -> El {
    let items = s.items();
    let title = s.category.title(s.vault.as_ref());
    let count = items.len();
    let noun = if count == 1 { "item" } else { "items" };
    let head = row().w_full().px(16.0).pt(14.0).pb(8.0).gap(6.0).center_y()
        .child(col().grow(1.0).min_w(0.0)
            .child(text(title).font(BIG).color(p.text).truncate())
            .child(text(format!("{count} {noun}")).font(SMALL).color(p.secondary)))
        .child(tool(&style::PLUS, "New Password (Ctrl+N)", p).key("new").on_press(|s: &mut App| s.new_item()));
    let search = row().w_full().h(28.0).px(8.0).gap(6.0).center_y().radius(7.0).fill(p.field)
        .child(icon(&style::SEARCH, 13.0).color(p.secondary))
        .child(input(&s.search, BODY, p).key("search").single_line().grow(1.0).min_w(0.0).placeholder("Search", p.tertiary)
            .on_edit(|s: &mut App| &mut s.search)
            .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut Tx| {
                match e.key {
                    rune::Key::Escape if !s.search.text.is_empty() => s.search = rune::TextEdit::new(""),
                    rune::Key::Down => s.step(1),
                    rune::Key::Up => s.step(-1),
                    _ => return platform_key(&mut s.search, e),
                }
                true
            }))
        .on_press(|s: &mut App| s.search.focused = true);
    let body = if items.is_empty() {
        let note = if s.search.text.trim().is_empty() { "Nothing here" } else { "No matches" };
        col().w_full().grow(1.0).center().child(text(note).font(BODY).color(p.secondary))
    } else {
        let rows = col().key("rows").w_full().px(8.0).growth(GrowthSpec::default())
            .virtual_rows(
                count,
                ROW,
                |s: &App, i| format!("item-{}", s.items().get(i).map_or("", |m| m.id.as_str())).into(),
                move |s: &App, i| s.items().get(i).map_or_else(el, |m| item_row(s, m, p)),
            );
        col().key("list-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(rows)
    };
    col().key("list").w(WIDTH).h_full().fill(p.content).gap(6.0)
        .child(head)
        .child(row().w_full().px(12.0).child(search))
        .child(body.pt(4.0))
}

fn item_row(s: &App, m: &Item, p: Palette) -> El {
    let on = s.selected.as_ref() == Some(&m.id);
    let (ink, muted) = if on { (p.on_selection, p.on_selection.alpha(0.8)) } else { (p.text, p.secondary) };
    let id = m.id.clone();
    let mut words = col().grow(1.0).min_w(0.0)
        .child(text(if m.name.is_empty() { "(no name)".into() } else { m.name.clone() }).font(BODY_BOLD).color(ink).truncate());
    let sub = m.subtitle();
    if !sub.is_empty() {
        words = words.child(text(sub).font(SMALL).color(muted).truncate());
    }
    let offset = s.swipe_offset(&m.id);
    let wheel_id = m.id.clone();
    // Slid by a swipe, the row covers its buttons with a solid fill, square where it meets them
    // (the stack clips the outer corners round).
    let card = row().key("card").w_full().h_full().px(8.0).gap(10.0).center_y().radius(if offset != 0.0 { 0.0 } else { 8.0 })
        .fill(if on { p.accent } else if offset != 0.0 { p.content } else { Color::TRANSPARENT })
        .hover(if on { Color::TRANSPARENT } else { p.hover })
        .offset(offset, 0.0)
        .on_press(move |s: &mut App| s.select(&id))
        .on_wheel(move |s: &mut App, e, tx| s.swipe_wheel(&wheel_id, e, tx))
        .child(badge(s, m, 32.0, p))
        .child(words);
    let mut stack = el().key(format!("row-{}", m.id)).w_full().h(ROW - 4.0).my(2.0).radius(8.0).clip_rounded(8.0);
    if offset != 0.0 {
        stack = stack.child(swipe_buttons(m, offset));
    }
    stack.child(card)
}

/// The buttons a swipe uncovers, on the right. Swiped far enough to delete, Delete fills the
/// row.
fn swipe_buttons(m: &Item, offset: f32) -> El {
    let actions = actions(m);
    let room = -offset;
    let button = |action: Action, w: f32| {
        let (label, glyph, colour) = match action {
            Action::CopyUser => ("User Name", &*style::PERSON, *style::GREY),
            Action::CopyPassword => ("Password", &*style::KEY, *style::BLUE),
            Action::Delete => ("Delete", &*style::TRASH, *style::RED),
            Action::Recover => ("Recover", &*style::REFRESH, *style::BLUE),
            Action::DeleteNow => ("Delete Now", &*style::TRASH, *style::RED),
        };
        let id = m.id.clone();
        col().key(format!("swipe-{action:?}")).w(w).h_full().center().gap(3.0).fill(colour)
            .help(match action {
                Action::CopyUser => "Copy User Name",
                Action::CopyPassword => "Copy Password",
                _ => label,
            })
            .on_press_tx(move |s: &mut App, tx: &mut Tx| s.swipe_action(&id, action, tx))
            .child(icon(glyph, 15.0).color(Color::WHITE))
            .child(text(label).font(style::TINY).color(Color::WHITE))
    };
    let open = BUTTON * actions.len() as f32;
    let mut r = row().pin(None, Some(0.0), Some(0.0), Some(0.0)).w(room.max(open)).justify_end();
    match actions.last() {
        Some(&last @ (Action::Delete | Action::DeleteNow)) if room >= FULL - 20.0 => r = r.child(button(last, room)),
        _ => {
            let w = (room / actions.len().max(1) as f32).min(BUTTON);
            for a in actions {
                r = r.child(button(a, w));
            }
        }
    }
    r
}

/// The item's tile: its site's icon, from the server, or the first letter of its site or
/// name on a colour of its own.
pub fn badge(s: &App, m: &Item, size: f32, p: Palette) -> El {
    let host = m.host();
    if let Some(img) = host.as_deref().and_then(|h| s.icons.get(h))
        && let Some([w, h]) = img.size()
    {
        // Icons are drawn on white, as sites draw them for, with a little room around.
        let fit = size * 0.68 / w.max(h).max(1) as f32;
        return el().size(size, size).radius(size * 0.22).center().fill(Color::WHITE).border(1.0, p.separator)
            .child(el().size(w as f32 * fit, h as f32 * fit).pixels(img.pixels()));
    }
    let name = host.unwrap_or_else(|| m.name.clone());
    let glyph = match m.kind {
        Kind::Note => Some(&*style::NOTE),
        Kind::Card => Some(&*style::CARD),
        _ => None,
    };
    let tile = el().size(size, size).radius(size * 0.22).center().fill(style::tile_colour(&name));
    match glyph {
        Some(i) => tile.child(icon(i, size * 0.5).color(Color::WHITE)),
        None => {
            let letter = name.chars().find(|c| c.is_alphanumeric()).map_or("?".to_owned(), |c| c.to_uppercase().collect());
            tile.child(text(letter).font(rune::TextStyle::new(size * 0.5, 700.0)).color(Color::WHITE))
        }
    }
}
