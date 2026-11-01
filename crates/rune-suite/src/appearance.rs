use serde::{Deserialize, Serialize};

/// Light, dark, or as the system is: every app's first setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn dark(self, system_dark: bool) -> bool {
        match self {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// For a segmented control.
    pub const OPTIONS: [(&'static str, Appearance); 3] =
        [("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)];
}
