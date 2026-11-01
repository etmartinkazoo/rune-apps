//! Downloading ahead, when local storage is on: every account's mailboxes, newest first, then
//! the bodies and attachments of those messages until the budget is nearly used. New mail
//! pushes out the oldest. It has its own connections, so it never holds up what you're doing,
//! and it never marks anything read (IMAP fetches with PEEK; JMAP downloads don't touch $seen).
//! Remote images are never fetched ahead. On a metered connection it waits unless allowed.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use crate::accounts::Saved;
use crate::backend::Backend;
use crate::settings::Storage;
use crate::store::{MAX_FILE_BYTES, Shared};

/// Messages listed per mailbox.
const LIST_PAGE: usize = 500;
/// Bodies asked for at a time.
const BATCH: usize = 20;
/// With nothing asked, a pass this often keeps the store topped up.
const IDLE: Duration = Duration::from_secs(15 * 60);

pub enum Job {
    /// The storage settings changed (or were read at launch).
    Settings(Storage),
    /// Mail may have arrived: a pass over these accounts.
    Kick(Vec<Saved>),
}

pub fn spawn(store: Shared) -> Sender<Job> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("prefetch".into())
        .spawn(move || {
            store.upgrade_index();
            Prefetch { store, rx, storage: Storage::default(), saved: Vec::new(), conns: HashMap::new(), skip: HashSet::new() }.run()
        })
        .expect("spawn the prefetch thread");
    tx
}

struct Prefetch {
    store: Shared,
    rx: Receiver<Job>,
    storage: Storage,
    saved: Vec<Saved>,
    conns: HashMap<String, Backend>,
    /// Bodies too big to keep or that failed, not tried again this run.
    skip: HashSet<String>,
}

impl Prefetch {
    fn run(mut self) {
        loop {
            match self.rx.recv_timeout(IDLE) {
                Ok(job) => self.take(job),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Only the latest of a burst matters.
            while let Ok(job) = self.rx.try_recv() {
                self.take(job);
            }
            self.pass();
        }
    }

    fn take(&mut self, job: Job) {
        match job {
            Job::Settings(s) => {
                let lower = s.budget_bytes().is_some_and(|b| self.storage.budget_bytes().is_none_or(|old| b < old));
                self.storage = s;
                if !self.storage.on {
                    self.conns.clear();
                }
                // A smaller budget prunes now, not at the end of the next pass.
                if lower && let Some(b) = self.storage.budget_bytes() {
                    self.store.prune(b);
                }
            }
            Job::Kick(saved) => self.saved = saved,
        }
    }

    /// False when the settings changed underneath (turned off): stop the pass.
    fn still_on(&mut self) -> bool {
        while let Ok(job) = self.rx.try_recv() {
            self.take(job);
        }
        self.allowed()
    }

    fn allowed(&self) -> bool {
        self.storage.on && self.store.is_enabled() && (self.storage.metered || !metered())
    }

    fn conn(&mut self, key: &str) -> Option<&mut Backend> {
        if !self.conns.contains_key(key) {
            let saved = self.saved.iter().find(|s| s.username.as_deref() == Some(key))?;
            let b = Backend::connect(saved).ok()?;
            self.conns.insert(key.to_owned(), b);
        }
        self.conns.get_mut(key)
    }

    fn pass(&mut self) {
        if !self.allowed() {
            return;
        }
        let budget = self.storage.budget_bytes();
        // Lists first, so the newest mail of every mailbox is known.
        let keys: Vec<String> = self.saved.iter().filter_map(|s| s.username.clone()).collect();
        for key in &keys {
            if !self.still_on() {
                return;
            }
            let Some(conn) = self.conn(key) else { continue };
            let Ok(boxes) = conn.mailboxes() else {
                self.conns.remove(key);
                continue;
            };
            for mb in boxes.iter().filter(|m| m.role.as_deref() != Some("junk")) {
                let Some(conn) = self.conn(key) else { break };
                match conn.messages(&mb.id, "", LIST_PAGE) {
                    Ok(list) => self.store.put_messages(&list),
                    Err(_) => {
                        self.conns.remove(key);
                        break;
                    }
                }
            }
        }
        // Then bodies, newest first. Once full, only mail newer than the oldest kept body comes
        // in, so a pass ends rather than fetching what pruning just dropped.
        let mut floor = budget.filter(|b| self.store.used_bytes() * 10 >= b * 9).and_then(|_| self.store.oldest_body());
        loop {
            if !self.still_on() {
                return;
            }
            let batch: Vec<_> = self
                .store
                .missing_bodies(BATCH + self.skip.len(), floor.as_deref())
                .into_iter()
                .filter(|(uid, ..)| !self.skip.contains(uid))
                .take(BATCH)
                .collect();
            if batch.is_empty() {
                break;
            }
            for (uid, account, id) in batch {
                if !self.fetch(&uid, &account, &id) {
                    return;
                }
            }
            if let Some(b) = budget
                && self.store.used_bytes() > b
            {
                self.store.prune(b);
                floor = self.store.oldest_body();
                if floor.is_none() {
                    break;
                }
            }
        }
        if let Some(b) = budget {
            self.store.prune(b);
        }
    }

    /// One message's body and its attachments. False when the store stopped taking them
    /// (turned off, or the disk is full), so the pass stops rather than download for nothing.
    fn fetch(&mut self, uid: &str, account: &str, id: &str) -> bool {
        let Some(conn) = self.conn(account) else {
            self.skip.insert(uid.to_owned());
            return true;
        };
        let body = match conn.body(id) {
            Ok(b) => b,
            Err(_) => {
                self.conns.remove(account);
                self.skip.insert(uid.to_owned());
                return true;
            }
        };
        match self.store.save_body(uid, &body, false) {
            Some(true) => {}
            Some(false) => {
                self.skip.insert(uid.to_owned()); // too big to keep
                return true;
            }
            None => return false,
        }
        for file in body.files.iter().filter(|f| usize::try_from(f.size).is_ok_and(|n| n <= MAX_FILE_BYTES)) {
            if self.store.has_file(uid, &file.part) {
                continue;
            }
            let Some(conn) = self.conn(account) else { return true };
            if let Ok(data) = conn.attachment(id, file) {
                self.store.save_file(uid, &file.part, &data);
            }
        }
        true
    }
}

/// Whether NetworkManager says the connection is metered (or guesses so).
pub fn metered() -> bool {
    let out = std::process::Command::new("gdbus")
        .args([
            "call", "--system", "--dest", "org.freedesktop.NetworkManager",
            "--object-path", "/org/freedesktop/NetworkManager",
            "--method", "org.freedesktop.DBus.Properties.Get", "org.freedesktop.NetworkManager", "Metered",
        ])
        .output();
    let Ok(out) = out else { return false };
    let s = String::from_utf8_lossy(&out.stdout);
    // "(<uint32 1>,)": 1 is yes, 3 a guessed yes.
    let n = s.split_whitespace().nth(1).and_then(|n| n.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().ok());
    matches!(n, Some(1 | 3))
}
