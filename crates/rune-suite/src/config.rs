//! Where the apps keep their files, and reading and writing them as JSON.
//!
//! Each app has `~/.config/rune-<app>/`; what the apps share (accounts, contacts) lives in
//! `~/.config/rune/` and `~/.local/share/rune/`. Inside the Rune Flatpak every app shares one
//! sandbox, so these resolve inside it and are still shared.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// `~/.config/<app>`, e.g. `dir("rune-mail")`.
pub fn dir(app: &str) -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(app)
}

/// `~/.local/share/<app>`.
pub fn data_dir(app: &str) -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(app)
}

/// `~/.config/rune`: what every app reads.
pub fn shared_dir() -> PathBuf {
    dir("rune")
}

/// `~/.local/share/rune`.
pub fn shared_data_dir() -> PathBuf {
    data_dir("rune")
}

/// The file's value, or the default when it's missing or unreadable.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    try_load(path).unwrap_or_default()
}

/// The file's value; None when it's missing, Err when it's there but won't parse.
pub fn try_load<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Writes `value` as pretty JSON through a temporary file, so a crash never leaves half a file.
pub fn save<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    write(path, value, false)
}

/// `save`, readable only by you: for files that may hold secrets.
pub fn save_private<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    write(path, value, true)
}

fn write<T: Serialize + ?Sized>(path: &Path, value: &T, private: bool) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    std::fs::rename(tmp, path)
}

/// A path as typed: `~` for home, else as is.
pub fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            dirs::home_dir().unwrap_or_default().join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(path),
    }
}

/// Points `dirs` at a scratch folder for this test process, so tests never touch your files.
/// Call at the start of any test that reads or writes settings.
pub fn use_scratch_dirs(app: &str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let app = app.to_owned();
    ONCE.call_once(move || {
        let dir = std::env::temp_dir().join(format!("{app}-test-{}", std::process::id()));
        // SAFETY: set once, before any test thread reads the environment through `dirs`.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
            std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_atomically_and_privately() {
        let dir = std::env::temp_dir().join(format!("rune-suite-config-{}", std::process::id()));
        let path = dir.join("a").join("x.json");
        save_private(&path, &vec![1, 2, 3]).unwrap();
        assert_eq!(load::<Vec<i32>>(&path), vec![1, 2, 3]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(load::<Vec<i32>>(&dir.join("missing.json")), Vec::<i32>::new());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn expands_home() {
        let home = dirs::home_dir().unwrap_or_default();
        assert_eq!(expand("~/Notes"), home.join("Notes"));
        assert_eq!(expand(" /tmp/x "), PathBuf::from("/tmp/x"));
        assert_eq!(expand("~other"), PathBuf::from("~other"));
    }
}
