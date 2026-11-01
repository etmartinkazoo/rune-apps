//! The app's state and what changes it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rune::{Drag, Phase, TextEdit, Tx, Waker};

use crate::settings::Settings;
use crate::store::{self, Note, Tree};

/// Seconds between looks for changes made outside the app.
const SCAN_SECS: f64 = 3.0;
/// Seconds after typing stops before the note is saved.
const SAVE_SECS: f64 = 0.8;
/// A new note's file is named after its title, so it waits for the first line to end, or
/// this long after typing stops.
const NEW_NOTE_SECS: f64 = 4.0;
/// How far the pointer travels before a press on a note becomes a drag.
const DRAG_START: f32 = 6.0;

/// What the list shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    All,
    Folder(PathBuf),
}

/// The note in the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    /// None for a new note not yet written.
    pub path: Option<PathBuf>,
    /// Where a new note goes.
    pub folder: PathBuf,
    /// The file's modified time when last read or written here.
    pub disk: Option<SystemTime>,
}

/// The name field of a folder being made or renamed, in the sidebar.
pub struct Naming {
    pub parent: PathBuf,
    /// The folder being renamed; None makes a new one.
    pub renaming: Option<PathBuf>,
    pub edit: TextEdit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Folder(PathBuf),
    Note(PathBuf),
}

/// A right-click menu, at a point in the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub target: Target,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    None,
    Settings,
    /// Choosing a folder for this note.
    MoveTo(PathBuf),
    /// Asking before a folder goes to the Trash.
    DeleteFolder(PathBuf),
}

pub struct Dragging {
    pub note: PathBuf,
    pub label: String,
    /// The pointer, in window points.
    pub x: f32,
    pub y: f32,
    /// The folder under the pointer.
    pub over: Option<PathBuf>,
}

/// Where note rows and folder rows are drawn, for dropping a dragged note on a folder.
#[derive(Default)]
pub struct Rects {
    pub notes: HashMap<PathBuf, [f32; 4]>,
    pub folders: HashMap<PathBuf, [f32; 4]>,
}

pub struct App {
    pub settings: Settings,
    pub root: PathBuf,
    pub tree: Tree,
    pub place: Place,
    pub open: Option<Open>,
    pub editor: TextEdit,
    /// The editor has changes not yet saved.
    pub dirty: bool,
    save_at: Option<f64>,
    /// The rendered Markdown shows instead of the source.
    pub preview: bool,
    pub search: TextEdit,
    pub naming: Option<Naming>,
    pub menu: Option<Menu>,
    pub sheet: Sheet,
    /// Settings' field for the notes folder.
    pub root_edit: TextEdit,
    pub dragging: Option<Dragging>,
    pub rects: Rects,
    pub status: Option<(String, f64)>,
    pub clock: f64,
    next_scan: f64,
}

pub fn init() -> App {
    let settings = Settings::load();
    let root = settings.root();
    let tree = store::scan(&root, &[]);
    App {
        root_edit: TextEdit::new(root.to_string_lossy()),
        place: Place::Folder(root.clone()),
        settings,
        root,
        tree,
        open: None,
        editor: TextEdit::new(""),
        dirty: false,
        save_at: None,
        preview: false,
        search: TextEdit::new(""),
        naming: None,
        menu: None,
        sheet: Sheet::None,
        dragging: None,
        rects: Rects::default(),
        status: None,
        clock: 0.0,
        next_scan: 0.0,
    }
}

pub fn setup(_s: &mut App, _wake: Waker) {}

pub fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = false;
    if s.save_at.is_some_and(|at| clock >= at) {
        s.save();
        changed = true;
    }
    if let Some(at) = s.save_at {
        tx.wake_after(Duration::from_secs_f64((at - clock).max(0.0)));
    }
    if clock >= s.next_scan {
        s.next_scan = clock + SCAN_SECS;
        changed |= s.rescan();
    }
    tx.wake_after(Duration::from_secs_f64((s.next_scan - clock).max(0.05)));
    match s.status {
        Some((_, until)) if clock >= until => {
            s.status = None;
            changed = true;
        }
        Some((_, until)) => tx.wake_after(Duration::from_secs_f64(until - clock)),
        None => {}
    }
    changed
}

