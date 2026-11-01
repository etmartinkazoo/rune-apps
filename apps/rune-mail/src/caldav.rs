//! Today's events over CalDAV, signed in with an app password. Fastmail's API tokens don't
//! reach calendars (JMAP Calendars is still a draft), so Fastmail JMAP accounts take a
//! separate app password; IMAP accounts on Fastmail or Gmail reuse theirs. Fastmail lists
//! every calendar; Gmail's app-password endpoint serves the primary calendar.

use base64::Engine;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

const HOST: &str = "https://caldav.fastmail.com";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub title: String,
    pub location: Option<String>,
    /// `None` for an all-day event.
    pub start: Option<DateTime<Local>>,
    pub end: Option<DateTime<Local>>,
    pub calendar: String,
    pub color: Option<String>,
}

struct Calendar {
    /// Where its events are: on the provider's own host.
    href: String,
    name: String,
    color: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Provider {
    Fastmail,
    Google,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Provider::Fastmail => "Fastmail",
            Provider::Google => "Google",
        }
    }
}

#[derive(Clone)]
pub struct CalDav {
    user: String,
    auth: String,
    provider: Provider,
}

impl CalDav {
    pub fn new(user: &str, password: &str) -> CalDav {
        CalDav::with(Provider::Fastmail, user, password)
    }

    pub fn with(provider: Provider, user: &str, password: &str) -> CalDav {
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        CalDav { user: user.to_owned(), auth: format!("Basic {auth}"), provider }
    }

    fn request(&self, method: &str, url: &str, depth: &str, body: &str) -> Result<String, String> {
        ureq::request(method, url)
            .set("Authorization", &self.auth)
            .set("Depth", depth)
            .set("Content-Type", "application/xml; charset=utf-8")
            .timeout(std::time::Duration::from_secs(20))
            .send_string(body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => format!("{} rejected the calendar app password.", self.provider.name()),
                e => format!("Calendar: {e}"),
            })?
            .into_string()
            .map_err(|e| e.to_string())
    }

    fn calendars(&self) -> Result<Vec<Calendar>, String> {
        if self.provider == Provider::Google {
            // Google's CalDAV endpoint that accepts app passwords: one collection, the primary calendar.
            return Ok(vec![Calendar {
                href: format!("https://www.google.com/calendar/dav/{}/events/", self.user),
                name: "Google Calendar".into(),
                color: None,
            }]);
        }
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
  <d:prop><d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/></d:prop>
</d:propfind>"#;
        let url = format!("{HOST}/dav/calendars/user/{}/", self.user);
        let xml = self.request("PROPFIND", &url, "1", body)?;
        let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for resp in doc.descendants().filter(|n| n.has_tag_name(("DAV:", "response"))) {
            let find = |ns: &str, name: &str| resp.descendants().find(|n| n.has_tag_name((ns, name)));
            let is_calendar = find("DAV:", "resourcetype")
                .is_some_and(|r| r.children().any(|c| c.has_tag_name(("urn:ietf:params:xml:ns:caldav", "calendar"))));
            let events = find("urn:ietf:params:xml:ns:caldav", "supported-calendar-component-set")
                .is_none_or(|set| set.children().any(|c| c.attribute("name") == Some("VEVENT")));
            let Some(href) = find("DAV:", "href").and_then(|h| h.text()).and_then(|h| on_host(HOST, h.trim())) else { continue };
            if is_calendar && events {
                out.push(Calendar {
                    href,
                    name: find("DAV:", "displayname").and_then(|n| n.text()).unwrap_or("Calendar").to_owned(),
                    color: find("http://apple.com/ns/ical/", "calendar-color")
                        .and_then(|n| n.text())
                        .map(|c| c.trim().chars().take(7).collect()),
                });
            }
        }
        Ok(out)
    }

    /// Events overlapping today in local time, recurrences expanded by the server.
    pub fn today(&self) -> Result<Vec<Event>, String> {
        let day = Local::now().date_naive();
        let start = local_midnight(day);
        let end = local_midnight(day.succ_opt().unwrap_or(day));
        let fmt = |d: DateTime<Local>| d.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string();
        let (s, e) = (fmt(start), fmt(end));
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><c:calendar-data><c:expand start="{s}" end="{e}"/></c:calendar-data></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">
    <c:time-range start="{s}" end="{e}"/>
  </c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#
        );
        let mut events = Vec::new();
        for cal in self.calendars()? {
            let xml = self.request("REPORT", &cal.href, "1", &body)?;
            let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
            for data in doc.descendants().filter(|n| n.has_tag_name(("urn:ietf:params:xml:ns:caldav", "calendar-data"))) {
                let ics: String = data.children().filter_map(|c| c.text()).collect();
                events.extend(parse_ics(&ics, &cal.name, cal.color.as_deref()));
            }
        }
        events.retain(|ev| overlaps(ev, start, end, day));
        sort(&mut events);
        Ok(events)
    }
}

