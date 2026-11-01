//! CalDAV, signed in with an app password: finds an account's calendars and reads the
//! events in a range of days, recurrences expanded by the server.
//!
//! Fastmail and iCloud are found from their well-known addresses (RFC 6764: the current
//! user's principal, then its calendar home). Google's endpoint that takes app passwords
//! serves only the primary calendar. "Other" starts from the URL you give.

use base64::Engine;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::ics::{self, unescape};
use crate::notes;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Provider {
    Fastmail,
    Google,
    ICloud,
    /// A CalDAV server at an address you give.
    Other,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Fastmail => "Fastmail",
            Provider::Google => "Google",
            Provider::ICloud => "iCloud",
            Provider::Other => "CalDAV",
        }
    }

    fn start(self, user: &str, server: &str) -> String {
        match self {
            Provider::Fastmail => format!("https://caldav.fastmail.com/dav/calendars/user/{user}/"),
            Provider::Google => format!("https://www.google.com/calendar/dav/{user}/events/"),
            Provider::ICloud => "https://caldav.icloud.com/".into(),
            Provider::Other => server.trim().to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Its full URL, which is also its id.
    pub href: String,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub uid: String,
    /// Its resource on the server, where changes are saved.
    pub href: String,
    pub title: String,
    pub location: Option<String>,
    pub notes: Option<String>,
    /// The calendar's href.
    pub calendar: String,
    pub all_day: bool,
    /// Local wall time. All-day events run from the start date's midnight to the end date's
    /// (exclusive), as iCalendar has them.
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    /// A video call to join: the event's conference, its URL, or a call link in its text.
    pub call: Option<String>,
    pub attendees: Vec<Attendee>,
    /// Who sent the invitation, when it is one.
    pub organizer: Option<String>,
    /// One occurrence of a repeating event.
    pub recurring: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    pub name: Option<String>,
    pub reply: Reply,
}

impl Attendee {
    /// Their name, else their address.
    pub fn label(&self) -> &str {
        self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.email)
    }
}

/// An invitee's answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Reply {
    #[default]
    Pending,
    Accepted,
    Declined,
    Tentative,
}

impl Reply {
    fn from_partstat(v: &str) -> Reply {
        match v.to_ascii_uppercase().as_str() {
            "ACCEPTED" => Reply::Accepted,
            "DECLINED" => Reply::Declined,
            "TENTATIVE" => Reply::Tentative,
            _ => Reply::Pending,
        }
    }

    pub fn partstat(self) -> &'static str {
        match self {
            Reply::Pending => "NEEDS-ACTION",
            Reply::Accepted => "ACCEPTED",
            Reply::Declined => "DECLINED",
            Reply::Tentative => "TENTATIVE",
        }
    }
}

impl Event {
    /// Whether it shows on `day`.
    pub fn on(&self, day: NaiveDate) -> bool {
        let (s, e) = (day.and_hms_opt(0, 0, 0).unwrap(), (day + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap());
        if self.start == self.end {
            return self.start >= s && self.start < e;
        }
        self.start < e && self.end > s
    }
}

#[derive(Clone)]
pub struct CalDav {
    provider: Provider,
    user: String,
    auth: String,
    server: String,
}

impl CalDav {
    pub fn new(provider: Provider, user: &str, password: &str, server: &str) -> CalDav {
        // Google and Apple show app passwords in groups; the spaces aren't part of them.
        let password: String = password.split_whitespace().collect();
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        CalDav { provider, user: user.to_owned(), auth: format!("Basic {auth}"), server: server.to_owned() }
    }

    fn request(&self, method: &str, url: &str, depth: &str, body: &str) -> Result<String> {
        ureq::request(method, url)
            .set("Authorization", &self.auth)
            .set("Depth", depth)
            .set("Content-Type", "application/xml; charset=utf-8")
            .timeout(std::time::Duration::from_secs(30))
            .send_string(body)
            .map_err(|e| match e {
                ureq::Error::Status(401 | 403, _) => format!("{} didn't accept that address and app password.", self.provider.name()),
                ureq::Error::Status(404, _) => format!("No calendars at {url}"),
                e => e.to_string(),
            })?
            .into_string()
            .map_err(|e| e.to_string())
    }

