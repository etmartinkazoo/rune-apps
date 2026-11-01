//! Today's events, under the mailboxes: every account's calendars, all-day events first.

use chrono::Local;
use rune::color::hex;
use rune::{El, TextStyle, col, el, icon, row, text};

use super::style::{self, Palette, SECTION, SMALL};
use crate::app::{App, Sheet};
use crate::caldav::Event;

const MAX_SHOWN: usize = 5;
const TITLE: TextStyle = TextStyle::new(12.5, 500.0);

pub fn view(s: &App, p: Palette) -> El {
    let now = Local::now();
    let head = row().w_full().px(8.0).pb(4.0).gap(6.0).center_y()
        .child(icon(&style::CALENDAR, 12.0).color(p.secondary))
        .child(text("TODAY").font(SECTION).color(p.secondary))
        .child(el().grow(1.0))
        .child(text(now.format("%a %-d %b").to_string()).font(SMALL).color(p.tertiary));
    let mut panel = col().key("today").w_full().px(10.0).pt(10.0).pb(8.0).gap(2.0)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(el().h(8.0))
        .child(head);
    if !s.has_calendar() {
        return panel.child(
            row().key("today-setup").w_full().px(8.0).py(4.0).radius(6.0).hover(p.hover)
                .on_press(|s: &mut App| s.sheet = Sheet::Settings)
                .child(text("Connect a calendar in Settings").font(SMALL).color(p.accent)),
        );
    }
    let events = s.today_events();
    if events.is_empty() {
        return panel.child(row().w_full().px(8.0).py(4.0).child(text("No events today").font(SMALL).color(p.tertiary)));
    }
    for ev in events.iter().take(MAX_SHOWN) {
        panel = panel.child(event_row(ev, p, now));
    }
    if events.len() > MAX_SHOWN {
        panel = panel.child(row().w_full().px(8.0).pt(2.0)
            .child(text(format!("{} more", events.len() - MAX_SHOWN)).font(SMALL).color(p.tertiary)));
    }
    panel
}

fn event_row(ev: &Event, p: Palette, now: chrono::DateTime<Local>) -> El {
    let color = ev.color.as_deref().and_then(hex).unwrap_or(p.accent);
    let over = ev.end.is_some_and(|e| e <= now);
    let happening = ev.start.is_some_and(|s| s <= now) && !over && ev.start.is_some();
    let when = match (ev.start, ev.end) {
        (None, _) => "All day".to_owned(),
        (Some(s), Some(e)) => format!("{} – {}", s.format("%-I:%M"), e.format("%-I:%M %p")),
        (Some(s), None) => s.format("%-I:%M %p").to_string(),
    };
    let detail = match &ev.location {
        Some(l) => format!("{when} · {l}"),
        None => when,
    };
    let title = if ev.title.is_empty() { "(no title)".to_owned() } else { ev.title.clone() };
    row().w_full().px(8.0).py(3.0).gap(8.0).items_start().radius(6.0)
        .fill(if happening { color.alpha(0.14) } else { rune::Color::TRANSPARENT })
        .opacity(if over { 0.5 } else { 1.0 })
        .help(format!("{} · {}", ev.calendar, detail))
        .child(el().w(3.0).h(30.0).radius(1.5).fill(color))
        .child(col().grow(1.0).min_w(0.0).gap(1.0)
            .child(text(title).font(TITLE).color(p.text).truncate())
            .child(text(detail).font(SMALL).color(p.secondary).truncate()))
}
