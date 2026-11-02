//! A message's HTML in the reader: the tiles `page.rs` paints, in rows, with the pointer's
//! hover, clicks and drags sent to the page thread (which knows what's under it).

use rune::{Cursor, El, Phase, col, el, row};

use crate::app::App;
use crate::page::Pointer;

/// How far past the window a tile paints ahead of being scrolled to, in points.
const AHEAD: f32 = 1200.0;
/// A press that moves less than this is a click; more, a selection.
const SLOP: f32 = 3.0;

pub fn view(s: &App, uid: &str) -> El {
    let measured = col().key(format!("page-{uid}")).w_full();
    let Some(page) = s.pages.pages.get(uid) else {
        // Laid out in a few milliseconds; holds the reader's place till then.
        return measured.min_h(60.0);
    };
    let (width, tile, cols) = (page.look.width as f32, page.look.tile(), page.look.cols());
    let rows = page.tiles.chunks(cols).enumerate().map(|(r, tiles)| {
        let top = r as f32 * tile;
        let h = (page.height - top).clamp(1.0, tile);
        row().h(h).children(tiles.iter().enumerate().map(|(c, pixels)| {
            let (u, i) = (uid.to_owned(), r * cols + c);
            let w = (width - c as f32 * tile).clamp(1.0, tile);
            el().w(w).h(h).pixels(pixels).on_drawn_layout(move |s: &mut App, at: [f32; 4]| {
                if at[1] < 2000.0 + AHEAD && at[1] + at[3] > -AHEAD {
                    s.pages.need(&u, i);
                }
            })
        }))
    });
    let (hover, press) = (uid.to_owned(), uid.to_owned());
    let surface = col()
        .w(width)
        .h(page.height)
        .cursor(if page.hover.is_some() { Cursor::Pointer } else { Cursor::Text })
        .on_pointer_move_local(move |s: &mut App, e, _| s.pages.pointer(&hover, e.x, e.y, Pointer::Hover))
        .on_drag(move |s: &mut App, d, _| match d.phase {
            Phase::Down => d.data = 0.0,
            Phase::Move if d.data == 0.0 && d.dx.hypot(d.dy) >= SLOP => {
                d.data = 1.0;
                s.pages.pointer(&press, d.start_x, d.start_y, Pointer::SelectFrom);
                s.pages.pointer(&press, d.x, d.y, Pointer::SelectTo);
            }
            Phase::Move if d.data == 1.0 => s.pages.pointer(&press, d.x, d.y, Pointer::SelectTo),
            Phase::Up if d.data == 0.0 && !d.cancelled() => s.pages.pointer(&press, d.x, d.y, Pointer::Click),
            _ => {}
        })
        .children(rows);
    measured.child(surface)
}