    /// The account's event calendars.
    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        let start = self.provider.start(&self.user, &self.server);
        if self.provider == Provider::Google {
            // Checks the password; the one calendar is the primary.
            self.request("PROPFIND", &start, "0", PROPS)?;
            return Ok(vec![Calendar { href: start, name: "Calendar".into(), color: None }]);
        }
        let home = match self.provider {
            Provider::Fastmail => start,
            _ => self.home(&start)?,
        };
        let xml = self.request("PROPFIND", &home, "1", PROPS)?;
        let mut list = parse_calendars(&xml, &home)?;
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(list)
    }

    /// The calendar home: the principal of the signed-in user, then its home set.
    fn home(&self, start: &str) -> Result<String> {
        if start.is_empty() {
            return Err("Enter the server's CalDAV address.".into());
        }
        let principal = (|| {
            let xml = self.request("PROPFIND", start, "0", PRINCIPAL)?;
            first_href(&xml, "current-user-principal").map(|h| join(start, &h)).ok_or_else(|| "no principal".to_owned())
        })();
        let principal = match principal {
            Ok(p) => p,
            // Some servers answer only at /.well-known/caldav.
            Err(e) if !start.contains(".well-known") => {
                let known = join(start, "/.well-known/caldav");
                let xml = self.request("PROPFIND", &known, "0", PRINCIPAL).map_err(|_| e)?;
                join(&known, &first_href(&xml, "current-user-principal").ok_or("The server didn't say whose calendars these are.")?)
            }
            Err(e) => return Err(e),
        };
        let xml = self.request("PROPFIND", &principal, "0", HOME)?;
        let home = first_href(&xml, "calendar-home-set").ok_or("The server has no calendars for this account.")?;
        Ok(join(&principal, &home))
    }

    /// Events of `cal` overlapping the days `from` to `to` (exclusive), in local time.
    pub fn events(&self, cal: &Calendar, from: NaiveDate, to: NaiveDate) -> Result<Vec<Event>> {
        let utc = |d: NaiveDate| local(d.and_hms_opt(0, 0, 0).unwrap()).with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string();
        let (s, e) = (utc(from), utc(to));
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data><c:expand start="{s}" end="{e}"/></c:calendar-data></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">
    <c:time-range start="{s}" end="{e}"/>
  </c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#
        );
        let xml = self.request("REPORT", &cal.href, "1", &body)?;
        let mut out = parse_report(&xml, &cal.href)?;
        let (a, b) = (from.and_hms_opt(0, 0, 0).unwrap(), to.and_hms_opt(0, 0, 0).unwrap());
        out.retain(|ev| ev.start < b && (ev.end > a || ev.start >= a));
        Ok(out)
    }

    /// Makes `changes` to the event at `href`: reads it as the server has it now, changes
    /// those properties and writes it back, refused if it changed in between.
    pub fn update(&self, href: &str, changes: &crate::ics::Changes) -> Result<()> {
        let refused = |e: ureq::Error| match e {
            ureq::Error::Status(401 | 403, _) => format!("{} didn't allow changing this event.", self.provider.name()),
            ureq::Error::Status(404, _) => "The event is no longer on the server.".to_owned(),
            ureq::Error::Status(412, _) => "The event changed on the server while saving; try again.".to_owned(),
            e => e.to_string(),
        };
        let timeout = std::time::Duration::from_secs(30);
        let got = ureq::get(href).set("Authorization", &self.auth).timeout(timeout).call().map_err(refused)?;
        let etag = got.header("ETag").map(str::to_owned);
        let ics = got.into_string().map_err(|e| e.to_string())?;
        let mut put = ureq::put(href)
            .set("Authorization", &self.auth)
            .set("Content-Type", "text/calendar; charset=utf-8")
            .timeout(timeout);
        if let Some(tag) = &etag {
            put = put.set("If-Match", tag);
        }
        put.send_string(&crate::ics::patch(&ics, changes, Utc::now())).map_err(refused)?;
        Ok(())
    }
}

