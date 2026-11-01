//! What the Rune apps share, so they look and behave as one family:
//!
//! - [`ui`]: the palette, type, icons, text fields and controls (buttons, switches, segmented
//!   controls, sheets).
//! - [`Appearance`] and [`config`]: settings files in the same places, written the same way.
//! - [`Keyring`]: secrets in the system keyring, with a file fallback.
//! - [`accounts`]: the accounts every app can use (one sign-in for Mail, Calendar and Contacts).
//! - [`contacts`] and [`vcard`]: the address book Rune Contacts keeps and the other apps read.
//! - [`dav`]: WebDAV for CalDAV and CardDAV.
//! - [`links`] and [`instance`]: `rune-*:` links between the apps, and one window per app.

pub mod accounts;
pub mod config;
pub mod contacts;
pub mod dav;
pub mod instance;
pub mod links;
pub mod secrets;
pub mod ui;
pub mod vcard;

mod appearance;

pub use appearance::Appearance;
pub use secrets::Keyring;

/// Sets the window's app id: the Flatpak's id for this app when running in one (so the dock
/// matches it to its launcher), else `name`. Call first thing in `main`.
pub fn set_app_id(name: &'static str) {
    rune::set_app_id(app_id(name));
}

/// `name`, or inside the Rune Flatpak `io.brem.Rune.<Name>`, as its launchers are named.
pub fn app_id(name: &'static str) -> &'static str {
    if std::env::var_os("FLATPAK_ID").is_none() {
        return name;
    }
    let short = name.strip_prefix("rune-").unwrap_or(name);
    let mut cap = short.chars();
    let short: String = cap.next().map(|c| c.to_ascii_uppercase()).into_iter().chain(cap).collect();
    Box::leak(format!("io.brem.Rune.{short}").into_boxed_str())
}
