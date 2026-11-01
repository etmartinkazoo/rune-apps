//! Notes on disk: each note a `.md` file, each folder a directory, all under one root.
//!
//! Nothing here is the app's own format: titles come from the text, names are plain file and
//! folder names, and deleting moves things into `.trash` under the root. Names that start with
//! a dot (`.trash`, `.git`, `.obsidian`) are left alone and never shown.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where deleted notes and folders go, under the root.
pub const TRASH: &str = ".trash";

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub path: PathBuf,
    pub title: String,
    /// The first line of the body after the title, for the list.
    pub snippet: String,
    pub modified: SystemTime,
    pub text: String,
}

impl Note {
    /// The folder it's in.
    pub fn folder(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub path: PathBuf,
    pub name: String,
    /// Notes directly inside.
    pub count: usize,
    pub children: Vec<Folder>,
}

impl Folder {
    /// This folder and every one under it, each with its depth, in sidebar order.
    pub fn walk(&self) -> Vec<(&Folder, usize)> {
        let mut out = Vec::new();
        fn go<'a>(f: &'a Folder, depth: usize, out: &mut Vec<(&'a Folder, usize)>) {
            out.push((f, depth));
            for c in &f.children {
                go(c, depth + 1, out);
            }
        }
        go(self, 0, &mut out);
        out
    }

    pub fn find(&self, path: &Path) -> Option<&Folder> {
        self.walk().into_iter().map(|(f, _)| f).find(|f| f.path == path)
    }
}

/// Everything under the root.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub root: Folder,
    /// Newest first.
    pub notes: Vec<Note>,
}

impl Tree {
    pub fn note(&self, path: &Path) -> Option<&Note> {
        self.notes.iter().find(|n| n.path == path)
    }
}

pub fn is_note(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("md" | "markdown"))
}

fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Reads the tree under `root`. Notes whose time and size match one in `known` keep its text
/// rather than being read again, so a re-scan costs a walk of the folders.
pub fn scan(root: &Path, known: &[Note]) -> Tree {
    let known: HashMap<&Path, &Note> = known.iter().map(|n| (n.path.as_path(), n)).collect();
    let mut notes = Vec::new();
    let mut folder = scan_folder(root, &known, &mut notes);
    folder.name = "Notes".into();
    notes.sort_by(|a: &Note, b: &Note| b.modified.cmp(&a.modified).then_with(|| a.title.cmp(&b.title)));
    Tree { root: folder, notes }
}

fn scan_folder(dir: &Path, known: &HashMap<&Path, &Note>, notes: &mut Vec<Note>) -> Folder {
    let name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut folder = Folder { path: dir.to_owned(), name, count: 0, children: Vec::new() };
    let Ok(entries) = fs::read_dir(dir) else { return folder };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if hidden(&name) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.is_dir() {
            folder.children.push(scan_folder(&path, known, notes));
        } else if meta.is_file() && is_note(&path) {
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let text = match known.get(path.as_path()) {
                Some(n) if n.modified == modified && n.text.len() as u64 == meta.len() => n.text.clone(),
                _ => fs::read_to_string(&path).unwrap_or_default(),
            };
            folder.count += 1;
            notes.push(Note { title: title_of(&text, &path), snippet: snippet_of(&text), modified, text, path });
        }
    }
    folder.children.sort_by_key(|f| f.name.to_lowercase());
    folder
}

/// The text after any YAML front matter (`---` … `---` at the top), as other editors write it.
fn body(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return text };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        if line.trim_end() == "---" {
            return &rest[at..];
        }
    }
    text
}

/// A heading's text without its `#`s, or None for other lines.
pub fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.bytes().take_while(|&b| b == b'#').count();
    let rest = &t[level..];
    ((1..=6).contains(&level) && (rest.is_empty() || rest.starts_with([' ', '\t']))).then(|| (level, rest.trim().trim_end_matches('#').trim_end()))
}

/// A note's title: its first `# heading`, else its first line, else its file name.
pub fn title_of(text: &str, path: &Path) -> String {
    let body = body(text);
    let first_h1 = body.lines().find_map(|l| heading(l).filter(|(level, t)| *level == 1 && !t.is_empty()).map(|(_, t)| t));
    let first_line = || body.lines().map(plain).find(|l| !l.is_empty());
    match first_h1.map(plain).or_else(first_line) {
        Some(t) => clip(&t, 120),
        None => path.file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned()),
    }
}

/// The first line with words in it after the title line, for the list.
pub fn snippet_of(text: &str) -> String {
    let body = body(text);
    let mut lines = body.lines().map(plain).filter(|l| !l.is_empty());
    let title = body.lines().find_map(|l| heading(l).filter(|(level, t)| *level == 1 && !t.is_empty()).map(|(_, t)| plain(t)));
    let first = lines.next();
    let line = match (&title, &first) {
        (Some(t), Some(f)) if t != f => first,
        _ => lines.next(),
    };
    line.map_or_else(String::new, |l| clip(&l, 160))
}