/// The events in a calendar-query's answer, each with the resource it came from.
fn parse_report(xml: &str, calendar: &str) -> Result<Vec<Event>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let href = resp.descendants().find(|n| n.has_tag_name((DAV, "href"))).and_then(|h| h.text()).map(|h| join(calendar, h));
        for data in resp.descendants().filter(|n| n.has_tag_name((CALDAV, "calendar-data"))) {
            let ics: String = data.children().filter_map(|c| c.text()).collect();
            out.extend(parse_ics(&ics, calendar).into_iter().map(|ev| Event { href: href.clone().unwrap_or_default(), ..ev }));
        }
    }
    Ok(out)
}

const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const APPLE: &str = "http://apple.com/ns/ical/";

const PROPS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
  <d:prop><d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/></d:prop>
</d:propfind>"#;
const PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#;

/// `href` made absolute against `base`.
fn join(base: &str, href: &str) -> String {
    url::Url::parse(base).and_then(|b| b.join(href.trim())).map_or_else(|_| href.trim().to_owned(), |u| u.to_string())
}

/// The href inside the first `prop` element named `name`.
fn first_href(xml: &str, name: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let prop = doc.descendants().find(|n| n.tag_name().name() == name)?;
    prop.descendants().find(|n| n.has_tag_name((DAV, "href")))?.text().map(str::to_owned)
}

fn parse_calendars(xml: &str, base: &str) -> Result<Vec<Calendar>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let find = |ns: &str, name: &str| resp.descendants().find(|n| n.has_tag_name((ns, name)));
        let is_calendar = find(DAV, "resourcetype").is_some_and(|r| r.children().any(|c| c.has_tag_name((CALDAV, "calendar"))));
        // Task lists are calendars too; only those holding events show.
        let events = find(CALDAV, "supported-calendar-component-set")
            .is_none_or(|set| set.children().any(|c| c.attribute("name") == Some("VEVENT")));
        let Some(href) = find(DAV, "href").and_then(|h| h.text()) else { continue };
        if is_calendar && events {
            out.push(Calendar {
                href: join(base, href),
                name: find(DAV, "displayname").and_then(|n| n.text()).map_or("Calendar".into(), |n| n.trim().to_owned()),
                color: find(APPLE, "calendar-color").and_then(|n| n.text()).map(|c| c.trim().chars().take(7).collect()),
            });
        }
    }
    Ok(out)
}

/// Local wall time to an instant (the earlier one when clocks go back).
pub fn local(t: NaiveDateTime) -> DateTime<Local> {
    Local.from_local_datetime(&t).earliest().unwrap_or_else(|| Local.from_utc_datetime(&t))
}

