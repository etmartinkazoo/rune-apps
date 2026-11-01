//! The accounts every app can use, like macOS's Internet Accounts: sign in to Fastmail,
//! Google or iCloud once, with an app password, and Mail, Calendar and Contacts all have it.
//!
//! The list is `~/.config/rune/accounts.json` (readable only by you); each password is in the
//! keyring under `rune-accounts`, or in that file where there's no keyring. An app adds an
//! account with [`upsert`] when you sign in there, and reads [`load`] for the ones it can use.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Keyring;

const KEYRING: Keyring = Keyring::new("rune-accounts");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Fastmail,
    Google,
    ICloud,
    /// Servers at addresses you give.
    Other,
}

impl Provider {
    pub const ALL: [Provider; 4] = [Provider::Fastmail, Provider::Google, Provider::ICloud, Provider::Other];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Fastmail => "Fastmail",
            Provider::Google => "Google",
            Provider::ICloud => "iCloud",
            Provider::Other => "Other",
        }
    }

    /// Where to make an app password for it.
    pub fn app_password_url(self) -> Option<&'static str> {
        match self {
            Provider::Fastmail => Some("https://app.fastmail.com/settings/security/apps"),
            Provider::Google => Some("https://myaccount.google.com/apppasswords"),
            Provider::ICloud => Some("https://account.apple.com/account/manage"),
            Provider::Other => None,
        }
    }

    /// Which provider serves this address's mail, from its domain.
    pub fn of_email(email: &str) -> Option<Provider> {
        let domain = email.rsplit('@').next()?.to_ascii_lowercase();
        match domain.as_str() {
            "gmail.com" | "googlemail.com" => Some(Provider::Google),
            "icloud.com" | "me.com" | "mac.com" => Some(Provider::ICloud),
            d if d.ends_with("fastmail.com") || d.ends_with("fastmail.fm") || d.ends_with("messagingengine.com") => Some(Provider::Fastmail),
            _ => None,
        }
    }

    /// Which provider an IMAP host belongs to.
    pub fn of_imap_host(host: &str) -> Option<Provider> {
        let host = host.to_ascii_lowercase();
        if host.ends_with("gmail.com") || host.ends_with("googlemail.com") {
            Some(Provider::Google)
        } else if host.ends_with("fastmail.com") || host.ends_with("messagingengine.com") {
            Some(Provider::Fastmail)
        } else if host.ends_with("mail.me.com") || host.ends_with("icloud.com") {
            Some(Provider::ICloud)
        } else {
            None
        }
    }
}

/// What an account is used for. Each app shows only the accounts switched on for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Services {
    pub mail: bool,
    pub calendar: bool,
    pub contacts: bool,
}

impl Default for Services {
    fn default() -> Self {
        Services { mail: true, calendar: true, contacts: true }
    }
}

/// Which app is asking: what it can use an account for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Mail,
    Calendar,
    Contacts,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub provider: Provider,
    /// The sign-in, usually the address.
    pub user: String,
    /// What lists call it; empty shows `user`.
    pub label: String,
    /// The name on mail it sends.
    pub full_name: String,
    /// For `Provider::Other`: where its calendars and contacts are (any address on the
    /// server will do; the rest is found from it).
    pub caldav: String,
    pub carddav: String,
    pub services: Services,
    /// The app password. In the file only when there's no keyring.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
}

/// Leaves the password out, so a stray `{:?}` can't print it.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account").field("provider", &self.provider).field("user", &self.user).field("services", &self.services).finish_non_exhaustive()
    }
}

impl Account {
    pub fn new(provider: Provider, user: &str, password: &str) -> Account {
        Account {
            provider,
            user: user.trim().to_owned(),
            // Google and Apple show app passwords in groups; the spaces aren't part of them.
            password: password.split_whitespace().collect(),
            ..Account::default()
        }
    }

    /// Unique per account: the provider and sign-in.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider.name(), self.user.to_lowercase())
    }

    pub fn name(&self) -> &str {
        if self.label.trim().is_empty() { &self.user } else { &self.label }
    }

    pub fn serves(&self, service: Service) -> bool {
        match service {
            Service::Mail => self.services.mail,
            Service::Calendar => self.services.calendar,
            Service::Contacts => self.services.contacts,
        }
    }

    /// Where its calendars are found from.
    pub fn caldav_start(&self) -> String {
        match self.provider {
            Provider::Fastmail => format!("https://caldav.fastmail.com/dav/calendars/user/{}/", self.user),
            Provider::Google => format!("https://www.google.com/calendar/dav/{}/events/", self.user),
            Provider::ICloud => "https://caldav.icloud.com/".into(),
            Provider::Other => self.caldav.trim().to_owned(),
        }
    }

    /// Where its address books are found from: the home itself for Fastmail and Google, a
    /// place to discover it from for the others.
    pub fn carddav_start(&self) -> String {
        match self.provider {
            Provider::Fastmail => format!("https://carddav.fastmail.com/dav/addressbooks/user/{}/", self.user),
            // Google's CardDAV that takes app passwords: one address book, the default.
            Provider::Google => format!("https://www.googleapis.com/carddav/v1/principals/{}/lists/default/", self.user),
            Provider::ICloud => "https://contacts.icloud.com/".into(),
            Provider::Other => self.carddav.trim().to_owned(),
        }
    }

    /// IMAP and SMTP servers: (imap host, port, smtp host, port, STARTTLS).
    pub fn mail_servers(&self) -> Option<(&'static str, u16, &'static str, u16, bool)> {
        match self.provider {
            Provider::Fastmail => Some(("imap.fastmail.com", 993, "smtp.fastmail.com", 465, false)),
            Provider::Google => Some(("imap.gmail.com", 993, "smtp.gmail.com", 465, false)),
            Provider::ICloud => Some(("imap.mail.me.com", 993, "smtp.mail.me.com", 587, true)),
            Provider::Other => None,
        }
    }

    fn secret(&self) -> String {
        format!("{}/password", self.id())
    }
}

