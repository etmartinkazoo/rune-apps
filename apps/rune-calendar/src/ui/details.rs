//! An event's details, beside the event as Apple Calendar's popover is: the title, location,
//! call, invitees and notes edit in place, and closing saves them.

use chrono::{Datelike, NaiveDate};
use rune::{Align, Color, El, ScrollSpec, Side, TextStyle, col, el, icon, row, text};

use super::style::{self, BODY, Palette, SMALL};
use super::{clock, month_name, weekday_name};
use crate::app::App;
use crate::caldav::{Attendee, Event, Reply};
use crate::draft::{Draft, Field};
use crate::notes;

const WIDTH: f32 = 320.0;
const TITLE: TextStyle = TextStyle::new(17.0, 700.0);
const LABEL: TextStyle = TextStyle::new(11.0, 600.0);
/// The notes scroll past this.
const NOTES_HEIGHT: f32 = 220.0;
/// Invitees listed before "and n more".
const MAX_INVITEES: usize = 8;
/// Between fields stacked one above another: each lights up on hover, and two lit ones must
/// not read as one.
const FIELD_GAP: f32 = 4.0;

pub fn view(s: &App, d: &Draft, p: Palette) -> El {
    let ev = &d.event;
    let colour = s.colour_of(&ev.calendar);
    let head = row().w_full().gap(8.0).items_start()
        .child(el().w(4.0).h(22.0).mt(3.0).radius(2.0).fill(colour))
        .child(col().grow(1.0).min_w(0.0).gap(FIELD_GAP)
            .child(field(d, Field::Title, TITLE, p.text, "New Event", p))
            .child(field(d, Field::Location, BODY, p.secondary, "Add Location", p)));
    let cal = s.calendar(&ev.calendar).map_or("Calendar".to_owned(), |c| c.name.clone());
    let calendar = row().gap(6.0).center_y()
        .child(el().size(8.0, 8.0).radius(4.0).fill(colour))
        .child(text(cal).font(SMALL).color(p.secondary).truncate());
    let mut timing = if d.editing_times { time_fields(d, calendar, p) } else { times(s, d, calendar, p) };
    if ev.recurring {
        timing = timing.child(row().gap(5.0).pt(2.0).center_y()
            .child(icon(&style::REPEAT, 11.0).color(p.tertiary))
            .child(text("Repeats; changes apply to every event in the series").font(SMALL).color(p.tertiary)));
    }
    let body = col().w_full().gap(10.0)
        .child(col().w_full().px(10.0).child(head))
        .child(rule(p))
        .child(timing)
        .child(rule(p))
        .child(call(d, p))
        .child(rule(p))
        .child(invitees(s, d, p))
        .child(rule(p))
        .child(col().key("notes-box").w_full().px(10.0)
            .child(bare(input(d, Field::Notes, BODY, p.text, "Add Notes", p).key("notes").min_h(36.0), d.notes.focused, p)
                .max_h(NOTES_HEIGHT).items_start().scroll(ScrollSpec { stick_bottom: false })));
    col().key("event-details").w(WIDTH).py(14.0).radius(12.0).clip_rounded(12.0)
        .fill(p.card).border(1.0, p.separator).shadow(12.0, 36.0, Color::BLACK.alpha(if p.dark { 0.45 } else { 0.18 }))
        .anchored(d.anchor.clone(), Side::Right, Align::Start, 8.0)
        .modal(true, |s: &mut App| s.close_details())
        .child(body)
}