/// The VEVENTs of an iCalendar object.
pub fn parse_ics(ics: &str, calendar: &str) -> Vec<Event> {
    let lines = ics::unfold(ics);
    let mut out = Vec::new();
    let mut cur: Option<Draft> = None;
    let mut depth = 0;
    for line in &lines {
        let Some((name, params, value)) = ics::split(line) else { continue };
        match (name, value) {
            ("BEGIN", "VEVENT") => {
                cur = Some(Draft::default());
                depth = 0;
            }
            ("BEGIN", _) if cur.is_some() => depth += 1, // VALARM and friends
            ("END", "VEVENT") => out.extend(cur.take().and_then(|d| d.finish(calendar))),
            ("END", _) if cur.is_some() => depth -= 1,
            _ if depth > 0 => {}
            _ => {
                let Some(d) = cur.as_mut() else { continue };
                match name {
                    "UID" => d.uid = value.to_owned(),
                    "SUMMARY" => d.title = unescape(value),
                    "LOCATION" if !value.is_empty() => d.location = Some(unescape(value)),
                    "DESCRIPTION" if !value.is_empty() => d.notes = Some(unescape(value)),
                    "URL" if !value.trim().is_empty() => d.url = Some(value.trim().to_owned()),
                    "CONFERENCE" | "X-GOOGLE-CONFERENCE" if !value.trim().is_empty() => d.conference = Some(value.trim().to_owned()),
                    "ORGANIZER" => d.organizer = Some(ics::email(value)),
                    "ATTENDEE" => d.attendees.push(Attendee {
                        email: ics::email(value),
                        name: ics::param(params, "CN").map(str::to_owned),
                        reply: ics::param(params, "PARTSTAT").map_or(Reply::Pending, Reply::from_partstat),
                    }),
                    "DTSTART" => d.start = when(value, params),
                    "DTEND" => d.end = when(value, params),
                    "DURATION" => d.duration = duration(value),
                    "RECURRENCE-ID" => d.recurrence = value.to_owned(),
                    "STATUS" if value == "CANCELLED" => d.cancelled = true,
                    _ => {}
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Draft {
    uid: String,
    title: String,
    location: Option<String>,
    notes: Option<String>,
    /// (time, all-day)
    start: Option<(NaiveDateTime, bool)>,
    end: Option<(NaiveDateTime, bool)>,
    duration: Option<Duration>,
    recurrence: String,
    cancelled: bool,
    url: Option<String>,
    conference: Option<String>,
    organizer: Option<String>,
    attendees: Vec<Attendee>,
}

impl Draft {
    fn finish(self, calendar: &str) -> Option<Event> {
        if self.cancelled {
            return None;
        }
        let (start, all_day) = self.start?;
        let end = match (self.end, self.duration) {
            (Some((e, _)), _) => e,
            (None, Some(d)) => start + d,
            // No end: an all-day event is that day, a timed one an instant.
            (None, None) if all_day => start + Duration::days(1),
            (None, None) => start,
        };
        // The conference wins, then the URL; else a call in the location or the notes.
        let call = self.conference.or(self.url).or_else(|| {
            [self.location.as_deref(), self.notes.as_deref()].into_iter().flatten().find_map(notes::call_link).map(str::to_owned)
        });
        Some(Event {
            // Each occurrence of an expanded series has the series' UID.
            uid: if self.recurrence.is_empty() { self.uid } else { format!("{}#{}", self.uid, self.recurrence) },
            href: String::new(),
            title: if self.title.is_empty() { "(No title)".into() } else { self.title },
            location: self.location,
            notes: self.notes,
            calendar: calendar.to_owned(),
            all_day,
            start,
            end: end.max(start),
            call,
            attendees: self.attendees,
            organizer: self.organizer,
            recurring: !self.recurrence.is_empty(),
        })
    }
}

/// A DATE (all-day), a DATE-TIME in UTC (`Z`), in a named zone (`TZID=`), or floating.
fn when(v: &str, params: &str) -> Option<(NaiveDateTime, bool)> {
    if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") || v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some((d.and_hms_opt(0, 0, 0)?, true));
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((Utc.from_utc_datetime(&t).with_timezone(&Local).naive_local(), false));
    }
    let t = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let zone = params.split(';').find_map(|p| p.strip_prefix("TZID=")).map(|z| z.trim_matches('"'));
    match zone.and_then(|z| z.parse::<chrono_tz::Tz>().ok()) {
        Some(tz) => {
            let at = tz.from_local_datetime(&t).earliest()?;
            Some((at.with_timezone(&Local).naive_local(), false))
        }
        None => Some((t, false)),
    }
}

/// An iCalendar DURATION: `P1D`, `PT1H30M`, `P1W`.
fn duration(v: &str) -> Option<Duration> {
    let (neg, v) = match v.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    for ch in v.chars() {
        match ch {
            '0'..='9' => num.push(ch),
            'T' => {}
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match unit {
                    'W' => Duration::weeks(n),
                    'D' => Duration::days(n),
                    'H' => Duration::hours(n),
                    'M' => Duration::minutes(n),
                    'S' => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}


#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn reads_timed_all_day_zoned_and_open_ended_events() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Stand\\, up\r\nLOCATION:Room 1\r\n\
                   DTSTART;TZID=Europe/London:20260115T090000\r\nDURATION:PT30M\r\nBEGIN:VALARM\r\nSUMMARY:alarm\r\n\
                   END:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Holi\r\n day\r\n\
                   DTSTART;VALUE=DATE:20261030\r\nDTEND;VALUE=DATE:20261101\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Gone\r\nSTATUS:CANCELLED\r\nDTSTART:20261030T100000Z\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20261031T080000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let evs = parse_ics(ics, "work");
        assert_eq!(evs.len(), 3, "the cancelled one is left out");
        let stand = &evs[0];
        assert_eq!(stand.title, "Stand, up");
        assert_eq!(stand.end - stand.start, Duration::minutes(30));
        let london = chrono_tz::Europe::London.with_ymd_and_hms(2026, 1, 15, 9, 0, 0).unwrap();
        assert_eq!(stand.start, london.with_timezone(&Local).naive_local());
        let holiday = &evs[1];
        assert_eq!(holiday.title, "Holiday");
        assert!(holiday.all_day);
        assert!(holiday.on(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()));
        assert!(!holiday.on(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()), "the end date is exclusive");
        assert_eq!(evs[2].title, "(No title)");
        assert_eq!((evs[2].start, evs[2].end), (at("2026-10-31 08:00"), at("2026-10-31 08:00")));
    }

    #[test]
    fn finds_hrefs_and_calendars() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
          <d:response><d:href>/cal/home/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
          <d:response><d:href>/cal/home/work/</d:href><d:propstat><d:prop>
            <d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Work</d:displayname>
            <a:calendar-color>#FF2968FF</a:calendar-color>
            <c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
          <d:response><d:href>/cal/home/tasks/</d:href><d:propstat><d:prop>
            <d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname>
            <c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
        </d:multistatus>"#;
        let cals = parse_calendars(xml, "https://dav.example.com/cal/home/").unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].href, "https://dav.example.com/cal/home/work/");
        assert_eq!(cals[0].color.as_deref(), Some("#FF2968"));
        let p = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:propstat><d:prop><d:current-user-principal><d:href>/123/principal/</d:href></d:current-user-principal></d:prop></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(first_href(p, "current-user-principal").as_deref(), Some("/123/principal/"));
        assert_eq!(join("https://caldav.icloud.com/", "/123/principal/"), "https://caldav.icloud.com/123/principal/");
    }

    #[test]
    fn reads_invitees_calls_and_where_each_event_lives() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response>
          <d:href>/cal/work/abc.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:abc
SUMMARY:Review
DTSTART:20261030T163000
DTEND:20261030T170000
ORGANIZER;CN=Ted:mailto:ted@x.com
ATTENDEE;CN="Tag: Jordan";PARTSTAT=ACCEPTED:mailto:jordan@x.com
ATTENDEE;PARTSTAT=TENTATIVE:MAILTO:cathy@x.com
DESCRIPTION:Join: https://acme.zoom.us/j/1
RECURRENCE-ID:20261030T163000
END:VEVENT
END:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#;
        let evs = parse_report(xml, "https://dav.example.com/cal/work/").unwrap();
        let ev = &evs[0];
        assert_eq!(ev.href, "https://dav.example.com/cal/work/abc.ics");
        assert_eq!(ev.organizer.as_deref(), Some("ted@x.com"));
        assert_eq!(ev.attendees.len(), 2);
        assert_eq!((ev.attendees[0].label(), ev.attendees[0].reply), ("Tag: Jordan", Reply::Accepted));
        assert_eq!((ev.attendees[1].label(), ev.attendees[1].reply), ("cathy@x.com", Reply::Tentative));
        assert_eq!(ev.call.as_deref(), Some("https://acme.zoom.us/j/1"), "found in the notes");
        assert!(ev.recurring);
        let google = parse_ics("BEGIN:VEVENT\nUID:g\nDTSTART:20261030T090000\nURL:https://example.com/page\nX-GOOGLE-CONFERENCE:https://meet.google.com/abc\nEND:VEVENT\n", "c");
        assert_eq!(google[0].call.as_deref(), Some("https://meet.google.com/abc"), "the conference wins over the URL");
    }

    #[test]
    fn durations() {
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1W"), Some(Duration::days(7)));
        assert_eq!(duration("-PT15M"), Some(Duration::minutes(-15)));
    }
}
