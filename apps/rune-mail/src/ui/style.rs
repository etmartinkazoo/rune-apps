//! Apple Mail's palette, type and icons.

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
    pub selection: Color,
    pub on_selection: Color,
    pub hover: Color,
    pub field: Color,
    pub flag: Color,
    pub scrim: Color,
    /// Behind the reader's message cards, and the cards themselves.
    pub reader_bg: Color,
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
                selection: c("#0a5fd1"),
                on_selection: c("#ffffff"),
                hover: c("#ffffff10"),
                field: c("#ffffff14"),
                flag: c("#ff9f0a"),
                scrim: c("#00000066"),
                reader_bg: c("#161618"),
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
                selection: c("#0064e1"),
                on_selection: c("#ffffff"),
                hover: c("#0000000a"),
                field: c("#0000000d"),
                flag: c("#ff9500"),
                scrim: c("#00000033"),
                reader_bg: c("#f2f2f5"),
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
pub const READER: TextStyle = TextStyle::new(13.5, 400.0).lh(20.0);

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
    INBOX "mail-inbox" [r#"<path d="M22 12h-6l-2 3h-4l-2-3H2"/>"#,
        r#"<path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>"#]
    SENT "mail-sent" [r#"<path d="m22 2-7 20-4-9-9-4Z"/>"#, r#"<path d="M22 2 11 13"/>"#]
    DRAFTS "mail-drafts" [r#"<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/>"#,
        r#"<path d="M14 2v6h6"/>"#]
    ARCHIVE "mail-archive" [r#"<rect width="20" height="5" x="2" y="3" rx="1"/>"#,
        r#"<path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/>"#, r#"<path d="M10 12h4"/>"#]
    TRASH "mail-trash" [r#"<path d="M3 6h18"/>"#, r#"<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>"#,
        r#"<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#]
    JUNK "mail-junk" [r#"<path d="M7.86 2h8.28L22 7.86v8.28L16.14 22H7.86L2 16.14V7.86L7.86 2z"/>"#,
        r#"<path d="m15 9-6 6"/>"#, r#"<path d="m9 9 6 6"/>"#]
    FOLDER "mail-folder" [r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#]
    COMPOSE "mail-compose" [r#"<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>"#,
        r#"<path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>"#]
    REPLY "mail-reply" [r#"<path d="M9 17 4 12l5-5"/>"#, r#"<path d="M20 18v-2a4 4 0 0 0-4-4H4"/>"#]
    REPLY_ALL "mail-reply-all" [r#"<path d="M7 17 2 12l5-5"/>"#, r#"<path d="M12 17 7 12l5-5"/>"#,
        r#"<path d="M22 18v-2a4 4 0 0 0-4-4H7"/>"#]
    FORWARD "mail-forward" [r#"<path d="m15 17 5-5-5-5"/>"#, r#"<path d="M4 18v-2a4 4 0 0 1 4-4h12"/>"#]
    FLAG "mail-flag" [r#"<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/>"#, r#"<path d="M4 22v-7"/>"#]
    ENVELOPE "mail-envelope" [r#"<rect width="20" height="16" x="2" y="4" rx="2"/>"#,
        r#"<path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/>"#]
    SEARCH "mail-search" [r#"<circle cx="11" cy="11" r="8"/>"#, r#"<path d="m21 21-4.3-4.3"/>"#]
    SUN "mail-sun" [r#"<circle cx="12" cy="12" r="4"/>"#, r#"<path d="M12 2v2"/>"#, r#"<path d="M12 20v2"/>"#,
        r#"<path d="m4.93 4.93 1.41 1.41"/>"#, r#"<path d="m17.66 17.66 1.41 1.41"/>"#, r#"<path d="M2 12h2"/>"#,
        r#"<path d="M20 12h2"/>"#, r#"<path d="m6.34 17.66-1.41 1.41"/>"#, r#"<path d="m19.07 4.93-1.41 1.41"/>"#]
    MOON "mail-moon" [r#"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>"#]
    CHECK "mail-check" [r#"<path d="M20 6 9 17l-5-5"/>"#]
    GEAR "mail-gear" [r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/>"#,
        r#"<circle cx="12" cy="12" r="3"/>"#]
    KEYBOARD "mail-keyboard" [r#"<rect width="20" height="16" x="2" y="4" rx="2"/>"#, r#"<path d="M6 8h.01"/>"#,
        r#"<path d="M10 8h.01"/>"#, r#"<path d="M14 8h.01"/>"#, r#"<path d="M18 8h.01"/>"#, r#"<path d="M8 12h8"/>"#,
        r#"<path d="M7 16h10"/>"#]
    CALENDAR "mail-calendar" [r#"<rect width="18" height="18" x="3" y="4" rx="2"/>"#, r#"<path d="M16 2v4"/>"#,
        r#"<path d="M8 2v4"/>"#, r#"<path d="M3 10h18"/>"#]
    UNSUBSCRIBE "mail-unsubscribe" [r#"<path d="M22 13V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v12c0 1.1.9 2 2 2h9"/>"#,
        r#"<path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/>"#, r#"<path d="m17 17 4 4"/>"#, r#"<path d="m21 17-4 4"/>"#]
    SHIELD "mail-shield" [r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#]
    CHEVRON_UP "mail-chevron-up" [r#"<path d="m18 15-6-6-6 6"/>"#]
    CHEVRON_DOWN "mail-chevron-down" [r#"<path d="m6 9 6 6 6-6"/>"#]
    CHEVRON_RIGHT "mail-chevron-right" [r#"<path d="m9 18 6-6-6-6"/>"#]
    REFRESH "mail-refresh" [r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/>"#, r#"<path d="M21 3v5h-5"/>"#,
        r#"<path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/>"#, r#"<path d="M8 16H3v5"/>"#]
    SPARKLES "mail-sparkles" [r#"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>"#]
    IMAGE "mail-image" [r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#, r#"<circle cx="9" cy="9" r="2"/>"#,
        r#"<path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#]
    DOWNLOAD "mail-download" [r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>"#, r#"<path d="m7 10 5 5 5-5"/>"#,
        r#"<path d="M12 15V3"/>"#]
    CLIP "mail-clip" [r#"<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>"#]
}

pub fn mailbox_icon(role: Option<&str>) -> &'static Icon {
    match role {
        Some("inbox") => &INBOX,
        Some("sent") => &SENT,
        Some("drafts") => &DRAFTS,
        Some("archive") => &ARCHIVE,
        Some("trash") => &TRASH,
        Some("junk") => &JUNK,
        _ => &FOLDER,
    }
}
