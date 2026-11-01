//! The palette (Rune Mail's and Rune Calendar's), type and icons.

use std::sync::LazyLock;

use rune::color::hex;
use rune::{Color, Icon, TextStyle};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub window: Color,
    pub sidebar: Color,
    pub sidebar_selection: Color,
    pub content: Color,
    pub separator: Color,
    pub text: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub accent: Color,
    pub on_selection: Color,
    pub hover: Color,
    pub field: Color,
    pub scrim: Color,
    /// The red of deleting.
    pub danger: Color,
    pub card: Color,
    /// The highlight behind selected text in the app's own colours.
    pub text_selection: Color,
}

fn c(s: &str) -> Color {
    hex(s).expect("palette colours are valid hex")
}

impl Palette {
    pub fn new(dark: bool) -> Palette {
        if dark {
            Palette {
                dark,
                window: c("#1e1e1e"),
                sidebar: c("#2a2a2c"),
                sidebar_selection: c("#ffffff1c"),
                content: c("#1e1e1e"),
                separator: c("#ffffff1a"),
                text: c("#f5f5f7"),
                secondary: c("#98989d"),
                tertiary: c("#636366"),
                accent: c("#0a84ff"),
                on_selection: c("#ffffff"),
                hover: c("#ffffff10"),
                field: c("#ffffff14"),
                scrim: c("#00000066"),
                danger: c("#ff453a"),
                card: c("#232325"),
                text_selection: c("#0a84ff59"),
            }
        } else {
            Palette {
                dark,
                window: c("#ffffff"),
                sidebar: c("#eeedf0"),
                sidebar_selection: c("#0000001a"),
                content: c("#ffffff"),
                separator: c("#0000001a"),
                text: c("#1d1d1f"),
                secondary: c("#86868b"),
                tertiary: c("#b0b0b5"),
                accent: c("#007aff"),
                on_selection: c("#ffffff"),
                hover: c("#0000000a"),
                field: c("#0000000d"),
                scrim: c("#00000033"),
                danger: c("#ff3b30"),
                card: c("#ffffff"),
                text_selection: c("#0064e140"),
            }
        }
    }
}

pub const SECTION: TextStyle = TextStyle::new(11.0, 600.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
pub const BODY_BOLD: TextStyle = TextStyle::new(13.0, 600.0);
pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0);
pub const COUNT: TextStyle = TextStyle::new(11.0, 600.0);
pub const TITLE: TextStyle = TextStyle::new(15.0, 700.0);
pub const BIG: TextStyle = TextStyle::new(22.0, 700.0);
/// The note's text, source and preview alike.
pub const NOTE: TextStyle = TextStyle::new(15.0, 400.0).lh(24.0);
/// Heading sizes, levels 1 to 3 (deeper ones as 3).
pub const HEADINGS: [f32; 3] = [26.0, 21.0, 17.5];

/// A Lucide icon (ISC licence): elements in a 24-point box, stroked in the element's colour.
fn lucide(name: &'static str, elements: &[&str]) -> Icon {
    let attrs = r#" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#;
    let body: String = elements.iter().map(|e| e.trim_end_matches("/>").to_owned() + attrs).collect();
    Icon { name, view: 24.0, body: Box::leak(body.into_boxed_str()) }
}

macro_rules! icons {
    ($($id:ident $name:literal [$($el:literal),+ $(,)?])*) => {
        $(pub static $id: LazyLock<Icon> = LazyLock::new(|| lucide($name, &[$($el),+]));)*
    };
}

icons! {
    CHEVRON_RIGHT "notes-chevron-right" [r#"<path d="m9 18 6-6-6-6"/>"#]
    CHEVRON_DOWN "notes-chevron-down" [r#"<path d="m6 9 6 6 6-6"/>"#]
    GEAR "notes-gear" [r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/>"#,
        r#"<circle cx="12" cy="12" r="3"/>"#]
    FOLDER "notes-folder" [r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#]
    FOLDER_PLUS "notes-folder-plus" [r#"<path d="M12 10v6"/>"#, r#"<path d="M9 13h6"/>"#,
        r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#]
    FOLDER_INPUT "notes-folder-input" [r#"<path d="M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-1.73"/>"#,
        r#"<path d="M2 13h10"/>"#, r#"<path d="m9 16 3-3-3-3"/>"#]
    FILES "notes-files" [r#"<path d="M20 7h-3a2 2 0 0 1-2-2V2"/>"#, r#"<path d="M9 18a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h7l4 4v10a2 2 0 0 1-2 2Z"/>"#,
        r#"<path d="M3 7.6v12.8A1.6 1.6 0 0 0 4.6 22h9.8"/>"#]
    FILE_TEXT "notes-file-text" [r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/>"#, r#"<path d="M14 2v4a2 2 0 0 0 2 2h4"/>"#,
        r#"<path d="M10 9H8"/>"#, r#"<path d="M16 13H8"/>"#, r#"<path d="M16 17H8"/>"#]
    NEW_NOTE "notes-square-pen" [r#"<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>"#,
        r#"<path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>"#]
    SEARCH "notes-search" [r#"<circle cx="11" cy="11" r="8"/>"#, r#"<path d="m21 21-4.3-4.3"/>"#]
    X "notes-x" [r#"<path d="M18 6 6 18"/>"#, r#"<path d="m6 6 12 12"/>"#]
    TRASH "notes-trash" [r#"<path d="M3 6h18"/>"#, r#"<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>"#,
        r#"<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#, r#"<line x1="10" x2="10" y1="11" y2="17"/>"#, r#"<line x1="14" x2="14" y1="11" y2="17"/>"#]
    EYE "notes-eye" [r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/>"#,
        r#"<circle cx="12" cy="12" r="3"/>"#]
    PENCIL "notes-pencil" [r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/>"#,
        r#"<path d="m15 5 4 4"/>"#]
}
