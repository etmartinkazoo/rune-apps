//! The vault, decrypted: items (logins, notes, cards, identities, SSH keys), folders and the
//! organisations they're shared through, from a sync. And the way back: an edited item
//! encrypted into what the server takes, its untouched fields left exactly as they came.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::crypto::{PrivateKey, Result, SymKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Login,
    Note,
    Card,
    Identity,
    SshKey,
    /// A kind newer than this app (bank account, driver's licence, passport…): its type
    /// number, shown by its details.
    Other(u64),
}

impl Kind {
    fn from_wire(n: u64) -> Option<Kind> {
        Some(match n {
            1 => Kind::Login,
            2 => Kind::Note,
            3 => Kind::Card,
            4 => Kind::Identity,
            5 => Kind::SshKey,
            0 => return None,
            n => Kind::Other(n),
        })
    }

    fn wire(self) -> u64 {
        match self {
            Kind::Login => 1,
            Kind::Note => 2,
            Kind::Card => 3,
            Kind::Identity => 4,
            Kind::SshKey => 5,
            Kind::Other(n) => n,
        }
    }
}

/// A custom field, or a detail of a card or identity.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub label: String,
    pub value: String,
    /// Hidden until revealed, as a password is.
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub org: Option<String>,
    pub folder: Option<String>,
    pub name: String,
    pub notes: String,
    pub username: String,
    pub password: String,
    pub uris: Vec<String>,
    pub totp: String,
    /// Passkeys stored with the login: the sites they're for.
    pub passkeys: Vec<String>,
    /// A card's or identity's details, and custom fields.
    pub details: Vec<Detail>,
    pub favorite: bool,
    pub revised: String,
    pub created: String,
    pub deleted: Option<String>,
    /// The account may change it (an organisation can share read-only).
    pub editable: bool,
    /// The item as the server sent it, for writing back what isn't edited.
    raw: Value,
}

impl Item {
    /// What the list shows under the name: the username, a card's last digits, a note's start.
    pub fn subtitle(&self) -> String {
        match self.kind {
            Kind::Login => self.username.clone(),
            Kind::Note => self.notes.lines().next().unwrap_or_default().to_owned(),
            _ => self.details.first().map(|d| if d.secret { "••••".into() } else { d.value.clone() }).unwrap_or_default(),
        }
    }

    /// The first website's host, for the icon and for matching.
    pub fn host(&self) -> Option<String> {
        let uri = self.uris.first()?;
        let with_scheme = if uri.contains("://") { uri.clone() } else { format!("https://{uri}") };
        let host = url::Url::parse(&with_scheme).ok()?.host_str()?.to_owned();
        Some(host.strip_prefix("www.").unwrap_or(&host).to_owned())
    }

