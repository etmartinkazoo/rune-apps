//! Day and Week: a column per day over the 24 hours, all-day events above it. Events that
//! overlap share their column side by side, as Apple Calendar does.

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, Timelike};
use rune::{Color, El, ScrollSpec, col, el, row, text};

use super::style::{self, EVENT, EVENT_TIME, Palette, SMALL};
use super::{clock, day_name};
use crate::app::App;
use crate::caldav::Event;

/// Points per hour.
pub const HOUR: f32 = 48.0;
/// The hour labels' column.
pub const GUTTER: f32 = 56.0;
/// All-day events shown per day before "+n more".
const ALL_DAY_ROWS: usize = 3;

pub fn view(s: &App, days: &[NaiveDate], width: f32, p: Palette) -> El {
    let column = ((width - GUTTER) / days.len() as f32).max(40.0);
    let week = days.len() > 1;
    let mut header = row().key("grid-header").w_full().h(if week { 52.0 } else { 0.0 }).child(el().w(GUTTER));
    if week {
        for &d in days {
            header = header.child(day_head(s, d, p));
        }
    }
    let all_day: Vec<Vec<&Event>> = days.iter().map(|d| s.on(*d).into_iter().filter(|e| e.all_day).collect()).collect();
    let mut body = col().w_full().h_full();
    if week {
        body = body.child(header).child(line(p));
    }
    if all_day.iter().any(|l| !l.is_empty()) {
        let mut r = row().key("all-day").w_full().py(4.0)
            .child(row().w(GUTTER).pr(8.0).pt(2.0).justify_end().child(text("all-day").font(EVENT_TIME).color(p.secondary)));
        for (d, list) in days.iter().zip(&all_day) {
            r = r.child(all_day_cell(s, *d, list, p));
        }
        body = body.child(r).child(line(p));
    }
    let mut hours = el().w(GUTTER).h(24.0 * HOUR);
    for h in 1..24 {
        hours = hours.child(row().absolute(0.0, h as f32 * HOUR - 7.0).w(GUTTER - 8.0).justify_end()
            .child(text(hour_label(h)).font(EVENT_TIME).color(p.secondary)));
    }
    let mut columns = row().w_full().h(24.0 * HOUR).child(hours);
    for &d in days {
        columns = columns.child(day_column(s, d, column, p));
    }
    body.child(col().key("grid").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(columns))
}

fn line(p: Palette) -> El {
    el().w_full().h(1.0).fill(p.separator)
}

fn day_head(s: &App, d: NaiveDate, p: Palette) -> El {
    let today = d == s.today;
    let number = text(d.day().to_string()).font(style::TITLE).color(if today { Color::WHITE } else { p.text });
    let number = if today {
        el().size(28.0, 28.0).radius(14.0).fill(p.today).center().child(number)
    } else {
        el().size(28.0, 28.0).center().child(number)
    };
    row().key(format!("head-{d}")).grow(1.0).min_w(0.0).h_full().gap(6.0).center().hover(p.hover)
        .help("Open this day")
        .on_press(move |s: &mut App| s.open_day(d))
        .child(text(day_name(d)).font(SMALL).color(if today { p.today } else { p.secondary }))
        .child(number)
}

fn all_day_cell(s: &App, d: NaiveDate, list: &[&Event], p: Palette) -> El {
    let mut c = col().grow(1.0).min_w(0.0).px(2.0).gap(2.0);
    for ev in list.iter().take(ALL_DAY_ROWS) {
        c = c.child(chip(s, ev, d, p));
    }
    if list.len() > ALL_DAY_ROWS {
        c = c.child(row().px(6.0).on_press(move |s: &mut App| s.open_day(d))
            .child(text(format!("{} more", list.len() - ALL_DAY_ROWS)).font(EVENT_TIME).color(p.secondary)));
    }
    c
}

/// An all-day event: a filled bar in its calendar's colour.
pub fn chip(s: &App, ev: &Event, day: NaiveDate, p: Palette) -> El {
    let colour = s.colour_of(&ev.calendar);
    let e = ev.clone();
    // An event over several days has a chip on each.
    let key = format!("chip-{}-{}-{day}", ev.uid, ev.start);
    let k = key.clone();
    row().key(key).w_full().h(18.0).px(6.0).center_y().radius(4.0).fill(p.content.mix(colour, if p.dark { 0.5 } else { 0.38 }))
        .on_press(move |s: &mut App| s.select(&e, k.clone()))
        .child(text(ev.title.clone()).font(EVENT).color(ink(colour, p)).truncate().min_w(0.0))
}

/// Text in an event's colour, dark enough to read on its tint.
pub fn ink(colour: Color, p: Palette) -> Color {
    if p.dark { colour.mix(Color::WHITE, 0.55) } else { colour.mix(Color::BLACK, 0.45) }
}

fn day_column(s: &App, d: NaiveDate, width: f32, p: Palette) -> El {
    let mut c = el().key(format!("day-{d}")).grow(1.0).min_w(0.0).h(24.0 * HOUR)
        .fill(if d.weekday().number_from_monday() >= 6 { p.shade } else { Color::TRANSPARENT })
        .child(el().pin(Some(0.0), Some(0.0), None, Some(0.0)).w(1.0).fill(p.separator));
    for h in 1..24 {
        c = c.child(el().pin(Some(0.0), Some(h as f32 * HOUR), Some(0.0), None).h(1.0).fill(p.separator));
    }
    let timed: Vec<&Event> = s.on(d).into_iter().filter(|e| !e.all_day).collect();
    for (ev, lane, lanes) in lanes(&timed, d) {
        c = c.child(block(s, ev, d, lane, lanes, width, p));
    }
    if d == s.today {
        let now = Local::now().naive_local();
        let y = minutes(now) / 60.0 * HOUR;
        c = c.child(el().key("now").pin(Some(0.0), Some(y - 1.0), Some(0.0), None).h(2.0).fill(p.today))
            .child(el().absolute(-4.0, y - 4.0).size(8.0, 8.0).radius(4.0).fill(p.today));
    }
    c
}

