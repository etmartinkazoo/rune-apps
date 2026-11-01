//! Tokens and passwords in the system keyring (the Secret Service on Linux: GNOME Keyring,
//! KWallet), so `accounts.json` holds only settings. Where there's no keyring, they stay in
//! that file, readable only by you, as before.

const SERVICE: &str = "rune-mail";

/// Whether the keyring can be used. Tests and `RUNE_MAIL_NO_KEYRING` never touch it.
pub fn available() -> bool {
    if cfg!(test) || std::env::var_os("RUNE_MAIL_NO_KEYRING").is_some() {
        return false;
    }
    keyring::Entry::store_status().is_ok()
}

fn entry(name: &str) -> Option<keyring::Entry> {
    available().then(|| keyring::Entry::new(SERVICE, name).ok()).flatten()
}

/// A secret, or None when there's none (or no keyring). Err when the keyring is there but
/// won't say: locked, or not answering. That isn't "none", so callers mustn't replace it.
pub fn get(name: &str) -> Result<Option<String>, keyring::Error> {
    if !available() {
        return Ok(None);
    }
    match keyring::Entry::new(SERVICE, name)?.get_password() {
        Ok(s) => Ok(Some(s).filter(|s| !s.is_empty())),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Stores a secret; false if the keyring couldn't take it.
pub fn set(name: &str, value: &str) -> bool {
    entry(name).is_some_and(|e| e.set_password(value).is_ok())
}

pub fn delete(name: &str) {
    if let Some(e) = entry(name) {
        let _ = e.delete_credential();
    }
}