    /// A new, empty login in `folder`.
    pub fn new_login(folder: Option<String>) -> Item {
        Item {
            id: String::new(),
            kind: Kind::Login,
            org: None,
            folder,
            name: String::new(),
            notes: String::new(),
            username: String::new(),
            password: String::new(),
            uris: Vec::new(),
            totp: String::new(),
            passkeys: Vec::new(),
            details: Vec::new(),
            favorite: false,
            revised: String::new(),
            created: String::new(),
            deleted: None,
            editable: true,
            raw: json!({ "type": 1, "login": {} }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Org {
    pub id: String,
    pub name: String,
}

/// What a sync holds, decrypted.
#[derive(Clone, Debug, Default)]
pub struct Vault {
    pub items: Vec<Item>,
    pub folders: Vec<Folder>,
    pub orgs: Vec<Org>,
    pub email: String,
    /// Items that didn't decrypt (a key this account doesn't have): how many.
    pub unreadable: usize,
}

/// The keys a vault opens with: the user's, and each organisation's.
pub struct Keys {
    /// The account's id: items say whose key encrypted them.
    pub user_id: String,
    pub user: SymKey,
    pub orgs: HashMap<String, SymKey>,
}

/// A value from the server by its camelCase name, or the PascalCase older servers send.
pub fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
    match v.get(name) {
        Some(x) => x,
        None => {
            let mut pascal = name.to_owned();
            pascal[..1].make_ascii_uppercase();
            v.get(&pascal).unwrap_or(&Value::Null)
        }
    }
}

fn text(v: &Value, name: &str) -> Option<String> {
    field(v, name).as_str().map(str::to_owned)
}

impl Keys {
    /// The organisation keys in a sync's profile, opened with the account's private key.
    pub fn open(user: SymKey, sync: &Value) -> Result<Keys> {
        let profile = field(sync, "profile");
        let mut orgs = HashMap::new();
        if let Some(enc) = text(profile, "privateKey") {
            let private = PrivateKey::open(&enc, &user)?;
            for o in field(profile, "organizations").as_array().into_iter().flatten() {
                if let (Some(id), Some(k)) = (text(o, "id"), text(o, "key"))
                    && let Ok(key) = private.decrypt_key(&k)
                {
                    orgs.insert(id, key);
                }
            }
        }
        Ok(Keys { user_id: text(profile, "id").unwrap_or_default(), user, orgs })
    }

    /// The key that encrypts `cipher`'s fields: its own (if it has one), else its owner's.
    fn for_cipher(&self, cipher: &Value) -> Result<SymKey> {
        let owner = match text(cipher, "organizationId") {
            Some(org) => self.orgs.get(&org).ok_or("An organisation's key is missing")?,
            None => &self.user,
        };
        match text(cipher, "key") {
            Some(own) => owner.decrypt_key(&own),
            None => Ok(owner.clone()),
        }
    }
}

impl Vault {
    pub fn open(sync: &Value, keys: &Keys) -> Vault {
        let profile = field(sync, "profile");
        let mut v = Vault { email: text(profile, "email").unwrap_or_default(), ..Vault::default() };
        for o in field(profile, "organizations").as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (text(o, "id"), text(o, "name")) {
                v.orgs.push(Org { id, name });
            }
        }
        for f in field(sync, "folders").as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (text(f, "id"), text(f, "name"))
                && let Ok(name) = keys.user.decrypt_str(&name)
            {
                v.folders.push(Folder { id, name });
            }
        }
        v.folders.sort_by_key(|f| f.name.to_lowercase());
        for c in field(sync, "ciphers").as_array().into_iter().flatten() {
            match decrypt_item(c, keys) {
                Some(item) => v.items.push(item),
                None => v.unreadable += 1,
            }
        }
        v.items.sort_by_key(|i| i.name.to_lowercase());
        v
    }
}

fn decrypt_item(c: &Value, keys: &Keys) -> Option<Item> {
    let kind = Kind::from_wire(field(c, "type").as_u64()?)?;
    let key = keys.for_cipher(c).ok()?;
    let dec = |v: &Value, name: &str| text(v, name).and_then(|e| key.decrypt_str(&e).ok()).unwrap_or_default();
    let login = field(c, "login");
    let mut details = Vec::new();
    let section = match kind {
        Kind::Card => Some((field(c, "card"), CARD)),
        Kind::Identity => Some((field(c, "identity"), IDENTITY)),
        Kind::SshKey => Some((field(c, "sshKey"), SSH)),
        _ => None,
    };
    if let Some((obj, labels)) = section {
        for (name, label, secret) in labels {
            let value = dec(obj, name);
            if !value.is_empty() {
                details.push(Detail { label: (*label).to_owned(), value, secret: *secret });
            }
        }
    }
    if let Kind::Other(n) = kind {
        // Whatever its section holds, by name; numbers and the like look secret enough.
        let name = match n {
            6 => "bankAccount",
            7 => "driversLicense",
            8 => "passport",
            _ => "",
        };
        for (k, v) in field(c, name).as_object().into_iter().flatten() {
            if let Some(value) = v.as_str().and_then(|e| key.decrypt_str(e).ok()).filter(|s| !s.is_empty()) {
                let secret = k.to_lowercase().contains("number");
                details.push(Detail { label: words(k), value, secret });
            }
        }
    }
    for f in field(c, "fields").as_array().into_iter().flatten() {
        details.push(Detail { label: dec(f, "name"), value: dec(f, "value"), secret: field(f, "type").as_u64() == Some(1) });
    }
    Some(Item {
        id: text(c, "id")?,
        kind,
        org: text(c, "organizationId"),
        folder: text(c, "folderId"),
        name: dec(c, "name"),
        notes: dec(c, "notes"),
        username: dec(login, "username"),
        password: dec(login, "password"),
        uris: field(login, "uris").as_array().into_iter().flatten().map(|u| dec(u, "uri")).filter(|u| !u.is_empty()).collect(),
        totp: dec(login, "totp"),
        passkeys: field(login, "fido2Credentials").as_array().into_iter().flatten().map(|p| dec(p, "rpId")).collect(),
        details,
        favorite: field(c, "favorite").as_bool().unwrap_or(false),
        revised: text(c, "revisionDate").unwrap_or_default(),
        created: text(c, "creationDate").unwrap_or_default(),
        deleted: text(c, "deletedDate"),
        editable: field(c, "edit").as_bool().unwrap_or(true),
        raw: c.clone(),
    })
}

/// `accountNumber` as "Account number".
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else if i == 0 {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// (wire name, label, hidden) for a card's, identity's and SSH key's details, in the order
/// they show.
const CARD: &[(&str, &str, bool)] = &[
    ("cardholderName", "Cardholder", false),
    ("number", "Number", true),
    ("brand", "Brand", false),
    ("expMonth", "Expiry month", false),
    ("expYear", "Expiry year", false),
    ("code", "Security code", true),
];
const IDENTITY: &[(&str, &str, bool)] = &[
    ("title", "Title", false),
    ("firstName", "First name", false),
    ("lastName", "Last name", false),
    ("email", "Email", false),
    ("phone", "Phone", false),
    ("company", "Company", false),
    ("address1", "Address", false),
    ("city", "City", false),
    ("postalCode", "Postal code", false),
    ("country", "Country", false),
    ("username", "Username", false),
    ("passportNumber", "Passport number", true),
    ("licenseNumber", "Licence number", true),
    ("ssn", "Social security number", true),
];
const SSH: &[(&str, &str, bool)] = &[("publicKey", "Public key", false), ("keyFingerprint", "Fingerprint", false), ("privateKey", "Private key", true)];

/// What the server takes to create or update `item`: the fields this app edits encrypted
/// afresh, everything else as it came. A changed password goes into the item's history, as
/// the official apps keep it.
pub fn encrypt_item(item: &Item, keys: &Keys, now: &str) -> Result<Value> {
    let raw = &item.raw;
    let key = if item.id.is_empty() { keys.user.clone() } else { keys.for_cipher(raw)? };
    let old = if item.id.is_empty() { None } else { decrypt_item(raw, keys) };
    let enc = |s: &str| if s.is_empty() { Value::Null } else { Value::String(key.encrypt_str(s)) };
    let mut out = Map::new();
    // What the server's cipher request has, copied as it came.
    for name in [
        "organizationId", "key", "fields", "secureNote", "card", "identity", "sshKey", "bankAccount", "driversLicense",
        "passport", "reprompt", "passwordHistory",
    ] {
        let v = field(raw, name);
        if !v.is_null() {
            out.insert(name.to_owned(), v.clone());
        }
    }
    out.insert("type".into(), json!(item.kind.wire()));
    out.insert("encryptedFor".into(), json!(keys.user_id));
    out.insert("folderId".into(), item.folder.clone().map_or(Value::Null, Value::String));
    out.insert("favorite".into(), json!(item.favorite));
    out.insert("name".into(), enc(&item.name));
    out.insert("notes".into(), enc(&item.notes));
    if !item.revised.is_empty() {
        out.insert("lastKnownRevisionDate".into(), json!(item.revised));
    }
    if item.kind == Kind::Note && out.get("secureNote").is_none() {
        out.insert("secureNote".into(), json!({ "type": 0 }));
    }
    if item.kind == Kind::Login {
        let mut login = field(raw, "login").as_object().cloned().unwrap_or_default();
        login.insert("username".into(), enc(&item.username));
        login.insert("password".into(), enc(&item.password));
        login.insert("totp".into(), enc(&item.totp));
        // Each address keeps its match setting where it stays at the same place.
        let before = field(field(raw, "login"), "uris").as_array().cloned().unwrap_or_default();
        let uris: Vec<Value> = item
            .uris
            .iter()
            .enumerate()
            .map(|(i, u)| {
                let mut o = before.get(i).and_then(Value::as_object).cloned().unwrap_or_default();
                o.insert("uri".into(), enc(u));
                o.remove("uriChecksum");
                Value::Object(o)
            })
            .collect();
        login.insert("uris".into(), Value::Array(uris));
        if let Some(old) = old.as_ref().filter(|o| !o.password.is_empty() && o.password != item.password) {
            let mut history = field(raw, "passwordHistory").as_array().cloned().unwrap_or_default();
            history.insert(0, json!({ "password": key.encrypt_str(&old.password), "lastUsedDate": now }));
            history.truncate(5);
            out.insert("passwordHistory".into(), Value::Array(history));
            login.insert("passwordRevisionDate".into(), json!(now));
        }
        out.insert("login".into(), Value::Object(login));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A sync as the server sends it, for `items` (name, username, password) under `user`.
    pub fn sync_of(user: &SymKey, items: &[(&str, &str, &str)]) -> Value {
        let ciphers: Vec<Value> = items
            .iter()
            .enumerate()
            .map(|(i, (name, username, password))| {
                json!({
                    "id": format!("c{i}"), "type": 1, "name": user.encrypt_str(name), "notes": null,
                    "login": { "username": user.encrypt_str(username), "password": user.encrypt_str(password),
                               "uris": [{ "uri": user.encrypt_str("https://www.example.com/login"), "match": 3 }] },
                    "favorite": false, "revisionDate": "2026-10-01T00:00:00Z", "edit": true,
                })
            })
            .collect();
        json!({ "profile": { "email": "a@b.c", "organizations": [] },
                "folders": [{ "id": "f1", "name": user.encrypt_str("Work") }], "ciphers": ciphers })
    }

    #[test]
    fn opens_a_sync_and_writes_an_edit_back() {
        let user = SymKey::random();
        let sync = sync_of(&user, &[("Etsy", "lanchi@example.com", "old-pass")]);
        let keys = Keys { user_id: "u1".into(), user: user.clone(), orgs: HashMap::new() };
        let v = Vault::open(&sync, &keys);
        assert_eq!(v.folders[0].name, "Work");
        let item = &v.items[0];
        assert_eq!((item.name.as_str(), item.username.as_str(), item.password.as_str()), ("Etsy", "lanchi@example.com", "old-pass"));
        assert_eq!(item.host().as_deref(), Some("example.com"));

        let mut edited = item.clone();
        edited.password = "new-pass".into();
        let out = encrypt_item(&edited, &keys, "2026-10-31T00:00:00Z").unwrap();
        assert_eq!(out["login"]["uris"][0]["match"], 3, "what isn't edited stays");
        assert_eq!(user.decrypt_str(out["login"]["password"].as_str().unwrap()).unwrap(), "new-pass");
        let history = user.decrypt_str(out["passwordHistory"][0]["password"].as_str().unwrap()).unwrap();
        assert_eq!(history, "old-pass", "the old password is kept in the history");
        assert_eq!(out["lastKnownRevisionDate"], "2026-10-01T00:00:00Z");
    }

    #[test]
    fn an_item_under_a_key_it_doesnt_have_is_counted_not_shown() {
        let user = SymKey::random();
        let mut sync = sync_of(&user, &[("Mine", "a", "b")]);
        sync["ciphers"].as_array_mut().unwrap().push(json!({ "id": "x", "type": 1, "organizationId": "o9", "name": "2.a|b|c" }));
        let v = Vault::open(&sync, &Keys { user_id: "u1".into(), user, orgs: HashMap::new() });
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.unreadable, 1);
    }
}
