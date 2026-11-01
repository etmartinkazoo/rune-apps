//! Secrets in the system keyring (the Secret Service on Linux: GNOME Keyring, KWallet), so
//! settings files hold only settings. Where there's no keyring, apps keep them in a file
//! readable only by you.

/// One app's entries in the keyring.
///
/// Tests, and `<SERVICE>_NO_KEYRING` in the environment (`RUNE_MAIL_NO_KEYRING` for
/// `rune-mail`), never touch it.
#[derive(Clone, Copy, Debug)]
pub struct Keyring {
    pub service: &'static str,
}

impl Keyring {
    pub const fn new(service: &'static str) -> Keyring {
        Keyring { service }
    }

    fn off_var(&self) -> String {
        format!("{}_NO_KEYRING", self.service.to_ascii_uppercase().replace('-', "_"))
    }

    /// Whether the keyring can be used.
    pub fn available(&self) -> bool {
        if cfg!(test) || std::env::var_os(self.off_var()).is_some() || std::env::var_os("RUNE_NO_KEYRING").is_some() {
            return false;
        }
        keyring::Entry::store_status().is_ok()
    }

    fn entry(&self, name: &str) -> Option<keyring::Entry> {
        self.available().then(|| keyring::Entry::new(self.service, name).ok()).flatten()
    }

    /// A secret, or None when there's none (or no keyring). Err when the keyring is there but
    /// won't say: locked, or not answering. That isn't "none", so callers mustn't replace it.
    pub fn get(&self, name: &str) -> Result<Option<String>, keyring::Error> {
        if !self.available() {
            return Ok(None);
        }
        match keyring::Entry::new(self.service, name)?.get_password() {
            Ok(s) => Ok(Some(s).filter(|s| !s.is_empty())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `get`, taking a locked keyring as none.
    pub fn find(&self, name: &str) -> Option<String> {
        self.get(name).ok().flatten()
    }

    /// Stores a secret; false if the keyring couldn't take it.
    pub fn set(&self, name: &str, value: &str) -> bool {
        self.entry(name).is_some_and(|e| e.set_password(value).is_ok())
    }

    pub fn delete(&self, name: &str) {
        if let Some(e) = self.entry(name) {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_never_touch_the_keyring() {
        let k = Keyring::new("rune-test");
        assert!(!k.available());
        assert_eq!(k.find("x"), None);
        assert!(!k.set("x", "y"));
        assert_eq!(k.off_var(), "RUNE_TEST_NO_KEYRING");
    }
}
