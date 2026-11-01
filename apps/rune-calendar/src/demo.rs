//! Offline sample calendars for headless snapshots (`--scene`); never touches the network.

use chrono::{Datelike, Days, Local, NaiveDate};

use crate::accounts::Account;
use crate::app::{App, Sheet, Status};
use crate::caldav::{Attendee, Calendar, Event, Provider, Reply};
use crate::settings::View;

fn cal(href: &str, name: &str, color: &str) -> Calendar {
    Calendar { href: href.into(), name: name.into(), color: Some(color.into()) }
}

pub fn scene(s: &mut App) {
    s.offline();
    s.settings = crate::settings::Settings::default();
    s.accounts.clear();
    s.importable = 0;
    let mut home = Account::new(Provider::Fastmail, "ted@fastmail.com", "", "");
    home.label = "Personal".into();
    let work = Account::new(Provider::Google, "ted.martin@gmail.com", "", "");
    let mut lumin = s_account(home, vec![cal("home", "Home", "#34c759"), cal("family", "Family", "#ff9500")]);
    lumin.status = Status::Ready;
    s.accounts.push(lumin);
    s.accounts.push(s_account(work, vec![cal("work", "Work", "#007aff")]));
    let today = Local::now().date_naive();
    s.today = today;
    s.date = today;
    s.events = events(s.week_start(today));
}

fn s_account(account: Account, calendars: Vec<Calendar>) -> crate::app::AccountState {
    crate::app::AccountState { label_edit: rune::TextEdit::new(&account.label), account, calendars, status: Status::Ready }
}

/// A plausible fortnight from `monday` on.
fn events(monday: NaiveDate) -> Vec<Event> {
    let mut out = Vec::new();
    let mut add = |day: u64, cal: &str, title: &str, from: (u32, u32), to: (u32, u32), location: Option<&str>| {
        let d = monday + Days::new(day);
        out.push(Event {
            uid: format!("{title}-{day}"),
            title: title.into(),
            location: location.map(str::to_owned),
            notes: None,
            calendar: cal.into(),
            all_day: false,
            start: d.and_hms_opt(from.0, from.1, 0).unwrap(),
            end: d.and_hms_opt(to.0, to.1, 0).unwrap(),
            ..Event::default()
        });
    };
    for day in [0, 1, 2, 3, 4, 7, 8, 9, 10, 11] {
        add(day, "work", "Team standup", (9, 0), (9, 15), Some("Zoom"));
    }
    add(0, "work", "Planning", (10, 0), (11, 30), Some("Room 4"));
    add(0, "home", "Gym", (18, 0), (19, 0), None);
    add(1, "work", "1:1 with Grace", (11, 0), (11, 30), None);
    add(1, "work", "Design review", (14, 0), (15, 30), Some("Room 2"));
    add(1, "work", "Compiler sync", (14, 30), (15, 0), None);
    add(2, "home", "Dentist", (16, 0), (16, 45), Some("Smile Dental"));
    add(2, "work", "Lunch with Ada", (12, 30), (13, 30), Some("Café Babbage"));
    add(3, "work", "Customer call", (15, 0), (16, 0), None);
    add(3, "family", "School pickup", (15, 15), (15, 45), None);
    add(4, "work", "Demo day", (13, 0), (15, 0), Some("Main hall"));
    add(4, "home", "Dinner with Linus", (19, 30), (21, 30), Some("Trattoria"));
    add(5, "family", "Football", (10, 0), (11, 30), Some("Park"));
    add(6, "home", "Brunch", (11, 0), (12, 30), None);
    add(8, "work", "Quarterly review", (13, 0), (16, 0), None);
    add(10, "home", "Concert", (20, 0), (22, 30), Some("Royal Hall"));
    let all_day = |d: NaiveDate, end: NaiveDate, cal: &str, title: &str| Event {
        uid: format!("{title}-all"),
        title: title.into(),
        location: None,
        notes: Some("Bring the presents.".into()),
        calendar: cal.into(),
        all_day: true,
        start: d.and_hms_opt(0, 0, 0).unwrap(),
        end: end.and_hms_opt(0, 0, 0).unwrap(),
        ..Event::default()
    };
    out.push(invitation(monday + Days::new(4)));
    out.push(all_day(monday + Days::new(4), monday + Days::new(5), "family", "Mum's birthday"));
    out.push(all_day(monday + Days::new(8), monday + Days::new(11), "work", "Offsite"));
    let first = monday.with_day(1).unwrap();
    out.push(all_day(first + Days::new(14), first + Days::new(15), "home", "Pay rent"));
    out.sort_by(|x, y| (!x.all_day, x.start).cmp(&(!y.all_day, y.start)));
    out
}

