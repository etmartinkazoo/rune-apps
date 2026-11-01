//! The event whose details show, as it's being edited. Changes are kept here and saved when
//! the details close, as Apple Calendar's popover saves.

use chrono::{Duration, NaiveDate, NaiveTime, Timelike};
use rune::TextEdit;

use crate::caldav::{Attendee, Event, Reply};
use crate::ics::{Changes, Times};
use crate::notes;
use crate::when;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Title,
    Location,
    Call,
    Invite,
    Notes,
    StartDate,
    StartTime,
    EndDate,
    EndTime,
}

impl Field {
    /// The date and time fields, in the order Tab goes through them.
    pub const TIMES: [Field; 4] = [Field::StartDate, Field::StartTime, Field::EndDate, Field::EndTime];

    pub fn is_date(self) -> bool {
        matches!(self, Field::StartDate | Field::EndDate)
    }
}

pub struct Draft {
    /// The event as it was opened.
    pub event: Event,
    /// The key of what was clicked, which the details sit beside.
    pub anchor: String,
    pub title: TextEdit,
    pub location: TextEdit,
    pub call: TextEdit,
    /// Addresses being typed, before Return adds them.
    pub invite: TextEdit,
    pub notes: TextEdit,
    pub attendees: Vec<Attendee>,
    /// The dates and times show as fields to edit.
    pub editing_times: bool,
    pub all_day: bool,
    pub start_date: TextEdit,
    pub start_time: TextEdit,
    pub end_date: TextEdit,
    pub end_time: TextEdit,
    /// The times as last settled (opened, or the last Return, step or Tab), and the fields'
    /// text then. An end not typed over since keeps the event's length as the start moves.
    base: Times,
    was: [String; 4],
}

impl Draft {
    pub fn new(event: Event, anchor: String) -> Draft {
        let title = if event.title == "(No title)" { "" } else { &event.title };
        let mut d = Draft {
            title: TextEdit::new(title),
            location: TextEdit::new(event.location.as_deref().unwrap_or("")),
            call: TextEdit::new(event.call.as_deref().unwrap_or("")),
            invite: TextEdit::new(""),
            notes: TextEdit::new(notes::clean(event.notes.as_deref().unwrap_or(""))),
            attendees: event.attendees.clone(),
            editing_times: false,
            all_day: event.all_day,
            start_date: TextEdit::new(""),
            start_time: TextEdit::new(""),
            end_date: TextEdit::new(""),
            end_time: TextEdit::new(""),
            base: Times { start: event.start, end: event.end, all_day: event.all_day },
            was: Default::default(),
            event,
            anchor,
        };
        d.rewrite();
        d
    }

    /// The fields rewritten from `base`, the way they show.
    fn rewrite(&mut self) {
        let b = self.base;
        self.start_date = TextEdit::new(when::show_date(b.start.date()));
        self.end_date = TextEdit::new(when::show_date(shown_end(b)));
        // An all-day event offers an hour in the morning should it get times.
        let at = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        if !b.all_day {
            self.start_time = TextEdit::new(when::show_time(b.start.time()));
            self.end_time = TextEdit::new(when::show_time(b.end.time()));
        } else if self.start_time.text.is_empty() {
            self.start_time = TextEdit::new(when::show_time(at(9)));
            self.end_time = TextEdit::new(when::show_time(at(10)));
        }
        self.was = [&self.start_date, &self.start_time, &self.end_date, &self.end_time].map(|e| e.text.trim().to_owned());
    }

    pub fn field(&mut self, f: Field) -> &mut TextEdit {
        match f {
            Field::Title => &mut self.title,
            Field::Location => &mut self.location,
            Field::Call => &mut self.call,
            Field::Invite => &mut self.invite,
            Field::Notes => &mut self.notes,
            Field::StartDate => &mut self.start_date,
            Field::StartTime => &mut self.start_time,
            Field::EndDate => &mut self.end_date,
            Field::EndTime => &mut self.end_time,
        }
    }

    pub fn time_field(&self, f: Field) -> &TextEdit {
        match f {
            Field::StartDate => &self.start_date,
            Field::StartTime => &self.start_time,
            Field::EndDate => &self.end_date,
            _ => &self.end_time,
        }
    }

    /// Whether a date or time field reads as one.
    pub fn readable(&self, f: Field) -> bool {
        let text = &self.time_field(f).text;
        if f.is_date() { when::date(text, self.base.start.date()).is_some() } else { when::time(text).is_some() }
    }