/// A line without the Markdown that marks it up: heading `#`s, list and quote markers, task
/// boxes, emphasis and code ticks.
pub fn plain(line: &str) -> String {
    let mut t = line.trim();
    if let Some((_, h)) = heading(t) {
        t = h;
    }
    loop {
        let before = t;
        for marker in ["> ", "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "- ", "* ", "+ "] {
            t = t.strip_prefix(marker).unwrap_or(t).trim_start();
        }
        if let Some(i) = t.find(". ").filter(|&i| i > 0 && i <= 3 && t[..i].bytes().all(|b| b.is_ascii_digit())) {
            t = t[i + 2..].trim_start();
        }
        if t == before {
            break;
        }
    }
    if t.chars().all(|c| matches!(c, '-' | '*' | '_' | '=' | '`' | ' ')) {
        return String::new();
    }
    t.replace("**", "").replace("__", "").replace('`', "")
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", s[..i].trim_end()),
        None => s.to_owned(),
    }
}

/// A file name for `title`: lower case, words joined with dashes, at most 60 characters.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(60).collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() { "untitled".into() } else { out.to_owned() }
}

/// `dir/stem.ext`, or `dir/stem-2.ext` and on when that's taken. An empty `ext` is a folder,
/// and numbers with a space: `Name 2`.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |n: usize| match (n, ext.is_empty()) {
        (1, true) => stem.to_owned(),
        (1, false) => format!("{stem}.{ext}"),
        (n, true) => format!("{stem} {n}"),
        (n, false) => format!("{stem}-{n}.{ext}"),
    };
    (1..).map(|n| dir.join(name(n))).find(|p| fs::symlink_metadata(p).is_err()).expect("some name is free")
}

/// Writes `text` to `path` whole or not at all: into a temporary file beside it, then renamed
/// over it. Returns the file's new modified time.
pub fn save_atomic(path: &Path, text: &str) -> io::Result<SystemTime> {
    let dir = path.parent().ok_or_else(|| io::Error::other("no folder"))?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = written.and_then(|_| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::metadata(path)?.modified()
}

/// Writes a new note in `dir`, named after its title.
pub fn create_note(dir: &Path, text: &str) -> io::Result<PathBuf> {
    let title = title_of(text, Path::new("untitled.md"));
    let path = unique_path(dir, &slug(&title), "md");
    save_atomic(&path, text)?;
    Ok(path)
}

/// A folder name as typed, or why it can't be one.
pub fn folder_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A folder needs a name.".into());
    }
    if name.contains(['/', '\\']) || name == ".." {
        return Err("A folder name can't have a slash in it.".into());
    }
    if name.starts_with('.') {
        return Err("A folder name can't start with a dot.".into());
    }
    Ok(name)
}

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let name = folder_name(name)?;
    let path = parent.join(name);
    if path.exists() {
        return Err(format!("There's already a folder called “{name}” there."));
    }
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn rename_folder(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = folder_name(name)?;
    let to = path.with_file_name(name);
    if to == path {
        return Ok(to);
    }
    // A change of case alone is a rename, not a clash, on a case-insensitive disk.
    if to.exists() && !path.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy()) {
        return Err(format!("There's already a folder called “{name}” there."));
    }
    fs::rename(path, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

/// Moves a note or folder into `dir`, keeping its name unless that's taken there.
pub fn move_into(path: &Path, dir: &Path) -> io::Result<PathBuf> {
    if dir.starts_with(path) {
        return Err(io::Error::other("A folder can't go inside itself."));
    }
    if path.parent() == Some(dir) {
        return Ok(path.to_owned());
    }
    let to = free_name(path, dir);
    fs::create_dir_all(dir)?;
    fs::rename(path, &to)?;
    Ok(to)
}

/// `path`'s name in `dir`, numbered when taken.
fn free_name(path: &Path, dir: &Path) -> PathBuf {
    let stem = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    match path.extension().filter(|_| path.is_file()) {
        Some(ext) => unique_path(dir, &stem, &ext.to_string_lossy()),
        None => unique_path(dir, &path.file_name().map_or(stem.clone(), |n| n.to_string_lossy().into_owned()), ""),
    }
}

/// Moves a note or folder into the root's `.trash`.
pub fn trash(root: &Path, path: &Path) -> io::Result<PathBuf> {
    move_into(path, &root.join(TRASH))
}

/// `path` with its `from` prefix swapped for `to`, when it's under `from`.
pub fn rebase(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| if rest.as_os_str().is_empty() { to.to_owned() } else { to.join(rest) })
}

