//! iCalendar lines (RFC 5545): reading them apart, and changing an event's properties in place
//! so everything the app doesn't edit (alarms, recurrence, a server's own properties) goes back
//! as it came.

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::caldav::Attendee;

/// When an event is: local wall times, an all-day event's end the midnight after its last day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Times {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
}

/// The lines of an iCalendar object with continuation lines joined (RFC 5545 §3.1).
pub fn unfold(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.lines() {
        let raw = raw.trim_end_matches('\r');
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(raw.to_owned()),
        }
    }
    lines
}

/// A content line's name, parameters and value. A colon or semicolon inside a quoted
/// parameter (`CN="Tag: Jordan"`, `DELEGATED-FROM="mailto:a@b"`) doesn't split it.
pub fn split(line: &str) -> Option<(&str, &str, &str)> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(i)
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    Some((name, params, value))
}

/// The value of parameter `key` in `params`, unquoted.
pub fn param<'a>(params: &'a str, key: &str) -> Option<&'a str> {
    let mut quoted = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in params.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&params[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&params[start..]);
    parts.into_iter().find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.eq_ignore_ascii_case(key).then(|| v.trim_matches('"'))
    })
}

pub fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n")
}

/// `line` folded to 75 octets a line, continuation lines starting with a space.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3);
    let mut room = 75;
    for c in line.chars() {
        if c.len_utf8() > room {
            out.push_str("\r\n ");
            room = 74;
        }
        out.push(c);
        room -= c.len_utf8();
    }
    out
}

/// What an edit changes. `None` leaves a property as it is; an empty string removes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub title: Option<String>,
    pub location: Option<String>,
    /// The whole DESCRIPTION to write.
    pub notes: Option<String>,
    /// A video call or other link, written as the event's URL. It replaces the conference
    /// properties, which would otherwise win over it when the event is read again.
    pub call: Option<String>,
    pub attendees: Option<Vec<Attendee>>,
    /// The organizer to name, for an event getting its first invitees.
    pub organizer: Option<String>,
    pub times: Option<Times>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }

    /// Whether this edit replaces the lines of property `name`.
    fn replaces(&self, name: &str) -> bool {
        match name {
            "SUMMARY" => self.title.is_some(),
            "LOCATION" => self.location.is_some(),
            // The HTML copy some servers keep would show the old notes.
            "DESCRIPTION" | "X-ALT-DESC" => self.notes.is_some(),
            "URL" | "CONFERENCE" | "X-GOOGLE-CONFERENCE" => self.call.is_some(),
            "ATTENDEE" => self.attendees.is_some(),
            "ORGANIZER" => self.organizer.is_some(),
            "DTSTART" | "DTEND" | "DURATION" => self.times.is_some(),
            "DTSTAMP" | "LAST-MODIFIED" | "SEQUENCE" => true,
            _ => false,
        }
    }
}

/// `ics` with `changes` made to each of its VEVENTs: a repeating event's series and every
/// occurrence changed on its own.
pub fn patch(ics: &str, changes: &Changes, now: DateTime<Utc>) -> String {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let mut out: Vec<String> = Vec::new();
    // Inside a VEVENT: its SEQUENCE, the ATTENDEE lines it had by address, and how its
    // DTSTART was written.
    let mut event: Option<(u32, Vec<(String, String)>, Zone)> = None;
    let mut depth = 0;
    for line in unfold(ics) {
        let Some((name, params, value)) = split(&line) else {
            out.push(line);
            continue;
        };
        let name = name.to_ascii_uppercase();
        match (name.as_str(), value) {
            ("BEGIN", "VEVENT") => {
                event = Some((0, Vec::new(), Zone::Utc));
                depth = 0;
            }
            ("BEGIN", _) if event.is_some() => depth += 1,
            ("END", "VEVENT") => {
                if let Some((sequence, had, zone)) = event.take() {
                    out.extend(added(changes, sequence, &had, &zone, &stamp));
                }
            }
            ("END", _) if event.is_some() => depth -= 1,
            _ => {
                if let Some((sequence, had, zone)) = event.as_mut()
                    && depth == 0
                    && changes.replaces(&name)
                {
                    match name.as_str() {
                        "SEQUENCE" => *sequence = value.trim().parse().unwrap_or(0),
                        "ATTENDEE" => had.push((email(value), line.clone())),
                        "DTSTART" => *zone = Zone::of(params, value),
                        _ => {}
                    }
                    continue;
                }
            }
        }
        out.push(line);
    }
    let mut text: String = out.iter().map(|l| fold(l) + "\r\n").collect();
    if !ics.ends_with('\n') {
        text.truncate(text.len() - 2);
    }
    text
}

