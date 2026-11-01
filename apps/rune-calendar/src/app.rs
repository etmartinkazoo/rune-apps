//! The app's state and what changes it.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use chrono::{Datelike, Days, Local, Months, NaiveDate};
use rune::{TextEdit, Tx, Waker};

use crate::accounts::{self, Account};
use crate::caldav::{Calendar, Event, Provider};
use crate::draft::{Draft, Field};
use crate::settings::{Settings, View, WeekStart};
use crate::worker::{self, Cmd};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sheet {
    None,
    Settings,
    AddAccount,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pane {
    General,
    Accounts,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Ready,
    Failed(String),
}

pub struct AccountState {
    pub account: Account,
    pub calendars: Vec<Calendar>,
    pub status: Status,
    /// Settings' field for the label.
    pub label_edit: TextEdit,
}

pub struct AddForm {
    pub provider: Provider,
    pub user: TextEdit,
    pub password: TextEdit,
    pub server: TextEdit,
    pub busy: bool,
    pub error: Option<String>,
}

impl AddForm {
    pub fn new(provider: Provider) -> AddForm {
        let mut user = TextEdit::new("");
        user.focused = true;
        AddForm { provider, user, password: TextEdit::new("").secure(), server: TextEdit::new(""), busy: false, error: None }
    }
}

pub struct App {
    pub settings: Settings,
    pub accounts: Vec<AccountState>,
    /// The day the view is on: Week and Month show the week or month around it.
    pub date: NaiveDate,
    pub today: NaiveDate,
    pub events: Vec<Event>,
    /// Months (their first days) asked for.
    loaded: HashSet<NaiveDate>,
    pub sheet: Sheet,
    pub pane: Pane,
    pub add: AddForm,
    /// The event whose details show, as edited so far.
    pub selected: Option<Draft>,
    /// What a field's handler edits when the details have just closed.
    spare: TextEdit,
    pub confirm_remove: Option<String>,
    /// Rune Mail's accounts that could be added, counted once at launch.
    pub importable: usize,
    pub status: Option<(String, f64)>,
    pub clock: f64,
    /// The Day or Week grid still needs scrolling to the morning.
    pub scroll_to_morning: bool,
    next_refresh: Option<f64>,
    cmds: Sender<Cmd>,
    events_rx: Receiver<worker::Event>,
    worker: Option<(Receiver<Cmd>, Sender<worker::Event>)>,
}

pub fn init() -> App {
    let (cmds, cmd_rx) = channel();
    let (event_tx, events_rx) = channel();
    let today = Local::now().date_naive();
    let settings = Settings::load();
    let saved = accounts::load();
    let importable = accounts::from_rune_mail().iter().filter(|r| !saved.iter().any(|a| a.id() == r.id())).count();
    let mut s = App {
        settings,
        accounts: Vec::new(),
        date: today,
        today,
        events: Vec::new(),
        loaded: HashSet::new(),
        sheet: Sheet::None,
        pane: Pane::General,
        add: AddForm::new(Provider::Fastmail),
        selected: None,
        spare: TextEdit::new(""),
        confirm_remove: None,
        importable,
        status: None,
        clock: 0.0,
        scroll_to_morning: true,
        next_refresh: None,
        cmds,
        events_rx,
        worker: Some((cmd_rx, event_tx)),
    };
    for a in saved {
        s.show_account(a, Vec::new(), Status::Connecting);
    }
    s
}

pub fn setup(s: &mut App, wake: Waker) {
    if let Some((cmds, events)) = s.worker.take() {
        worker::spawn(cmds, events, wake);
        for a in &s.accounts {
            s.send(Cmd::Connect { account: a.account.clone(), add: false });
        }
        s.load_visible();
    }
}

pub fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = false;
    while let Ok(e) = s.events_rx.try_recv() {
        s.apply(e, tx);
        changed = true;
    }
    if s.scroll_to_morning && s.settings.view != View::Month {
        s.scroll_to_morning = false;
        tx.scroll_to_y("grid", s.settings.day_starts as f32 * crate::ui::grid::HOUR - 12.0);
        changed = true;
    }
    // Past midnight: today moves on.
    let today = Local::now().date_naive();
    if today != s.today {
        if s.date == s.today {
            s.date = today;
        }
        s.today = today;
        s.load_visible();
        changed = true;
    }
    if s.status.as_ref().is_some_and(|(_, until)| clock >= *until) {
        s.status = None;
        changed = true;
    }
    match s.next_refresh {
        Some(at) if clock >= at => {
            s.refresh();
            s.schedule_refresh(tx);
        }
        None => s.schedule_refresh(tx),
        _ => {}
    }
    // The red line for now moves each minute.
    tx.wake_after(Duration::from_secs(60));
    changed
}