/// A copy of the note at `path` beside it, for when it changed on disk while edited here.
pub fn keep_copy(path: &Path) -> io::Result<PathBuf> {
    let stem = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let to = unique_path(path.parent().unwrap_or(Path::new(".")), &format!("{stem} (other copy)"), "md");
    fs::copy(path, &to)?;
    Ok(to)
}

/// For Return in the editor: what the next line starts with to carry on a list (`- `, `* `,
/// `1. ` counting on, `- [ ] `, quote `> `), with the line's indent. `Some("")` when the line is
/// an empty item, which Return ends.
pub fn continue_list(line: &str) -> Option<String> {
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let t = &line[indent.len()..];
    let (marker, rest) = if let Some(r) = ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] "].iter().find_map(|m| t.strip_prefix(m).map(|r| (&m[..2], r))) {
        (format!("{}[ ] ", r.0), r.1)
    } else if let Some(r) = ["- ", "* ", "+ ", "> "].iter().find_map(|m| t.strip_prefix(m).map(|r| (*m, r))) {
        (r.0.to_owned(), r.1)
    } else {
        let digits = t.bytes().take_while(u8::is_ascii_digit).count();
        let rest = t[digits..].strip_prefix(". ")?;
        if digits == 0 || digits > 3 {
            return None;
        }
        let n: u32 = t[..digits].parse().ok()?;
        (format!("{}. ", n + 1), rest)
    };
    Some(if rest.trim().is_empty() { String::new() } else { format!("{indent}{marker}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty folder for one test.
    pub fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rune-notes-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn titles_come_from_the_first_heading_then_line_then_name() {
        let p = Path::new("/n/shopping-list.md");
        assert_eq!(title_of("intro\n# Groceries\n- milk", p), "Groceries");
        assert_eq!(title_of("## Sub first\nbody", p), "Sub first", "no # heading: the first line, without its marks");
        assert_eq!(title_of("\n\n  Just text  \nmore", p), "Just text");
        assert_eq!(title_of("", p), "shopping-list");
        assert_eq!(title_of("---\ntags: [a]\n---\n# Real\n", p), "Real", "front matter is skipped");
        assert_eq!(title_of("#hashtag line", p), "#hashtag line", "a hashtag isn't a heading");
        assert_eq!(title_of("# **Bold** title #", p), "Bold title");
    }

    #[test]
    fn snippets_skip_the_title() {
        assert_eq!(snippet_of("# Groceries\n\n- milk\n- eggs"), "milk");
        assert_eq!(snippet_of("First line\nSecond line"), "Second line");
        assert_eq!(snippet_of("# Only a title"), "");
        assert_eq!(snippet_of("# T\n---\n> quoted **words**"), "quoted words");
        assert_eq!(snippet_of("# T\n1. one\n"), "one");
    }

    #[test]
    fn slugs_and_unique_names() {
        assert_eq!(slug("Meeting notes: Q3 / Planning!"), "meeting-notes-q3-planning");
        assert_eq!(slug("  Café über  "), "café-über");
        assert_eq!(slug("!!!"), "untitled");
        assert_eq!(slug(&"word ".repeat(30)).len(), 59, "clipped, without a trailing dash");
        let dir = temp("unique");
        assert_eq!(unique_path(&dir, "a", "md"), dir.join("a.md"));
        fs::write(dir.join("a.md"), "").unwrap();
        fs::write(dir.join("a-2.md"), "").unwrap();
        assert_eq!(unique_path(&dir, "a", "md"), dir.join("a-3.md"));
        fs::create_dir(dir.join("Work")).unwrap();
        assert_eq!(unique_path(&dir, "Work", ""), dir.join("Work 2"));
    }

    #[test]
    fn saves_atomically_and_creates_notes_named_by_title() {
        let dir = temp("save");
        let path = create_note(&dir, "# Trip to Rome\nPack light").unwrap();
        assert_eq!(path, dir.join("trip-to-rome.md"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Trip to Rome\nPack light");
        let again = create_note(&dir.join("sub"), "# Trip to Rome").unwrap();
        assert_eq!(again, dir.join("sub/trip-to-rome.md"), "the folder is made when missing");
        assert_eq!(create_note(&dir, "# Trip to Rome").unwrap(), dir.join("trip-to-rome-2.md"));
        let t = save_atomic(&path, "replaced").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "replaced");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), t);
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty(), "no temp file left behind");
    }

    #[test]
    fn scans_folders_and_notes_skipping_hidden_ones() {
        let dir = temp("scan");
        fs::create_dir_all(dir.join("Work/Projects")).unwrap();
        fs::create_dir_all(dir.join(".trash")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("todo.md"), "# Todo\n- one").unwrap();
        fs::write(dir.join("Work/plan.md"), "Plan\nsteps").unwrap();
        fs::write(dir.join("Work/Projects/x.markdown"), "# X").unwrap();
        fs::write(dir.join("Work/image.png"), "not a note").unwrap();
        fs::write(dir.join(".trash/old.md"), "# Old").unwrap();
        let tree = scan(&dir, &[]);
        assert_eq!(tree.root.name, "Notes");
        assert_eq!(tree.root.count, 1);
        let names: Vec<(String, usize, usize)> = tree.root.walk().iter().map(|(f, d)| (f.name.clone(), *d, f.count)).collect();
        assert_eq!(names, [("Notes".into(), 0, 1), ("Work".into(), 1, 1), ("Projects".into(), 2, 1)]);
        let mut titles: Vec<&str> = tree.notes.iter().map(|n| n.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["Plan", "Todo", "X"]);
        assert_eq!(tree.note(&dir.join("Work/plan.md")).unwrap().folder(), dir.join("Work"));
        // A re-scan with nothing changed is the same tree.
        assert_eq!(scan(&dir, &tree.notes), tree);
        fs::write(dir.join("todo.md"), "# Todo, edited elsewhere\n").unwrap();
        let again = scan(&dir, &tree.notes);
        assert_eq!(again.note(&dir.join("todo.md")).unwrap().title, "Todo, edited elsewhere");
    }

    #[test]
    fn creates_renames_moves_and_trashes() {
        let dir = temp("ops");
        let work = create_folder(&dir, " Work ").unwrap();
        assert_eq!(work, dir.join("Work"));
        assert!(create_folder(&dir, "Work").is_err(), "taken");
        assert!(create_folder(&dir, "a/b").is_err());
        assert!(create_folder(&dir, ".hidden").is_err());
        assert!(create_folder(&dir, "  ").is_err());
        let inner = create_folder(&work, "Inner").unwrap();
        let note = create_note(&inner, "# Deep").unwrap();

        let job = rename_folder(&work, "Job").unwrap();
        assert!(job.join("Inner/deep.md").is_file());
        assert_eq!(rebase(&note, &work, &job).unwrap(), job.join("Inner/deep.md"));
        assert_eq!(rebase(&work, &work, &job).unwrap(), job);
        assert_eq!(rebase(&dir.join("other"), &work, &job), None);
        let home = create_folder(&dir, "Home").unwrap();
        assert!(rename_folder(&job, "Home").is_err(), "taken");

        let moved = move_into(&job.join("Inner/deep.md"), &home).unwrap();
        assert_eq!(moved, home.join("deep.md"));
        create_note(&job, "# Deep").unwrap();
        assert_eq!(move_into(&job.join("deep.md"), &home).unwrap(), home.join("deep-2.md"), "renamed when taken");
        assert!(move_into(&job, &job.join("Inner")).is_err(), "not into itself");
        assert_eq!(move_into(&moved, &home).unwrap(), moved, "already there");

        let gone = trash(&dir, &moved).unwrap();
        assert_eq!(gone, dir.join(".trash/deep.md"));
        let gone_folder = trash(&dir, &job).unwrap();
        assert_eq!(gone_folder, dir.join(".trash/Job"));
        assert!(gone_folder.join("Inner").is_dir(), "a folder goes with what's in it");
        let job2 = create_folder(&dir, "Job").unwrap();
        assert_eq!(trash(&dir, &job2).unwrap(), dir.join(".trash/Job 2"));
        assert!(scan(&dir, &[]).notes.iter().all(|n| !n.path.starts_with(dir.join(".trash"))));
    }

    #[test]
    fn keeps_a_copy_beside() {
        let dir = temp("copy");
        let p = dir.join("a.md");
        fs::write(&p, "theirs").unwrap();
        let c = keep_copy(&p).unwrap();
        assert_eq!(c, dir.join("a (other copy).md"));
        assert_eq!(fs::read_to_string(c).unwrap(), "theirs");
    }

    #[test]
    fn lists_carry_on() {
        assert_eq!(continue_list("- milk").as_deref(), Some("- "));
        assert_eq!(continue_list("  * eggs").as_deref(), Some("  * "));
        assert_eq!(continue_list("- [x] done").as_deref(), Some("- [ ] "));
        assert_eq!(continue_list("9. nine").as_deref(), Some("10. "));
        assert_eq!(continue_list("> said").as_deref(), Some("> "));
        assert_eq!(continue_list("- ").as_deref(), Some(""), "an empty item ends the list");
        assert_eq!(continue_list("plain"), None);
        assert_eq!(continue_list("2024. was a year"), None);
    }
}
