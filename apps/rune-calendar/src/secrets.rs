//! App passwords in the system keyring (the Secret Service on Linux: GNOME Keyring, KWallet),
//! so `accounts.json` holds only settings. Without a keyring they stay in that file, readable
//! only by you.

pub const SERVICE: &str = "rune-calendar";
/// Rune Mail's entries, read when importing its accounts.
pub const MAIL: &str = "rune-mail";

/// Whether the keyring can be used. Tests and `RUNE_CALENDAR_NO_KEYRING` never touch it.
pub fn available() -> bool {
    if cfg!(test) || std::env::var_os("RUNE_CALENDAR_NO_KEYRING").is_some() {
        return false;
    }
    keyring::Entry::store_status().is_ok()
}

fn entry(service: &str, name: &str) -> Option<keyring::Entry> {
    available().then(|| keyring::Entry::new(service, name).ok()).flatten()
}

pub fn get_from(service: &str, name: &str) -> Option<String> {
    entry(service, name)?.get_password().ok().filter(|s| !s.is_empty())
}

pub fn get(name: &str) -> Option<String> {
    get_from(SERVICE, name)
}

/// Stores a secret; false if the keyring couldn't take it.
pub fn set(name: &str, value: &str) -> bool {
    entry(SERVICE, name).is_some_and(|e| e.set_password(value).is_ok())
}

pub fn delete(name: &str) {
    if let Some(e) = entry(SERVICE, name) {
        let _ = e.delete_credential();
    }
}
