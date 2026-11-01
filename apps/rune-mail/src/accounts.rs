//! Saved accounts: a Fastmail API token (and optionally a calendar app password), or the
//! servers and password of an IMAP/SMTP account such as Gmail. Kept in
//! `~/.config/rune-mail/accounts.json`, readable only by you.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// A Fastmail API token; empty for IMAP accounts.
    #[serde(default)]
    pub token: String,
    /// Filled in once the token connects, to match an account to its entry.
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub calendar_password: Option<String>,
    /// Set for IMAP/SMTP accounts (Gmail included) instead of `token`.
    #[serde(default)]
    pub imap: Option<ImapConfig>,
    /// The name shown in the sidebar, instead of the address.
    #[serde(default)]
    pub label: Option<String>,
    /// The name on mail it sends, unlike `label` (which only you see). None uses the IMAP
    /// sign-in's name, or the Fastmail identity's.
    #[serde(default)]
    pub from_name: Option<String>,
    /// An IMAP account on Fastmail or Gmail shows its calendar unless this is set.
    #[serde(default)]
    pub calendar_off: bool,
    /// Its token and passwords are in the system keyring, not in this file.
    #[serde(default)]
    pub in_keyring: bool,
    /// Its secrets are in the keyring, which wouldn't give them at launch (locked, or not
    /// answering). It's saved back as it was, and doesn't try to sign in without them.
    #[serde(skip)]
    pub locked: bool,
}

/// Leaves the secrets out, so a stray `{:?}` can't print them.
impl std::fmt::Debug for Saved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Saved")
            .field("username", &self.username)
            .field("label", &self.label)
            .field("imap", &self.imap)
            .field("in_keyring", &self.in_keyring)
            .field("locked", &self.locked)
            .finish_non_exhaustive()
    }
}

impl Saved {
    /// The name its sent mail carries, when one is set here.
    pub fn sender_name(&self) -> Option<String> {
        self.from_name.clone().or_else(|| self.imap.as_ref().and_then(|c| c.name.clone())).filter(|n| !n.trim().is_empty())
    }

    fn key(&self, what: &str) -> Option<String> {
        self.username.as_ref().map(|u| format!("{u}/{what}"))
    }

    /// Moves the secrets into the keyring; true if they're all there now.
    fn stash(&self) -> bool {
        let put = |what: &str, value: &str| value.is_empty() || self.key(what).is_some_and(|k| crate::secrets::set(&k, value));
        self.username.is_some()
            && put("token", &self.token)
            && put("password", self.imap.as_ref().map_or("", |c| c.password.as_str()))
            && put("calendar", self.calendar_password.as_deref().unwrap_or(""))
    }

    /// Fills the secrets back in from the keyring, or marks it `locked` when it can't.
    fn unstash(&mut self) {
        let Some(user) = self.username.clone() else { return };
        if !crate::secrets::available() {
            self.locked = true;
            return;
        }
        let get = |what: &str| crate::secrets::get(&format!("{user}/{what}"));
        let (Ok(token), Ok(password), Ok(calendar)) = (get("token"), get("password"), get("calendar")) else {
            self.locked = true;
            return;
        };
        if let Some(t) = token {
            self.token = t;
        }
        if let Some(p) = password
            && let Some(cfg) = self.imap.as_mut()
        {
            cfg.password = p;
        }
        if let Some(c) = calendar {
            self.calendar_password = Some(c);
        }
    }

    /// This entry as written to the file: without secrets once they're in the keyring.
    fn stripped(&self) -> Saved {
        let mut s = self.clone();
        s.in_keyring = true;
        s.token.clear();
        s.calendar_password = None;
        if let Some(cfg) = s.imap.as_mut() {
            cfg.password.clear();
        }
        s
    }

    /// Removes this account's secrets from the keyring.
    pub fn forget_secrets(&self) {
        for what in ["token", "password", "calendar"] {
            if let Some(k) = self.key(what) {
                crate::secrets::delete(&k);
            }
        }
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImapConfig {
    pub gmail: bool,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    /// The login name, usually the address.
    pub login: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// STARTTLS on `smtp_port` (usually 587) rather than TLS from the start (465).
    pub smtp_starttls: bool,
    /// Append sent mail to the Sent mailbox (Gmail files it itself).
    pub save_sent: bool,
}

impl std::fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConfig")
            .field("email", &self.email)
            .field("imap_host", &self.imap_host)
            .field("imap_port", &self.imap_port)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .finish_non_exhaustive()
    }
}

impl ImapConfig {
    /// The calendar its app password also opens, for Gmail and Fastmail accounts.
    pub fn calendar(&self) -> Option<crate::caldav::Provider> {
        let host = self.imap_host.to_ascii_lowercase();
        if self.gmail || host.ends_with("gmail.com") {
            Some(crate::caldav::Provider::Google)
        } else if host.ends_with("fastmail.com") || host.ends_with("messagingengine.com") {
            Some(crate::caldav::Provider::Fastmail)
        } else {
            None
        }
    }

