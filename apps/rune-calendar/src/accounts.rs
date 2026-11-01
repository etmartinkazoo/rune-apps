//! Saved accounts, in `~/.config/rune-calendar/accounts.json`; passwords in the keyring.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::caldav::{CalDav, Provider};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub provider: Option<Provider>,
    /// The sign-in name, usually the address.
    pub user: String,
    /// The CalDAV address, for `Provider::Other`.
    pub server: String,
    /// What the sidebar calls it; empty shows the address.
    pub label: String,
    /// Calendars (hrefs) switched off in the sidebar.
    pub hidden: Vec<String>,
    /// Kept here only when there's no keyring.
    pub password: String,
    pub in_keyring: bool,
}

impl Account {
    pub fn new(provider: Provider, user: &str, password: &str, server: &str) -> Account {
        Account { provider: Some(provider), user: user.trim().to_owned(), server: server.trim().to_owned(), password: password.to_owned(), ..Account::default() }
    }

    /// Unique per account: the provider and sign-in.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider().name(), self.user)
    }

    pub fn provider(&self) -> Provider {
        self.provider.unwrap_or(Provider::Fastmail)
    }

    pub fn name(&self) -> &str {
        if self.label.trim().is_empty() { &self.user } else { &self.label }
    }

    pub fn client(&self) -> CalDav {
        CalDav::new(self.provider(), &self.user, &self.password, &self.server)
    }

    fn secret(&self) -> String {
        format!("{}/password", self.id())
    }

    pub fn forget_secret(&self) {
        crate::secrets::delete(&self.secret());
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("rune-calendar")
}

fn path() -> PathBuf {
    config_dir().join("accounts.json")
}

pub fn load() -> Vec<Account> {
    let mut list: Vec<Account> =
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for a in &mut list {
        if a.in_keyring
            && let Some(p) = crate::secrets::get(&a.secret())
        {
            a.password = p;
        }
    }
    list
}

/// Saves the list, each password in the keyring where there is one.
pub fn save(list: &[Account]) -> std::io::Result<()> {
    let stripped: Vec<Account> = list
        .iter()
        .map(|a| {
            let mut a = a.clone();
            if crate::secrets::set(&a.secret(), &a.password) {
                a.password.clear();
                a.in_keyring = true;
            } else {
                a.in_keyring = false;
            }
            a
        })
        .collect();
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(&stripped)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The calendars Rune Mail can already open: Gmail and Fastmail accounts added with an app
/// password, and Fastmail accounts given a calendar app password.
pub fn from_rune_mail() -> Vec<Account> {
    let path = dirs::config_dir().unwrap_or_default().join("rune-mail").join("accounts.json");
    let Some(raw) = std::fs::read_to_string(path).ok() else { return Vec::new() };
    let saved: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    import(&saved, &|name| crate::secrets::get_from(crate::secrets::MAIL, name))
}

fn import(saved: &[serde_json::Value], secret: &dyn Fn(&str) -> Option<String>) -> Vec<Account> {
    let str_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned();
    let mut out = Vec::new();
    for s in saved {
        let username = str_of(s, "username");
        let label = str_of(s, "label");
        let (provider, user, password) = match s.get("imap").filter(|v| v.is_object()) {
            Some(imap) => {
                let host = str_of(imap, "imap_host").to_ascii_lowercase();
                let provider = if imap.get("gmail").and_then(|g| g.as_bool()) == Some(true) || host.ends_with("gmail.com") {
                    Provider::Google
                } else if host.ends_with("fastmail.com") || host.ends_with("messagingengine.com") {
                    Provider::Fastmail
                } else {
                    continue;
                };
                if s.get("calendar_off").and_then(|c| c.as_bool()) == Some(true) {
                    continue;
                }
                let password = Some(str_of(imap, "password")).filter(|p| !p.is_empty()).or_else(|| secret(&format!("{username}/password")));
                (provider, str_of(imap, "login"), password)
            }
            None => {
                let password = s.get("calendar_password").and_then(|p| p.as_str()).map(str::to_owned)
                    .or_else(|| secret(&format!("{username}/calendar")));
                (Provider::Fastmail, username.clone(), password)
            }
        };
        let Some(password) = password.filter(|p| !p.is_empty()) else { continue };
        if user.is_empty() {
            continue;
        }
        let mut a = Account::new(provider, &user, &password, "");
        a.label = label;
        out.push(a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_what_rune_mail_can_open() {
        let saved: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"username": "me@gmail.com", "label": "Lumin", "in_keyring": true,
             "imap": {"gmail": true, "login": "me@gmail.com", "password": "", "imap_host": "imap.gmail.com"}},
            {"username": "ted@fastmail.com", "token": "", "calendar_password": "cal-pass"},
            {"username": "jmap@fastmail.com", "token": ""},
            {"username": "a@other.org", "imap": {"login": "a@other.org", "password": "x", "imap_host": "mail.other.org"}},
            {"username": "off@gmail.com", "calendar_off": true,
             "imap": {"gmail": true, "login": "off@gmail.com", "password": "p", "imap_host": "imap.gmail.com"}}
        ]"#).unwrap();
        let keyring = |name: &str| (name == "me@gmail.com/password").then(|| "app pass".to_owned());
        let got = import(&saved, &keyring);
        assert_eq!(got.len(), 2, "no password, another provider and a switched-off calendar are skipped");
        assert_eq!((got[0].provider(), got[0].user.as_str(), got[0].label.as_str()), (Provider::Google, "me@gmail.com", "Lumin"));
        assert_eq!(got[0].password, "app pass");
        assert_eq!((got[1].provider(), got[1].password.as_str()), (Provider::Fastmail, "cal-pass"));
    }
}