pub fn path() -> PathBuf {
    crate::config::shared_dir().join("accounts.json")
}

/// Every account, with its password.
pub fn load() -> Vec<Account> {
    let mut list: Vec<Account> = crate::config::load(&path());
    for a in &mut list {
        if a.password.is_empty()
            && let Some(p) = KEYRING.find(&a.secret())
        {
            a.password = p;
        }
    }
    list
}

/// The accounts an app can use.
pub fn for_service(service: Service) -> Vec<Account> {
    load().into_iter().filter(|a| a.serves(service) && !a.password.is_empty()).collect()
}

/// Saves the list, each password in the keyring where there is one.
pub fn save(list: &[Account]) -> std::io::Result<()> {
    let stripped: Vec<Account> = list
        .iter()
        .map(|a| {
            let mut a = a.clone();
            if !a.password.is_empty() && KEYRING.set(&a.secret(), &a.password) {
                a.password.clear();
            }
            a
        })
        .collect();
    crate::config::save_private(&path(), &stripped)
}

/// Adds the account, or updates the one with its id (keeping its label and what it's used for
/// unless the new one says otherwise).
pub fn upsert(account: Account) -> std::io::Result<()> {
    let mut list = load();
    match list.iter_mut().find(|a| a.id() == account.id()) {
        Some(old) => {
            let label = if account.label.is_empty() { old.label.clone() } else { account.label.clone() };
            let password = if account.password.is_empty() { old.password.clone() } else { account.password.clone() };
            *old = Account { label, password, ..account };
        }
        None => list.push(account),
    }
    save(&list)
}

/// Adds the accounts that aren't in the list yet, leaving the ones that are as they are.
/// For apps bringing in the accounts they had before the shared list.
pub fn adopt(accounts: impl IntoIterator<Item = Account>) -> std::io::Result<usize> {
    let mut list = load();
    let mut added = 0;
    for a in accounts {
        if !a.user.is_empty() && !a.password.is_empty() && !list.iter().any(|b| b.id() == a.id()) {
            list.push(a);
            added += 1;
        }
    }
    if added > 0 {
        save(&list)?;
    }
    Ok(added)
}

/// Removes the account and its password.
pub fn remove(id: &str) -> std::io::Result<()> {
    let mut list = load();
    if let Some(a) = list.iter().find(|a| a.id() == id) {
        KEYRING.delete(&a.secret());
    }
    list.retain(|a| a.id() != id);
    save(&list)
}

/// Switches one service on or off for an account.
pub fn set_service(id: &str, service: Service, on: bool) -> std::io::Result<()> {
    let mut list = load();
    if let Some(a) = list.iter_mut().find(|a| a.id() == id) {
        match service {
            Service::Mail => a.services.mail = on,
            Service::Calendar => a.services.calendar = on,
            Service::Contacts => a.services.contacts = on,
        }
    }
    save(&list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_updates_and_removes() {
        crate::config::use_scratch_dirs("rune-suite");
        let _ = std::fs::remove_file(path());
        upsert(Account::new(Provider::Google, "me@gmail.com", "abcd efgh ijkl mnop")).unwrap();
        let got = load();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].password, "abcdefghijklmnop", "spaces in an app password are dropped");
        assert_eq!(got[0].id(), "Google:me@gmail.com");

        let mut renamed = got[0].clone();
        renamed.label = "Personal".into();
        renamed.password.clear();
        upsert(renamed).unwrap();
        let got = load();
        assert_eq!((got[0].name(), got[0].password.as_str()), ("Personal", "abcdefghijklmnop"), "an update without a password keeps it");

        assert_eq!(adopt([Account::new(Provider::Google, "ME@gmail.com", "x"), Account::new(Provider::Fastmail, "a@fastmail.com", "y")]).unwrap(), 1);
        set_service("Fastmail:a@fastmail.com", Service::Mail, false).unwrap();
        assert_eq!(for_service(Service::Mail).len(), 1);
        assert_eq!(for_service(Service::Contacts).len(), 2);
        remove("Google:me@gmail.com").unwrap();
        assert_eq!(load().len(), 1);
    }

    #[test]
    fn knows_providers_by_address() {
        assert_eq!(Provider::of_email("x@gmail.com"), Some(Provider::Google));
        assert_eq!(Provider::of_email("x@me.com"), Some(Provider::ICloud));
        assert_eq!(Provider::of_imap_host("imap.fastmail.com"), Some(Provider::Fastmail));
        assert_eq!(Provider::of_email("x@example.org"), None);
    }
}
