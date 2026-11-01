//! Mail kept on this computer: an encrypted SQLite database (SQLCipher), off unless Settings
//! turn it on. On, it keeps each account's mailboxes and contacts, the lists of the mailboxes
//! you've opened, message bodies (the original HTML, compressed, parsed again when opened),
//! attachment files, remote images you chose to load (if that's on), a full-text index over
//! all of it, and actions waiting for the network. The prefetcher (`prefetch.rs`) fills it
//! with the newest mail; `prune` keeps it inside the budget, dropping the oldest first while
//! sparing what you've opened in the last week.
//!
//! The key is random, kept in the system keyring (or, without one, in a file only you can
//! read). Passwords and tokens are never stored here. Remote images are kept only for
//! messages you loaded them for, and never fetched ahead.

use std::io::{Read, Write};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rusqlite::{Connection, OptionalExtension, params};

use crate::contacts::Contact;
use crate::jmap::{Body, Mailbox, Summary};
use crate::worker::{Threads, View};

/// Bodies (as stored, compressed) larger than this aren't kept.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
/// Attachments larger than this aren't kept (they still open, from the server).
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
/// What was opened this recently is spared when pruning.
const RECENT_SECS: i64 = 7 * 24 * 3600;
/// Bumped when the tables change; an older store is rebuilt (it's a cache).
const SCHEMA: i64 = 3;
/// Index pages merged after each batch prune drops.
const MERGE_PAGES: i64 = 200;

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS mailbox (account TEXT NOT NULL, json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS mailbox_account ON mailbox(account);
CREATE TABLE IF NOT EXISTS message (
    uid TEXT PRIMARY KEY, account TEXT NOT NULL, id TEXT NOT NULL, thread_key TEXT NOT NULL,
    received_at TEXT NOT NULL, mailboxes TEXT NOT NULL, json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS message_thread ON message(thread_key);
CREATE INDEX IF NOT EXISTS message_received ON message(received_at);
CREATE TABLE IF NOT EXISTS listing (view TEXT NOT NULL, threads INTEGER NOT NULL, rows TEXT NOT NULL,
    keys TEXT NOT NULL, PRIMARY KEY (view, threads));
CREATE TABLE IF NOT EXISTS body (uid TEXT PRIMARY KEY, meta TEXT NOT NULL, html BLOB,
    received_at TEXT NOT NULL, used INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS body_received ON body(received_at);
CREATE TABLE IF NOT EXISTS file (uid TEXT NOT NULL, part TEXT NOT NULL, data BLOB NOT NULL,
    received_at TEXT NOT NULL, PRIMARY KEY (uid, part));
CREATE TABLE IF NOT EXISTS image (src TEXT PRIMARY KEY, data BLOB NOT NULL, received_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS contact (account TEXT NOT NULL, json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS contact_account ON contact(account);
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS outbox (seq INTEGER PRIMARY KEY AUTOINCREMENT, account TEXT NOT NULL,
    json TEXT NOT NULL, tries INTEGER NOT NULL DEFAULT 0);
CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(
    uid UNINDEXED, sender, recipients, subject, preview, body, tokenize = 'unicode61 remove_diacritics 2',
    prefix = '1 2 3');
-- Each message's row in `search`. Its `uid` column has no index, so finding a message there
-- by uid reads the whole index; by rowid it's one lookup.
CREATE TABLE IF NOT EXISTS search_row (uid TEXT PRIMARY KEY, row INTEGER NOT NULL);
-- Search goes from index rows to messages, and sorts and filters the matches by what this
-- index holds, so only the rows shown are read whole.
CREATE INDEX IF NOT EXISTS search_row_row ON search_row (row);
CREATE INDEX IF NOT EXISTS message_find ON message (uid, received_at, account, mailboxes);
";

pub fn path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-mail").join("mail.db")
}

/// What the store holds, for Settings' meter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub bytes: u64,
    pub messages: u64,
    pub bodies: u64,
    pub files: u64,
    /// The oldest message whose body is kept.
    pub oldest: Option<String>,
}

/// The local mail store; cheap to share between threads.
pub struct Store {
    db: Mutex<Connection>,
    /// A second connection, read-only, for what the UI reads: in WAL mode it reads while the
    /// prefetcher and worker write, rather than waiting for the lock they hold.
    reader: Option<Mutex<Connection>>,
    /// Off (Settings), nothing is read or written.
    enabled: AtomicBool,
}

pub type Shared = Arc<Store>;

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_str(s).ok()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn view_key(view: &View) -> String {
    json(view)
}

pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

pub fn decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    DeflateDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The database key: random, from the keyring, else from a file only you can read. None when
/// the keyring won't answer, since a new key would mean starting the store again.
fn key(existing: bool) -> Option<String> {
    const NAME: &str = "store/key";
    // Two opening at once must agree on one key.
    static MAKING: Mutex<()> = Mutex::new(());
    let _one = MAKING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(k) = crate::secrets::get(NAME).ok()? {
        return Some(k);
    }
    let file = crate::worker::config_dir().join("store.key");
    if let Ok(k) = std::fs::read_to_string(&file) {
        let k = k.trim().to_owned();
        // Moves into the keyring when there is one now.
        if crate::secrets::set(NAME, &k) {
            let _ = std::fs::remove_file(&file);
        }
        return Some(k);
    }
    // A store on disk with its key in neither place: the key is in a keyring that isn't
    // there right now. Without the store for this run, rather than throwing it away.
    if existing && !crate::secrets::available() {
        return None;
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    let k: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if !crate::secrets::set(NAME, &k) {
        std::fs::create_dir_all(file.parent()?).ok()?;
        std::fs::write(&file, &k).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        }
    }
    Some(k)
}

/// A message's row in the search index.
fn search_row(db: &Connection, uid: &str) -> rusqlite::Result<Option<i64>> {
    db.query_row("SELECT row FROM search_row WHERE uid = ?1", [uid], |r| r.get(0)).optional()
}

fn remove_files(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

enum Opened {
    Store(Connection),
    /// The file isn't this store: another key, unencrypted, or an older schema.
    Foreign,
    /// It couldn't be read just now, such as while another copy of the app has it locked.
    Failed,
}

fn open_with(path: &Path, key: &str) -> Opened {
    let opened = || -> rusqlite::Result<Option<Connection>> {
        let db = Connection::open(path)?;
        db.execute_batch(&format!("PRAGMA key = \"x'{key}'\";"))?;
        let tables: i64 = db.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        Ok((tables == 0 || version == SCHEMA).then_some(db))
    };
    match opened() {
        Ok(Some(db)) => Opened::Store(db),
        Ok(None) => Opened::Foreign,
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::NotADatabase => Opened::Foreign,
        Err(_) => Opened::Failed,
    }
}

impl Store {
    /// Opens (creating, or rebuilding one it can't read) the store at `path` with `key`.
    pub fn open_at(path: &Path, key: &str) -> Option<Shared> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        let db = match open_with(path, key) {
            Opened::Store(db) => db,
            Opened::Foreign => {
                remove_files(path);
                match open_with(path, key) {
                    Opened::Store(db) => db,
                    _ => return None,
                }
            }
            Opened::Failed => return None,
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        // Incremental vacuum lets pruning give space back without rewriting the file.
        db.execute_batch(&format!(
            "PRAGMA auto_vacuum = INCREMENTAL; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;
             {TABLES} PRAGMA user_version = {SCHEMA};"
        ))
        .ok()?;
        // A store from before `search_row`: map its index once (and drop any stray second
        // entries for a message), rather than rebuilding it.
        let mapped: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM search_row)", [], |r| r.get(0)).ok()?;
        if !mapped {
            db.execute_batch(
                "BEGIN; INSERT OR IGNORE INTO search_row (uid, row) SELECT uid, rowid FROM search ORDER BY rowid DESC;
                 DELETE FROM search WHERE rowid NOT IN (SELECT row FROM search_row); COMMIT;",
            )
            .ok()?;
        }
        let reader = match open_with(path, key) {
            Opened::Store(r) => r.execute_batch("PRAGMA query_only = 1;").ok().map(|()| Mutex::new(r)),
            _ => None,
        };
        Some(Arc::new(Store { db: Mutex::new(db), reader, enabled: AtomicBool::new(true) }))
    }

    /// Rebuilds a search index from before it kept prefixes, which searches as you type are
    /// made of: "a" read most of the index without them. A rewrite, so it runs once, off the
    /// UI thread (some 0.7 s for 6,000 messages); the UI's reads don't wait for it.
    pub fn upgrade_index(&self) {
        self.with(|db| {
            let sql: String = db.query_row("SELECT sql FROM sqlite_master WHERE name = 'search'", [], |r| r.get(0))?;
            if sql.contains("prefix") {
                return Ok(());
            }
            db.execute_batch(
                "BEGIN;
                 CREATE VIRTUAL TABLE search_new USING fts5(
                     uid UNINDEXED, sender, recipients, subject, preview, body,
                     tokenize = 'unicode61 remove_diacritics 2', prefix = '1 2 3');
                 INSERT INTO search_new (rowid, uid, sender, recipients, subject, preview, body)
                     SELECT rowid, uid, sender, recipients, subject, preview, body FROM search;
                 DROP TABLE search;
                 ALTER TABLE search_new RENAME TO search;
                 COMMIT;",
            )
        });
    }

    pub fn open() -> Option<Shared> {
        let path = path();
        Store::open_at(&path, &key(path.exists())?)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Turns the store on or off. Off, nothing reads or writes it from the moment this returns,
    /// even what was already waiting for the lock; [`Store::clear`] empties it.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }

    /// Empties the store, on or off, and hands back the actions that were waiting for the
    /// network as (account, action), so they can still be sent. Slow on a big store: not for
    /// the UI thread.
    pub fn clear(&self) -> Vec<(String, String)> {
        let db = self.lock();
        let queued = db
            .prepare("SELECT account, json FROM outbox ORDER BY seq")
            .and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
            .unwrap_or_default();
        let _ = db.execute_batch(
            "DELETE FROM mailbox; DELETE FROM message; DELETE FROM listing; DELETE FROM body;
             DELETE FROM file; DELETE FROM image; DELETE FROM contact; DELETE FROM kv;
             DELETE FROM outbox; DELETE FROM search; DELETE FROM search_row; VACUUM;",
        );
        queued
    }

    /// The connection. A thread that panicked holding it leaves it usable: a transaction it
    /// left open rolls back as it's dropped.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// [`Store::with`] for reads, on the reader connection when there is one.
    fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
        let Some(reader) = &self.reader else { return self.with(f) };
        if !self.is_enabled() {
            return None;
        }
        let db = reader.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_enabled() {
            return None;
        }
        f(&db).ok()
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
        if !self.is_enabled() {
            return None;
        }
        let db = self.lock();
        // Again under the lock: it may have been turned off while this waited for it.
        if !self.is_enabled() {
            return None;
        }
        f(&db).ok()
    }

    // Small values: the last view, sync states.

    pub fn get(&self, key: &str) -> Option<String> {
        self.read(|db| db.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()).flatten()
    }

    pub fn set(&self, key: &str, value: &str) {
        self.with(|db| db.execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)", [key, value]));
    }

    /// An account's events for `day` (YYYY-MM-DD), so the sidebar shows them before it connects.
    pub fn save_events(&self, account: &str, day: &str, events: &[crate::caldav::Event]) {
        self.set(&format!("events/{account}"), &json(&(day, events)));
    }

    pub fn events(&self, account: &str, day: &str) -> Vec<crate::caldav::Event> {
        self.get(&format!("events/{account}"))
            .and_then(|v| parse::<(String, Vec<crate::caldav::Event>)>(&v))
            .filter(|(d, _)| d == day)
            .map(|(_, e)| e)
            .unwrap_or_default()
    }

    // Mailboxes and contacts: replaced whole, per account.

    pub fn save_mailboxes(&self, account: &str, list: &[Mailbox]) {
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute("DELETE FROM mailbox WHERE account = ?1", [account])?;
            for m in list {
                tx.execute("INSERT INTO mailbox (account, json) VALUES (?1, ?2)", [account, &json(m)])?;
            }
            tx.commit()
        });
    }

    pub fn mailboxes(&self, account: &str) -> Vec<Mailbox> {
        self.rows("SELECT json FROM mailbox WHERE account = ?1", account)
    }

    pub fn save_contacts(&self, account: &str, list: &[Contact]) {
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute("DELETE FROM contact WHERE account = ?1", [account])?;
            for c in list {
                tx.execute("INSERT INTO contact (account, json) VALUES (?1, ?2)", [account, &json(c)])?;
            }
            tx.commit()
        });
    }

    pub fn contacts(&self, account: &str) -> Vec<Contact> {
        self.rows("SELECT json FROM contact WHERE account = ?1", account)
    }

    fn rows<T: serde::de::DeserializeOwned>(&self, sql: &str, arg: &str) -> Vec<T> {
        self.read(|db| {
            let mut q = db.prepare(sql)?;
            let it = q.query_map([arg], |r| r.get::<_, String>(0))?;
            Ok(it.filter_map(|s| s.ok().and_then(|s| parse(&s))).collect())
        })
        .unwrap_or_default()
    }

    // Messages and the lists of the mailboxes you've opened.

    fn put_message(tx: &rusqlite::Transaction, m: &Summary) -> rusqlite::Result<()> {
        // Most of a list is as it was last time; rewriting it would also churn the index.
        let new = json(m);
        let old: Option<String> =
            tx.query_row("SELECT json FROM message WHERE uid = ?1", [&m.uid], |r| r.get(0)).optional()?;
        if old.as_ref() == Some(&new) {
            return Ok(());
        }
        // A flag or read change leaves what's indexed as it was.
        let same_words = old.as_deref().and_then(parse::<Summary>).is_some_and(|o| {
            o.from == m.from && o.to == m.to && o.subject == m.subject && o.preview == m.preview
        });
        tx.execute(
            "INSERT OR REPLACE INTO message (uid, account, id, thread_key, received_at, mailboxes, json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![m.uid, m.account, m.id, m.thread_key, m.received_at, json(&m.mailbox_ids), new],
        )?;
        // One index entry per message; the body column fills in once the body is kept.
        let names = |list: &[crate::jmap::Addr]| {
            list.iter().map(|a| format!("{} {}", a.name.as_deref().unwrap_or(""), a.email)).collect::<Vec<_>>().join(" ")
        };
        let (from, to) = (names(&m.from), names(&m.to));
        match search_row(tx, &m.uid)? {
            Some(_) if same_words => {}
            Some(row) => {
                tx.execute(
                    "UPDATE search SET sender = ?2, recipients = ?3, subject = ?4, preview = ?5 WHERE rowid = ?1",
                    params![row, from, to, m.subject, m.preview],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO search (uid, sender, recipients, subject, preview, body) VALUES (?1, ?2, ?3, ?4, ?5, '')",
                    params![m.uid, from, to, m.subject, m.preview],
                )?;
                tx.execute("INSERT INTO search_row (uid, row) VALUES (?1, ?2)", params![m.uid, tx.last_insert_rowid()])?;
            }
        }
        Ok(())
    }

    /// Keeps messages seen in a mailbox (the prefetcher's lists), outside any view's list.
    pub fn put_messages(&self, list: &[Summary]) {
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            for m in list {
                Store::put_message(&tx, m)?;
            }
            tx.commit()
        });
    }

    /// Keeps a view's rows and conversations as they came from the server.
    pub fn save_listing(&self, view: &View, threads_on: bool, rows: &[Summary], threads: &Threads) {
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            for m in rows.iter().chain(threads.values().flatten()) {
                Store::put_message(&tx, m)?;
            }
            let row_ids: Vec<&str> = rows.iter().map(|m| m.uid.as_str()).collect();
            let keys: Vec<(&str, Vec<&str>)> =
                threads.iter().map(|(k, v)| (k.as_str(), v.iter().map(|m| m.uid.as_str()).collect())).collect();
            tx.execute(
                "INSERT OR REPLACE INTO listing (view, threads, rows, keys) VALUES (?1, ?2, ?3, ?4)",
                params![view_key(view), threads_on, json(&row_ids), json(&keys)],
            )?;
            tx.commit()
        });
    }

    fn message(db: &Connection, uid: &str) -> Option<Summary> {
        db.query_row("SELECT json FROM message WHERE uid = ?1", [uid], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            .and_then(|s| parse(&s))
    }

    /// A view's rows and conversations as last seen, if it's been opened before.
    pub fn listing(&self, view: &View, threads_on: bool) -> Option<(Vec<Summary>, Threads)> {
        self.read(|db| {
            let (rows, keys): (String, String) = db.query_row(
                "SELECT rows, keys FROM listing WHERE view = ?1 AND threads = ?2",
                params![view_key(view), threads_on],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let rows: Vec<String> = parse(&rows).unwrap_or_default();
            let keys: Vec<(String, Vec<String>)> = parse(&keys).unwrap_or_default();
            let rows: Vec<Summary> = rows.iter().filter_map(|u| Store::message(db, u)).collect();
            let threads: Threads = keys
                .into_iter()
                .map(|(k, uids)| (k, uids.iter().filter_map(|u| Store::message(db, u)).collect()))
                .collect();
            Ok((rows, threads))
        })
    }

    /// Kept messages without a kept body, newest first: (uid, account, id), what the
    /// prefetcher fetches next.
    /// Only those received after `newer_than` when given.
    pub fn missing_bodies(&self, limit: usize, newer_than: Option<&str>) -> Vec<(String, String, String)> {
        self.with(|db| {
            let mut q = db.prepare(
                "SELECT m.uid, m.account, m.id FROM message m LEFT JOIN body b ON b.uid = m.uid
                 WHERE b.uid IS NULL AND m.received_at > ?2 ORDER BY m.received_at DESC LIMIT ?1",
            )?;
            let it = q.query_map(params![limit as i64, newer_than.unwrap_or("")], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(it.filter_map(Result::ok).collect())
        })
        .unwrap_or_default()
    }

    fn received_of(&self, uid: &str) -> Option<String> {
        self.with(|db| db.query_row("SELECT received_at FROM message WHERE uid = ?1", [uid], |r| r.get(0)).optional())
            .flatten()
    }

    // Bodies: the meta as JSON, the HTML source compressed and parsed again when read.

    /// The date of the oldest kept body.
    pub fn oldest_body(&self) -> Option<String> {
        self.with(|db| db.query_row("SELECT min(received_at) FROM body", [], |r| r.get::<_, Option<String>>(0))).flatten()
    }

    /// `opened`: you read it (pruning spares it a week), rather than it being fetched ahead.
    /// Whether it's kept (false: too big to), or None when the store couldn't take it.
    pub fn save_body(&self, uid: &str, body: &Body, opened: bool) -> Option<bool> {
        let meta = json(&Body { html: None, html_src: None, ..body.clone() });
        let html = body.html_src.as_ref().map(|h| compress(h.as_bytes()));
        if meta.len() + html.as_ref().map_or(0, Vec::len) > MAX_BODY_BYTES {
            return Some(false);
        }
        let received = self.received_of(uid).unwrap_or_default();
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute(
                "INSERT OR REPLACE INTO body (uid, meta, html, received_at, used) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![uid, meta, html, received, if opened { now() } else { 0 }],
            )?;
            tx.execute(
                "UPDATE search SET body = ?2 WHERE rowid = (SELECT row FROM search_row WHERE uid = ?1)",
                params![uid, body.text],
            )?;
            tx.commit()
        })
        .map(|()| true)
    }

    #[cfg(test)]
    pub fn has_body(&self, uid: &str) -> bool {
        self.with(|db| db.query_row("SELECT 1 FROM body WHERE uid = ?1", [uid], |_| Ok(())).optional()).flatten().is_some()
    }

    /// A kept body, marked used so pruning spares it a while.
    pub fn body(&self, uid: &str) -> Option<Body> {
        let (meta, html): (String, Option<Vec<u8>>) = self
            .with(|db| {
                let row = db
                    .query_row("SELECT meta, html FROM body WHERE uid = ?1", [uid], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                if row.is_some() {
                    db.execute("UPDATE body SET used = ?2 WHERE uid = ?1", params![uid, now()])?;
                }
                Ok(row)
            })
            .flatten()?;
        let mut body: Body = parse(&meta)?;
        if let Some(src) = html.and_then(|h| decompress(&h)).and_then(|h| String::from_utf8(h).ok()) {
            body.html = Some(crate::html::parse(&src, &|_| None));
            body.html_src = Some(src);
        }
        Some(body)
    }

    // Attachment files and images.

    pub fn save_file(&self, uid: &str, part: &str, data: &[u8]) {
        if data.len() > MAX_FILE_BYTES {
            return;
        }
        let received = self.received_of(uid).unwrap_or_default();
        // Compressed before taking the lock: a large file takes a while.
        let data = compress(data);
        self.with(|db| {
            db.execute(
                "INSERT OR REPLACE INTO file (uid, part, data, received_at) VALUES (?1, ?2, ?3, ?4)",
                params![uid, part, data, received],
            )
        });
    }

    pub fn file(&self, uid: &str, part: &str) -> Option<Vec<u8>> {
        let data: Vec<u8> = self
            .with(|db| {
                db.query_row("SELECT data FROM file WHERE uid = ?1 AND part = ?2", params![uid, part], |r| r.get(0)).optional()
            })
            .flatten()?;
        decompress(&data)
    }

    pub fn has_file(&self, uid: &str, part: &str) -> bool {
        self.with(|db| {
            db.query_row("SELECT 1 FROM file WHERE uid = ?1 AND part = ?2", params![uid, part], |_| Ok(())).optional()
        })
        .flatten()
        .is_some()
    }

    /// Keeps an image a message showed (`uid` is the message, whose date prunes it).
    pub fn save_image(&self, src: &str, data: &[u8], uid: &str) {
        let received = self.received_of(uid).unwrap_or_default();
        self.with(|db| {
            db.execute(
                "INSERT OR REPLACE INTO image (src, data, received_at) VALUES (?1, ?2, ?3)",
                params![src, data, received],
            )
        });
    }

    pub fn image(&self, src: &str) -> Option<Vec<u8>> {
        self.with(|db| db.query_row("SELECT data FROM image WHERE src = ?1", [src], |r| r.get(0)).optional()).flatten()
    }

    // Size.

    /// Bytes the data takes (pages in use, not free ones awaiting vacuum).
    pub fn used_bytes(&self) -> u64 {
        self.read(|db| {
            let pages: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
            let free: i64 = db.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
            // SQLCipher answers page_size as text.
            let size: i64 = db.query_row("PRAGMA page_size", [], |r| {
                Ok(match r.get::<_, rusqlite::types::Value>(0)? {
                    rusqlite::types::Value::Integer(n) => n,
                    rusqlite::types::Value::Text(t) => t.trim().parse().unwrap_or(4096),
                    _ => 4096,
                })
            })?;
            Ok(((pages - free).max(0) * size) as u64)
        })
        .unwrap_or(0)
    }

    pub fn usage(&self) -> Usage {
        let count = |sql: &str| self.read(|db| db.query_row(sql, [], |r| r.get::<_, i64>(0))).map_or(0, |n| n.max(0) as u64);
        Usage {
            bytes: self.used_bytes(),
            messages: count("SELECT count(*) FROM message"),
            bodies: count("SELECT count(*) FROM body"),
            files: count("SELECT count(*) FROM file"),
            oldest: self
                .read(|db| {
                    db.query_row("SELECT min(received_at) FROM body WHERE received_at <> ''", [], |r| r.get::<_, Option<String>>(0))
                })
                .flatten(),
        }
    }

    /// Drops the oldest mail until the data fits in `budget` bytes: whichever of images, files
    /// and bodies holds the oldest goes first, then messages no list shows (and their index
    /// entries). What was opened in the last week stays. The space goes back to the disk.
    pub fn prune(&self, budget: u64) {
        let recent = now() - RECENT_SECS;
        // Every message a saved list shows, as a row or in a row's conversation.
        let mut shown: Option<HashSet<String>> = None;
        for _ in 0..500 {
            if self.used_bytes() <= budget {
                break;
            }
            let dropped = self
                .with(|db| {
                    let tx = db.unchecked_transaction()?;
                    let oldest = |sql: &str| -> rusqlite::Result<Option<String>> { tx.query_row(sql, [], |r| r.get(0)).optional() };
                    let candidates = [
                        ("image", oldest("SELECT received_at FROM image ORDER BY received_at LIMIT 1")?),
                        ("file", oldest("SELECT received_at FROM file ORDER BY received_at LIMIT 1")?),
                        ("body", oldest(&format!("SELECT received_at FROM body WHERE used < {recent} ORDER BY received_at LIMIT 1"))?),
                    ];
                    let pick = candidates
                        .iter()
                        .filter_map(|(t, r)| r.as_ref().map(|r| (*t, r.clone())))
                        .min_by(|a, b| a.1.cmp(&b.1))
                        .map(|(t, _)| t);
                    let n = match pick {
                        Some("body") => {
                            // The index forgets the text too, or the space never comes back.
                            let pick = format!("SELECT uid FROM body WHERE used < {recent} ORDER BY received_at LIMIT 5");
                            tx.execute(
                                &format!("UPDATE search SET body = '' WHERE rowid IN (SELECT row FROM search_row WHERE uid IN ({pick}))"),
                                [],
                            )?;
                            let n = tx.execute(&format!("DELETE FROM body WHERE uid IN ({pick})"), [])?;
                            // Dropped text leaves the index only as its segments merge. A
                            // bounded merge, not 'optimize', which rewrites the whole index.
                            tx.execute("INSERT INTO search (search, rank) VALUES ('merge', ?1)", [MERGE_PAGES])?;
                            n
                        }
                        Some(table) => tx.execute(
                            &format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} ORDER BY received_at LIMIT 5)"),
                            [],
                        )?,
                        None => {
                            // Only list rows left: the oldest messages no saved list shows.
                            if shown.is_none() {
                                shown = Some(Store::shown(&tx)?);
                            }
                            let shown = shown.as_ref().expect("filled just above");
                            let mut q = tx.prepare("SELECT uid FROM message ORDER BY received_at LIMIT 400")?;
                            let old: Vec<String> = q.query_map([], |r| r.get(0))?.filter_map(Result::ok).collect();
                            drop(q);
                            let mut n = 0;
                            for uid in old.iter().filter(|u| !shown.contains(*u)).take(100) {
                                n += tx.execute("DELETE FROM message WHERE uid = ?1", [uid])?;
                                if let Some(row) = search_row(&tx, uid)? {
                                    tx.execute("DELETE FROM search WHERE rowid = ?1", [row])?;
                                    tx.execute("DELETE FROM search_row WHERE uid = ?1", [uid])?;
                                }
                            }
                            n
                        }
                    };
                    tx.commit()?;
                    Ok(n)
                })
                .unwrap_or(0);
            if dropped == 0 {
                break;
            }
        }
        self.with(|db| db.execute_batch("PRAGMA incremental_vacuum;"));
    }

    fn shown(db: &Connection) -> rusqlite::Result<HashSet<String>> {
        let mut shown = HashSet::new();
        let mut q = db.prepare("SELECT rows, keys FROM listing")?;
        for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (rows, keys) = r?;
            shown.extend(parse::<Vec<String>>(&rows).unwrap_or_default());
            shown.extend(parse::<Vec<(String, Vec<String>)>>(&keys).unwrap_or_default().into_iter().flat_map(|(_, uids)| uids));
        }
        Ok(shown)
    }

    // Search.

    /// Kept messages matching every word of `query` (as prefixes), newest first, within
    /// `mailboxes` (account, mailbox id) when given.
    pub fn search(&self, query: &str, mailboxes: &[(String, String)], limit: usize) -> Vec<Summary> {
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '@' || *c == '.').collect::<String>())
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{}\"*", w.replace('"', "")))
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        let fts = words.join(" AND ");
        // The mailboxes are matched here rather than after reading, so only the rows shown are
        // read and parsed. `mailboxes` holds each message's ids as a JSON array.
        let mut args: Vec<String> = vec![fts, limit.to_string()];
        let mut within = String::new();
        for (i, (account, id)) in mailboxes.iter().enumerate() {
            within.push_str(if i == 0 { " AND (" } else { " OR " });
            within.push_str(&format!("(mm.account = ?{} AND instr(mm.mailboxes, ?{}) > 0)", args.len() + 1, args.len() + 2));
            args.push(account.clone());
            args.push(json(id));
        }
        if !within.is_empty() {
            within.push(')');
        }
        self.read(|db| {
            let mut q = db.prepare(&format!(
                "SELECT m.json FROM message m JOIN (
                     SELECT r.uid AS uid, mm.received_at AS at FROM search s
                     JOIN search_row r ON r.row = s.rowid
                     JOIN message mm INDEXED BY message_find ON mm.uid = r.uid
                     WHERE search MATCH ?1{within} ORDER BY mm.received_at DESC LIMIT ?2
                 ) t ON m.uid = t.uid
                 ORDER BY t.at DESC"
            ))?;
            let it = q.query_map(rusqlite::params_from_iter(&args), |r| r.get::<_, String>(0))?;
            Ok(it.filter_map(|s| s.ok().and_then(|s| parse::<Summary>(&s))).collect())
        })
        .unwrap_or_default()
    }

    // Actions waiting for the network.

    /// Keeps an action for later; false if the store couldn't take it.
    pub fn queue(&self, account: &str, op: &str) -> bool {
        self.with(|db| db.execute("INSERT INTO outbox (account, json) VALUES (?1, ?2)", [account, op])).is_some()
    }

    /// Queued actions, oldest first: (seq, account, op).
    pub fn queued(&self) -> Vec<(i64, String, String)> {
        self.with(|db| {
            let mut q = db.prepare("SELECT seq, account, json FROM outbox ORDER BY seq")?;
            let it = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(it.filter_map(Result::ok).collect())
        })
        .unwrap_or_default()
    }

    /// A queued action went through, or gave up after its tries (the server wins).
    pub fn done(&self, seq: i64) {
        self.with(|db| db.execute("DELETE FROM outbox WHERE seq = ?1", [seq]));
    }

    /// Counts a failed attempt; returns how many there have been, None if it couldn't count.
    pub fn tried(&self, seq: i64) -> Option<i64> {
        self.with(|db| {
            db.execute("UPDATE outbox SET tries = tries + 1 WHERE seq = ?1", [seq])?;
            db.query_row("SELECT tries FROM outbox WHERE seq = ?1", [seq], |r| r.get(0))
        })
    }

    /// Drops everything kept for an account (it was removed).
    pub fn forget(&self, account: &str) {
        // Matched exactly, not with LIKE: `_` in an address is a LIKE wildcard.
        let prefix = format!("{account}:");
        let mine = "substr(uid, 1, length(?1)) = ?1";
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute("DELETE FROM mailbox WHERE account = ?1", [account])?;
            tx.execute("DELETE FROM contact WHERE account = ?1", [account])?;
            tx.execute(&format!("DELETE FROM search WHERE rowid IN (SELECT row FROM search_row WHERE {mine})"), [&prefix])?;
            tx.execute(&format!("DELETE FROM search_row WHERE {mine}"), [&prefix])?;
            tx.execute(&format!("DELETE FROM body WHERE {mine}"), [&prefix])?;
            tx.execute(&format!("DELETE FROM file WHERE {mine}"), [&prefix])?;
            tx.execute("DELETE FROM message WHERE account = ?1", [account])?;
            tx.execute("DELETE FROM outbox WHERE account = ?1", [account])?;
            tx.execute("DELETE FROM listing WHERE instr(view, ?1) > 0", [&format!("\"{account}\"")])?;
            tx.commit()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!("rune-mail-store-{}-{}", std::process::id(), fastrand::u32(..)))
    }

    fn store() -> Shared {
        Store::open_at(&dir().join("mail.db"), KEY).expect("open")
    }

    fn msg(id: &str, subject: &str, mailbox: &str) -> Summary {
        Summary {
            id: id.into(),
            account: "a@x.com".into(),
            uid: crate::jmap::uid("a@x.com", id),
            thread_id: id.into(),
            thread_key: crate::jmap::uid("a@x.com", id),
            from: vec![crate::jmap::Addr { name: Some("Grace Hopper".into()), email: "grace@example.com".into() }],
            to: vec![],
            subject: subject.into(),
            preview: "about the compiler".into(),
            received_at: format!("2026-10-{id:0>2}T10:00:00Z"),
            seen: false,
            flagged: false,
            has_attachment: false,
            mailbox_ids: vec![mailbox.into()],
        }
    }

    fn body(text: &str, html: Option<&str>) -> Body {
        Body {
            text: text.into(),
            html: html.map(|h| crate::html::parse(h, &|_| None)),
            html_src: html.map(str::to_owned),
            ..Body::default()
        }
    }

    #[test]
    fn keeps_listings_bodies_and_finds_words() {
        let s = store();
        let view = View::Mailbox { account: "a@x.com".into(), id: "in".into() };
        let rows = vec![msg("2", "Moth found", "in"), msg("1", "Relay trouble", "in")];
        let threads: Threads = rows.iter().map(|m| (m.thread_key.clone(), vec![m.clone()])).collect();
        s.save_listing(&view, true, &rows, &threads);
        let (back, back_threads) = s.listing(&view, true).expect("kept");
        assert_eq!(back.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Moth found", "Relay trouble"]);
        assert_eq!(back_threads.len(), 2);
        assert!(s.listing(&view, false).is_none(), "conversations on and off are kept apart");

        s.save_body(&rows[1].uid, &body("The bug was a moth in relay 70", Some("<p>The bug was a <b>moth</b></p>")), true);
        let back = s.body(&rows[1].uid).expect("kept body");
        assert_eq!(back.text, "The bug was a moth in relay 70");
        assert!(back.html.is_some(), "the HTML is parsed again on reading");

        let found = |q: &str| s.search(q, &[], 10).into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(found("moth"), ["2", "1"], "subject, then the body of the one read");
        assert_eq!(found("relay 70"), ["1"]);
        assert_eq!(found("grace"), ["2", "1"], "senders are indexed");
        assert!(s.search("moth", &[("a@x.com".into(), "other".into())], 10).is_empty(), "limited to mailboxes");
        assert_eq!(s.missing_bodies(10, None).into_iter().map(|(_, _, id)| id).collect::<Vec<_>>(), ["2"]);

        s.forget("a@x.com");
        assert!(s.listing(&view, true).is_none());
        assert!(found("moth").is_empty());
    }

    #[test]
    fn a_message_seen_again_keeps_one_index_entry_and_its_body() {
        let s = store();
        let mut m = msg("3", "Moth found", "in");
        s.put_messages(std::slice::from_ref(&m));
        s.save_body(&m.uid, &body("relay seventy", None), true);
        m.subject = "Moth found again".into();
        m.seen = true;
        s.put_messages(std::slice::from_ref(&m));
        let found = |q: &str| s.search(q, &[], 10).into_iter().map(|m| m.subject).collect::<Vec<_>>();
        assert_eq!(found("moth"), ["Moth found again"], "one entry, updated");
        assert_eq!(found("seventy"), ["Moth found again"], "the body's words stay indexed");
    }

    #[test]
    fn an_index_from_before_the_row_map_is_mapped_on_opening() {
        let p = dir().join("mail.db");
        let s = Store::open_at(&p, KEY).unwrap();
        s.put_messages(&[msg("4", "Moth found", "in"), msg("5", "Relay trouble", "in")]);
        // As an older store has it: index entries, no map, and a stray second entry.
        s.with(|db| db.execute_batch(
            "DELETE FROM search_row; INSERT INTO search (uid, sender, recipients, subject, preview, body)
             SELECT uid, sender, recipients, subject, preview, body FROM search WHERE subject = 'Moth found';",
        ));
        drop(s);
        let s = Store::open_at(&p, KEY).unwrap();
        let found = s.search("moth", &[], 10);
        assert_eq!(found.len(), 1, "the stray entry went");
        let mut m = msg("5", "Relay fixed", "in");
        m.seen = true;
        s.put_messages(&[m]);
        assert_eq!(s.search("relay", &[], 10).into_iter().map(|m| m.subject).collect::<Vec<_>>(), ["Relay fixed"]);
    }

    #[test]
    fn is_encrypted_and_rebuilt_under_another_key() {
        let p = dir().join("mail.db");
        let s = Store::open_at(&p, KEY).unwrap();
        s.set("hello", "world");
        drop(s);
        let raw = std::fs::read(&p).unwrap();
        assert!(!raw.windows(15).any(|w| w == b"SQLite format 3"), "no plain SQLite header on disk");
        assert!(!raw.windows(5).any(|w| w == b"world"));
        let other = Store::open_at(&p, &"f".repeat(64)).unwrap();
        assert_eq!(other.get("hello"), None, "a store it can't read is rebuilt, not trusted");
    }

    #[test]
    fn a_store_another_copy_has_locked_is_left_alone() {
        let p = dir().join("mail.db");
        let s = Store::open_at(&p, KEY).unwrap();
        s.set("hello", "world");
        drop(s);
        // Another copy, holding it.
        let other = Connection::open(&p).unwrap();
        other.execute_batch(&format!("PRAGMA key = \"x'{KEY}'\"; PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE;")).unwrap();
        assert!(Store::open_at(&p, KEY).is_none(), "busy isn't foreign");
        other.execute_batch("COMMIT;").unwrap();
        drop(other);
        assert_eq!(Store::open_at(&p, KEY).unwrap().get("hello").as_deref(), Some("world"));
    }

    #[test]
    fn prunes_the_oldest_first_and_spares_what_was_opened() {
        let s = store();
        // Text that doesn't compress away, so each body takes real space.
        let noise = |seed: u64| {
            let mut x = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
            (0..20_000)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    char::from(b'a' + (x % 26) as u8)
                })
                .collect::<String>()
        };
        let list: Vec<Summary> = (1..=28).map(|d| msg(&d.to_string(), "s", "in")).collect();
        s.put_messages(&list);
        for m in &list {
            // Fetched ahead, except the oldest, which was opened.
            s.save_body(&m.uid, &body(&noise(m.id.parse().unwrap()), Some(&noise(99 + m.id.parse::<u64>().unwrap()))), m.uid == list[0].uid);
        }
        let before = s.used_bytes();
        s.prune(before / 2);
        assert!(s.used_bytes() <= before / 2, "{} > {}", s.used_bytes(), before / 2);
        assert!(s.has_body(&list[0].uid), "opened this week: kept");
        assert!(!s.has_body(&list[1].uid), "the oldest otherwise: gone");
        assert!(s.has_body(&list[27].uid), "the newest: kept");
    }

    #[test]
    fn keeps_todays_events_for_the_next_launch() {
        let s = store();
        let e = crate::caldav::Event {
            title: "Standup".into(), location: None, start: None, end: None, calendar: "Work".into(), color: None,
        };
        s.save_events("a@x.com", "2026-10-30", std::slice::from_ref(&e));
        assert_eq!(s.events("a@x.com", "2026-10-30"), [e]);
        assert!(s.events("a@x.com", "2026-10-31").is_empty(), "yesterday's aren't today's");
    }

    #[test]
    fn pruning_keeps_the_earlier_messages_of_a_kept_conversation() {
        let s = store();
        let view = View::Mailbox { account: "a@x.com".into(), id: "in".into() };
        let (older, newer, stray) = (msg("1", "Question", "in"), msg("2", "Re: Question", "in"), msg("0", "Stray", "in"));
        s.put_messages(std::slice::from_ref(&stray));
        let threads: Threads = [(newer.thread_key.clone(), vec![older.clone(), newer.clone()])].into_iter().collect();
        s.save_listing(&view, true, std::slice::from_ref(&newer), &threads);
        s.prune(0);
        let (_, threads) = s.listing(&view, true).expect("kept");
        assert_eq!(threads[&newer.thread_key].len(), 2, "the conversation keeps its first message");
        assert!(s.search("stray", &[], 10).is_empty(), "what no list shows goes");
    }

    #[test]
    fn removing_an_account_leaves_lookalike_addresses_alone() {
        let s = store();
        let at = |account: &str| Summary { account: account.into(), uid: crate::jmap::uid(account, "1"), ..msg("1", "Hello", "in") };
        let (mine, theirs) = (at("j_doe@x.com"), at("jxdoe@x.com"));
        s.put_messages(&[mine.clone(), theirs.clone()]);
        s.save_body(&theirs.uid, &body("kept", None), true);
        s.forget("j_doe@x.com");
        assert!(s.has_body(&theirs.uid));
        assert_eq!(s.search("hello", &[], 10).into_iter().map(|m| m.account).collect::<Vec<_>>(), ["jxdoe@x.com"]);
    }

    #[test]
    fn clearing_hands_back_waiting_actions_and_off_means_off() {
        let s = store();
        s.set("hello", "world");
        s.queue("a@x.com", r#"{"op":1}"#);
        s.set_enabled(false);
        assert_eq!(s.get("hello"), None, "off: nothing reads");
        assert!(!s.queue("a@x.com", "{}"), "off: nothing writes");
        assert_eq!(s.clear(), [("a@x.com".to_owned(), r#"{"op":1}"#.to_owned())]);
        s.set_enabled(true);
        assert_eq!(s.get("hello"), None);
        assert!(s.queued().is_empty());
    }

    #[test]
    fn reads_dont_wait_for_a_writer() {
        let s = store();
        s.set("view", "inbox");
        let _writing = s.lock();
        assert_eq!(s.get("view").as_deref(), Some("inbox"), "read while the write lock is held");
    }

    #[test]
    fn queues_actions_until_they_go_through() {
        let s = store();
        s.queue("a@x.com", r#"{"op":1}"#);
        let q = s.queued();
        assert_eq!(q.len(), 1);
        assert_eq!(s.tried(q[0].0), Some(1));
        s.done(q[0].0);
        assert!(s.queued().is_empty());
    }
}
