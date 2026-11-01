//! Sample notes for headless snapshots (`--scene`), written to a folder of their own under the
//! system's temp folder; never touches your notes or settings.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::app::{App, Place, Sheet};

const NOTES: &[(&str, u64, &str)] = &[
    ("welcome.md", 60 * 24 * 9, "# Welcome to Rune Notes\n\nEvery note is a Markdown file in your notes folder, and every folder a folder.\n"),
    ("groceries.md", 50, "# Groceries\n\n- [ ] Oat milk\n- [x] Sourdough\n- [ ] Lemons\n- [ ] Parmesan\n"),
    ("Work/q4-planning.md", 12, "# Q4 planning\n\nThree things for the quarter, in order:\n\n\
        1. Ship **Rune Notes** to the team\n2. Move the docs site to the new renderer\n3. Halve cold start on Linux\n\n\
        ## Risks\n\n- The renderer change touches every app\n- [ ] Ask Grace about the release train\n- [x] Book the offsite\n\n\
        > Small, steady releases beat one big one.\n\n## Notes from Monday\n\nKeep the scope tight; `cargo test` stays green.\n"),
    ("Work/1-1-grace.md", 60 * 26, "# 1:1 with Grace\n\nShe'd like a design review before the launch.\n"),
    ("Work/compiler-sync.md", 60 * 24 * 3, "# Compiler sync\n\nIncremental builds are down to 1.8 s.\n"),
    ("Work/Projects/rune-notes.md", 60 * 5, "# Rune Notes\n\nA notes app on the Rune stack: plain Markdown files, real folders.\n"),
    ("Work/Projects/launch-checklist.md", 60 * 24 * 12, "# Launch checklist\n\n- [x] Icon\n- [ ] Screenshots\n- [ ] README\n"),
    ("Personal/books.md", 60 * 24 * 2, "# Books to read\n\n- The Rust Programming Language\n- A Pattern Language\n"),
    ("Personal/recipes.md", 60 * 24 * 40, "# Recipes\n\nRisotto: stock warm, rice toasted, stir.\n"),
    ("Personal/Travel/rome.md", 60 * 24 * 5, "# Trip to Rome\n\nFlights on the 14th. Book the Borghese a week ahead.\n"),
    ("Journal/2026-10-29.md", 60 * 24, "Thursday\n\nLong walk by the river; the leaves are turning.\n"),
];

/// A fresh copy of the sample notes, the same for every scene in a run.
pub fn sample_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("rune-notes-demo-{}", std::process::id()));
    if root.exists() {
        return root;
    }
    let now = SystemTime::now();
    for (path, minutes_ago, text) in NOTES {
        let p = root.join(path);
        fs::create_dir_all(p.parent().unwrap()).expect("temp folder is writable");
        fs::write(&p, text).expect("temp folder is writable");
        let f = fs::File::options().write(true).open(&p).expect("just written");
        let _ = f.set_modified(now - Duration::from_secs(minutes_ago * 60));
    }
    root
}

pub fn scene(s: &mut App) {
    s.settings = crate::settings::Settings::default();
    s.set_root(sample_root());
    let work = s.root.join("Work");
    s.select_place(Place::Folder(work.clone()));
    s.open_note(&work.join("q4-planning.md"));
}

pub fn notes(s: &mut App) {
    scene(s);
}

pub fn preview(s: &mut App) {
    scene(s);
    s.preview = true;
}

pub fn search(s: &mut App) {
    scene(s);
    s.search.replace_text("book");
    s.search.focused = true;
    let first = s.visible().first().map(|n| n.path.clone());
    if let Some(p) = first {
        s.open_note(&p);
    }
}

pub fn all(s: &mut App) {
    scene(s);
    s.select_place(Place::All);
}

pub fn new_folder(s: &mut App) {
    scene(s);
    s.select_place(Place::Folder(s.root.join("Personal")));
    s.new_folder();
}

pub fn delete_folder(s: &mut App) {
    scene(s);
    s.sheet = Sheet::DeleteFolder(s.root.join("Work"));
}

pub fn move_to(s: &mut App) {
    scene(s);
    s.sheet = Sheet::MoveTo(s.root.join("Work/q4-planning.md"));
}

pub fn settings(s: &mut App) {
    scene(s);
    s.open_settings();
}

pub fn empty(s: &mut App) {
    s.settings = crate::settings::Settings::default();
    let root = std::env::temp_dir().join(format!("rune-notes-empty-{}", std::process::id()));
    let _ = fs::create_dir_all(&root);
    s.set_root(root);
}

/// Points the config away from yours, for tests.
#[cfg(test)]
pub fn test_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("rune-notes-test-{}", std::process::id()));
        // SAFETY: set once, before any test thread reads the environment through `dirs`.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
        }
    });
}
