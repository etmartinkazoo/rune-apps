//! The network side. The UI sends `Cmd`s and never waits; results come back as `Event`s and
//! the waker asks Rune for a frame. Each account signs in and loads on a thread of its own,
//! so a slow server holds up only its own calendars.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use rune::Waker;

use crate::accounts::{self, Account};
use crate::caldav::{Calendar, Event as CalEvent};

pub enum Cmd {
    /// Signs in and lists the calendars. `add` came from Add Account (so failing says so there).
    Connect { account: Account, add: bool },
    /// Saves the accounts as they are now (added, removed, renamed, reordered, calendars hidden).
    Save(Vec<Account>),
    /// The events of every calendar in days `from` to `to` (exclusive).
    Load { from: NaiveDate, to: NaiveDate },
    /// Calendars listed again, and every range loaded so far fetched again.
    Refresh,
    Forget { id: String },
    /// Saves `changes` to the event at `href`, then loads its calendar's month again.
    Update { calendar: String, href: String, changes: crate::ics::Changes, from: NaiveDate, to: NaiveDate },
}

pub enum Event {
    Calendars { id: String, account: Account, add: bool, list: Vec<Calendar> },
    ConnectFailed { id: String, add: bool, error: String },
    /// Everything in one calendar for a range, replacing what was there.
    Events { calendar: String, from: NaiveDate, to: NaiveDate, events: Vec<CalEvent> },
    Error(String),
}

/// Signed-in accounts and their calendars, shared with the threads that sign them in.
type Connected = Arc<Mutex<HashMap<String, (Account, Vec<Calendar>)>>>;

struct Worker {
    events: Sender<Event>,
    wake: Waker,
    accounts: Connected,
    /// Every range asked for, so an account that connects later, or a refresh, loads them.
    ranges: Arc<Mutex<Vec<(NaiveDate, NaiveDate)>>>,
}

pub fn spawn(cmds: Receiver<Cmd>, events: Sender<Event>, wake: Waker) {
    std::thread::Builder::new()
        .name("calendar".into())
        .spawn(move || {
            let mut w = Worker { events, wake, accounts: Connected::default(), ranges: Arc::default() };
            for cmd in cmds {
                w.handle(cmd);
            }
        })
        .expect("spawn the calendar thread");
}

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Connect { account, add } => self.connect(account, add),
            Cmd::Save(list) => {
                if let Err(e) = accounts::save(&list) {
                    self.send(Event::Error(format!("Couldn't save accounts: {e}")));
                }
            }
            Cmd::Load { from, to } => {
                {
                    let mut r = self.ranges.lock().unwrap();
                    if !r.contains(&(from, to)) {
                        r.push((from, to));
                    }
                }
                let connected: Vec<_> = self.accounts.lock().unwrap().values().cloned().collect();
                for (account, cals) in connected {
                    let (events, wake) = (self.events.clone(), self.wake.clone());
                    std::thread::Builder::new()
                        .name("load".into())
                        .spawn(move || fetch(&account, &cals, &[(from, to)], &events, &wake))
                        .expect("spawn a load thread");
                }
            }
            Cmd::Refresh => {
                let connected: Vec<Account> = self.accounts.lock().unwrap().values().map(|(a, _)| a.clone()).collect();
                for account in connected {
                    self.connect(account, false);
                }
            }
            Cmd::Update { calendar, href, changes, from, to } => {
                let found = self.accounts.lock().unwrap().values().find_map(|(a, cals)| {
                    cals.iter().find(|c| c.href == calendar).map(|c| (a.clone(), c.clone()))
                });
                let Some((account, cal)) = found else {
                    self.send(Event::Error("That calendar isn't signed in yet; the change wasn't saved.".into()));
                    return;
                };
                let (events, wake) = (self.events.clone(), self.wake.clone());
                std::thread::Builder::new()
                    .name("save".into())
                    .spawn(move || {
                        if let Err(e) = account.client().update(&href, &changes) {
                            let _ = events.send(Event::Error(format!("Couldn't save the event: {e}")));
                        }
                        // What the server has now, which undoes the change here if it failed.
                        fetch(&account, &[cal], &[(from, to)], &events, &wake);
                    })
                    .expect("spawn a save thread");
            }
            Cmd::Forget { id } => {
                if let Some((a, _)) = self.accounts.lock().unwrap().remove(&id) {
                    a.forget_secret();
                }
            }
        }
    }

    /// Signs in on a thread of its own, lists the calendars, then loads every range asked
    /// for so far.
    fn connect(&mut self, account: Account, add: bool) {
        let (events, wake, accounts, ranges) = (self.events.clone(), self.wake.clone(), self.accounts.clone(), self.ranges.clone());
        let id = account.id();
        std::thread::Builder::new()
            .name("connect".into())
            .spawn(move || match account.client().calendars() {
                Ok(list) => {
                    accounts.lock().unwrap().insert(id.clone(), (account.clone(), list.clone()));
                    let _ = events.send(Event::Calendars { id, account: account.clone(), add, list: list.clone() });
                    wake();
                    let ranges = ranges.lock().unwrap().clone();
                    fetch(&account, &list, &ranges, &events, &wake);
                }
                Err(error) => {
                    let _ = events.send(Event::ConnectFailed { id, add, error });
                    wake();
                }
            })
            .expect("spawn a connect thread");
    }
}

/// Each calendar's events for each range, newest range first, sent as they arrive.
fn fetch(account: &Account, cals: &[Calendar], ranges: &[(NaiveDate, NaiveDate)], events: &Sender<Event>, wake: &Waker) {
    let client = account.client();
    for &(from, to) in ranges.iter().rev() {
        for cal in cals {
            let e = match client.events(cal, from, to) {
                Ok(list) => Event::Events { calendar: cal.href.clone(), from, to, events: list },
                Err(e) => Event::Error(format!("{}: {e}", cal.name)),
            };
            let _ = events.send(e);
            wake();
        }
    }
}