fn local_midnight(day: NaiveDate) -> DateTime<Local> {
    let midnight = day.and_hms_opt(0, 0, 0).unwrap();
    Local.from_local_datetime(&midnight).earliest().unwrap_or_else(|| Local.from_utc_datetime(&midnight))
}

fn overlaps(ev: &Event, start: DateTime<Local>, end: DateTime<Local>, _day: NaiveDate) -> bool {
    match (ev.start, ev.end) {
        (Some(s), Some(e)) => s < end && e > start,
        (Some(s), None) => s >= start && s < end,
        _ => true, // all-day events come back only for days they cover
    }
}

pub fn sort(events: &mut [Event]) {
    events.sort_by(|a, b| a.start.is_some().cmp(&b.start.is_some()).then(a.start.cmp(&b.start)));
}

/// A URL for an `href` the server sent: a path on `host`, or a URL on `host`. None for one
/// anywhere else, which mustn't be sent the app password.
pub fn on_host(host: &str, href: &str) -> Option<String> {
    if href.starts_with('/') && !href.starts_with("//") {
        Some(format!("{host}{href}"))
    } else if href.strip_prefix(host).is_some_and(|rest| rest.starts_with('/')) {
        Some(href.to_owned())
    } else {
        None
    }
}

/// The VEVENTs of an iCalendar object.
pub fn parse_ics(ics: &str, calendar: &str, color: Option<&str>) -> Vec<Event> {
    // Unfold continuation lines (RFC 5545 §3.1).
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.lines() {
        let raw = raw.trim_end_matches('\r');
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(raw.to_owned()),
        }
    }
    let mut out = Vec::new();
    let mut cur: Option<Event> = None;
    let mut all_day = false;
    let mut depth = 0;
    for line in &lines {
        let Some((head, value)) = line.split_once(':') else { continue };
        let (name, params) = head.split_once(';').unwrap_or((head, ""));
        match (name, value) {
            ("BEGIN", "VEVENT") => {
                cur = Some(Event {
                    title: String::new(),
                    location: None,
                    start: None,
                    end: None,
                    calendar: calendar.to_owned(),
                    color: color.map(str::to_owned),
                });
                all_day = false;
                depth = 0;
            }
            ("BEGIN", _) if cur.is_some() => depth += 1, // VALARM and friends
            ("END", "VEVENT") => {
                if let Some(ev) = cur.take() {
                    out.push(Event { start: if all_day { None } else { ev.start }, ..ev });
                }
            }
            ("END", _) if cur.is_some() => depth -= 1,
            _ if depth > 0 => {}
            ("SUMMARY", v) => cur.iter_mut().for_each(|e| e.title = unescape(v)),
            ("LOCATION", v) if !v.is_empty() => cur.iter_mut().for_each(|e| e.location = Some(unescape(v))),
            ("DTSTART", v) => {
                all_day = params.contains("VALUE=DATE") || v.len() == 8;
                if let Some(e) = cur.as_mut() {
                    e.start = time(v, params);
                }
            }
            ("DTEND", v) => cur.iter_mut().for_each(|e| e.end = time(v, params)),
            _ => {}
        }
    }
    out
}

