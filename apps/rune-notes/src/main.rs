mod app;
mod demo;
mod settings;
mod store;
mod ui;

use rune::AppSpec;

fn main() {
    // Matches assets/rune-notes.desktop, so the dock shows the window with its icon.
    rune::set_app_id("rune-notes");
    rune::prewarm_gpu();
    let spec = AppSpec {
        scenes: &[
            ("demo", demo::notes),
            ("notes", demo::notes),
            ("preview", demo::preview),
            ("search", demo::search),
            ("all", demo::all),
            ("new-folder", demo::new_folder),
            ("delete-folder", demo::delete_folder),
            ("move", demo::move_to),
            ("settings", demo::settings),
            ("empty", demo::empty),
        ],
        min_size: Some((900.0, 520.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("Rune Notes", (1280.0, 800.0), app::init, ui::view)
    };
    // Saves the open note as the window closes.
    rune::App::new(spec).on_close(app::on_close).run();
}