impl App {
    pub(crate) fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }

    /// Never reaches the network: for headless demo scenes and tests.
    pub fn offline(&mut self) {
        self.worker = None;
        self.cmds = channel().0;
    }

    fn schedule_refresh(&mut self, tx: &mut Tx) {
        let m = self.settings.refresh_minutes;
        self.next_refresh = Some(if m > 0 { self.clock + m as f64 * 60.0 } else { f64::INFINITY });
        if m > 0 {
            tx.wake_after(Duration::from_secs(m as u64 * 60));
        }
    }

    pub fn say(&mut self, text: impl Into<String>, tx: &mut Tx) {
        self.status = Some((text.into(), self.clock + 4.0));
        tx.wake_after(Duration::from_secs(4));
    }

    fn apply(&mut self, e: worker::Event, tx: &mut Tx) {
        match e {
            worker::Event::Calendars { id, account, add, list } => {
                match self.accounts.iter_mut().find(|a| a.account.id() == id) {
                    Some(a) => {
                        a.calendars = list;
                        a.status = Status::Ready;
                    }
                    None => self.show_account(account, list, Status::Ready),
                }
                if add {
                    self.importable = self.importable.saturating_sub(1);
                    self.save_accounts();
                    self.add = AddForm::new(Provider::Fastmail);
                    if self.sheet == Sheet::AddAccount {
                        self.sheet = Sheet::None;
                    }
                }
            }
            worker::Event::ConnectFailed { id, add, error } => {
                if add {
                    self.add.busy = false;
                    self.add.error = Some(error);
                } else if let Some(a) = self.accounts.iter_mut().find(|a| a.account.id() == id) {
                    a.status = Status::Failed(error);
                }
            }
            worker::Event::Events { calendar, from, to, events } => {
                let (a, b) = (from.and_hms_opt(0, 0, 0).unwrap(), to.and_hms_opt(0, 0, 0).unwrap());
                // What this calendar had starting in the range goes; what came replaces it.
                self.events.retain(|e| e.calendar != calendar || e.start < a || e.start >= b);
                for ev in events {
                    if !self.events.iter().any(|e| e.calendar == ev.calendar && e.uid == ev.uid && e.start == ev.start) {
                        self.events.push(ev);
                    }
                }
                self.events.sort_by(|x, y| (!x.all_day, x.start, x.end).cmp(&(!y.all_day, y.start, y.end)));
            }
            worker::Event::Error(e) => self.say(e, tx),
        }
    }

    fn show_account(&mut self, account: Account, calendars: Vec<Calendar>, status: Status) {
        let label_edit = TextEdit::new(&account.label);
        self.accounts.push(AccountState { account, calendars, status, label_edit });
    }

    pub fn save_accounts(&self) {
        self.send(Cmd::Save(self.accounts.iter().map(|a| a.account.clone()).collect()));
    }

    // What shows.

    /// The days the current view covers.
    pub fn span(&self) -> (NaiveDate, NaiveDate) {
        match self.settings.view {
            View::Day => (self.date, self.date + Days::new(1)),
            View::Week => {
                let start = self.week_start(self.date);
                (start, start + Days::new(7))
            }
            View::Month => {
                let first = self.date.with_day(1).unwrap();
                let start = self.week_start(first);
                (start, start + Days::new(42))
            }
        }
    }

    pub fn week_start(&self, d: NaiveDate) -> NaiveDate {
        let back = match self.settings.week_start {
            WeekStart::Monday => d.weekday().num_days_from_monday(),
            WeekStart::Sunday => d.weekday().num_days_from_sunday(),
        };
        d - Days::new(back as u64)
    }

    /// Whether events of `href` show (its account's switch for it).
    pub fn shown(&self, href: &str) -> bool {
        !self.accounts.iter().any(|a| a.account.hidden.iter().any(|h| h == href))
    }

    pub fn calendar(&self, href: &str) -> Option<&Calendar> {
        self.accounts.iter().flat_map(|a| &a.calendars).find(|c| c.href == href)
    }

    /// The events showing on `day`: all-day ones first, then by start.
    pub fn on(&self, day: NaiveDate) -> Vec<&Event> {
        self.events.iter().filter(|e| e.on(day) && self.shown(&e.calendar)).collect()
    }

    /// Asks for the months the view touches, and one either side, so paging is instant.
    fn load_visible(&mut self) {
        let (from, to) = self.span();
        let first = from.with_day(1).unwrap();
        let mut months = vec![first - Months::new(1)];
        let mut m = first;
        while m < to {
            months.push(m);
            m = m + Months::new(1);
        }
        months.push(m);
        // The ones on screen last: the worker loads the newest request first.
        months.sort_by_key(|m| (*m >= first && *m < to) as u8);
        for m in months {
            if self.loaded.insert(m) {
                self.send(Cmd::Load { from: m, to: m + Months::new(1) });
            }
        }
    }

    pub fn refresh(&mut self) {
        self.send(Cmd::Refresh);
    }

    // Navigation.

    pub fn set_view(&mut self, v: View) {
        self.close_details();
        if self.settings.view != v {
            self.settings.view = v;
            let _ = self.settings.save();
            self.scroll_to_morning = true;
        }
        self.load_visible();
    }

    /// Back (-1) or on (1) a day, week or month.
    pub fn step(&mut self, by: i32) {
        self.close_details();
        let n = by.unsigned_abs() as u64;
        self.date = match (self.settings.view, by > 0) {
            (View::Day, true) => self.date + Days::new(n),
            (View::Day, false) => self.date - Days::new(n),
            (View::Week, true) => self.date + Days::new(7 * n),
            (View::Week, false) => self.date - Days::new(7 * n),
            (View::Month, true) => self.date + Months::new(n as u32),
            (View::Month, false) => self.date - Months::new(n as u32),
        };
        self.load_visible();
    }

    /// Goes to `day` in the current view.
    pub fn jump(&mut self, day: NaiveDate) {
        self.close_details();
        self.date = day;
        self.load_visible();
    }

    /// A calendar's colour: its own, else one picked for it.
    pub fn colour_of(&self, href: &str) -> rune::Color {
        crate::ui::style::calendar_colour(href, self.calendar(href).and_then(|c| c.color.as_deref()))
    }

    pub fn go_today(&mut self) {
        self.close_details();
        self.date = self.today;
        self.scroll_to_morning = true;
        self.load_visible();
    }

    /// Opens `day` in the Day view.
    pub fn open_day(&mut self, day: NaiveDate) {
        self.date = day;
        self.set_view(View::Day);
    }

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        f(&mut self.settings);
        if self.settings != before {
            let _ = self.settings.save();
            if self.settings.week_start != before.week_start {
                self.load_visible();
            }
            if self.settings.refresh_minutes != before.refresh_minutes {
                self.next_refresh = None;
            }
        }
    }

    // An event's details.

    /// Shows `ev`'s details beside the element keyed `anchor`, saving the one open before.
    pub fn select(&mut self, ev: &Event, anchor: String) {
        if self.is_selected(ev) {
            return;
        }
        self.close_details();
        self.selected = Some(Draft::new(ev.clone(), anchor));
    }

    pub fn is_selected(&self, ev: &Event) -> bool {
        self.selected.as_ref().is_some_and(|d| d.event.uid == ev.uid && d.event.start == ev.start)
    }

    /// Closes the details, saving what changed.
    pub fn close_details(&mut self) {
        let Some(draft) = self.selected.take() else { return };
        let user = self.owner(&draft.event.calendar).map(|a| a.user.clone()).unwrap_or_default();
        let changes = draft.changes(&user);
        if let Err(e) = draft.times() {
            self.status = Some((format!("{e}, so its time wasn't changed."), self.clock + 5.0));
        }
        if changes.is_empty() {
            return;
        }
        let ev = &draft.event;
        // Every occurrence of a series lives in one resource, and the change is to all of them.
        for e in &mut self.events {
            let same = if ev.href.is_empty() { e.uid == ev.uid && e.start == ev.start } else { e.href == ev.href };
            if same && e.calendar == ev.calendar {
                crate::draft::apply(e, &changes);
            }
        }
        if !ev.href.is_empty() {
            // The months it was in and the months it moved to.
            let (a, b) = changes.times.map_or((ev.start, ev.end), |t| (t.start.min(ev.start), t.end.max(ev.end)));
            let from = a.date().with_day(1).unwrap();
            let to = b.date().with_day(1).unwrap() + Months::new(1);
            self.send(Cmd::Update { calendar: ev.calendar.clone(), href: ev.href.clone(), changes, from, to });
        }
    }

    /// Whether an event's date and time can change here: your own event, not one occurrence
    /// of a series (moving one would move them all).
    pub fn can_move(&self, ev: &Event) -> bool {
        !ev.recurring && self.can_invite(ev)
    }

    /// Shows the dates and times as fields, `f` focused with all of it selected.
    pub fn edit_times(&mut self, f: Field) {
        let Some(d) = &mut self.selected else { return };
        // A press in the field being typed in places the caret, as any field's does.
        if d.editing_times && d.time_field(f).focused {
            return;
        }
        if !d.editing_times {
            d.editing_times = true;
        } else {
            d.settle();
        }
        for g in Field::TIMES {
            d.field(g).focused = g == f;
        }
        // Selected through the edit itself: the field isn't on screen until the next frame.
        let e = d.field(f);
        (e.anchor, e.focus) = (0, e.text.len());
    }

    pub fn step_time(&mut self, f: Field, by: i64) {
        if let Some(d) = &mut self.selected {
            d.step(f, by);
        }
    }

    pub fn settle_times(&mut self) {
        if let Some(d) = &mut self.selected {
            d.settle();
        }
    }

    pub fn set_all_day(&mut self, on: bool) {
        if let Some(d) = &mut self.selected {
            d.set_all_day(on);
        }
    }

    pub fn draft_field(&mut self, f: Field) -> &mut TextEdit {
        match &mut self.selected {
            Some(d) => d.field(f),
            None => &mut self.spare,
        }
    }

    pub fn add_invitees(&mut self, tx: &mut Tx) {
        if let Some(Err(e)) = self.selected.as_mut().map(Draft::add_invitees) {
            self.say(e, tx);
        }
    }

    pub fn remove_invitee(&mut self, email: &str) {
        if let Some(d) = &mut self.selected {
            d.attendees.retain(|a| a.email != email);
        }
    }

    /// The account a calendar belongs to.
    pub fn owner(&self, calendar: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.calendars.iter().any(|c| c.href == calendar)).map(|a| &a.account)
    }

    /// Whether you can change who's invited: your own event, not one you were invited to.
    pub fn can_invite(&self, ev: &Event) -> bool {
        match (&ev.organizer, self.owner(&ev.calendar)) {
            (None, _) => true,
            (Some(o), Some(a)) => o.eq_ignore_ascii_case(&a.user),
            (Some(_), None) => false,
        }
    }

    // Accounts.

    pub fn toggle_calendar(&mut self, href: &str) {
        let Some(a) = self.accounts.iter_mut().find(|a| a.calendars.iter().any(|c| c.href == href)) else { return };
        if let Some(i) = a.account.hidden.iter().position(|h| h == href) {
            a.account.hidden.remove(i);
        } else {
            a.account.hidden.push(href.to_owned());
        }
        self.save_accounts();
    }

    pub fn add_account(&mut self) {
        self.add = AddForm::new(Provider::Fastmail);
        self.sheet = Sheet::AddAccount;
    }

    pub fn submit_add(&mut self) {
        let f = &self.add;
        let user = f.user.text.trim();
        if user.is_empty() || f.password.text.trim().is_empty() || f.provider == Provider::Other && f.server.text.trim().is_empty() {
            self.add.error = Some("Fill in every field.".into());
            return;
        }
        let account = Account::new(f.provider, user, &f.password.text, &f.server.text);
        if self.accounts.iter().any(|a| a.account.id() == account.id()) {
            self.add.error = Some(format!("{user} is already added."));
            return;
        }
        self.add.busy = true;
        self.add.error = None;
        self.send(Cmd::Connect { account, add: true });
    }

    /// Adds the calendars Rune Mail already signs in to.
    pub fn import_rune_mail(&mut self, tx: &mut Tx) {
        let found: Vec<Account> = accounts::from_rune_mail()
            .into_iter()
            .filter(|a| !self.accounts.iter().any(|s| s.account.id() == a.id()))
            .collect();
        if found.is_empty() {
            self.say("Rune Mail has no calendars to add", tx);
            return;
        }
        self.say(format!("Adding {} from Rune Mail…", found.len()), tx);
        for a in found {
            self.send(Cmd::Connect { account: a, add: true });
        }
        self.importable = 0;
    }

    pub fn remove_account(&mut self, id: &str) {
        let Some(i) = self.accounts.iter().position(|a| a.account.id() == id) else { return };
        let gone = self.accounts.remove(i);
        let hrefs: HashSet<&str> = gone.calendars.iter().map(|c| c.href.as_str()).collect();
        self.events.retain(|e| !hrefs.contains(e.calendar.as_str()));
        self.confirm_remove = None;
        self.send(Cmd::Forget { id: id.to_owned() });
        gone.account.forget_secret();
        self.save_accounts();
    }

    pub fn move_account(&mut self, id: &str, by: isize) {
        let Some(i) = self.accounts.iter().position(|a| a.account.id() == id) else { return };
        let j = (i as isize + by).clamp(0, self.accounts.len() as isize - 1) as usize;
        if i != j {
            self.accounts.swap(i, j);
            self.save_accounts();
        }
    }

    pub fn label_field(&mut self, id: &str) -> &mut TextEdit {
        let i = self.accounts.iter().position(|a| a.account.id() == id).unwrap_or(0);
        &mut self.accounts[i].label_edit
    }

    pub fn save_label(&mut self, id: &str) {
        let Some(a) = self.accounts.iter_mut().find(|a| a.account.id() == id) else { return };
        a.account.label = a.label_edit.text.trim().to_owned();
        self.save_accounts();
    }

    pub fn close_sheet(&mut self) {
        self.sheet = Sheet::None;
        self.confirm_remove = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(view: View, start: WeekStart, date: (i32, u32, u32)) -> App {
        let mut s = init();
        s.offline();
        s.settings.view = view;
        s.settings.week_start = start;
        s.date = NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap();
        s
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn spans_follow_the_view_and_week_start() {
        crate::demo::test_config();
        // Friday 30 October 2026.
        assert_eq!(app(View::Day, WeekStart::Monday, (2026, 10, 30)).span(), (d(2026, 10, 30), d(2026, 10, 31)));
        assert_eq!(app(View::Week, WeekStart::Monday, (2026, 10, 30)).span(), (d(2026, 10, 26), d(2026, 11, 2)));
        assert_eq!(app(View::Week, WeekStart::Sunday, (2026, 10, 30)).span(), (d(2026, 10, 25), d(2026, 11, 1)));
        let month = app(View::Month, WeekStart::Monday, (2026, 10, 30)).span();
        assert_eq!(month, (d(2026, 9, 28), d(2026, 11, 9)), "six weeks from the week holding the 1st");
    }

    #[test]
    fn steps_by_the_view() {
        crate::demo::test_config();
        let mut s = app(View::Month, WeekStart::Monday, (2026, 1, 31));
        s.step(1);
        assert_eq!(s.date, d(2026, 2, 28), "a month on from the 31st lands on the month's last day");
        s.settings.view = View::Week;
        s.step(-1);
        assert_eq!(s.date, d(2026, 2, 21));
    }

    #[test]
    fn a_reload_replaces_a_calendars_events_in_its_range() {
        crate::demo::test_config();
        let mut s = app(View::Week, WeekStart::Monday, (2026, 10, 30));
        let ev = |uid: &str, day: u32| Event {
            uid: uid.into(), title: uid.into(), calendar: "c".into(),
            start: d(2026, 10, day).and_hms_opt(9, 0, 0).unwrap(), end: d(2026, 10, day).and_hms_opt(10, 0, 0).unwrap(),
            ..Event::default()
        };
        let mut tx = rune::Tx::default();
        s.apply(worker::Event::Events { calendar: "c".into(), from: d(2026, 10, 1), to: d(2026, 11, 1), events: vec![ev("a", 5), ev("b", 6)] }, &mut tx);
        s.apply(worker::Event::Events { calendar: "c".into(), from: d(2026, 10, 1), to: d(2026, 11, 1), events: vec![ev("b", 6)] }, &mut tx);
        assert_eq!(s.events.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), ["b"], "deleted on the server: gone here");
        assert_eq!(s.on(d(2026, 10, 6)).len(), 1);
    }
}
