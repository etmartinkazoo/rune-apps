//! The look every app shares: [`style`] (palette, type, icons), [`field`] (text fields that
//! edit the platform's way) and [`controls`] (buttons, switches, segmented controls, sheets).

pub mod controls;
pub mod field;
pub mod style;

pub use controls::{button, section, segmented, setting, sheet, switch, tool};
pub use field::{boxed, input, platform_key};
pub use style::{BODY, BODY_BOLD, Base, COUNT, SECTION, SMALL, TITLE};
