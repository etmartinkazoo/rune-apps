//! The sidebar: a small month to jump around in, then each account's calendars with their
//! colours, each one a switch for whether it shows.

use chrono::{Datelike, Days, Months, NaiveDate};
use rune::{Color, El, ScrollSpec, col, el, icon, row, text};

use super::button;
use super::style::{self, BODY, BODY_BOLD, COUNT, Palette, SECTION, SMALL};
use crate::app::{App, Status};

pub const WIDTH: f32 = 228.0;

pub fn view(s: &App, p: Palette) -> El {
    let mut list = col().w_full().px(12.0).pt(8.0).pb(12.0).gap(2.0);
    if s.accounts.is_empty() {
        list = list.child(empty(s, p));
    }
    for a in &s.accounts {
        let mut head = row().w_full().pt(14.0).pb(4.0).px(4.0).gap(6.0).center_y()
            .child(text(a.account.name().to_uppercase()).font(SECTION).color(p.secondary).truncate().grow(1.0).min_w(0.0));
        match &a.status {
            Status::Connecting => head = head.child(text("Connecting…").font(COUNT).color(p.tertiary)),
            Status::Failed(e) => head = head.child(text("Failed").font(COUNT).color(p.today).help(e.clone())),
            Status::Ready => {}
        }
        list = list.child(head);
        if let Status::Failed(e) = &a.status {
            list = list.child(text(e.clone()).font(SMALL).color(p.secondary).wrap().min_w(0.0));
        }
        for cal in &a.calendars {
            let on = s.shown(&cal.href);
            let colour = s.colour_of(&cal.href);
            let href = cal.href.clone();
            let check = el().size(14.0, 14.0).radius(4.0).center()
                .fill(if on { colour } else { Color::TRANSPARENT })
                .border(1.5, colour)
                .child(if on { icon(&style::CHECK, 10.0).color(Color::WHITE) } else { el() });
            list = list.child(row().key(format!("cal-{}", cal.href)).role("checkbox").w_full().h(26.0).px(6.0).gap(8.0)
                .center_y().radius(6.0).hover(p.hover)
                .on_press(move |s: &mut App| s.toggle_calendar(&href))
                .child(check)
                .child(text(cal.name.clone()).font(BODY).color(p.text).truncate().grow(1.0).min_w(0.0)));
        }
    }
    let settings = row().key("open-settings").w_full().h(40.0).px(16.0).gap(8.0).center_y().hover(p.hover)
        .on_press(|s: &mut App| s.sheet = crate::app::Sheet::Settings)
        .child(icon(&style::GEAR, 14.0).color(p.secondary))
        .child(text("Settings").font(BODY).color(p.secondary));
    col().key("sidebar").w(WIDTH).h_full().fill(p.sidebar)
        .child(el().w_full().h(52.0))
        .child(mini(s, p))
        .child(col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list))
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(settings)
}

fn empty(s: &App, p: Palette) -> El {
    let mut c = col().w_full().padding(14.0).gap(10.0).radius(10.0).fill(p.content)
        .child(text("No calendars yet").font(BODY_BOLD).color(p.text))
        .child(text("Add a Fastmail, Google, iCloud or other CalDAV account with an app password.")
            .font(SMALL).color(p.secondary).wrap())
        .child(button("Add Account…", p.on_selection, p.accent, p).key("empty-add").on_press(|s: &mut App| s.add_account()));
    if s.importable > 0 {
        let n = s.importable;
        c = c.child(button(&format!("Use Rune Mail's ({n})"), p.text, p.field, p).key("empty-import")
            .help("The Fastmail and Gmail calendars Rune Mail already signs in to")
            .on_press_tx(|s: &mut App, tx| s.import_rune_mail(tx)));
    }
    c
}

/// The month of the date shown, Monday or Sunday first as set; a press goes to that day.
fn mini(s: &App, p: Palette) -> El {
    let first = s.date.with_day(1).unwrap();
    let start = s.week_start(first);
    let bar = row().w_full().h(26.0).px(4.0).center_y()
        .child(text(format!("{} {}", super::month_name(first), first.year())).font(BODY_BOLD).color(p.text).grow(1.0))
        .child(arrow(&style::CHEVRON_LEFT, "Previous month", p).on_press(move |s: &mut App| s.jump(first - Months::new(1))))
        .child(arrow(&style::CHEVRON_RIGHT, "Next month", p).on_press(move |s: &mut App| s.jump(first + Months::new(1))));
    let mut names = row().w_full().h(20.0);
    for i in 0..7 {
        let d = start + Days::new(i);
        names = names.child(row().grow(1.0).center()
            .child(text(super::day_name(d).chars().next().unwrap().to_string()).font(COUNT).color(p.tertiary)));
    }
    // Day and Week mark their days here; Month is the whole grid already.
    let (from, to) = if s.settings.view == crate::settings::View::Month { (s.today, s.today) } else { s.span() };
    let mut weeks = col().w_full();
    for w in 0..6 {
        let mut r = row().w_full().h(24.0);
        for i in 0..7 {
            let d: NaiveDate = start + Days::new(w * 7 + i);
            let today = d == s.today;
            let in_view = d >= from && d < to;
            let ink = if today { Color::WHITE } else if d.month() == first.month() { p.text } else { p.tertiary };
            r = r.child(row().grow(1.0).h_full().center()
                .fill(if in_view && !today { p.sidebar_selection } else { Color::TRANSPARENT })
                .on_press(move |s: &mut App| s.jump(d))
                .child(el().size(22.0, 22.0).radius(11.0).center().fill(if today { p.today } else { Color::TRANSPARENT })
                    .hover(p.hover)
                    .child(text(d.day().to_string()).font(SMALL).color(ink))));
        }
        weeks = weeks.child(r);
    }
    col().key("mini-month").w_full().px(12.0).pb(4.0).child(bar).child(names).child(weeks)
}

fn arrow(i: &'static rune::Icon, help: &str, p: Palette) -> El {
    el().size(22.0, 22.0).radius(5.0).center().hover(p.hover).help(help).child(icon(i, 13.0).color(p.secondary))
}
