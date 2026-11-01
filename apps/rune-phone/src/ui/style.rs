//! The palette (Rune Mail's), type and icons, and the colours of Messages' bubbles and the
//! call buttons.

use std::sync::LazyLock;

use rune::color::hex;
use rune::{Color, Icon, TextStyle};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub window: Color,
    pub sidebar: Color,
    pub content: Color,
    pub separator: Color,
    pub text: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub accent: Color,
    pub on_selection: Color,
    pub hover: Color,
    pub field: Color,
    /// A grouped section's background, as the detail's cards.
    pub card: Color,
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
                content: c("#1e1e1e"),
                separator: c("#ffffff1a"),
                text: c("#f5f5f7"),
                secondary: c("#98989d"),
                tertiary: c("#636366"),
                accent: c("#0a84ff"),
                on_selection: c("#ffffff"),
                hover: c("#ffffff10"),
                field: c("#ffffff14"),
                card: c("#262628"),
            }
        } else {
            Palette {
                dark,
                window: c("#ffffff"),
                sidebar: c("#eeedf0"),
                content: c("#ffffff"),
                separator: c("#0000001a"),
                text: c("#1d1d1f"),
                secondary: c("#86868b"),
                tertiary: c("#b0b0b5"),
                accent: c("#007aff"),
                on_selection: c("#ffffff"),
                hover: c("#0000000a"),
                field: c("#0000000d"),
                card: c("#f5f5f7"),
            }
        }
    }
}

pub static GREEN: LazyLock<Color> = LazyLock::new(|| c("#30d158"));
pub static RED: LazyLock<Color> = LazyLock::new(|| c("#ff3b30"));

/// The bubble of a message received.
pub fn received(p: &Palette) -> Color {
    if p.dark { c("#3a3a3c") } else { c("#e9e9eb") }
}

pub const SECTION: TextStyle = TextStyle::new(11.0, 600.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
pub const BODY_BOLD: TextStyle = TextStyle::new(13.0, 600.0);
pub const SMALL: TextStyle = TextStyle::new(11.5, 400.0);
pub const BUBBLE: TextStyle = TextStyle::new(14.0, 400.0);
pub const NAME: TextStyle = TextStyle::new(15.0, 600.0);
pub const BIG: TextStyle = TextStyle::new(22.0, 700.0);
pub const HUGE: TextStyle = TextStyle::new(30.0, 300.0);
pub const KEY: TextStyle = TextStyle::new(26.0, 400.0);

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
    PHONE "ph-phone" [r#"<path d="M13.832 16.568a1 1 0 0 0 1.213-.303l.355-.465A2 2 0 0 1 17 15h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2A18 18 0 0 1 2 4a2 2 0 0 1 2-2h3a2 2 0 0 1 2 2v3a2 2 0 0 1-.8 1.6l-.468.351a1 1 0 0 0-.292 1.233 14 14 0 0 0 6.392 6.384"/>"#]
    HANG_UP "ph-hang-up" [r#"<path d="M10.1 13.9a14 14 0 0 0 3.732 2.668 1 1 0 0 0 1.213-.303l.355-.465A2 2 0 0 1 17 15h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2 18 18 0 0 1-12.728-5.272"/>"#,
        r#"<path d="M22 2 2 22"/>"#, r#"<path d="M4.76 13.582A18 18 0 0 1 2 4a2 2 0 0 1 2-2h3a2 2 0 0 1 2 2v3a2 2 0 0 1-.8 1.6l-.468.351a1 1 0 0 0-.292 1.233 14 14 0 0 0 .244.473"/>"#]
    MESSAGE "ph-message" [r#"<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>"#]
    PERSON "ph-person" [r#"<circle cx="12" cy="8" r="5"/>"#, r#"<path d="M20 21a8 8 0 0 0-16 0"/>"#]
    KEYPAD "ph-keypad" [r#"<circle cx="6" cy="5" r="1.2"/>"#, r#"<circle cx="12" cy="5" r="1.2"/>"#, r#"<circle cx="18" cy="5" r="1.2"/>"#,
        r#"<circle cx="6" cy="11" r="1.2"/>"#, r#"<circle cx="12" cy="11" r="1.2"/>"#, r#"<circle cx="18" cy="11" r="1.2"/>"#,
        r#"<circle cx="6" cy="17" r="1.2"/>"#, r#"<circle cx="12" cy="17" r="1.2"/>"#, r#"<circle cx="18" cy="17" r="1.2"/>"#, r#"<circle cx="12" cy="22" r="1.2"/>"#]
    MIC "ph-mic" [r#"<path d="M12 19v3"/>"#, r#"<path d="M19 10v2a7 7 0 0 1-14 0v-2"/>"#, r#"<rect x="9" y="2" width="6" height="13" rx="3"/>"#]
    MIC_OFF "ph-mic-off" [r#"<path d="M12 19v3"/>"#, r#"<path d="M15 9.34V5a3 3 0 0 0-5.68-1.33"/>"#, r#"<path d="M16.95 16.95A7 7 0 0 1 5 12v-2"/>"#,
        r#"<path d="M18.89 13.23A7 7 0 0 0 19 12v-2"/>"#, r#"<path d="m2 2 20 20"/>"#, r#"<path d="M9 9v3a3 3 0 0 0 5.12 2.12"/>"#]
    SEND "ph-send" [r#"<path d="m5 12 7-7 7 7"/>"#, r#"<path d="M12 19V5"/>"#]
    COMPOSE "ph-compose" [r#"<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>"#,
        r#"<path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>"#]
    INCOMING "ph-incoming" [r#"<path d="M17 7 7 17"/>"#, r#"<path d="M17 17H7V7"/>"#]
    OUTGOING "ph-outgoing" [r#"<path d="M7 7h10v10"/>"#, r#"<path d="M7 17 17 7"/>"#]
    BLUETOOTH "ph-bluetooth" [r#"<path d="m7 7 10 10-5 5V2l5 5L7 17"/>"#]
    SEARCH "ph-search" [r#"<circle cx="11" cy="11" r="8"/>"#, r#"<path d="m21 21-4.3-4.3"/>"#]
    PIN "ph-pin" [r#"<path d="M12 17v5"/>"#, r#"<path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#]
    TRASH "ph-trash" [r#"<path d="M3 6h18"/>"#, r#"<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>"#, r#"<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#]
    MAIL "ph-mail" [r#"<rect width="20" height="16" x="2" y="4" rx="2"/>"#, r#"<path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/>"#]
    CHECK "ph-check" [r#"<path d="M20 6 9 17l-5-5"/>"#]
    CLOSE "ph-close" [r#"<path d="M18 6 6 18"/>"#, r#"<path d="m6 6 12 12"/>"#]
    MINUS "ph-minus" [r#"<path d="M5 12h14"/>"#]
    PHOTO "ph-photo" [r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/>"#, r#"<circle cx="9" cy="9" r="2"/>"#, r#"<path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#]
}

/// A colour for a person's initials, the same each time for the same name.
pub fn avatar_colour(name: &str) -> Color {
    const COLOURS: [&str; 7] = ["#8e8e93", "#5e5ce6", "#30b0c7", "#34c759", "#ff9f0a", "#ff375f", "#bf5af2"];
    let h = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b.to_ascii_lowercase())));
    c(COLOURS[h as usize % COLOURS.len()])
}
