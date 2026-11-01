//! Apple Calendar's layout: the sidebar runs the full height; the toolbar sits over Day,
//! Week or Month beside it.

pub mod details;
pub mod field;
pub mod grid;
pub mod month;
pub mod sheets;
pub mod sidebar;
pub mod style;

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, Timelike};
use rune::{Color, Cx, El, Key, KeyEvent, Tx, col, el, icon, row, text};

use crate::app::{App, Sheet};
use crate::settings::View;
use style::{BIG, BODY, Palette, SMALL};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let p = Palette::new(s.settings.dark(cx.system_dark()));
    let [w, h] = cx.layout_size("main").unwrap_or([1000.0, 700.0]);
    let content = match s.settings.view {
        View::Day => grid::view(s, &[s.date], w, p),
        View::Week => {
            let (start, _) = s.span();
            let days: Vec<NaiveDate> = (0..7).map(|i| start + Days::new(i)).collect();
            grid::view(s, &days, w, p)
        }
        View::Month => month::view(s, h, p),
    };
    let main = row().w_full().h_full().fill(p.window)
        .child(sidebar::view(s, p))
        .child(el().w(1.0).h_full().fill(p.separator))
        .child(col().grow(1.0).min_w(0.0).h_full().fill(p.content)
            .child(toolbar(s, p))
            .child(el().w_full().h(1.0).fill(p.separator))
            .child(col().key("main").w_full().grow(1.0).min_h(0.0).child(content)));
    let mut root = el().key("root").w_full().h_full().child(main);
    if let Some(d) = &s.selected {
        root = root.child(details::view(s, d, p));
    }
    if let Some((status, _)) = &s.status {
        root = root.child(row().key("status").pin(None, None, Some(16.0), Some(16.0)).px(14.0).h(32.0).center_y()
            .radius(16.0).fill(p.card).border(1.0, p.separator).shadow(4.0, 14.0, Color::BLACK.alpha(if p.dark { 0.4 } else { 0.15 }))
            .child(text(status.clone()).font(BODY).color(p.text)));
    }
    match s.sheet {
        Sheet::None => root,
        Sheet::Settings => root.child(sheets::settings(s, p)),
        Sheet::AddAccount => root.child(sheets::add_account(s, p)),
    }
}

fn toolbar(s: &App, p: Palette) -> El {
    let (title, sub) = title(s);
    let mut heading = row().gap(8.0).center_y().grow(1.0).min_w(0.0).child(text(title).font(BIG).color(p.text));
    if let Some(sub) = sub {
        heading = heading.child(text(sub).font(rune::TextStyle::new(22.0, 400.0)).color(p.secondary));
    }
    let views = segmented(
        "view",
        &[("Day", View::Day), ("Week", View::Week), ("Month", View::Month)],
        s.settings.view,
        |s, v| s.set_view(v),
        p,
    );
    let busy = s.accounts.iter().any(|a| a.status == crate::app::Status::Connecting);
    let nav = row().gap(4.0).center_y()
        .child(tool(&style::REFRESH, if busy { "Loading…" } else { "Refresh (Ctrl+R)" }, p).key("refresh")
            .opacity(if busy { 0.4 } else { 1.0 })
            .on_press(|s: &mut App| s.refresh()))
        .child(tool(&style::CHEVRON_LEFT, "Previous (←)", p).key("prev").on_press(|s: &mut App| s.step(-1)))
        .child(button("Today", p.text, p.field, p).key("today").help("Today (t)").on_press(|s: &mut App| s.go_today()))
        .child(tool(&style::CHEVRON_RIGHT, "Next (→)", p).key("next").on_press(|s: &mut App| s.step(1)));
    row().key("toolbar").w_full().h(52.0).px(20.0).gap(16.0).center_y()
        .child(heading)
        .child(views)
        .child(row().grow(1.0).justify_end().child(nav))
}

