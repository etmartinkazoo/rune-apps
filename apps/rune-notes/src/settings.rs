//! Preferences, in `~/.config/rune-notes/settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The folder notes live in; empty is `~/Notes`.
    pub root: String,
    pub appearance: Appearance,
    /// Folders folded shut in the sidebar, by path.
    pub collapsed: Vec<PathBuf>,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-notes")
}

fn path() -> PathBuf {
    config_dir().join("settings.json")
}

/// `~/Notes`.
pub fn default_root() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("Notes")
}

/// A path as typed: `~` for home, else as is.
pub fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            dirs::home_dir().unwrap_or_default().join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(path),
    }
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_string_pretty(self)?)
    }

    pub fn root(&self) -> PathBuf {
        if self.root.trim().is_empty() { default_root() } else { expand(&self.root) }
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
    fn the_root_defaults_to_notes_in_home_and_expands_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(Settings::default().root(), home.join("Notes"));
        assert_eq!(expand("~/Dropbox/Notes"), home.join("Dropbox/Notes"));
        assert_eq!(expand("~"), home);
        assert_eq!(expand(" /srv/notes "), PathBuf::from("/srv/notes"));
        assert_eq!(expand("~bob/x"), PathBuf::from("~bob/x"));
    }
}