    /// The start and end the fields say. An end left as it was follows the start, keeping
    /// the event's length; one typed over is taken as typed.
    pub fn resolve(&self) -> Result<Times, String> {
        let b = self.base;
        let touched = |i: usize, e: &TextEdit| e.text.trim() != self.was[i];
        let date = |e: &TextEdit, near: NaiveDate| {
            when::date(&e.text, near).ok_or_else(|| format!("“{}” isn't a date", e.text.trim()))
        };
        let time = |e: &TextEdit| when::time(&e.text).ok_or_else(|| format!("“{}” isn't a time", e.text.trim()));
        let start_day = date(&self.start_date, b.start.date())?;
        // The end's date when it isn't typed: as many days after the start as it was.
        let end_day = |near| if touched(2, &self.end_date) { date(&self.end_date, near) } else { Ok(near + (shown_end(b) - b.start.date())) };
        let midnight = NaiveTime::MIN;
        let times = if self.all_day {
            let last = end_day(start_day)?;
            Times { start: start_day.and_time(midnight), end: (last + Duration::days(1)).and_time(midnight), all_day: true }
        } else {
            let start = start_day.and_time(time(&self.start_time)?);
            let end = if b.all_day || touched(2, &self.end_date) || touched(3, &self.end_time) {
                end_day(start_day)?.and_time(time(&self.end_time)?)
            } else {
                start + (b.end - b.start)
            };
            Times { start, end, all_day: false }
        };
        if times.end < times.start {
            return Err("The event would end before it starts".into());
        }
        Ok(times)
    }

    /// Settles the fields on what they say, written the way they show; false when they don't
    /// read.
    pub fn settle(&mut self) -> bool {
        let Ok(times) = self.resolve() else { return false };
        let focused = Field::TIMES.into_iter().find(|f| self.time_field(*f).focused);
        self.base = times;
        self.rewrite();
        if let Some(f) = focused {
            let e = self.field(f);
            e.focused = true;
            e.set_caret(e.text.len());
        }
        true
    }

    /// Moves a date a day or a time a quarter hour, from what the field says.
    pub fn step(&mut self, f: Field, by: i64) {
        let near = self.base.start.date();
        let e = self.field(f);
        let text = if f.is_date() {
            when::date(&e.text, near).and_then(|d| d.checked_add_signed(Duration::days(by))).map(when::show_date)
        } else {
            when::time(&e.text).map(|t| {
                // To the next quarter hour that way: 9:10 goes up to 9:15 and down to 9:00.
                let m = (t.hour() * 60 + t.minute()) as i64;
                let m = if by > 0 { (m / 15 + 1) * 15 } else if m % 15 == 0 { m - 15 } else { m / 15 * 15 };
                when::show_time(NaiveTime::MIN + Duration::minutes(m.rem_euclid(24 * 60)))
            })
        };
        if let Some(text) = text {
            *e = TextEdit::new(text);
            e.focused = true;
            e.set_caret(e.text.len());
            self.settle();
        }
    }

    pub fn set_all_day(&mut self, on: bool) {
        self.settle();
        self.all_day = on;
        self.settle();
    }

    /// The new times, when they differ from the event's.
    pub fn times(&self) -> Result<Option<Times>, String> {
        let t = self.resolve()?;
        let ev = &self.event;
        Ok((t != Times { start: ev.start, end: ev.end, all_day: ev.all_day }).then_some(t))
    }

    /// Adds the addresses typed in the invite field ("Ada <ada@x.com>, bob@y.com"). An error
    /// names what isn't an address, and leaves it in the field.
    pub fn add_invitees(&mut self) -> Result<(), String> {
        let mut bad = Vec::new();
        for part in self.invite.text.split([',', ';', '\n']).map(str::trim).filter(|p| !p.is_empty()) {
            match parse_address(part) {
                Some(a) if self.attendees.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) => {}
                Some(a) => self.attendees.push(a),
                None => bad.push(part.to_owned()),
            }
        }
        self.invite = TextEdit::new(bad.join(", "));
        self.invite.focused = true;
        match bad.len() {
            0 => Ok(()),
            1 => Err(format!("“{}” isn't an email address", bad[0])),
            _ => Err(format!("{} aren't email addresses", bad.join(", "))),
        }
    }

    /// What the edit changes, for an event in the calendar of `user`.
    pub fn changes(&self, user: &str) -> Changes {
        let ev = &self.event;
        let differs = |now: &str, was: Option<&str>| (now.trim() != was.unwrap_or("").trim()).then(|| now.trim().to_owned());
        let was_title = if ev.title == "(No title)" { "" } else { &ev.title };
        let call = differs(&self.call.text, ev.call.as_deref());
        let raw = ev.notes.as_deref().unwrap_or("");
        let block = notes::google_block(raw);
        // Clearing the call clears Google's joining instructions with it.
        let drop_block = block.is_some() && call.as_deref() == Some("");
        let notes = if self.notes.text.trim_end() != notes::clean(raw) || drop_block {
            let keep = block.filter(|_| !drop_block).map(|b| format!("\n\n{}", b.trim_end())).unwrap_or_default();
            Some(format!("{}{keep}", self.notes.text.trim_end()).trim().to_owned())
        } else {
            None
        };
        let attendees = (self.attendees != ev.attendees).then(|| self.attendees.clone());
        // An event getting its first invitees is organised by whoever invites them.
        let organizer = (ev.organizer.is_none() && attendees.as_ref().is_some_and(|a| !a.is_empty())).then(|| user.to_owned());
        let attendees = match (&organizer, attendees) {
            (Some(me), Some(mut list)) if !list.iter().any(|a| a.email.eq_ignore_ascii_case(me)) => {
                list.insert(0, Attendee { email: me.clone(), name: None, reply: Reply::Accepted });
                Some(list)
            }
            (_, list) => list,
        };
        Changes {
            title: differs(&self.title.text, Some(was_title)),
            location: differs(&self.location.text, ev.location.as_deref()),
            notes,
            call,
            attendees,
            organizer,
            times: self.times().ok().flatten(),
        }
    }
}

