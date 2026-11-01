mod actions;
mod app;
mod bluez;
mod bmsg;
mod cache;
mod calls;
mod demo;
#[cfg(test)]
mod fakes;
mod number;
mod obex;
mod settings;
mod ui;
mod vcard;
mod worker;

use rune::AppSpec;

fn main() {
    // Matches assets/rune-phone.desktop, so the dock shows the window with its icon.
    rune::set_app_id("rune-phone");
    rune::prewarm_gpu();
    rune::run(AppSpec {
        scenes: &[
            ("demo", demo::ready),
            ("pinned", demo::pinned),
            ("incoming", demo::incoming),
            ("call", demo::on_call),
            ("keypad", demo::keypad),
            ("contact", demo::contact),
            ("setup", demo::setup),
            ("pair", demo::pair),
            ("remove", demo::remove),
        ],
        min_size: Some((860.0, 540.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("Phone", (1180.0, 760.0), app::init, ui::view)
    });
}