/// A DATE-TIME in UTC (`Z`), in the zone its TZID names, or floating local time; DATE values
/// are all-day.
fn time(v: &str, params: &str) -> Option<DateTime<Local>> {
    if let Some(utc) = v.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some(Utc.from_utc_datetime(&t).with_timezone(&Local));
    }
    if v.len() == 8 {
        return None;
    }
    let t = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    if let Some(tz) = params.split(';').find_map(|p| p.strip_prefix("TZID=")).and_then(zone) {
        return tz.from_local_datetime(&t).earliest().map(|t| t.with_timezone(&Local));
    }
    // A zone chrono-tz doesn't know (such as Windows names) reads as local, as floating does.
    Local.from_local_datetime(&t).earliest()
}

/// An IANA zone from a TZID, which may be quoted or carry a prefix such as
/// `/mozilla.org/20050126_1/America/New_York`.
fn zone(tzid: &str) -> Option<chrono_tz::Tz> {
    let tzid = tzid.trim_matches('"');
    std::iter::once(tzid)
        .chain(tzid.match_indices('/').map(|(i, _)| &tzid[i + 1..]))
        .find_map(|name| name.parse::<chrono_tz::Tz>().ok())
}

fn unescape(v: &str) -> String {
    v.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timed_and_all_day_events_and_skips_alarms() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Stand\\, up\r\nLOCATION:Room 1\r\n\
                   DTSTART:20261030T090000Z\r\nDTEND:20261030T093000Z\r\nBEGIN:VALARM\r\nSUMMARY:alarm\r\n\
                   END:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Holi\r\n day\r\n\
                   DTSTART;VALUE=DATE:20261030\r\nDTEND;VALUE=DATE:20261031\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut evs = parse_ics(ics, "Work", Some("#ff0000"));
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].title, "Stand, up");
        assert_eq!(evs[0].location.as_deref(), Some("Room 1"));
        assert!(evs[0].start.is_some());
        assert_eq!(evs[1].title, "Holiday");
        assert_eq!(evs[1].start, None);
        sort(&mut evs);
        assert_eq!(evs[0].title, "Holiday", "all-day events come first");
    }

    #[test]
    fn only_hrefs_on_the_providers_host_are_followed() {
        let host = "https://caldav.fastmail.com";
        assert_eq!(on_host(host, "/dav/cal/").as_deref(), Some("https://caldav.fastmail.com/dav/cal/"));
        assert_eq!(on_host(host, "https://caldav.fastmail.com/dav/cal/").as_deref(), Some("https://caldav.fastmail.com/dav/cal/"));
        assert_eq!(on_host(host, "https://caldav.fastmail.com.evil.example/dav/"), None);
        assert_eq!(on_host(host, "http://other.host/dav/"), None);
        assert_eq!(on_host(host, "//other.host/dav/"), None);
    }

    #[test]
    fn times_in_a_named_zone_are_read_in_that_zone() {
        let at = |line: &str| {
            let ics = format!("BEGIN:VEVENT\r\nSUMMARY:x\r\n{line}\r\nEND:VEVENT\r\n");
            parse_ics(&ics, "Work", None)[0].start.map(|t| t.with_timezone(&Utc))
        };
        let utc = Utc.with_ymd_and_hms(2026, 10, 30, 13, 0, 0).single();
        assert_eq!(at("DTSTART;TZID=America/New_York:20261030T090000"), utc);
        assert_eq!(at("DTSTART;TZID=\"/mozilla.org/20050126_1/America/New_York\":20261030T090000"), utc);
        assert_eq!(at("DTSTART:20261030T130000Z"), utc);
    }
}
