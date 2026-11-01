//! Rune Mail: a JMAP mail client for Fastmail, laid out like Apple Mail, drawn with Rune.

mod accounts;
mod ai;
mod page;
mod prefetch;
mod actions;
mod app;
mod attach;
mod backend;
mod caldav;
mod contacts;
mod demo;
mod gestures;
mod html;
mod imap;
mod jmap;
mod privacy;
mod secrets;
mod settings;
mod store;
mod ui;
mod undo;
mod worker;

use rune::AppSpec;

fn main() {
    // Matches assets/rune-mail.desktop, so the dock shows the window with its icon.
    rune::set_app_id("rune-mail");
    rune::prewarm_gpu();
    rune::run(AppSpec {
        scenes: &[
            ("demo", demo::scene),
            ("single", demo::single),
            ("compose", demo::compose_scene),
            ("html", demo::html_scene),
            ("thread", demo::thread_scene),
            ("white", demo::white_scene),
            ("suggest", demo::suggest_scene),
            ("failed", demo::failed_scene),
            ("new", demo::new_message_scene),
            ("marked", demo::marked_scene),
            ("settings", demo::settings_scene),
            ("shortcuts", demo::shortcuts_scene),
            ("calendar", demo::calendar_scene),
            ("add-account", demo::add_account_scene),
        ],
        min_size: Some((900.0, 520.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("Rune Mail", (1280.0, 800.0), app::init, ui::view)
    });
}