/// How an event's times were written, so new ones are written the same way.
enum Zone {
    Utc,
    Floating,
    Named(chrono_tz::Tz),
}

impl Zone {
    fn of(params: &str, value: &str) -> Zone {
        match param(params, "TZID").and_then(|z| z.parse().ok()) {
            Some(tz) => Zone::Named(tz),
            // A date (an all-day event's) gets times in UTC.
            None if value.trim().ends_with('Z') || value.trim().len() == 8 => Zone::Utc,
            None => Zone::Floating,
        }
    }

    /// `name`'s line for local wall time `t`.
    fn line(&self, name: &str, t: NaiveDateTime) -> String {
        let at = crate::caldav::local(t);
        match self {
            Zone::Utc => format!("{name}:{}", at.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ")),
            Zone::Floating => format!("{name}:{}", t.format("%Y%m%dT%H%M%S")),
            Zone::Named(tz) => format!("{name};TZID={}:{}", tz.name(), at.with_timezone(tz).format("%Y%m%dT%H%M%S")),
        }
    }
}

/// The lines an edited VEVENT ends with.
fn added(changes: &Changes, sequence: u32, had: &[(String, String)], zone: &Zone, stamp: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = changes.times {
        if t.all_day {
            out.push(format!("DTSTART;VALUE=DATE:{}", t.start.format("%Y%m%d")));
            out.push(format!("DTEND;VALUE=DATE:{}", t.end.format("%Y%m%d")));
        } else {
            out.push(zone.line("DTSTART", t.start));
            out.push(zone.line("DTEND", t.end));
        }
    }
    let mut text = |name: &str, v: &Option<String>| {
        if let Some(v) = v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            out.push(format!("{name}:{}", escape(v)));
        }
    };
    text("SUMMARY", &changes.title);
    text("LOCATION", &changes.location);
    text("DESCRIPTION", &changes.notes);
    if let Some(url) = changes.call.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        out.push(format!("URL:{url}"));
    }
    if let Some(o) = &changes.organizer {
        out.push(format!("ORGANIZER:mailto:{o}"));
    }
    if let Some(list) = &changes.attendees {
        for a in list {
            match had.iter().find(|(e, _)| e.eq_ignore_ascii_case(&a.email)) {
                Some((_, line)) => out.push(line.clone()),
                None => out.push(attendee_line(a)),
            }
        }
    }
    out.push(format!("SEQUENCE:{}", sequence + 1));
    out.push(format!("DTSTAMP:{stamp}"));
    out.push(format!("LAST-MODIFIED:{stamp}"));
    out
}

fn attendee_line(a: &Attendee) -> String {
    let cn = match a.name.as_deref().filter(|n| !n.is_empty()) {
        Some(n) if n.contains([';', ':', ',']) => format!("CN=\"{}\";", n.replace('"', "'")),
        Some(n) => format!("CN={n};"),
        None => String::new(),
    };
    let partstat = a.reply.partstat();
    let rsvp = if a.reply == crate::caldav::Reply::Pending { ";RSVP=TRUE" } else { "" };
    format!("ATTENDEE;{cn}ROLE=REQ-PARTICIPANT;PARTSTAT={partstat}{rsvp}:mailto:{}", a.email)
}

