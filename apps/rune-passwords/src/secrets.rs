//! The session's refresh token in the system keyring (the Secret Service on Linux: GNOME
//! Keyring, KWallet). Without a keyring it stays in a file readable only by you.

pub const SERVICE: &str = "rune-passwords";

/// Whether the keyring can be used. Tests and `RUNE_PASSWORDS_NO_KEYRING` never touch it.
pub fn available() -> bool {
    if cfg!(test) || std::env::var_os("RUNE_PASSWORDS_NO_KEYRING").is_some() {
        return false;
    }
    keyring::Entry::store_status().is_ok()
}

fn entry(service: &str, name: &str) -> Option<keyring::Entry> {
    available().then(|| keyring::Entry::new(service, name).ok()).flatten()
}

pub fn get(name: &str) -> Option<String> {
    entry(SERVICE, name)?.get_password().ok().filter(|s| !s.is_empty())
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
