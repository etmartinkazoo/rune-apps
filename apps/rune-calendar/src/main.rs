mod accounts;
mod app;
mod caldav;
mod demo;
mod draft;
mod ics;
mod notes;
mod secrets;
mod settings;
mod when;
mod ui;
mod worker;

use rune::AppSpec;

fn main() {
    // Matches assets/rune-calendar.desktop, so the dock shows the window with its icon.
    rune::set_app_id("rune-calendar");
    rune::prewarm_gpu();
    rune::run(AppSpec {
        scenes: &[
            ("demo", demo::week),
            ("week", demo::week),
            ("day", demo::day),
            ("month", demo::month),
            ("details", demo::details),
            ("times", demo::times),
            ("settings", demo::settings),
            ("add", demo::add),
            ("empty", demo::empty),
        ],
        min_size: Some((860.0, 520.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("Rune Calendar", (1280.0, 800.0), app::init, ui::view)
    });
}
