//! Preferences, in `~/.config/rune-calendar/settings.json`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum View {
    Day,
    #[default]
    Week,
    Month,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub view: View,
    pub week_start: WeekStart,
    pub appearance: Appearance,
    /// The hour Day and Week open scrolled to.
    pub day_starts: u32,
    /// Minutes between checks for changes; 0 checks only on request.
    pub refresh_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { view: View::Week, week_start: WeekStart::Monday, appearance: Appearance::System, day_starts: 8, refresh_minutes: 5 }
    }
}

fn path() -> std::path::PathBuf {
    crate::accounts::config_dir().join("settings.json")
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

    pub fn dark(&self, system_dark: bool) -> bool {
        match self.appearance {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }
}