/// The day and time as text; a press on either edits them, when they can change.
fn times(s: &App, d: &Draft, calendar: El, p: Palette) -> El {
    // Shown as edited so far, should an edit have been settled and the fields closed.
    let ev = Event { start: d.event.start, end: d.event.end, all_day: d.event.all_day, ..Event::default() };
    let ev = match d.times() {
        Ok(Some(t)) => Event { start: t.start, end: t.end, all_day: t.all_day, ..ev },
        _ => ev,
    };
    let (day, time) = when(&ev);
    let movable = s.can_move(&d.event);
    let part = |label: String, ink: Color, f: Field| {
        let t = row().px(6.0).py(2.0).radius(5.0).min_w(0.0).child(text(label).font(BODY).color(ink).truncate());
        if movable {
            t.key(format!("when-{f:?}")).hover(p.hover).help("Change the date and time")
                .on_press(move |s: &mut App| s.edit_times(f))
        } else {
            t
        }
    };
    let mut c = col().w_full().px(10.0).gap(FIELD_GAP)
        .child(row().w_full().gap(8.0).center_y()
            .child(row().grow(1.0).min_w(0.0).child(part(day, p.text, Field::StartDate)))
            .child(calendar));
    if let Some(time) = time {
        c = c.child(row().child(part(time, p.secondary, Field::StartTime)));
    }
    c
}


/// "starts" and "ends", each a date and (unless all day) a time to type over, and all-day.
fn time_fields(d: &Draft, calendar: El, p: Palette) -> El {
    let line = |label: &str, date: Field, time: Field| {
        let mut r = row().w_full().gap(6.0).center_y()
            .child(text(label).font(SMALL).color(p.secondary).w(38.0))
            .child(time_field(d, date, p).grow(1.0));
        if !d.all_day {
            r = r.child(time_field(d, time, p).w(84.0));
        }
        r
    };
    let on = d.all_day;
    let check = el().size(14.0, 14.0).radius(4.0).center()
        .fill(if on { p.accent } else { Color::TRANSPARENT })
        .border(1.5, if on { p.accent } else { p.tertiary })
        .child(if on { icon(&style::CHECK, 10.0).color(Color::WHITE) } else { el() });
    let all_day = row().key("all-day-check").role("checkbox").px(6.0).py(3.0).gap(6.0).center_y().radius(5.0).hover(p.hover)
        .on_press(move |s: &mut App| s.set_all_day(!on))
        .child(check)
        .child(text("all-day").font(SMALL).color(p.text));
    let mut c = col().w_full().px(16.0).gap(6.0)
        .child(line("starts", Field::StartDate, Field::StartTime))
        .child(line("ends", Field::EndDate, Field::EndTime))
        .child(row().w_full().ml(32.0).center_y().child(all_day).child(el().grow(1.0)).child(calendar.mr(32.0)));
    if let Err(e) = d.resolve() {
        c = c.child(text(e).font(SMALL).color(p.today).wrap().min_w(0.0));
    }
    c
}

/// A date or time to type over: ↑ and ↓ move it a day or a quarter hour, Return settles it.
fn time_field(d: &Draft, f: Field, p: Palette) -> El {
    let edit = d.time_field(f);
    let bad = !d.readable(f);
    let input = super::field::input(edit, BODY, p).key(format!("time-{f:?}")).single_line().grow(1.0).min_w(0.0)
        .on_edit(move |s: &mut App| s.draft_field(f))
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| {
            let plain = !e.mods.cmd && !e.mods.ctrl && !e.mods.alt;
            match e.key {
                rune::Key::Up if plain => s.step_time(f, 1),
                rune::Key::Down if plain => s.step_time(f, -1),
                // Settled before Tab moves on, so the end shows where the start took it.
                rune::Key::Tab => {
                    s.settle_times();
                    return false;
                }
                _ => return super::field::platform_key(s.draft_field(f), e),
            }
            true
        })
        .on_submit(|s: &mut App, _| s.settle_times());
    let fill = if bad { p.today.alpha(0.14) } else { p.field };
    row().h(26.0).px(7.0).radius(6.0).center_y().fill(fill).cursor(rune::Cursor::Text)
        .border(1.0, if edit.focused { p.accent } else { Color::TRANSPARENT })
        .on_press(move |s: &mut App| s.edit_times(f))
        .child(input)
}

fn rule(p: Palette) -> El {
    el().mx(16.0).h(1.0).fill(p.separator)
}