/// "October 2026" and the like; Day says the weekday too.
fn title(s: &App) -> (String, Option<String>) {
    let d = s.date;
    match s.settings.view {
        View::Day => (format!("{} {} {}", d.day(), month_name(d), d.year()), Some(weekday_name(d).to_owned())),
        View::Week => {
            let (a, b) = s.span();
            let last = b - Days::new(1);
            match (a.month() == last.month(), a.year() == last.year()) {
                (true, _) => (month_name(a).to_owned(), Some(a.year().to_string())),
                (false, true) => (format!("{} – {}", month_short(a), month_short(last)), Some(a.year().to_string())),
                (false, false) => (format!("{} {} – {} {}", month_short(a), a.year(), month_short(last), last.year()), None),
            }
        }
        View::Month => (month_name(d).to_owned(), Some(d.year().to_string())),
    }
}

fn tool(i: &'static rune::Icon, help: &str, p: Palette) -> El {
    el().size(28.0, 26.0).radius(6.0).center().hover(p.hover).help(help).child(icon(i, 15.0).color(p.secondary))
}

pub fn button(label: &str, ink: Color, fill: Color, p: Palette) -> El {
    row().h(26.0).px(12.0).radius(6.0).center().fill(fill).hover(p.hover).child(text(label).font(BODY).color(ink))
}

pub fn segmented<T: Copy + PartialEq + 'static>(key: &str, options: &[(&str, T)], current: T, set: fn(&mut App, T), p: Palette) -> El {
    let mut r = row().key(key.to_owned()).role("radiogroup").h(26.0).padding(2.0).gap(2.0).radius(7.0).fill(p.field);
    for (label, value) in options {
        let (label, value) = (*label, *value);
        let on = value == current;
        r = r.child(row().key(format!("{key}-{}", label.to_lowercase().replace(' ', "-"))).h(22.0).px(12.0).center().radius(5.0)
            .fill(if on { p.content } else { Color::TRANSPARENT })
            .hover(p.hover)
            .on_press(move |s: &mut App| set(s, value))
            .child(text(label).font(SMALL).color(if on { p.text } else { p.secondary })));
    }
    r
}

/// "9:30 AM".
pub fn clock(t: NaiveDateTime) -> String {
    let (h, m) = (t.hour(), t.minute());
    let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    if m == 0 { format!("{h12} {half}") } else { format!("{h12}:{m:02} {half}") }
}

pub fn day_name(d: NaiveDate) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][d.weekday().num_days_from_monday() as usize]
}

pub fn weekday_name(d: NaiveDate) -> &'static str {
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][d.weekday().num_days_from_monday() as usize]
}

pub fn month_name(d: NaiveDate) -> &'static str {
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
        [d.month0() as usize]
}

pub fn month_short(d: NaiveDate) -> &'static str {
    &month_name(d)[..3]
}

pub fn on_key(s: &mut App, e: &KeyEvent, _tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        if s.selected.is_some() {
            s.close_details();
            return true;
        }
        if s.sheet != Sheet::None {
            s.close_sheet();
            return true;
        }
        return false;
    }
    if e.is_command(',') {
        s.sheet = Sheet::Settings;
        return true;
    }
    if e.is_command('r') {
        s.refresh();
        return true;
    }
    // The rest are plain keys, for the calendar itself, not for fields in a sheet.
    if s.sheet != Sheet::None || s.selected.is_some() || e.mods.cmd || e.mods.ctrl || e.mods.alt {
        return false;
    }
    match &e.key {
        Key::Left => s.step(-1),
        Key::Right => s.step(1),
        Key::Text(t) => match t.as_str() {
            "t" => s.go_today(),
            "d" => s.set_view(View::Day),
            "w" => s.set_view(View::Week),
            "m" => s.set_view(View::Month),
            "h" | "k" => s.step(-1),
            "l" | "j" => s.step(1),
            _ => return false,
        },
        _ => return false,
    }
    true
}