/// The window is closing: nothing typed is lost.
pub fn on_close(s: &mut App, _tx: &mut Tx) {
    s.save();
}

impl App {
    pub fn say(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), self.clock + 4.0));
    }

    /// Reads the tree again, picking up changes made elsewhere; true when anything changed.
    /// The note in the editor follows its file unless it has changes of its own here.
    pub fn rescan(&mut self) -> bool {
        let tree = store::scan(&self.root, &self.tree.notes);
        let mut changed = tree != self.tree;
        self.tree = tree;
        if let Some(open) = &mut self.open
            && let Some(path) = &open.path
        {
            match self.tree.notes.iter().find(|n| &n.path == path) {
                Some(n) if Some(n.modified) != open.disk && !self.dirty => {
                    open.disk = Some(n.modified);
                    if n.text != self.editor.text {
                        let caret = self.editor.focus.min(n.text.len());
                        self.editor.replace_text(n.text.clone());
                        self.editor.set_caret((0..=caret).rev().find(|&i| n.text.is_char_boundary(i)).unwrap_or(0));
                        changed = true;
                    }
                }
                // Gone (moved or deleted elsewhere) with nothing unsaved: close it.
                None if !self.dirty => {
                    self.open = None;
                    changed = true;
                }
                _ => {}
            }
        }
        if let Place::Folder(f) = &self.place
            && self.tree.root.find(f).is_none()
        {
            self.place = Place::Folder(self.root.clone());
            changed = true;
        }
        changed
    }

    // What shows.

    /// The folder new notes and folders go in.
    pub fn current_folder(&self) -> PathBuf {
        match &self.place {
            Place::All => self.root.clone(),
            Place::Folder(f) => f.clone(),
        }
    }

    pub fn query(&self) -> &str {
        self.search.text.trim()
    }

    /// The notes in the list: search results from every folder, else the place's, newest first.
    pub fn visible(&self) -> Vec<&Note> {
        let q = self.query().to_lowercase();
        self.tree
            .notes
            .iter()
            .filter(|n| match (&self.place, q.is_empty()) {
                (_, false) => n.title.to_lowercase().contains(&q) || n.text.to_lowercase().contains(&q),
                (Place::All, true) => true,
                (Place::Folder(f), true) => n.folder() == f,
            })
            .collect()
    }

    pub fn place_title(&self) -> String {
        if !self.query().is_empty() {
            return "Search".into();
        }
        match &self.place {
            Place::All => "All Notes".into(),
            Place::Folder(f) => self.folder_name(f),
        }
    }

    pub fn folder_name(&self, f: &Path) -> String {
        if f == self.root { "Notes".into() } else { f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()) }
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.open.as_ref().and_then(|o| o.path.as_deref()) == Some(path)
    }

    pub fn is_collapsed(&self, f: &Path) -> bool {
        self.settings.collapsed.iter().any(|c| c == f)
    }

    // Folders.

    pub fn select_place(&mut self, place: Place) {
        self.finish_naming();
        self.menu = None;
        self.search.replace_text("");
        self.place = place;
    }

    pub fn toggle_folder(&mut self, f: &Path) {
        if let Some(i) = self.settings.collapsed.iter().position(|c| c == f) {
            self.settings.collapsed.remove(i);
        } else {
            self.settings.collapsed.push(f.to_owned());
        }
        let _ = self.settings.save();
    }

    /// Opens a name field for a new folder inside the selected one.
    pub fn new_folder(&mut self) {
        self.finish_naming();
        self.menu = None;
        let parent = self.current_folder();
        self.settings.collapsed.retain(|c| c != &parent);
        self.naming = Some(Naming { parent, renaming: None, edit: name_field("New Folder") });
    }

    pub fn start_rename(&mut self, f: &Path) {
        self.finish_naming();
        self.menu = None;
        if f == self.root {
            return;
        }
        let parent = f.parent().unwrap_or(&self.root).to_owned();
        self.naming = Some(Naming { parent, renaming: Some(f.to_owned()), edit: name_field(&self.folder_name(f)) });
    }

    /// Makes or renames the folder as named; the field stays when the name won't do.
    pub fn commit_naming(&mut self) {
        let Some(n) = &self.naming else { return };
        let result = match &n.renaming {
            Some(from) => store::rename_folder(from, &n.edit.text).map(|to| (Some(from.clone()), to)),
            None => store::create_folder(&n.parent, &n.edit.text).map(|to| (None, to)),
        };
        match result {
            Ok((from, to)) => {
                self.naming = None;
                match from {
                    Some(from) => self.rebase(&from, &to),
                    None => self.place = Place::Folder(to),
                }
                self.rescan();
            }
            Err(e) => self.say(e),
        }
    }

    pub fn cancel_naming(&mut self) {
        self.naming = None;
    }

    /// Ends naming when something else is pressed: keeps a name typed, drops an empty one.
    fn finish_naming(&mut self) {
        if self.naming.as_ref().is_some_and(|n| !n.edit.text.trim().is_empty()) {
            self.commit_naming();
        }
        self.naming = None;
    }

    /// After `from` became `to`: the selection, editor and folded folders follow.
    fn rebase(&mut self, from: &Path, to: &Path) {
        if let Place::Folder(f) = &self.place
            && let Some(p) = store::rebase(f, from, to)
        {
            self.place = Place::Folder(p);
        }
        if let Some(open) = &mut self.open {
            if let Some(p) = open.path.as_ref().and_then(|p| store::rebase(p, from, to)) {
                open.path = Some(p);
            }
            if let Some(p) = store::rebase(&open.folder, from, to) {
                open.folder = p;
            }
        }
        for c in &mut self.settings.collapsed {
            if let Some(p) = store::rebase(c, from, to) {
                *c = p;
            }
        }
        let _ = self.settings.save();
    }

    pub fn ask_delete_folder(&mut self, f: &Path) {
        self.menu = None;
        if f != self.root {
            self.sheet = Sheet::DeleteFolder(f.to_owned());
        }
    }

    /// Notes in `f` and every folder under it.
    pub fn notes_under(&self, f: &Path) -> usize {
        self.tree.notes.iter().filter(|n| n.path.starts_with(f)).count()
    }

    pub fn trash_folder(&mut self, f: &Path) {
        self.sheet = Sheet::None;
        if self.open.as_ref().is_some_and(|o| o.folder.starts_with(f)) {
            self.save();
            self.open = None;
        }
        match store::trash(&self.root, f) {
            Ok(_) => {
                if let Place::Folder(p) = &self.place
                    && p.starts_with(f)
                {
                    self.place = Place::Folder(f.parent().unwrap_or(&self.root).to_owned());
                }
                self.say(format!("Moved “{}” to the Trash", self.folder_name(f)));
            }
            Err(e) => self.say(format!("Couldn't delete the folder: {e}")),
        }
        self.rescan();
    }

    // Notes.

    pub fn open_note(&mut self, path: &Path) {
        self.finish_naming();
        self.menu = None;
        if self.is_open(path) {
            return;
        }
        self.save();
        let Some(n) = self.tree.note(path) else { return };
        self.open = Some(Open { path: Some(n.path.clone()), folder: n.folder().to_owned(), disk: Some(n.modified) });
        self.editor = TextEdit::new(n.text.clone());
        self.dirty = false;
    }

    /// A blank note in the selected folder, with the caret in it. Its file is written once
    /// there's something in it.
    pub fn new_note(&mut self) {
        self.finish_naming();
        self.menu = None;
        self.save();
        self.search.replace_text("");
        self.search.focused = false;
        self.open = Some(Open { path: None, folder: self.current_folder(), disk: None });
        self.editor = TextEdit::new("");
        self.editor.focused = true;
        self.preview = false;
        self.dirty = false;
    }

    /// The editor's text changed: save shortly.
    pub fn edited(&mut self) {
        self.dirty = true;
        let new = self.open.as_ref().is_some_and(|o| o.path.is_none());
        let wait = if new && !self.editor.text.trim_start().contains('\n') { NEW_NOTE_SECS } else { SAVE_SECS };
        self.save_at = Some(self.clock + wait);
    }

    /// Writes the editor's changes, if any. If the file changed on disk meanwhile, that
    /// version is kept beside it as a copy first.
    pub fn save(&mut self) {
        self.save_at = None;
        if !self.dirty {
            return;
        }
        let Some(open) = &mut self.open else { return };
        let text = self.editor.text.clone();
        let result = match &open.path {
            None if text.trim().is_empty() => Ok(None),
            None => store::create_note(&open.folder, &text).map(|p| {
                open.path = Some(p.clone());
                Some(p)
            }),
            Some(p) => {
                let on_disk = std::fs::metadata(p).and_then(|m| m.modified()).ok();
                let copy = match (on_disk, open.disk) {
                    (Some(now), Some(was)) if now != was => store::keep_copy(p).ok(),
                    _ => None,
                };
                if let Some(c) = copy {
                    let name = c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.status = Some((format!("Changed elsewhere too: that version is kept as {name}"), self.clock + 6.0));
                }
                Ok(Some(p.clone()))
            }
        };
        match result {
            Ok(Some(p)) => match store::save_atomic(&p, &text) {
                Ok(t) => {
                    open.disk = Some(t);
                    self.dirty = false;
                }
                Err(e) => self.say(format!("Couldn't save: {e}")),
            },
            Ok(None) => self.dirty = false,
            Err(e) => self.say(format!("Couldn't save: {e}")),
        }
        self.rescan();
    }

    pub fn trash_note(&mut self, path: &Path) {
        self.menu = None;
        let next = self.neighbour(path);
        if self.is_open(path) {
            self.open = None;
            self.dirty = false;
            self.save_at = None;
        }
        match store::trash(&self.root, path) {
            Ok(_) => self.say("Moved to the Trash"),
            Err(e) => self.say(format!("Couldn't delete the note: {e}")),
        }
        self.rescan();
        if self.open.is_none()
            && let Some(n) = next
        {
            self.open_note(&n);
        }
    }

    /// The note after `path` in the list, else the one before, to open when it goes.
    fn neighbour(&self, path: &Path) -> Option<PathBuf> {
        let list = self.visible();
        let i = list.iter().position(|n| n.path == path)?;
        list.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| list.get(j))).map(|n| n.path.clone())
    }

    pub fn trash_open(&mut self) {
        match self.open.clone() {
            Some(Open { path: Some(p), .. }) => {
                self.save();
                self.trash_note(&p);
            }
            Some(Open { path: None, .. }) => {
                self.open = None;
                self.dirty = false;
            }
            None => {}
        }
    }

    pub fn move_note(&mut self, path: &Path, dir: &Path) {
        self.sheet = Sheet::None;
        self.menu = None;
        if self.is_open(path) {
            self.save();
        }
        match store::move_into(path, dir) {
            Ok(to) => {
                if to != path {
                    if let Some(open) = self.open.as_mut().filter(|o| o.path.as_deref() == Some(path)) {
                        open.path = Some(to);
                        open.folder = dir.to_owned();
                    }
                    self.say(format!("Moved to “{}”", self.folder_name(dir)));
                }
            }
            Err(e) => self.say(format!("Couldn't move the note: {e}")),
        }
        self.rescan();
    }

    /// Up (-1) or down (1) the list.
    pub fn step(&mut self, by: isize) {
        let list: Vec<PathBuf> = self.visible().iter().map(|n| n.path.clone()).collect();
        let at = self.open.as_ref().and_then(|o| o.path.as_ref()).and_then(|p| list.iter().position(|x| x == p));
        let i = match at {
            Some(i) => (i as isize + by).clamp(0, list.len() as isize - 1) as usize,
            None => 0,
        };
        if let Some(p) = list.get(i) {
            self.open_note(&p.clone());
        }
    }

    pub fn toggle_preview(&mut self) {
        if self.open.is_some() {
            self.preview = !self.preview;
            self.editor.focused = !self.preview;
        }
    }

    /// A task box ticked in the preview: `[ ]` at `at` (the byte of `[`) becomes `[x]`.
    pub fn set_task(&mut self, at: usize, done: bool) {
        let t = &self.editor.text;
        if t.get(at..at + 3).is_some_and(|m| m.starts_with('[') && m.ends_with(']')) {
            let mut text = t.clone();
            text.replace_range(at + 1..at + 2, if done { "x" } else { " " });
            self.editor.replace_text(text);
            self.edited();
        }
    }

    // Dragging a note onto a folder.

    pub fn note_drag(&mut self, path: &Path, d: &mut Drag, tx: &mut Tx) {
        match d.phase {
            Phase::Down => {}
            Phase::Move => {
                if self.dragging.is_none() {
                    if d.dx.hypot(d.dy) < DRAG_START {
                        return;
                    }
                    let label = self.tree.note(path).map_or_else(String::new, |n| n.title.clone());
                    self.dragging = Some(Dragging { note: path.to_owned(), label, x: 0.0, y: 0.0, over: None });
                }
                let origin = self.rects.notes.get(path).copied().unwrap_or_default();
                let (x, y) = (origin[0] + d.x, origin[1] + d.y);
                let from = path.parent().map(Path::to_owned);
                let over = self
                    .tree
                    .root
                    .walk()
                    .into_iter()
                    .map(|(f, _)| &f.path)
                    .filter(|f| Some(*f) != from.as_ref())
                    .find(|f| self.rects.folders.get(*f).is_some_and(|r| inside(*r, x, y)))
                    .cloned();
                if let Some(dr) = self.dragging.as_mut() {
                    (dr.x, dr.y, dr.over) = (x, y, over);
                }
                tx.instant();
            }
            Phase::Up => {
                let Some(dr) = self.dragging.take() else { return };
                if let Some(dir) = dr.over.filter(|_| !d.cancelled()) {
                    self.move_note(&dr.note, &dir);
                }
            }
        }
    }

    pub fn drop_over(&self, f: &Path) -> bool {
        self.dragging.as_ref().and_then(|d| d.over.as_deref()) == Some(f)
    }

    // Settings.

    pub fn open_settings(&mut self) {
        self.menu = None;
        self.root_edit = TextEdit::new(self.root.to_string_lossy());
        self.sheet = Sheet::Settings;
    }

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        f(&mut self.settings);
        if self.settings != before {
            let _ = self.settings.save();
        }
    }

    /// Keeps notes in the folder typed in Settings, making it if need be.
    pub fn use_root(&mut self) {
        let path = crate::settings::expand(&self.root_edit.text);
        if !path.is_absolute() {
            self.say("Type the folder's full path, such as ~/Notes");
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&path) {
            self.say(format!("Couldn't use that folder: {e}"));
            return;
        }
        self.save();
        self.update_settings(|s| s.root = path.to_string_lossy().into_owned());
        self.set_root(path);
        self.say("Notes now come from this folder");
    }

    pub fn set_root(&mut self, path: PathBuf) {
        self.root = path;
        self.root_edit = TextEdit::new(self.root.to_string_lossy());
        self.tree = store::scan(&self.root, &[]);
        self.place = Place::Folder(self.root.clone());
        self.open = None;
        self.dirty = false;
        self.naming = None;
    }

    pub fn close_sheet(&mut self) {
        self.sheet = Sheet::None;
    }
}

