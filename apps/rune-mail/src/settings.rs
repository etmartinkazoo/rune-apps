//! Preferences, saved as JSON next to the token.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// What swiping a row all the way left does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum SwipeLeft {
    #[default]
    Archive,
    Delete,
}

/// Local storage: off by default; on, it keeps the newest mail up to `budget_mb`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Storage {
    pub on: bool,
    /// The most the store may use, in MB; 0 is no limit.
    pub budget_mb: u64,
    /// Keep remote images of messages you've loaded them for (never fetched ahead).
    pub remote_images: bool,
    /// Download ahead on a metered connection (a phone hotspot) too.
    pub metered: bool,
}

impl Default for Storage {
    fn default() -> Self {
        Storage { on: false, budget_mb: 1024, remote_images: false, metered: false }
    }
}

impl Storage {
    pub fn budget_bytes(&self) -> Option<u64> {
        (self.budget_mb > 0).then_some(self.budget_mb.saturating_mul(1024 * 1024))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub vim: bool,
    pub render_html: bool,
    pub remote_images: bool,
    pub mark_read_on_open: bool,
    /// Minutes between checks for new mail; 0 checks only on request.
    pub refresh_minutes: u32,
    pub preview_lines: usize,
    pub appearance: Appearance,
    /// Group the list into conversations.
    pub conversations: bool,
    /// Show a conversation's newest message at the top of the reader.
    pub newest_first: bool,
    /// Sidebar sections folded away: "favorites", or an account's address.
    pub collapsed: Vec<String>,
    pub swipe_left: SwipeLeft,
    /// Mail kept on this computer (`store.rs`).
    pub storage: Storage,
    /// The old on/off for the store, read once to carry a choice over; never written.
    #[serde(skip_serializing)]
    keep_local: Option<bool>,
    /// What Copy for AI puts before the messages.
    pub ai_prompt: String,
    /// Seconds a sent message waits, so it can be undone (0: sends at once).
    pub undo_send_secs: u32,
    /// Why the file couldn't be read at launch. These are the defaults then, which say
    /// storage is off, so the store isn't emptied on their word.
    #[serde(skip)]
    pub unreadable: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            vim: true,
            render_html: true,
            remote_images: false,
            mark_read_on_open: true,
            refresh_minutes: 1,
            preview_lines: 2,
            appearance: Appearance::System,
            conversations: true,
            newest_first: true,
            collapsed: Vec::new(),
            swipe_left: SwipeLeft::Archive,
            storage: Storage::default(),
            keep_local: None,
            ai_prompt: crate::ai::DEFAULT_PROMPT.to_owned(),
            undo_send_secs: 5,
            unreadable: None,
        }
    }
}

fn path() -> PathBuf {
    crate::worker::config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        let path = path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => return Settings { unreadable: Some(e.to_string()), ..Settings::default() },
        };
        let mut s: Settings = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => {
                // Kept aside: the next save writes the defaults over it.
                let aside = path.with_extension("json.unreadable");
                let _ = std::fs::copy(&path, &aside);
                let why = format!("{e}; the file was kept as {}", aside.display());
                return Settings { unreadable: Some(why), ..Settings::default() };
            }
        };
        s.migrate(&raw);
        s
    }

    /// Before storage had its own settings, `keep_local` said whether it was on: carry that over.
    fn migrate(&mut self, raw: &str) {
        let keep = self.keep_local.take();
        if !raw.contains("\"storage\"") && let Some(on) = keep {
            self.storage.on = on;
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        // Whole or not at all: a write cut short would leave a file that doesn't read.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn dark(&self, system_dark: bool) -> bool {
        match self.appearance {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_starts_off_and_keep_local_carries_over() {
        assert!(!Settings::default().storage.on);
        assert_eq!(Settings::default().storage.budget_mb, 1024);
        let from = |json: &str| {
            let mut s: Settings = serde_json::from_str(json).unwrap();
            s.migrate(json);
            s
        };
        assert!(from(r#"{"keep_local": true}"#).storage.on, "a saved yes is kept");
        assert!(!from(r#"{"keep_local": false}"#).storage.on);
        assert!(!from("{}").storage.on);
        assert!(!from(r#"{"keep_local": true, "storage": {"on": false}}"#).storage.on, "the new setting wins");
    }

    #[test]
    fn a_settings_file_that_wont_read_is_kept_and_said() {
        let _one = crate::demo::test_app();
        std::fs::create_dir_all(path().parent().unwrap()).unwrap();
        std::fs::write(path(), "{ cut short").unwrap();
        let s = Settings::load();
        assert!(s.unreadable.is_some());
        assert_eq!(std::fs::read_to_string(path().with_extension("json.unreadable")).unwrap(), "{ cut short");
        Settings::default().save().unwrap();
        assert!(Settings::load().unreadable.is_none());
        assert_eq!(Storage { budget_mb: u64::MAX, ..Storage::default() }.budget_bytes(), Some(u64::MAX));
    }
}
