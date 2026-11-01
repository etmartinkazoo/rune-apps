//! The palette (Rune Mail's and Rune Calendar's), type, icons and the colours of Apple
//! Passwords' categories.

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
    /// A grouped section's background, as the detail's cards.
    pub card: Color,
    /// The tiles of the sidebar's categories.
    pub tile: Color,
    pub danger: Color,
}

pub fn c(s: &str) -> Color {
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
                card: c("#262628"),
                tile: c("#ffffff12"),
                danger: c("#ff453a"),
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
                card: c("#f5f5f7"),
                tile: c("#0000000d"),
                danger: c("#ff3b30"),
            }
        }
    }
}

pub static BLUE: LazyLock<Color> = LazyLock::new(|| c("#0a84ff"));
pub static GREEN: LazyLock<Color> = LazyLock::new(|| c("#30d158"));
pub static YELLOW: LazyLock<Color> = LazyLock::new(|| c("#ffcc00"));
pub static ORANGE: LazyLock<Color> = LazyLock::new(|| c("#ff9f0a"));
pub static RED: LazyLock<Color> = LazyLock::new(|| c("#ff453a"));
pub static GREY: LazyLock<Color> = LazyLock::new(|| c("#8e8e93"));
pub static PURPLE: LazyLock<Color> = LazyLock::new(|| c("#bf5af2"));

pub const SECTION: TextStyle = TextStyle::new(11.0, 600.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
pub const BODY_BOLD: TextStyle = TextStyle::new(13.0, 600.0);
pub const SMALL: TextStyle = TextStyle::new(11.5, 400.0);
pub const TILE_LABEL: TextStyle = TextStyle::new(13.0, 600.0);
pub const COUNT: TextStyle = TextStyle::new(15.0, 600.0);
/// A swiped row's button labels.
pub const TINY: TextStyle = TextStyle::new(10.5, 500.0);
pub const NAME: TextStyle = TextStyle::new(19.0, 700.0);
pub const BIG: TextStyle = TextStyle::new(22.0, 700.0);
pub const CODE: TextStyle = TextStyle::new(17.0, 600.0);
pub const MONO: TextStyle = TextStyle::new(13.0, 400.0);

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
    KEY "pw-key" [r#"<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"/>"#,
        r#"<circle cx="16.5" cy="7.5" r=".5"/>"#]
    LOCK "pw-lock" [r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/>"#, r#"<path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#]
    PERSON "pw-person" [r#"<circle cx="12" cy="8" r="5"/>"#, r#"<path d="M20 21a8 8 0 0 0-16 0"/>"#]
    CLOCK "pw-clock" [r#"<circle cx="12" cy="12" r="10"/>"#, r#"<path d="M12 6v6l4 2"/>"#]
    NOTE "pw-note" [r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/>"#, r#"<path d="M14 2v4a2 2 0 0 0 2 2h4"/>"#,
        r#"<path d="M16 13H8"/>"#, r#"<path d="M16 17H8"/>"#]
    SHIELD "pw-shield" [r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#,
        r#"<path d="M12 8v4"/>"#, r#"<path d="M12 16h.01"/>"#]
    SHIELD_OK "pw-shield-ok" [r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#,
        r#"<path d="m9 12 2 2 4-4"/>"#]
    TRASH "pw-trash" [r#"<path d="M3 6h18"/>"#, r#"<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>"#, r#"<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#]
    FOLDER "pw-folder" [r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#]
    PEOPLE "pw-people" [r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/>"#, r#"<circle cx="9" cy="7" r="4"/>"#,
        r#"<path d="M22 21v-2a4 4 0 0 0-3-3.87"/>"#, r#"<path d="M16 3.13a4 4 0 0 1 0 7.75"/>"#]
    PLUS "pw-plus" [r#"<path d="M5 12h14"/>"#, r#"<path d="M12 5v14"/>"#]
    SEARCH "pw-search" [r#"<circle cx="11" cy="11" r="8"/>"#, r#"<path d="m21 21-4.3-4.3"/>"#]
    EYE "pw-eye" [r#"<path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/>"#, r#"<circle cx="12" cy="12" r="3"/>"#]
    SPARKLES "pw-sparkles" [r#"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>"#]
    REFRESH "pw-refresh" [r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/>"#, r#"<path d="M21 3v5h-5"/>"#,
        r#"<path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/>"#, r#"<path d="M8 16H3v5"/>"#]
    GEAR "pw-gear" [r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/>"#,
        r#"<circle cx="12" cy="12" r="3"/>"#]
    CHEVRONS "pw-chevrons" [r#"<path d="m7 15 5 5 5-5"/>"#, r#"<path d="m7 9 5-5 5 5"/>"#]
    CHECK "pw-check" [r#"<path d="M20 6 9 17l-5-5"/>"#]
    CARD "pw-card" [r#"<rect width="20" height="14" x="2" y="5" rx="2"/>"#, r#"<path d="M2 10h20"/>"#]
}

/// A colour for a site's or item's letter tile, the same each time for the same name.
pub fn tile_colour(name: &str) -> Color {
    const COLOURS: [&str; 8] = ["#0a84ff", "#30b0c7", "#34c759", "#ff9f0a", "#ff375f", "#bf5af2", "#5e5ce6", "#a2845e"];
    let h = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b.to_ascii_lowercase())));
    c(COLOURS[h as usize % COLOURS.len()])
}
