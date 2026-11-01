//! Dates and times as the details show them and as they're typed: "Fri 30 Oct 2026",
//! "4:30 PM", and the looser forms people type ("30/10", "tomorrow", "16:30", "4p").

use chrono::{Datelike, Days, NaiveDate, NaiveTime, Timelike, Weekday};

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [(&str, Weekday); 7] = [
    ("mon", Weekday::Mon), ("tue", Weekday::Tue), ("wed", Weekday::Wed), ("thu", Weekday::Thu),
    ("fri", Weekday::Fri), ("sat", Weekday::Sat), ("sun", Weekday::Sun),
];

/// "Fri 30 Oct 2026".
pub fn show_date(d: NaiveDate) -> String {
    format!("{} {} {} {}", crate::ui::day_name(d), d.day(), crate::ui::month_short(d), d.year())
}

/// "4:30 PM", "4:00 PM": always with minutes, so the field reads the same as it's edited.
pub fn show_time(t: NaiveTime) -> String {
    let h = t.hour();
    format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, t.minute(), if h < 12 { "AM" } else { "PM" })
}

/// A typed date, `near` filling in what's left out: the year, or the week for a weekday alone.
/// Numbers alone read day first, as the dates show.
pub fn date(text: &str, near: NaiveDate) -> Option<NaiveDate> {
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "today" => return Some(chrono::Local::now().date_naive()),
        "tomorrow" => return chrono::Local::now().date_naive().succ_opt(),
        "yesterday" => return chrono::Local::now().date_naive().pred_opt(),
        _ => {}
    }
    let words: Vec<&str> = t.split(|c: char| c.is_whitespace() || ",/-.".contains(c)).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return None;
    }
    let mut weekday = None;
    let mut month = None;
    let mut numbers = Vec::new();
    for w in &words {
        if let Ok(n) = w.trim_end_matches(['s', 't', 'n', 'd', 'h', 'r']).parse::<u32>() {
            // "1st", "22nd": the suffix goes; a bare word that only looks like one doesn't parse.
            numbers.push((n, w.chars().take_while(char::is_ascii_digit).count()));
        } else if let Some(i) = MONTHS.iter().position(|m| w.starts_with(m)) {
            month = Some(i as u32 + 1);
        } else if let Some((_, d)) = WEEKDAYS.iter().find(|(name, _)| w.starts_with(name)) {
            weekday = Some(*d);
        } else {
            return None;
        }
    }
    let year = |n: u32, digits: usize| if digits <= 2 { 2000 + n as i32 } else { n as i32 };
    let (y, m, d) = match (month, numbers.as_slice()) {
        // A weekday alone: that day this week or the next, from `near` on.
        (None, []) => {
            let w = weekday?;
            let ahead = (w.num_days_from_monday() + 7 - near.weekday().num_days_from_monday()) % 7;
            return near.checked_add_days(Days::new(ahead as u64));
        }
        (None, [(y, 4), (m, _), (d, _)]) => (*y as i32, *m, *d),
        (None, [(d, _), (m, _)]) => (near.year(), *m, *d),
        (None, [(d, _), (m, _), (y, digits)]) => (year(*y, *digits), *m, *d),
        (Some(m), [(d, _)]) => (near.year(), m, *d),
        (Some(m), [(a, 4), (b, _)]) | (Some(m), [(b, _), (a, 4)]) => (*a as i32, m, *b),
        (Some(m), [(d, _), (y, digits)]) => (year(*y, *digits), m, *d),
        _ => return None,
    };
    let found = NaiveDate::from_ymd_opt(y, m, d)?;
    // "Thu 30 Oct" when the 30th is a Friday is a mistake, not a date.
    if weekday.is_some_and(|w| w != found.weekday()) {
        return None;
    }
    Some(found)
}

/// A typed time: "4:30 PM", "4:30pm", "4p", "16:30", "1630", "noon".
pub fn time(text: &str) -> Option<NaiveTime> {
    let t: String = text.trim().to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '.').collect();
    match t.as_str() {
        "noon" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }
    let (digits, half) = if let Some(rest) = t.strip_suffix("am").or_else(|| t.strip_suffix('a')) {
        (rest, Some(false))
    } else if let Some(rest) = t.strip_suffix("pm").or_else(|| t.strip_suffix('p')) {
        (rest, Some(true))
    } else {
        (t.as_str(), None)
    };
    let (h, m): (u32, u32) = match digits.split_once([':', 'h']) {
        Some((h, m)) => (h.parse().ok()?, if m.is_empty() { 0 } else { m.parse().ok()? }),
        None if digits.len() >= 3 && digits.len() <= 4 => {
            let split = digits.len() - 2;
            (digits[..split].parse().ok()?, digits[split..].parse().ok()?)
        }
        None => (digits.parse().ok()?, 0),
    };
    let h = match half {
        Some(_) if h == 0 || h > 12 => return None,
        Some(true) => h % 12 + 12,
        Some(false) => h % 12,
        None => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn t(h: u32, m: u32) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(h, m, 0)
    }

    #[test]
    fn reads_what_it_shows_and_what_people_type() {
        let near = d(2026, 10, 30);
        assert_eq!(date(&show_date(d(2027, 1, 4)), near), Some(d(2027, 1, 4)));
        assert_eq!(date("2 Nov", near), Some(d(2026, 11, 2)));
        assert_eq!(date("November 2nd", near), Some(d(2026, 11, 2)));
        assert_eq!(date("Nov 2, 2027", near), Some(d(2027, 11, 2)));
        assert_eq!(date("2/11", near), Some(d(2026, 11, 2)));
        assert_eq!(date("2/11/27", near), Some(d(2027, 11, 2)));
        assert_eq!(date("2026-12-24", near), Some(d(2026, 12, 24)));
        assert_eq!(date("mon", near), Some(d(2026, 11, 2)));
        assert_eq!(date("fri", near), Some(near));
        assert_eq!(date("Thu 30 Oct", near), None, "the 30th is a Friday");
        assert_eq!(date("31 Nov", near), None);
        assert_eq!(date("soon", near), None);
        assert_eq!(date("", near), None);
    }

    #[test]
    fn times() {
        assert_eq!(time(&show_time(t(16, 30).unwrap())), t(16, 30));
        assert_eq!(show_time(t(0, 5).unwrap()), "12:05 AM");
        assert_eq!(time("4:30pm"), t(16, 30));
        assert_eq!(time("4 p.m."), t(16, 0));
        assert_eq!(time("4p"), t(16, 0));
        assert_eq!(time("12 AM"), t(0, 0));
        assert_eq!(time("12:15 PM"), t(12, 15));
        assert_eq!(time("16:30"), t(16, 30));
        assert_eq!(time("1630"), t(16, 30));
        assert_eq!(time("930"), t(9, 30));
        assert_eq!(time("9"), t(9, 0));
        assert_eq!(time("noon"), t(12, 0));
        assert_eq!(time("13pm"), None);
        assert_eq!(time("25:00"), None);
        assert_eq!(time("later"), None);
    }
}