    pub fn gmail(email: &str, name: Option<String>, app_password: &str) -> ImapConfig {
        ImapConfig {
            gmail: true,
            email: email.to_owned(),
            name,
            login: email.to_owned(),
            // Google shows app passwords in groups of four; the spaces aren't part of it.
            password: app_password.split_whitespace().collect(),
            imap_host: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 465,
            smtp_starttls: false,
            save_sent: false,
        }
    }
}

fn path() -> PathBuf {
    crate::worker::config_dir().join("accounts.json")
}

#[cfg(test)]
pub fn path_for_tests() -> PathBuf {
    path()
}

fn legacy_token_path() -> PathBuf {
    crate::worker::config_dir().join("token")
}

/// Why the accounts file couldn't be read at launch, for the sign-in screen.
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);

pub fn load_problem() -> Option<String> {
    PROBLEM.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The accounts in the file. One that's there but won't read is copied aside first, since
/// the next save would replace it with the accounts added since.
fn read() -> Vec<Saved> {
    let path = path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return unreadable(&path, &e.to_string()),
    };
    serde_json::from_str(&text).unwrap_or_else(|e| unreadable(&path, &e.to_string()))
}

fn unreadable(path: &std::path::Path, why: &str) -> Vec<Saved> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let aside = path.with_extension(format!("json.unreadable-{stamp}"));
    let kept = std::fs::copy(path, &aside).is_ok();
    let mut text = format!("Your saved accounts couldn't be read ({why}).");
    if kept {
        text.push_str(&format!(" The file was kept as {}.", aside.display()));
    }
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
    Vec::new()
}

/// Saved accounts, moving a token from the old single-account file in, plus `FASTMAIL_TOKEN`
/// (never saved).
pub fn load() -> Vec<Saved> {
    let mut list = read();
    for s in list.iter_mut().filter(|s| s.in_keyring) {
        s.unstash();
    }
    if let Ok(t) = std::fs::read_to_string(legacy_token_path()) {
        let token = t.trim().to_owned();
        if !token.is_empty() && !list.iter().any(|s| s.token == token) {
            list.push(Saved { token, ..Saved::default() });
        }
        if save(&list).is_ok() {
            let _ = std::fs::remove_file(legacy_token_path());
        }
    }
    if let Ok(t) = std::env::var("FASTMAIL_TOKEN") {
        let token = t.trim().to_owned();
        if !token.is_empty() && !list.iter().any(|s| s.token == token) {
            list.push(Saved { token, ..Saved::default() });
        }
    }
    list
}

pub fn save(list: &[Saved]) -> std::io::Result<()> {
    let env = std::env::var("FASTMAIL_TOKEN").ok().map(|t| t.trim().to_owned());
    let keyring = crate::secrets::available();
    let keep: Vec<Saved> = list
        .iter()
        .filter(|s| s.imap.is_some() || Some(&s.token) != env.as_ref())
        .map(|s| {
            if s.locked || (keyring && s.stash()) {
                s.stripped()
            } else {
                Saved { in_keyring: false, ..s.clone() }
            }
        })
        .collect();
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    // Made readable only by you before anything is written, since it can hold passwords.
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new();
    file.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut file, 0o600);
    std::io::Write::write_all(&mut file.open(&tmp)?, serde_json::to_string_pretty(&keep)?.as_bytes())?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caldav::Provider;

    #[test]
    fn app_passwords_open_the_calendars_of_gmail_and_fastmail() {
        assert_eq!(ImapConfig::gmail("me@gmail.com", None, "abcd efgh ijkl mnop").calendar(), Some(Provider::Google));
        let fastmail = ImapConfig { imap_host: "imap.fastmail.com".into(), ..ImapConfig::default() };
        assert_eq!(fastmail.calendar(), Some(Provider::Fastmail));
        let other = ImapConfig { imap_host: "mail.example.org".into(), ..ImapConfig::default() };
        assert_eq!(other.calendar(), None);
    }

    #[test]
    fn debug_leaves_secrets_out() {
        let s = Saved {
            token: "fmu1-secret".into(),
            calendar_password: Some("calsecret".into()),
            imap: Some(ImapConfig::gmail("me@gmail.com", None, "pass word")),
            ..Saved::default()
        };
        let shown = format!("{s:?}");
        assert!(!shown.contains("secret") && !shown.contains("password"), "{shown}");
    }

    #[test]
    fn an_imap_account_missing_newer_fields_still_loads() {
        let old: Vec<Saved> =
            serde_json::from_str(r#"[{"username":"a@b.c","imap":{"gmail":true,"email":"a@b.c","imap_host":"imap.gmail.com"}}]"#)
                .unwrap();
        assert_eq!(old[0].imap.as_ref().unwrap().imap_host, "imap.gmail.com");
    }

    #[test]
    fn older_saved_accounts_still_load() {
        let old: Vec<Saved> = serde_json::from_str(r#"[{"token":"fmu1-x","username":"a@b.c"}]"#).unwrap();
        assert_eq!(old[0].label, None);
        assert!(!old[0].calendar_off);
    }
}
