//! The palette (Rune Mail's), type, icons and calendar colours.

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
    /// The faint tint of weekends and days outside the month.
    pub shade: Color,
    /// The red of today and of the line for now.
    pub today: Color,
    pub card: Color,
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
                today: c("#ff453a"),
                shade: c("#ffffff07"),
                card: c("#232325"),
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
                today: c("#ff3b30"),
                shade: c("#00000006"),
                card: c("#ffffff"),
            }
        }
    }
}

/// Replies: accepted, maybe.
pub static GREEN: LazyLock<Color> = LazyLock::new(|| c("#34c759"));
pub static ORANGE: LazyLock<Color> = LazyLock::new(|| c("#ff9f0a"));

pub const SECTION: TextStyle = TextStyle::new(11.0, 600.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
pub const BODY_BOLD: TextStyle = TextStyle::new(13.0, 600.0);
pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0);
pub const COUNT: TextStyle = TextStyle::new(11.0, 600.0);
pub const TITLE: TextStyle = TextStyle::new(15.0, 700.0);
pub const BIG: TextStyle = TextStyle::new(22.0, 700.0);
pub const EVENT: TextStyle = TextStyle::new(11.5, 500.0);
pub const EVENT_TIME: TextStyle = TextStyle::new(11.0, 400.0);

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
    CHEVRON_LEFT "cal-chevron-left" [r#"<path d="m15 18-6-6 6-6"/>"#]
    CHEVRON_RIGHT "cal-chevron-right" [r#"<path d="m9 18 6-6-6-6"/>"#]
    CHEVRON_UP "cal-chevron-up" [r#"<path d="m18 15-6-6-6 6"/>"#]
    CHEVRON_DOWN "cal-chevron-down" [r#"<path d="m6 9 6 6 6-6"/>"#]
    REFRESH "cal-refresh" [r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/>"#, r#"<path d="M21 3v5h-5"/>"#,
        r#"<path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/>"#, r#"<path d="M8 16H3v5"/>"#]
    GEAR "cal-gear" [r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/>"#,
        r#"<circle cx="12" cy="12" r="3"/>"#]
    CHECK "cal-check" [r#"<path d="M20 6 9 17l-5-5"/>"#]
    X "cal-x" [r#"<path d="M18 6 6 18"/>"#, r#"<path d="m6 6 12 12"/>"#]
    HELP "cal-help" [r#"<path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/>"#, r#"<path d="M12 17h.01"/>"#]
    DOT "cal-dot" [r#"<circle cx="12" cy="12" r="3"/>"#]
    VIDEO "cal-video" [r#"<path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/>"#,
        r#"<rect x="2" y="6" width="14" height="12" rx="2"/>"#]
    REPEAT "cal-repeat" [r#"<path d="m17 2 4 4-4 4"/>"#, r#"<path d="M3 11v-1a4 4 0 0 1 4-4h14"/>"#,
        r#"<path d="m7 22-4-4 4-4"/>"#, r#"<path d="M21 13v1a4 4 0 0 1-4 4H3"/>"#]
}

/// Colours for calendars that don't name one, as Apple Calendar's.
const COLOURS: [&str; 7] = ["#007aff", "#34c759", "#ff9500", "#af52de", "#ff2d55", "#5ac8fa", "#a2845e"];

/// A calendar's colour: its own, else one picked from its address.
pub fn calendar_colour(href: &str, own: Option<&str>) -> Color {
    own.and_then(hex).unwrap_or_else(|| {
        let h = href.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        c(COLOURS[h as usize % COLOURS.len()])
    })
}
