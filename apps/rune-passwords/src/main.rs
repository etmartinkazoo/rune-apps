mod account;
mod api;
mod app;
mod crypto;
mod demo;
mod generator;
mod icons;
mod secrets;
mod settings;
mod strength;
mod swipe;
mod totp;
mod ui;
mod vault;
mod worker;

use rune::AppSpec;

fn main() {
    // Matches assets/rune-passwords.desktop, so the dock shows the window with its icon.
    rune::set_app_id("rune-passwords");
    rune::prewarm_gpu();
    rune::run(AppSpec {
        scenes: &[
            ("demo", demo::ready),
            ("edit", demo::edit),
            ("new", demo::new_item),
            ("security", demo::security),
            ("deleted", demo::deleted),
            ("swipe", demo::swipe),
            ("confirm", demo::confirm),
            ("settings", demo::settings),
            ("signin", demo::sign_in),
            ("unlock", demo::unlock),
            ("accounts", demo::account_menu),
            ("folders", demo::many_folders),
        ],
        min_size: Some((900.0, 520.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("Passwords", (1280.0, 800.0), app::init, ui::view)
    });
}