fn minutes(t: NaiveDateTime) -> f32 {
    t.hour() as f32 * 60.0 + t.minute() as f32
}

/// Where `ev` sits on `day`: from its start (or midnight) to its end (or the next midnight).
fn extent(ev: &Event, day: NaiveDate) -> (f32, f32) {
    let start = if ev.start.date() < day { 0.0 } else { minutes(ev.start) };
    let end = if ev.end.date() > day { 24.0 * 60.0 } else { minutes(ev.end) };
    (start, end.max(start + 15.0))
}

/// Each event with its lane and how many lanes its overlapping group has.
pub fn lanes<'a>(events: &[&'a Event], day: NaiveDate) -> Vec<(&'a Event, usize, usize)> {
    let mut sorted: Vec<&Event> = events.to_vec();
    sorted.sort_by(|a, b| extent(a, day).0.total_cmp(&extent(b, day).0).then(extent(b, day).1.total_cmp(&extent(a, day).1)));
    let mut out: Vec<(&Event, usize, usize)> = Vec::new();
    let mut group: Vec<(&Event, usize)> = Vec::new();
    let mut lane_ends: Vec<f32> = Vec::new();
    let mut group_end = f32::MIN;
    let flush = |group: &mut Vec<(&'a Event, usize)>, lane_ends: &mut Vec<f32>, out: &mut Vec<(&'a Event, usize, usize)>| {
        let n = lane_ends.len().max(1);
        out.extend(group.drain(..).map(|(e, l)| (e, l, n)));
        lane_ends.clear();
    };
    for ev in sorted {
        let (start, end) = extent(ev, day);
        if start >= group_end && !group.is_empty() {
            flush(&mut group, &mut lane_ends, &mut out);
        }
        let lane = match lane_ends.iter().position(|e| *e <= start) {
            Some(i) => {
                lane_ends[i] = end;
                i
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        group.push((ev, lane));
        group_end = if group.len() == 1 { end } else { group_end.max(end) };
    }
    flush(&mut group, &mut lane_ends, &mut out);
    out
}

fn block(s: &App, ev: &Event, day: NaiveDate, lane: usize, lanes: usize, width: f32, p: Palette) -> El {
    let (start, end) = extent(ev, day);
    let (top, height) = (start / 60.0 * HOUR + 1.0, ((end - start) / 60.0 * HOUR - 2.0).max(16.0));
    let w = (width - 6.0) / lanes as f32;
    let colour = s.colour_of(&ev.calendar);
    let selected = s.is_selected(ev);
    // Solid, so the hour lines don't show through.
    let fill = if selected { colour } else { p.content.mix(colour, if p.dark { 0.4 } else { 0.3 }) };
    let ink = if selected { Color::WHITE } else { ink(colour, p) };
    let mut words = col().grow(1.0).min_w(0.0).px(5.0).py(2.0).gap(1.0)
        .child(text(ev.title.clone()).font(EVENT).color(ink).truncate().min_w(0.0));
    if height >= 30.0 {
        let when = format!("{} – {}", clock(ev.start), clock(ev.end));
        words = words.child(text(when).font(EVENT_TIME).color(ink.alpha(0.8)).truncate().min_w(0.0));
    }
    if height >= 46.0
        && let Some(l) = &ev.location
    {
        words = words.child(text(l.clone()).font(EVENT_TIME).color(ink.alpha(0.8)).truncate().min_w(0.0));
    }
    let e = ev.clone();
    let key = format!("event-{}-{}-{day}", ev.uid, ev.start);
    let k = key.clone();
    row().key(key).absolute(2.0 + lane as f32 * w, top).w(w - 2.0).h(height)
        .radius(5.0).clip_rounded(5.0).fill(fill)
        .help(ev.title.clone())
        .on_press(move |s: &mut App| s.select(&e, k.clone()))
        .child(el().w(3.0).h_full().fill(colour))
        .child(words)
}

fn hour_label(h: u32) -> String {
    match h {
        12 => "Noon".into(),
        h if h < 12 => format!("{h} AM"),
        h => format!("{} PM", h - 12),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(uid: &str, from: (u32, u32), to: (u32, u32)) -> Event {
        let d = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        Event {
            uid: uid.into(), title: uid.into(), calendar: "c".into(),
            start: d.and_hms_opt(from.0, from.1, 0).unwrap(), end: d.and_hms_opt(to.0, to.1, 0).unwrap(),
            ..Event::default()
        }
    }

    #[test]
    fn overlapping_events_share_the_column() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let (a, b, c, z) = (ev("a", (9, 0), (10, 0)), ev("b", (9, 30), (11, 0)), ev("c", (10, 0), (10, 30)), ev("z", (14, 0), (15, 0)));
        let got: Vec<(String, usize, usize)> =
            lanes(&[&a, &b, &c, &z], d).into_iter().map(|(e, l, n)| (e.uid.clone(), l, n)).collect();
        assert_eq!(got, [
            ("a".into(), 0, 2), ("b".into(), 1, 2), ("c".into(), 0, 2), // c takes a's lane once a ends
            ("z".into(), 0, 1),
        ]);
    }
}
