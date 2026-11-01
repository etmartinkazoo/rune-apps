//! What the window last showed of the phone, kept on disk so the next start shows it at once:
//! the phone, its messages, contacts and recent calls. The phone thread brings it up to date
//! once it has connected. One phone's worth, in the user's cache folder.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::bluez::Device;
use crate::vcard::{Call, Contact};
use crate::worker::Message;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub phone: Option<Device>,
    pub messages: Vec<Message>,
    pub contacts: Vec<Contact>,
    pub recents: Vec<Call>,
}

fn path() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-phone").join("phone.json")
}

/// What was kept of the phone at `address`, if that's the phone it was kept for.
pub fn load(address: &str) -> Option<Cache> {
    let c: Cache = serde_json::from_slice(&std::fs::read(path()).ok()?).ok()?;
    c.phone.as_ref().is_some_and(|p| p.address.eq_ignore_ascii_case(address)).then_some(c)
}

/// What to keep: the phone, and each part the phone has given this time (`None` keeps what was
/// kept before, as when the phone didn't let messages be read this time).
pub struct Update {
    pub phone: Device,
    pub messages: Option<Vec<Message>>,
    pub contacts: Option<Vec<Contact>>,
    pub recents: Option<Vec<Call>>,
}

/// Keeps `u`, off the window's thread. Each write goes to a file of its own and is renamed over
/// the cache, so a reader never sees half of one and the last write wins.
pub fn save(u: Update) {
    static WRITES: AtomicU32 = AtomicU32::new(0);
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    std::thread::spawn(move || {
        // One save at a time: each starts from what the one before it kept.
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let old = load(&u.phone.address).unwrap_or_default();
        let c = Cache {
            phone: Some(u.phone),
            messages: u.messages.unwrap_or(old.messages),
            contacts: u.contacts.unwrap_or(old.contacts),
            recents: u.recents.unwrap_or(old.recents),
        };
        let path = path();
        let Some(dir) = path.parent() else { return };
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(format!("phone.{}.{n}.tmp", std::process::id()));
        let Ok(json) = serde_json::to_vec(&c) else { return };
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    });
}

/// Forgets the phone (it was removed).
pub fn clear() {
    let _ = std::fs::remove_file(path());
}
