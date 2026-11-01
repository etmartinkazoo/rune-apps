//! Dates as Apple Mail shows them.

use chrono::{DateTime, Datelike, Local};

fn parse(iso: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|d| d.with_timezone(&Local))
}

/// The message list's date: a time today, "Yesterday", a weekday this week, else a date.
pub fn short(iso: &str) -> String {
    let Some(d) = parse(iso) else { return String::new() };
    let today = Local::now().date_naive();
    let days = (today - d.date_naive()).num_days();
    match days {
        0 => d.format("%-I:%M %p").to_string(),
        1 => "Yesterday".to_owned(),
        2..=6 => d.format("%A").to_string(),
        _ if d.year() == today.year() => d.format("%-d %B").to_string(),
        _ => d.format("%-d/%-m/%y").to_string(),
    }
}

/// The reader's date: "28 October 2026 at 9:41 AM".
pub fn long(iso: &str) -> String {
    parse(iso).map_or_else(String::new, |d| d.format("%-d %B %Y at %-I:%M %p").to_string())
}

/// A day and always the time, for messages in a conversation: "Yesterday 2:34 AM",
/// "Wednesday 3:34 PM", "28 Oct, 3:34 PM".
pub fn with_time(iso: &str) -> String {
    let Some(d) = parse(iso) else { return String::new() };
    let today = Local::now().date_naive();
    let time = d.format("%-I:%M %p");
    match (today - d.date_naive()).num_days() {
        0 => format!("Today {time}"),
        1 => format!("Yesterday {time}"),
        2..=6 => format!("{} {time}", d.format("%A")),
        _ if d.year() == today.year() => format!("{}, {time}", d.format("%-d %b")),
        _ => format!("{}, {time}", d.format("%-d %b %Y")),
    }
}

/// A short day and the time, for the conversation map: "3:46 PM", "Wed 3:46 PM",
/// "28 Oct 3:46 PM".
pub fn compact(iso: &str) -> String {
    let Some(d) = parse(iso) else { return String::new() };
    let time = d.format("%-I:%M %p");
    match (Local::now().date_naive() - d.date_naive()).num_days() {
        0 => time.to_string(),
        1..=6 => format!("{} {time}", d.format("%a")),
        _ => format!("{} {time}", d.format("%-d %b")),
    }
}