/// A field that reads as plain text until it's hovered or edited, as Apple's do.
fn field(d: &Draft, f: Field, style: TextStyle, ink: Color, placeholder: &str, p: Palette) -> El {
    let focused = draft_edit(d, f).focused;
    let input = input(d, f, style, ink, placeholder, p)
        .on_submit(move |s: &mut App, _| s.draft_field(f).focused = false);
    bare(input, focused, p).on_press(move |s: &mut App| s.draft_field(f).focused = true)
}

fn input(d: &Draft, f: Field, style: TextStyle, ink: Color, placeholder: &str, p: Palette) -> El {
    // Long unbroken text, a link or a rule of `~:~`, breaks anywhere rather than leaving the card.
    super::field::input(draft_edit(d, f), style, p).color(ink).placeholder(placeholder, p.tertiary).wrap_anywhere()
        .grow(1.0).min_w(0.0)
        .on_edit(move |s: &mut App| s.draft_field(f))
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(s.draft_field(f), e))
}

fn bare(input: El, focused: bool, p: Palette) -> El {
    row().w_full().px(6.0).py(3.0).radius(6.0).center_y().cursor(rune::Cursor::Text)
        .fill(if focused { p.field } else { Color::TRANSPARENT })
        .hover(p.hover)
        .child(input)
}

fn draft_edit(d: &Draft, f: Field) -> &rune::TextEdit {
    match f {
        Field::Title => &d.title,
        Field::Location => &d.location,
        Field::Call => &d.call,
        Field::Invite => &d.invite,
        Field::Notes => &d.notes,
        f => d.time_field(f),
    }
}

/// The call: a Join button and its service when there's one, and the link to change.
fn call(d: &Draft, p: Palette) -> El {
    let link = d.call.text.trim().to_owned();
    let mut r = row().w_full().px(10.0).gap(8.0).center_y();
    let opens = link.starts_with("https://") || link.starts_with("http://");
    if opens && !d.call.focused {
        let name = notes::service(&link).map(str::to_owned).or_else(|| notes::host(&link)).unwrap_or_else(|| "Link".into());
        let join = row().key("join").h(26.0).px(10.0).gap(5.0).radius(6.0).center().fill(p.accent).hover(p.accent.mix(Color::BLACK, 0.15))
            .help(link.clone())
            .on_press_tx(move |_: &mut App, tx: &mut rune::Tx| {
                let _ = tx.open_url(&link);
            })
            .child(icon(&style::VIDEO, 13.0).color(Color::WHITE))
            .child(text(if notes::service(&d.call.text).is_some() { "Join" } else { "Open" }).font(style::BODY_BOLD).color(Color::WHITE));
        r = r.child(join).child(col().grow(1.0).min_w(0.0)
            .child(text(name).font(BODY).color(p.text).truncate().px(6.0))
            .child(field_single(d, Field::Call, SMALL, p.secondary, "Add Video Call", p)));
    } else {
        r = r.child(field_single(d, Field::Call, BODY, p.text, "Add Video Call", p));
    }
    r
}

fn field_single(d: &Draft, f: Field, style: TextStyle, ink: Color, placeholder: &str, p: Palette) -> El {
    let focused = draft_edit(d, f).focused;
    let input = input(d, f, style, ink, placeholder, p).single_line()
        .on_submit(move |s: &mut App, _| s.draft_field(f).focused = false);
    bare(input, focused, p).on_press(move |s: &mut App| s.draft_field(f).focused = true)
}