/// The last day an event shows as on: an all-day event's end is the midnight after it.
fn shown_end(t: Times) -> NaiveDate {
    if t.all_day { (t.end - Duration::days(1)).date().max(t.start.date()) } else { t.end.date() }
}

/// `ev` as it will read once `c` is saved.
pub fn apply(ev: &mut Event, c: &Changes) {
    let opt = |v: &str| (!v.is_empty()).then(|| v.to_owned());
    if let Some(t) = &c.title {
        ev.title = if t.is_empty() { "(No title)".into() } else { t.clone() };
    }
    if let Some(l) = &c.location {
        ev.location = opt(l);
    }
    if let Some(n) = &c.notes {
        ev.notes = opt(n);
    }
    if let Some(u) = &c.call {
        ev.call = opt(u);
    }
    if let Some(a) = &c.attendees {
        ev.attendees = a.clone();
    }
    if let Some(o) = &c.organizer {
        ev.organizer = Some(o.clone());
    }
    if let Some(t) = c.times {
        (ev.start, ev.end, ev.all_day) = (t.start, t.end, t.all_day);
    }
}

/// "Ada Lovelace <ada@x.com>" or "ada@x.com".
fn parse_address(s: &str) -> Option<Attendee> {
    let (name, email) = match (s.find('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => (Some(s[..a].trim().trim_matches('"').to_owned()).filter(|n| !n.is_empty()), s[a + 1..b].trim()),
        _ => (None, s),
    };
    let valid = email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.') && !domain.ends_with('.'))
        && !email.contains(char::is_whitespace);
    valid.then(|| Attendee { email: email.to_owned(), name, reply: Reply::Pending })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> Event {
        Event {
            uid: "a".into(),
            title: "Review".into(),
            location: Some("Google Meet (instructions in description)".into()),
            notes: Some("Agenda <b>here</b>\n\n-::~:~::~-\nJoin with Google Meet: https://meet.google.com/abc\n-::~:~::~-\n".into()),
            call: Some("https://meet.google.com/abc".into()),
            ..Event::default()
        }
    }

    #[test]
    fn nothing_changed_saves_nothing() {
        let d = Draft::new(event(), "k".into());
        assert_eq!(d.notes.text, "Agenda here");
        assert!(d.changes("me@x.com").is_empty());
    }

    #[test]
    fn edited_notes_keep_googles_block_unless_the_call_goes() {
        let mut d = Draft::new(event(), "k".into());
        d.notes = TextEdit::new("New agenda\n");
        d.title = TextEdit::new("  Review 2 ");
        let c = d.changes("me@x.com");
        assert_eq!(c.title.as_deref(), Some("Review 2"));
        assert_eq!(c.notes.as_deref(), Some("New agenda\n\n-::~:~::~-\nJoin with Google Meet: https://meet.google.com/abc\n-::~:~::~-"));
        assert_eq!((c.location, c.call), (None, None));
        d.call = TextEdit::new("");
        let c = d.changes("me@x.com");
        assert_eq!((c.call.as_deref(), c.notes.as_deref()), (Some(""), Some("New agenda")));
    }

    #[test]
    fn notes_that_are_only_googles_block_show_empty() {
        let block = "-::~:~::~:~::~:~::~:~::~:~::~:~::~:~::~:~::~:~::~:~::-\nJoin with Google Meet: https://meet.google.com/kmg-mbuq-trc\n\
                     Or dial: (US) +1 510-555-0120 PIN: 963046413#\n\nPlease do not edit this section.\n\
                     -::~:~::~:~::~:~::~:~::~:~::~:~::~:~::~:~::~:~::~:~::-";
        let ev = Event { title: "Team Checkin".into(), notes: Some(block.into()), call: notes::call_link(block).map(str::to_owned), ..Event::default() };
        let d = Draft::new(ev, "k".into());
        assert_eq!(d.notes.text, "");
        assert_eq!(d.call.text, "https://meet.google.com/kmg-mbuq-trc");
        assert!(d.changes("me@x.com").is_empty());
    }

    #[test]
    fn first_invitees_make_you_the_organizer() {
        let mut d = Draft::new(event(), "k".into());
        d.invite = TextEdit::new("Ada Lovelace <ada@x.com>, nope, bob@y.org");
        assert_eq!(d.add_invitees(), Err("“nope” isn't an email address".into()));
        assert_eq!(d.invite.text, "nope");
        let c = d.changes("me@x.com");
        assert_eq!(c.organizer.as_deref(), Some("me@x.com"));
        let emails: Vec<&str> = c.attendees.as_ref().unwrap().iter().map(|a| a.email.as_str()).collect();
        assert_eq!(emails, ["me@x.com", "ada@x.com", "bob@y.org"]);
        assert_eq!(c.attendees.unwrap()[1].name.as_deref(), Some("Ada Lovelace"));
        let mut ev = event();
        apply(&mut ev, &d.changes("me@x.com"));
        assert_eq!(ev.attendees.len(), 3);
    }

    fn timed() -> Event {
        let d = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        Event { uid: "t".into(), title: "Call".into(), start: d.and_hms_opt(16, 30, 0).unwrap(), end: d.and_hms_opt(17, 15, 0).unwrap(), ..Event::default() }
    }

    fn at(text: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn a_moved_start_takes_the_end_with_it() {
        let mut d = Draft::new(timed(), "k".into());
        assert_eq!((d.start_date.text.as_str(), d.start_time.text.as_str(), d.end_time.text.as_str()), ("Fri 30 Oct 2026", "4:30 PM", "5:15 PM"));
        assert_eq!(d.times(), Ok(None));
        d.start_date = TextEdit::new("2 nov");
        d.start_time = TextEdit::new("9am");
        let t = d.times().unwrap().unwrap();
        assert_eq!((t.start, t.end), (at("2026-11-02 09:00"), at("2026-11-02 09:45")), "45 minutes, as it was");
        assert!(d.settle());
        assert_eq!((d.end_date.text.as_str(), d.end_time.text.as_str()), ("Mon 2 Nov 2026", "9:45 AM"));
        d.step(Field::StartTime, 1);
        assert_eq!((d.start_time.text.as_str(), d.end_time.text.as_str()), ("9:15 AM", "10:00 AM"), "stepping keeps following");
        d.end_time = TextEdit::new("11:00");
        let t = d.times().unwrap().unwrap();
        assert_eq!((t.start, t.end), (at("2026-11-02 09:15"), at("2026-11-02 11:00")), "a typed end is kept");
        assert!(d.changes("me@x.com").times.is_some());
        d.end_time = TextEdit::new("8am");
        assert_eq!(d.times(), Err("The event would end before it starts".into()));
        assert!(d.changes("me@x.com").times.is_none(), "what doesn't read isn't saved");
        d.end_time = TextEdit::new("whenever");
        assert!(!d.readable(Field::EndTime));
    }

    #[test]
    fn all_day_and_back() {
        let mut d = Draft::new(timed(), "k".into());
        d.set_all_day(true);
        let t = d.times().unwrap().unwrap();
        assert_eq!((t.start, t.end, t.all_day), (at("2026-10-30 00:00"), at("2026-10-31 00:00"), true));
        assert_eq!(d.end_date.text, "Fri 30 Oct 2026", "the last day, not the midnight after it");
        d.end_date = TextEdit::new("1 Nov");
        assert_eq!(d.times().unwrap().unwrap().end, at("2026-11-02 00:00"));
        d.set_all_day(true);
        d.set_all_day(false);
        let t = d.times().unwrap().unwrap();
        assert_eq!((t.start, t.end, t.all_day), (at("2026-10-30 16:30"), at("2026-11-01 17:15"), false), "the times it had come back");
        let mut ev = timed();
        apply(&mut ev, &d.changes("me@x.com"));
        assert_eq!((ev.start, ev.end), (t.start, t.end));
    }

    #[test]
    fn an_all_day_event_gets_a_morning_hour() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let ev = Event { all_day: true, start: day.and_hms_opt(0, 0, 0).unwrap(), end: (day + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap(), ..timed() };
        let mut d = Draft::new(ev, "k".into());
        d.set_all_day(false);
        let t = d.times().unwrap().unwrap();
        assert_eq!((t.start, t.end), (at("2026-10-30 09:00"), at("2026-10-30 10:00")));
    }
}