fn name_field(name: &str) -> TextEdit {
    let mut e = TextEdit::new(name);
    e.set_selection(0, name.len());
    e.focused = true;
    e
}

fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn app(name: &str) -> App {
        crate::demo::test_config();
        let dir = std::env::temp_dir().join(format!("rune-notes-app-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut s = init();
        s.set_root(dir);
        s
    }

    fn type_text(s: &mut App, text: &str) {
        s.editor.replace_text(text);
        s.edited();
    }

    #[test]
    fn a_new_note_is_written_once_it_has_words_named_by_its_title() {
        let mut s = app("new");
        s.new_note();
        s.save();
        assert!(s.tree.notes.is_empty(), "nothing typed, nothing written");
        type_text(&mut s, "# Ideas");
        assert!(s.save_at.unwrap() >= NEW_NOTE_SECS, "the first line waits for its end");
        type_text(&mut s, "# Ideas\n");
        assert_eq!(s.save_at, Some(SAVE_SECS));
        s.save();
        let path = s.root.join("ideas.md");
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Ideas\n");
        assert!(s.is_open(&path));
        type_text(&mut s, "# Better ideas\nbody");
        s.save();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Better ideas\nbody", "same file: names don't follow edits");
        assert_eq!(s.visible()[0].title, "Better ideas");
    }

    #[test]
    fn outside_changes_reach_the_editor_unless_it_has_its_own() {
        let mut s = app("outside");
        fs::write(s.root.join("a.md"), "# A\none").unwrap();
        s.rescan();
        let a = s.root.join("a.md");
        s.open_note(&a);
        assert_eq!(s.editor.text, "# A\none");
        // Some file systems keep whole seconds: make the change visible.
        std::thread::sleep(Duration::from_millis(20));
        fs::write(&a, "# A\ntwo, from elsewhere").unwrap();
        bump(&a);
        s.rescan();
        assert_eq!(s.editor.text, "# A\ntwo, from elsewhere");
        type_text(&mut s, "# A\nmine");
        fs::write(&a, "# A\nthree, from elsewhere").unwrap();
        bump(&a);
        s.rescan();
        assert_eq!(s.editor.text, "# A\nmine", "unsaved typing isn't clobbered");
        s.save();
        assert_eq!(fs::read_to_string(&a).unwrap(), "# A\nmine");
        assert_eq!(fs::read_to_string(s.root.join("a (other copy).md")).unwrap(), "# A\nthree, from elsewhere");
    }

    fn bump(p: &Path) {
        let f = fs::File::options().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn folders_are_made_renamed_and_trashed_with_the_selection_following() {
        let mut s = app("folders");
        s.new_folder();
        s.naming.as_mut().unwrap().edit.replace_text("Work");
        s.commit_naming();
        let work = s.root.join("Work");
        assert!(work.is_dir());
        assert_eq!(s.place, Place::Folder(work.clone()), "the new folder is selected");
        s.new_folder();
        s.naming.as_mut().unwrap().edit.replace_text("Inner");
        s.commit_naming();
        assert!(work.join("Inner").is_dir(), "made inside the selected folder");
        s.new_note();
        type_text(&mut s, "# Deep\n");
        s.save();
        s.start_rename(&work);
        s.naming.as_mut().unwrap().edit.replace_text("Job");
        s.commit_naming();
        let job = s.root.join("Job");
        assert_eq!(s.place, Place::Folder(job.join("Inner")));
        assert!(s.is_open(&job.join("Inner/deep.md")), "the open note follows its folder");
        s.start_rename(&job);
        s.naming.as_mut().unwrap().edit.replace_text("a/b");
        s.commit_naming();
        assert!(s.naming.is_some() && s.status.is_some(), "a bad name keeps the field, says why");
        s.cancel_naming();
        s.trash_folder(&job);
        assert!(!job.exists() && s.root.join(".trash/Job/Inner/deep.md").is_file());
        assert_eq!(s.place, Place::Folder(s.root.clone()));
        assert!(s.open.is_none());
    }

    #[test]
    fn notes_move_and_go_to_the_trash() {
        let mut s = app("notes");
        fs::create_dir(s.root.join("Home")).unwrap();
        fs::write(s.root.join("a.md"), "# A").unwrap();
        fs::write(s.root.join("b.md"), "# B").unwrap();
        s.rescan();
        let (a, b, home) = (s.root.join("a.md"), s.root.join("b.md"), s.root.join("Home"));
        s.open_note(&a);
        s.move_note(&a, &home);
        assert!(s.is_open(&home.join("a.md")));
        assert_eq!(s.tree.root.find(&home).unwrap().count, 1);
        s.search.replace_text("b");
        assert_eq!(s.visible().len(), 1, "search reaches every folder");
        s.search.replace_text("");
        s.open_note(&b);
        s.trash_open();
        assert!(s.root.join(".trash/b.md").is_file());
        assert!(s.open.is_none() && s.visible().is_empty());
    }

    #[test]
    fn ticking_a_task_in_the_preview_edits_the_source() {
        let mut s = app("task");
        s.new_note();
        type_text(&mut s, "- [ ] milk\n- [x] eggs");
        s.set_task(2, true);
        s.set_task(13, false);
        assert_eq!(s.editor.text, "- [x] milk\n- [ ] eggs");
        s.set_task(0, true);
        assert_eq!(s.editor.text, "- [x] milk\n- [ ] eggs", "not a box: unchanged");
    }
}
