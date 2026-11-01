//! Month: six weeks of days, each listing what it holds as far as it fits.

use chrono::{Datelike, Days, NaiveDate};
use rune::{Color, El, col, el, row, text};

use super::grid::{chip, ink};
use super::style::{EVENT, EVENT_TIME, Palette, SMALL};
use super::{clock, day_name};
use crate::app::App;

/// Points per event line in a day.
const LINE: f32 = 19.0;
/// The date at the top of each day.
const HEAD: f32 = 28.0;

pub fn view(s: &App, height: f32, p: Palette) -> El {
    let (start, _) = s.span();
    let mut names = row().key("month-names").w_full().h(28.0);
    for i in 0..7 {
        names = names.child(row().grow(1.0).min_w(0.0).h_full().pr(10.0).center_y().justify_end()
            .child(text(day_name(start + Days::new(i))).font(SMALL).color(p.secondary)));
    }
    // Lines that fit in a day once its date is drawn, one kept for "n more".
    let cell = ((height - 28.0) / 6.0).max(60.0);
    let fits = (((cell - HEAD - 4.0) / LINE).floor() as usize).max(1);
    let mut weeks = col().key("month").w_full().grow(1.0).min_h(0.0);
    for w in 0..6 {
        let mut r = row().w_full().grow(1.0).min_h(0.0);
        for i in 0..7 {
            r = r.child(day(s, start + Days::new(w * 7 + i), fits, p));
        }
        weeks = weeks.child(el().w_full().h(1.0).fill(p.separator)).child(r);
    }
    col().w_full().h_full().child(names).child(weeks)
}

fn day(s: &App, d: NaiveDate, fits: usize, p: Palette) -> El {
    let this_month = d.month() == s.date.month();
    let today = d == s.today;
    let selected = d == s.date;
    let number = text(if d.day() == 1 { format!("{} {}", super::month_short(d), d.day()) } else { d.day().to_string() })
        .font(if today { super::style::BODY_BOLD } else { super::style::BODY })
        .color(if today { Color::WHITE } else if this_month { p.text } else { p.tertiary });
    let number = row().h(22.0).px(if today { 7.0 } else { 2.0 }).radius(11.0).center()
        .fill(if today { p.today } else { Color::TRANSPARENT })
        .help("Open this day")
        .on_press(move |s: &mut App| s.open_day(d))
        .child(number);
    let mut c = col().key(format!("cell-{d}")).grow(1.0).min_w(0.0).h_full().px(4.0).gap(1.0)
        .fill(if selected { p.accent.alpha(0.08) } else if this_month { Color::TRANSPARENT } else { p.shade })
        .on_press(move |s: &mut App| s.date = d)
        .child(row().w_full().h(HEAD).center_y().justify_end().child(number));
    let list = s.on(d);
    let shown = if list.len() > fits { fits.saturating_sub(1) } else { list.len() };
    for ev in &list[..shown] {
        c = c.child(if ev.all_day { chip(s, ev, d, p) } else { timed(s, ev, p) });
    }
    if list.len() > shown {
        c = c.child(row().w_full().h(LINE - 2.0).px(4.0).center_y().radius(4.0).hover(p.hover)
            .on_press(move |s: &mut App| s.open_day(d))
            .child(text(format!("{} more", list.len() - shown)).font(EVENT_TIME).color(p.secondary)));
    }
    // A line between days.
    row().grow(1.0).min_w(0.0).h_full().child(c).child(el().w(1.0).h_full().fill(p.separator))
}

/// A timed event in a day: its calendar's dot, the title and when it starts.
fn timed(s: &App, ev: &crate::caldav::Event, p: Palette) -> El {
    let colour = s.colour_of(&ev.calendar);
    let e = ev.clone();
    let selected = s.is_selected(ev);
    let key = format!("month-{}-{}", ev.uid, ev.start);
    let k = key.clone();
    let (fill, title_ink, time_ink) = if selected {
        (colour, Color::WHITE, Color::WHITE.alpha(0.85))
    } else {
        (Color::TRANSPARENT, p.text, p.secondary)
    };
    row().key(key).w_full().h(LINE - 2.0).px(4.0).gap(5.0).center_y().radius(4.0).fill(fill).hover(p.hover)
        .on_press(move |s: &mut App| s.select(&e, k.clone()))
        .child(el().size(7.0, 7.0).radius(3.5).fill(if selected { Color::WHITE } else { colour }))
        .child(text(ev.title.clone()).font(EVENT).color(if selected { title_ink } else { ink(colour, p).mix(p.text, 0.6) })
            .truncate().grow(1.0).min_w(0.0))
        .child(text(clock(ev.start)).font(EVENT_TIME).color(time_ink))
}
