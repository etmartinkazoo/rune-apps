//! Headless scenes (`--scene NAME`): a made-up vault, no server.

use crate::app::{AccountEntry, App, Category, Phase};
use crate::vault::{Detail, Folder, Item, Kind, Org, Vault};

fn login(id: &str, name: &str, user: &str, password: &str, site: &str) -> Item {
    let mut i = Item::new_login(None);
    i.id = id.into();
    i.name = name.into();
    i.username = user.into();
    i.password = password.into();
    i.uris = vec![format!("https://www.{site}")];
    i.revised = chrono::Utc::now().to_rfc3339();
    i
}

pub fn vault() -> Vault {
    let user = "lanchi.pedersen@example.com";
    let mut items = vec![
        login("doordash", "DoorDash", user, "vH7#qL2m!xR9@wZ4", "doordash.com"),
        login("atlas", "Atlas Obscura", user, "hufpyb-6xamqo-tisgEk", "atlasobscura.com"),
        login("kobu", "Kobu", user, "summer2024", "kobu.co"),
        login("quickbooks", "QuickBooks", user, "Lr8$pQe2!mVz7#Kd", "quickbooks.intuit.com"),
        login("linkedin", "LinkedIn", user, "summer2024", "linkedin.com"),
        login("etsy", "Etsy", user, "nobqyc-2Wurvo-dyjhuf", "etsy.com"),
        login("duolingo", "Duolingo", user, "Tq3!xG8@pLw2#Vn6", "duolingo.com"),
        login("latimes", "LA Times", user, "bR4@kN9!hZ2$wQ7m", "latimes.com"),
        login("target", "Target", user, "password1", "target.com"),
        login("apple", "Apple", user, "cajzyq-8Mexwo-pubhoz", "apple.com"),
        login("moo", "Moo", user, "Zp6#tY3!qM8@rX1v", "moo.com"),
    ];
    items[5].totp = "otpauth://totp/Etsy?secret=JBSWY3DPEHPK3PXP".into();
    items[5].folder = Some("work".into());
    items[3].folder = Some("work".into());
    items[3].totp = "JBSWY3DPEHPK3PXP".into();
    items[0].passkeys = vec!["doordash.com".into()];
    items[9].passkeys = vec!["apple.com".into()];
    items[6].org = Some("family".into());
    items[7].org = Some("family".into());
    let mut note = Item::new_login(None);
    note.id = "wifi".into();
    note.kind = Kind::Note;
    note.name = "Home Wi-Fi".into();
    note.notes = "Network: Pedersen 5G\nPassword: on the router".into();
    note.revised = chrono::Utc::now().to_rfc3339();
    items.push(note);
    let mut card = Item::new_login(None);
    card.id = "card".into();
    card.kind = Kind::Card;
    card.name = "Travel card".into();
    card.details = vec![
        Detail { label: "Cardholder".into(), value: "Lanchi Pedersen".into(), secret: false },
        Detail { label: "Number".into(), value: "4111 1111 1111 1111".into(), secret: true },
    ];
    items.push(card);
    let mut gone = login("old", "Old forum", user, "letmein", "forum.example");
    gone.deleted = Some(chrono::Utc::now().to_rfc3339());
    items.push(gone);
    items.sort_by_key(|i| i.name.to_lowercase());
    Vault {
        items,
        folders: vec![Folder { id: "work".into(), name: "Work".into() }],
        orgs: vec![Org { id: "family".into(), name: "Pedersen Family".into() }],
        email: user.into(),
        unreadable: 0,
    }
}

fn accounts(s: &mut App) {
    let entry = |id: &str, email: &str, host: &str| AccountEntry { id: id.into(), email: email.into(), server: format!("https://{host}"), host: host.into() };
    s.accounts = vec![
        entry("work", "lanchi@pedersen-studio.com", "vault.pedersen-studio.com"),
        entry("home", "lanchi.pedersen@example.com", "vault.example.com"),
    ];
    s.account_id = Some("home".into());
}

pub fn ready(s: &mut App) {
    s.show_vault(vault());
    accounts(s);
    s.select("etsy");
}

pub fn edit(s: &mut App) {
    ready(s);
    s.edit();
}

pub fn new_item(s: &mut App) {
    ready(s);
    s.new_item();
}

pub fn security(s: &mut App) {
    ready(s);
    s.choose(Category::Security);
    s.select("linkedin");
}

pub fn deleted(s: &mut App) {
    ready(s);
    s.choose(Category::Deleted);
}

/// LinkedIn's row swiped open, and Etsy's delete being asked about.
pub fn swipe(s: &mut App) {
    ready(s);
    s.select("linkedin");
    s.swipe = Some(crate::swipe::Swipe::open("linkedin", 3));
}

pub fn confirm(s: &mut App) {
    ready(s);
    s.ask_delete("etsy");
}

pub fn settings(s: &mut App) {
    ready(s);
    s.settings_open = true;
}

pub fn sign_in(s: &mut App) {
    s.phase = Phase::SignIn;
}

/// A vault with more folders than the sidebar holds, to show it scrolling.
pub fn many_folders(s: &mut App) {
    let mut v = vault();
    v.folders = (1..=30).map(|n| Folder { id: format!("f{n}"), name: format!("Folder {n}") }).collect();
    s.show_vault(v);
    accounts(s);
}

/// The sidebar's account menu, open.
pub fn account_menu(s: &mut App) {
    ready(s);
    s.accounts_open = true;
}

pub fn unlock(s: &mut App) {
    accounts(s);
    s.phase = Phase::Unlock;
    s.email = "lanchi.pedersen@example.com".into();
}