/// A Calendly booking as Google serves it: HTML in the notes and Google's joining block.
fn invitation(day: NaiveDate) -> Event {
    let person = |email: &str, name: &str, reply| Attendee { email: email.into(), name: Some(name.into()), reply };
    let meet = "https://meet.google.com/unb-tovf-wnw";
    Event {
        uid: "review".into(),
        title: "Safety American Express Cargo Co. and Jordan Tag".into(),
        location: Some("Google Meet (instructions in description)".into()),
        notes: Some(format!(
            "Event Name\nAccount Review W/ Senior Strategist\n\nPhone number: 630-555-0105\n\nWebsite: \
             <a href=\"https://calendly.com/url?q=https%3A%2F%2Fexample.com%2F&amp;stage=1\">example.com/</a>\n\n\
             Need to make changes to this event?\nCancel: https://calendly.com/cancellations/748bf182\n\
             Reschedule: https://calendly.com/reschedulings/748bf182\n\nPowered by Calendly.com\n\n\
             -::~:~::~:~::~:~::~:~::~:~::~:~::~-\nJoin with Google Meet: {meet}\nOr dial: (US) +1 484-555-0164 PIN: 226294330#\n\n\
             Please do not edit this section.\n-::~:~::~:~::~:~::~:~::~:~::~:~::~-\n"
        )),
        calendar: "work".into(),
        start: day.and_hms_opt(16, 30, 0).unwrap(),
        end: day.and_hms_opt(17, 0, 0).unwrap(),
        call: Some(meet.into()),
        organizer: Some("ted.martin@gmail.com".into()),
        attendees: vec![
            person("ted.martin@gmail.com", "Ted Martin", Reply::Accepted),
            person("jordan@example.com", "Jordan Tag", Reply::Accepted),
            person("cathy@example.com", "Cathy Brooks", Reply::Pending),
        ],
        ..Event::default()
    }
}

pub fn week(s: &mut App) {
    scene(s);
    s.settings.view = View::Week;
}

pub fn day(s: &mut App) {
    scene(s);
    s.settings.view = View::Day;
}

pub fn month(s: &mut App) {
    scene(s);
    s.settings.view = View::Month;
}

pub fn details(s: &mut App) {
    scene(s);
    s.settings.view = View::Week;
    if let Some(ev) = s.events.iter().find(|e| e.uid == "review").cloned() {
        s.select(&ev, format!("event-{}-{}-{}", ev.uid, ev.start, ev.start.date()));
    }
}

/// The details with their dates and times open to edit, the start time being typed over.
pub fn times(s: &mut App) {
    scene(s);
    s.settings.view = View::Week;
    if let Some(ev) = s.events.iter().find(|e| e.title == "Dentist").cloned() {
        s.select(&ev, format!("event-{}-{}-{}", ev.uid, ev.start, ev.start.date()));
        if let Some(d) = &mut s.selected {
            d.editing_times = true;
            d.start_time = rune::TextEdit::new("5:15 PM");
            d.start_time.focused = true;
            d.settle();
            d.end_date = rune::TextEdit::new("someday");
        }
    }
}

pub fn settings(s: &mut App) {
    scene(s);
    s.sheet = Sheet::Settings;
    s.pane = crate::app::Pane::Accounts;
}

pub fn add(s: &mut App) {
    scene(s);
    s.sheet = Sheet::AddAccount;
}

pub fn empty(s: &mut App) {
    scene(s);
    s.accounts.clear();
    s.events.clear();
    s.importable = 2;
}

/// Points the config and keyring away from yours, for tests.
#[cfg(test)]
pub fn test_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("rune-calendar-test-{}", std::process::id()));
        // SAFETY: set once, before any test thread reads the environment through `dirs`.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            std::env::set_var("RUNE_CALENDAR_NO_KEYRING", "1");
        }
    });
}