fn invitees(s: &App, d: &Draft, p: Palette) -> El {
    let mut c = col().w_full().px(10.0).gap(FIELD_GAP);
    let editable = s.can_invite(&d.event);
    if !d.attendees.is_empty() {
        c = c.child(text("INVITEES").font(LABEL).color(p.tertiary).px(6.0).pb(2.0));
    }
    for a in d.attendees.iter().take(MAX_INVITEES) {
        let organizer = d.event.organizer.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&a.email));
        c = c.child(invitee(a, organizer, editable && !organizer, p));
    }
    if d.attendees.len() > MAX_INVITEES {
        c = c.child(text(format!("and {} more", d.attendees.len() - MAX_INVITEES)).font(SMALL).color(p.secondary).px(6.0));
    }
    if editable {
        let focused = d.invite.focused;
        let input = input(d, Field::Invite, BODY, p.text, "Add Invitees", p).single_line()
            .on_submit(|s: &mut App, tx: &mut rune::Tx| s.add_invitees(tx));
        c = c.child(bare(input, focused, p).on_press(|s: &mut App| s.draft_field(Field::Invite).focused = true));
    } else if let Some(o) = &d.event.organizer {
        let by = d.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(o)).map_or(o.as_str(), |a| a.label());
        c = c.child(text(format!("Invited by {by}")).font(SMALL).color(p.tertiary).px(6.0).pt(2.0));
    }
    c
}

fn invitee(a: &Attendee, organizer: bool, removable: bool, p: Palette) -> El {
    let (mark, tint, says) = match a.reply {
        Reply::Accepted => (&*style::CHECK, *style::GREEN, "Accepted"),
        Reply::Declined => (&*style::X, p.today, "Declined"),
        Reply::Tentative => (&*style::HELP, *style::ORANGE, "Maybe"),
        Reply::Pending => (&*style::DOT, p.tertiary, "No reply yet"),
    };
    let avatar = el().size(24.0, 24.0).radius(12.0).center().fill(p.field)
        .child(text(initials(a.label())).font(LABEL).color(p.secondary));
    let mut words = col().grow(1.0).min_w(0.0)
        .child(text(a.label().to_owned()).font(BODY).color(p.text).truncate());
    if organizer || a.name.is_some() {
        let sub = if organizer { "organizer".to_owned() } else { a.email.clone() };
        words = words.child(text(sub).font(SMALL).color(p.secondary).truncate());
    }
    let mut r = row().key(format!("invitee-{}", a.email)).w_full().px(6.0).py(3.0).gap(8.0).center_y().radius(6.0).hover(p.hover)
        .help(format!("{} · {says}", a.email))
        .child(avatar)
        .child(words)
        .child(el().size(16.0, 16.0).radius(8.0).center().fill(tint.alpha(0.16)).child(icon(mark, 10.0).color(tint)));
    if removable {
        let email = a.email.clone();
        r = r.child(el().key(format!("remove-{}", a.email)).size(20.0, 20.0).radius(10.0).center().hover(p.hover).help("Remove")
            .on_press(move |s: &mut App| s.remove_invitee(&email))
            .child(icon(&style::X, 11.0).color(p.tertiary)));
    }
    r
}

/// "TJ" for "Tag Jordan", "A" for "ada@x.com".
fn initials(name: &str) -> String {
    let name = name.split('@').next().unwrap_or(name);
    let mut words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let last = words.last().and_then(|w| w.chars().next());
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// The day, and the time unless it's all day: ("Friday 30 October", "4:30 PM to 5 PM").
fn when(ev: &Event) -> (String, Option<String>) {
    let day = |d: NaiveDate| format!("{} {} {}", weekday_name(d), d.day(), month_name(d));
    if ev.all_day {
        let last = (ev.end - chrono::Duration::days(1)).date().max(ev.start.date());
        return if last == ev.start.date() {
            (day(last), Some("All day".into()))
        } else {
            (format!("{} – {}", day(ev.start.date()), day(last)), None)
        };
    }
    if ev.start.date() == ev.end.date() {
        (day(ev.start.date()), Some(format!("{} to {}", clock(ev.start), clock(ev.end))))
    } else {
        (format!("{} {} –", day(ev.start.date()), clock(ev.start)), Some(format!("{} {}", day(ev.end.date()), clock(ev.end))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_from_names_and_addresses() {
        assert_eq!(initials("Tag Jordan"), "TJ");
        assert_eq!(initials("Cathy Lee-Smith"), "CS");
        assert_eq!(initials("ada@x.com"), "A");
        assert_eq!(initials("jordan.tag@x.com"), "JT");
    }
}
