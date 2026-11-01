//! The signed-in accounts, kept between runs: where each one's server is, how its key is
//! derived, its user key as the server keeps it (encrypted with the master key), and its last
//! sync, which holds nothing but ciphertext. So a vault opens offline, and only its master
//! password opens it. Each session's refresh token goes in the keyring.
//!
//! On disk: `accounts/<id>.json` in the config folder for each account, `active` naming the
//! one in use, and `vaults/<id>.json` in the data folder for each one's last sync.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::Server;
use crate::crypto::Kdf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// This app's name for the account, naming its files.
    #[serde(default)]
    pub id: String,
    pub server: Server,
    pub email: String,
    pub kdf: Kdf,
    /// What the key is salted with: the address, or what the server says.
    pub salt: String,
    /// This install's device id, which the server knows it by.
    pub device: String,
    /// The user key, encrypted with the stretched master key.
    pub protected_key: String,
    /// The refresh token, here only when there's no keyring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<String>,
}

impl Account {
    /// The server's host, to tell apart one address on two servers.
    pub fn host(&self) -> String {
        url::Url::parse(&self.server.0).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| self.server.0.clone())
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-passwords")
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-passwords")
}

fn accounts_dir() -> PathBuf {
    config_dir().join("accounts")
}

fn account_path(id: &str) -> PathBuf {
    accounts_dir().join(format!("{id}.json"))
}

fn active_path() -> PathBuf {
    config_dir().join("active")
}

fn sync_path(id: &str) -> PathBuf {
    data_dir().join("vaults").join(format!("{id}.json"))
}

fn refresh_name(id: &str) -> String {
    format!("refresh-token:{id}")
}

/// An id is a file name: nothing that could reach outside its folder.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Writes `text` readable only by you, whole or not at all.
fn write_private(path: &PathBuf, text: &str) -> std::io::Result<()> {
    let dir = path.parent().expect("a file in a folder");
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&tmp)?, text.as_bytes())?;
    std::fs::rename(tmp, path)
}

/// Moves the one account of earlier versions (`account.json`, `vault.json`, the keyring's
/// `refresh-token`) to where accounts are kept now.
fn migrate() {
    let old = config_dir().join("account.json");
    let Ok(text) = std::fs::read_to_string(&old) else { return };
    let Ok(mut a) = serde_json::from_str::<Account>(&text) else { return };
    a.id = new_id();
    if a.refresh.is_none() {
        a.refresh = crate::secrets::get("refresh-token");
    }
    if save(&a).is_err() {
        return;
    }
    let old_sync = data_dir().join("vault.json");
    if let Ok(sync) = std::fs::read_to_string(&old_sync) {
        let _ = write_private(&sync_path(&a.id), &sync);
    }
    let _ = set_active(&a.id);
    crate::secrets::delete("refresh-token");
    let _ = std::fs::remove_file(old);
    let _ = std::fs::remove_file(old_sync);
}

pub fn load(id: &str) -> Option<Account> {
    if !valid_id(id) {
        return None;
    }
    let mut a: Account = serde_json::from_str(&std::fs::read_to_string(account_path(id)).ok()?).ok()?;
    a.id = id.to_owned();
    if a.refresh.is_none() {
        a.refresh = crate::secrets::get(&refresh_name(id));
    }
    Some(a)
}

/// Every account kept, by address.
pub fn list() -> Vec<Account> {
    migrate();
    let Ok(dir) = std::fs::read_dir(accounts_dir()) else { return Vec::new() };
    let mut all: Vec<Account> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| e.path().file_name()?.to_str()?.strip_suffix(".json").map(str::to_owned))
        .filter_map(|id| load(&id))
        .collect();
    all.sort_by(|a, b| (a.email.as_str(), a.server.0.as_str()).cmp(&(b.email.as_str(), b.server.0.as_str())));
    all
}

/// The account in use: the one last chosen, or else the first kept.
pub fn active() -> Option<Account> {
    migrate();
    std::fs::read_to_string(active_path())
        .ok()
        .and_then(|id| load(id.trim()))
        .or_else(|| list().into_iter().next())
}

pub fn set_active(id: &str) -> std::io::Result<()> {
    write_private(&active_path(), id)
}

pub fn save(account: &Account) -> std::io::Result<()> {
    if !valid_id(&account.id) {
        return Err(std::io::Error::other("the account has no id"));
    }
    let mut kept = account.clone();
    if let Some(token) = &account.refresh
        && crate::secrets::set(&refresh_name(&account.id), token)
    {
        kept.refresh = None;
    }
    write_private(&account_path(&account.id), &serde_json::to_string_pretty(&kept)?)
}

/// An account's last sync, as the server sent it.
pub fn cached_sync(id: &str) -> Option<Value> {
    if !valid_id(id) {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(sync_path(id)).ok()?).ok()
}

pub fn cache_sync(id: &str, sync: &Value) {
    if valid_id(id) {
        let _ = write_private(&sync_path(id), &sync.to_string());
    }
}

/// Signs an account out: it, its token and its cached vault go.
pub fn forget(id: &str) {
    if !valid_id(id) {
        return;
    }
    crate::secrets::delete(&refresh_name(id));
    let _ = std::fs::remove_file(account_path(id));
    let _ = std::fs::remove_file(sync_path(id));
    if std::fs::read_to_string(active_path()).is_ok_and(|a| a.trim() == id) {
        let _ = std::fs::remove_file(active_path());
    }
}

#[cfg(test)]
pub fn use_scratch_dirs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("rune-passwords-test-{}", std::process::id()));
        // SAFETY: set once, before any test thread reads the environment through `dirs`.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            std::env::set_var("XDG_DATA_HOME", &dir);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: &str) -> Account {
        Account {
            id: new_id(),
            server: Server("https://vault.example.com".into()),
            email: email.into(),
            kdf: Kdf::Pbkdf2 { iterations: 600_000 },
            salt: email.into(),
            device: "d".into(),
            protected_key: "2.x|y|z".into(),
            refresh: Some("tok".into()),
        }
    }

    #[test]
    fn keeps_accounts_private_and_forgets_each() {
        use_scratch_dirs();
        let a = account("keeps-a@b.c");
        let b = account("keeps-b@b.c");
        save(&a).unwrap();
        save(&b).unwrap();
        assert_eq!(load(&a.id).unwrap(), a, "no keyring in tests: the token stays in the file");
        assert!(list().iter().any(|x| x.id == b.id));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(account_path(&a.id)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        set_active(&b.id).unwrap();
        assert_eq!(active().unwrap().id, b.id);
        cache_sync(&a.id, &serde_json::json!({ "ciphers": [] }));
        assert!(cached_sync(&a.id).is_some() && cached_sync(&b.id).is_none(), "each account its own sync");
        forget(&a.id);
        assert!(load(&a.id).is_none() && cached_sync(&a.id).is_none());
        assert!(load(&b.id).is_some(), "the other account stays");
        forget(&b.id);
        assert!(load("../account").is_none(), "an id can't leave its folder");
    }
}
