//! The palette and type of macOS's own apps, and Lucide icons.
//!
//! Every app's `Palette` holds a [`Base`] and derefs to it, adding only its own colours
//! (Mail's flag orange, Calendar's today red), so `p.text` and `p.accent` are the same
//! everywhere.

use rune::color::hex;
use rune::{Color, Icon, TextStyle};

/// The colours every app uses.
#[derive(Clone, Copy, Debug)]
pub struct Base {
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
    /// Text and icons on the accent colour.
    pub on_selection: Color,
    pub hover: Color,
    pub field: Color,
    /// Behind a sheet.
    pub scrim: Color,
    /// The red of deleting.
    pub danger: Color,
    /// A raised card on the content colour.
    pub card: Color,
    /// The highlight behind selected text.
    pub text_selection: Color,
}

/// A colour from `#rrggbb` or `#rrggbbaa`.
pub fn c(s: &str) -> Color {
    hex(s).expect("palette colours are valid hex")
}

impl Base {
    pub fn new(dark: bool) -> Base {
        if dark {
            Base {
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
            Base {
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

/// A Lucide icon (ISC licence): elements in a 24-point box, stroked in the element's colour.
pub fn lucide(name: &'static str, elements: &[&str]) -> Icon {
    let attrs = r#" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#;
    let body: String = elements.iter().map(|e| e.trim_end_matches("/>").to_owned() + attrs).collect();
    Icon { name, view: 24.0, body: Box::leak(body.into_boxed_str()) }
}

/// Declares Lucide icons as statics, each built on first use:
///
/// ```ignore
/// rune_suite::icons! {
///     SEARCH "notes-search" [r#"<circle cx="11" cy="11" r="8"/>"#, r#"<path d="m21 21-4.3-4.3"/>"#]
/// }
/// ```
#[macro_export]
macro_rules! icons {
    ($($(#[$m:meta])* $id:ident $name:literal [$($el:literal),+ $(,)?])*) => {
        $($(#[$m])* pub static $id: ::std::sync::LazyLock<::rune::Icon> =
            ::std::sync::LazyLock::new(|| $crate::ui::style::lucide($name, &[$($el),+]));)*
    };
}
