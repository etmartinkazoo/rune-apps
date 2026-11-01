//! The address book every app can read: who has which addresses and numbers.
//!
//! Rune Contacts keeps it, in `~/.local/share/rune/contacts.json`, each time it syncs or you
//! change a card. Mail completes addresses from it and Phone puts names to numbers, without
//! signing in to anything themselves. [`Directory`] reads it again when it changes.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::vcard::Card;

/// One person, as the other apps need them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Contact {
    /// The card's UID, for a `rune-contacts://contact/<uid>` link.
    pub uid: String,
    pub name: String,
    pub org: String,
    pub emails: Vec<String>,
    /// As written on the card.
    pub phones: Vec<String>,
}

impl Contact {
    pub fn of(card: &Card) -> Contact {
        Contact {
            uid: card.uid.clone(),
            name: card.name(),
            org: card.org.clone(),
            emails: card.emails.iter().map(|e| e.value.trim().to_owned()).filter(|e| !e.is_empty()).collect(),
            phones: card.phones.iter().map(|p| p.value.trim().to_owned()).filter(|p| !p.is_empty()).collect(),
        }
    }
}

pub fn path() -> PathBuf {
    crate::config::shared_data_dir().join("contacts.json")
}

/// Everyone in the shared address book; empty when Rune Contacts hasn't written it.
pub fn load() -> Vec<Contact> {
    crate::config::load(&path())
}

/// Replaces the shared address book with these cards.
pub fn publish<'a>(cards: impl IntoIterator<Item = &'a Card>) -> std::io::Result<()> {
    let mut list: Vec<Contact> = cards.into_iter().map(Contact::of).collect();
    list.sort_by_key(|c| c.name.to_lowercase());
    crate::config::save_private(&path(), &list)
}

/// A number's digits, for comparing: the last nine, so `+44 7700 900123` and `07700 900123`
/// match.
pub fn number_key(number: &str) -> String {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    digits[digits.len().saturating_sub(9)..].to_owned()
}

/// The shared address book, read again when the file changes.
#[derive(Default)]
pub struct Directory {
    pub contacts: Vec<Contact>,
    seen: Option<SystemTime>,
}

impl Directory {
    pub fn new() -> Directory {
        let mut d = Directory::default();
        d.refresh();
        d
    }

    /// Reads the file again if it changed; true when it did.
    pub fn refresh(&mut self) -> bool {
        let modified = std::fs::metadata(path()).and_then(|m| m.modified()).ok();
        if modified == self.seen {
            return false;
        }
        self.seen = modified;
        self.contacts = load();
        true
    }

    /// Whoever has this address.
    pub fn by_email(&self, email: &str) -> Option<&Contact> {
        let email = email.trim();
        self.contacts.iter().find(|c| c.emails.iter().any(|e| e.eq_ignore_ascii_case(email)))
    }

    /// Whoever has this number.
    pub fn by_phone(&self, number: &str) -> Option<&Contact> {
        let key = number_key(number);
        if key.len() < 4 {
            return None;
        }
        self.contacts.iter().find(|c| c.phones.iter().any(|p| number_key(p) == key))
    }

    /// People whose name or address starts a word with `prefix`, for completing an address.
    pub fn complete(&self, prefix: &str) -> Vec<(&Contact, &str)> {
        let q = prefix.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for c in &self.contacts {
            let name_hit = c.name.to_lowercase().split_whitespace().any(|w| w.starts_with(&q));
            for e in &c.emails {
                if name_hit || e.to_lowercase().starts_with(&q) {
                    out.push((c, e.as_str()));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcard::Labeled;

    fn dir() -> Directory {
        let card = Card {
            uid: "1".into(),
            given: "Ada".into(),
            family: "Lovelace".into(),
            emails: vec![Labeled::new("home", "Ada@Example.com")],
            phones: vec![Labeled::new("mobile", "+44 7700 900123")],
            ..Card::default()
        };
        Directory { contacts: vec![Contact::of(&card)], seen: None }
    }

    #[test]
    fn finds_people_by_address_and_number() {
        let d = dir();
        assert_eq!(d.by_email("ada@example.com").map(|c| c.name.as_str()), Some("Ada Lovelace"));
        assert_eq!(d.by_phone("07700 900123").map(|c| c.uid.as_str()), Some("1"), "national and international forms match");
        assert!(d.by_phone("123").is_none(), "too few digits to say");
        assert_eq!(d.complete("love").len(), 1);
        assert_eq!(d.complete("ada@").len(), 1);
        assert!(d.complete("x").is_empty());
    }
}