/// The address in a `mailto:` value.
pub fn email(value: &str) -> String {
    let v = value.trim();
    v.get(..7).filter(|p| p.eq_ignore_ascii_case("mailto:")).map_or(v, |_| &v[7..]).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caldav::{Attendee, Reply};

    const EVENT: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Old\r\nSEQUENCE:2\r\n\
        ATTENDEE;CN=\"Tag: Jordan\";PARTSTAT=ACCEPTED;X-KEEP=1:mailto:jordan@x.com\r\n\
        ATTENDEE;CN=Gone:mailto:gone@x.com\r\nX-GOOGLE-CONFERENCE:https://meet.google.com/abc\r\n\
        BEGIN:VALARM\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\nDESCRIPTION:Old notes\r\nX-ALT-DESC;FMTTYPE=text/html:<b>Old</b>\r\n\
        END:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn splits_lines_and_reads_quoted_parameters() {
        let (name, params, value) = split("ATTENDEE;CN=\"Tag: Jordan\";PARTSTAT=ACCEPTED:mailto:j@x.com").unwrap();
        assert_eq!((name, value), ("ATTENDEE", "mailto:j@x.com"));
        assert_eq!(param(params, "cn"), Some("Tag: Jordan"));
        assert_eq!(param(params, "PARTSTAT"), Some("ACCEPTED"));
        assert_eq!(unescape("a\\, b\\nc\\\\n"), "a, b\nc\\n");
    }

    #[test]
    fn patches_only_what_changed() {
        let changes = Changes {
            title: Some("New, title".into()),
            notes: Some("Line one\nLine two".into()),
            call: Some("https://zoom.us/j/1".into()),
            attendees: Some(vec![
                Attendee { email: "jordan@x.com".into(), name: Some("Jordan".into()), reply: Reply::Accepted },
                Attendee { email: "new@x.com".into(), name: Some("New; Person".into()), reply: Reply::Pending },
            ]),
            ..Changes::default()
        };
        let out = patch(EVENT, &changes, Utc::now());
        let lines = unfold(&out);
        assert!(lines.contains(&"SUMMARY:New\\, title".to_owned()));
        assert!(lines.contains(&"DESCRIPTION:Line one\\nLine two".to_owned()));
        assert!(lines.contains(&"URL:https://zoom.us/j/1".to_owned()));
        assert!(lines.contains(&"SEQUENCE:3".to_owned()));
        assert!(lines.contains(&"ATTENDEE;CN=\"Tag: Jordan\";PARTSTAT=ACCEPTED;X-KEEP=1:mailto:jordan@x.com".to_owned()), "kept as it was");
        assert!(lines.contains(&"ATTENDEE;CN=\"New; Person\";ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:new@x.com".to_owned()));
        assert!(!out.contains("gone@x.com") && !out.contains("X-GOOGLE-CONFERENCE") && !out.contains("X-ALT-DESC"));
        assert!(lines.contains(&"DESCRIPTION:Reminder".to_owned()), "the alarm's own description stays");
        assert!(lines.contains(&"UID:x".to_owned()));
    }

    #[test]
    fn leaves_untouched_properties_and_folds_long_lines() {
        let out = patch(EVENT, &Changes { location: Some("é".repeat(60)), ..Changes::default() }, Utc::now());
        assert!(out.contains("SUMMARY:Old\r\n") && out.contains("X-GOOGLE-CONFERENCE"));
        assert!(out.split("\r\n").all(|l| l.len() <= 75), "folded at 75 octets");
        assert!(unfold(&out).contains(&format!("LOCATION:{}", "é".repeat(60))));
        let cleared = patch(EVENT, &Changes { title: Some(String::new()), ..Changes::default() }, Utc::now());
        assert!(!cleared.contains("SUMMARY"), "an empty value removes the property");
    }

    #[test]
    fn writes_new_times_the_way_the_event_had_them() {
        let zoned = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nBEGIN:STANDARD\r\nDTSTART:19701025T020000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
            BEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Europe/London:20261030T163000\r\nDURATION:PT45M\r\n\
            BEGIN:VALARM\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let t = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        let (start, end) = (t("2026-11-02 09:00"), t("2026-11-02 10:30"));
        let out = patch(zoned, &Changes { times: Some(Times { start, end, all_day: false }), ..Changes::default() }, Utc::now());
        let lines = unfold(&out);
        let london = |at: chrono::NaiveDateTime| crate::caldav::local(at).with_timezone(&chrono_tz::Europe::London).format("%Y%m%dT%H%M%S").to_string();
        assert!(lines.contains(&format!("DTSTART;TZID=Europe/London:{}", london(start))), "{out}");
        assert!(lines.contains(&format!("DTEND;TZID=Europe/London:{}", london(end))));
        assert!(!out.contains("DURATION") && out.contains("DTSTART:19701025T020000"), "the zone's own DTSTART stays");
        assert!(out.contains("TRIGGER:-PT10M"));
        let back = crate::caldav::parse_ics(&out, "c");
        assert_eq!((back[0].start, back[0].end), (start, end), "reads back as written");

        let day = Times { start: t("2026-11-02 00:00"), end: t("2026-11-04 00:00"), all_day: true };
        let out = patch(zoned, &Changes { times: Some(day), ..Changes::default() }, Utc::now());
        assert!(out.contains("DTSTART;VALUE=DATE:20261102\r\nDTEND;VALUE=DATE:20261104"));
        assert!(!out.contains("TZID=Europe/London:2026"));
    }
}
