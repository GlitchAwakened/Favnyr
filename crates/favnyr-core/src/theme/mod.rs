//! Declarative application themes, independent of the UI toolkit.

mod catalog;
mod palette;
mod registry;
mod roles;
mod schema;

#[cfg(test)]
mod tests;

pub use catalog::{Catalog, Entry, discover, id_is_known, load, user_file_name};
pub use palette::{Palette, Rgba, ThemePalettes};
pub use registry::BuiltinTheme;
pub use roles::ColorRole;
pub use schema::{MAX_THEME_BYTES, parse};

use serde::Deserialize;

/// The base family palette, retained as the fallback for personal themes.
pub fn builtin(scheme: Scheme) -> Palette {
    BuiltinTheme::Nord.palette(scheme)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Light,
    Dark,
}

impl Scheme {
    pub fn preference(self) -> crate::Theme {
        match self {
            Self::Light => crate::Theme::Light,
            Self::Dark => crate::Theme::Dark,
        }
    }
}
