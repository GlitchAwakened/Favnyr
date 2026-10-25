use super::palette::ThemePalettes;
use super::{Palette, Scheme};
use std::sync::OnceLock;

/// Built-in theme families, each providing a light and a dark palette.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinTheme {
    #[default]
    Nord,
    Atom,
    CatppuccinMocha,
    CatppuccinMacchiato,
    CatppuccinFrappe,
    TokyoNight,
    Fluvy,
    Everforest,
    SonokaiAtlantis,
    RosePine,
    GruvboxHard,
}

/// Immutable metadata and a lazily initialized palette cache for one family.
struct BuiltinDefinition {
    stored_id: &'static str,
    label_key: &'static str,
    source: &'static str,
    palettes: OnceLock<ThemePalettes>,
}

impl BuiltinDefinition {
    const fn new(stored_id: &'static str, label_key: &'static str, source: &'static str) -> Self {
        Self {
            stored_id,
            label_key,
            source,
            palettes: OnceLock::new(),
        }
    }
}

impl BuiltinTheme {
    /// Display order of the theme selector; saved choices use identifiers.
    pub const ALL: &'static [Self] = &[
        Self::Nord,
        Self::Atom,
        Self::TokyoNight,
        Self::SonokaiAtlantis,
        Self::CatppuccinFrappe,
        Self::CatppuccinMacchiato,
        Self::CatppuccinMocha,
        Self::RosePine,
        Self::Everforest,
        Self::GruvboxHard,
        Self::Fluvy,
    ];

    /// Existing identifiers remain stable independently of display names.
    pub fn id(self) -> Option<&'static str> {
        if self == Self::Nord {
            None
        } else {
            Some(self.stored_id())
        }
    }

    /// The identifier a stored choice uses to name this family. The base
    /// family answers the alias it has always accepted, because a choice that
    /// names it must stay distinguishable from the absence of a choice, which
    /// is what [`Self::id`] returning `None` means.
    pub fn stored_id(self) -> &'static str {
        self.definition().stored_id
    }

    pub fn from_id(id: Option<&str>) -> Option<Self> {
        match id {
            None => Some(Self::Nord),
            Some(id) => Self::ALL
                .iter()
                .copied()
                .find(|family| family.stored_id() == id),
        }
    }

    pub fn label_key(self) -> &'static str {
        self.definition().label_key
    }

    pub fn palette(self, scheme: Scheme) -> Palette {
        self.palettes().variant(scheme).clone()
    }

    /// The palette file bundled into the executable. It is an ordinary theme
    /// file, so copying one into the themes directory reproduces the theme.
    pub(crate) fn source(self) -> &'static str {
        self.definition().source
    }

    // Each family is parsed once on demand, along with its Nord baseline.
    pub(crate) fn palettes(self) -> &'static ThemePalettes {
        self.definition().palettes.get_or_init(|| {
            // The base family is the one file that cannot inherit; the others
            // are read exactly like a theme file found on disk.
            if self == Self::Nord {
                super::schema::parse_base(self.source())
                    .expect("the embedded base theme must be valid and complete")
            } else {
                super::schema::parse(self.source()).expect("an embedded theme must be valid")
            }
        })
    }

    // Keep the match exhaustive and the definitions static: metadata access
    // neither parses palettes nor creates a new cache.
    fn definition(self) -> &'static BuiltinDefinition {
        match self {
            Self::Nord => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:base",
                    "theme_nord",
                    include_str!("builtins/nord.toml"),
                );
                &DEFINITION
            }
            Self::Atom => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:atom",
                    "theme_atom",
                    include_str!("builtins/atom.toml"),
                );
                &DEFINITION
            }
            Self::CatppuccinMocha => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:catppuccin",
                    "theme_catppuccin_mocha",
                    include_str!("builtins/catppuccin_mocha.toml"),
                );
                &DEFINITION
            }
            Self::CatppuccinMacchiato => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:catppuccin-macchiato",
                    "theme_catppuccin_macchiato",
                    include_str!("builtins/catppuccin_macchiato.toml"),
                );
                &DEFINITION
            }
            Self::CatppuccinFrappe => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:catppuccin-frappe",
                    "theme_catppuccin_frappe",
                    include_str!("builtins/catppuccin_frappe.toml"),
                );
                &DEFINITION
            }
            Self::TokyoNight => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:tokyo-night",
                    "theme_tokyo_night",
                    include_str!("builtins/tokyo_night.toml"),
                );
                &DEFINITION
            }
            Self::Fluvy => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:fluvy",
                    "theme_fluvy",
                    include_str!("builtins/fluvy.toml"),
                );
                &DEFINITION
            }
            Self::Everforest => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:everforest",
                    "theme_everforest",
                    include_str!("builtins/everforest.toml"),
                );
                &DEFINITION
            }
            Self::SonokaiAtlantis => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:sonokai-atlantis",
                    "theme_sonokai_atlantis",
                    include_str!("builtins/sonokai_atlantis.toml"),
                );
                &DEFINITION
            }
            Self::RosePine => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:rose-pine",
                    "theme_rose_pine",
                    include_str!("builtins/rose_pine.toml"),
                );
                &DEFINITION
            }
            Self::GruvboxHard => {
                static DEFINITION: BuiltinDefinition = BuiltinDefinition::new(
                    "builtin:gruvbox-hard",
                    "theme_gruvbox_hard",
                    include_str!("builtins/gruvbox_hard.toml"),
                );
                &DEFINITION
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_families_have_stable_identities_and_both_appearances() {
        assert_eq!(BuiltinTheme::from_id(None), Some(BuiltinTheme::Nord));
        assert_eq!(
            BuiltinTheme::from_id(Some("builtin:base")),
            Some(BuiltinTheme::Nord)
        );
        assert_eq!(BuiltinTheme::from_id(Some("builtin:unknown")), None);
        assert_eq!(
            BuiltinTheme::from_id(Some("builtin:catppuccin")),
            Some(BuiltinTheme::CatppuccinMocha)
        );
        assert_eq!(BuiltinTheme::from_id(Some("user:my_theme.toml")), None);
        for (index, family) in BuiltinTheme::ALL.iter().copied().enumerate() {
            assert_eq!(BuiltinTheme::from_id(family.id()), Some(family));
            assert!(!family.label_key().is_empty());
            assert!(
                !BuiltinTheme::ALL[..index]
                    .iter()
                    .any(|other| other.id() == family.id())
            );
            for scheme in [Scheme::Light, Scheme::Dark] {
                assert_eq!(family.palette(scheme).scheme, scheme);
            }
        }
    }

    #[test]
    fn saved_identifiers_and_translation_keys_remain_stable() {
        // Explicit references protect saved preferences from accidental renaming.
        let expected = [
            (BuiltinTheme::Nord, "builtin:base", "theme_nord"),
            (BuiltinTheme::Atom, "builtin:atom", "theme_atom"),
            (
                BuiltinTheme::CatppuccinMocha,
                "builtin:catppuccin",
                "theme_catppuccin_mocha",
            ),
            (
                BuiltinTheme::CatppuccinMacchiato,
                "builtin:catppuccin-macchiato",
                "theme_catppuccin_macchiato",
            ),
            (
                BuiltinTheme::CatppuccinFrappe,
                "builtin:catppuccin-frappe",
                "theme_catppuccin_frappe",
            ),
            (
                BuiltinTheme::TokyoNight,
                "builtin:tokyo-night",
                "theme_tokyo_night",
            ),
            (BuiltinTheme::Fluvy, "builtin:fluvy", "theme_fluvy"),
            (
                BuiltinTheme::Everforest,
                "builtin:everforest",
                "theme_everforest",
            ),
            (
                BuiltinTheme::SonokaiAtlantis,
                "builtin:sonokai-atlantis",
                "theme_sonokai_atlantis",
            ),
            (
                BuiltinTheme::RosePine,
                "builtin:rose-pine",
                "theme_rose_pine",
            ),
            (
                BuiltinTheme::GruvboxHard,
                "builtin:gruvbox-hard",
                "theme_gruvbox_hard",
            ),
        ];
        assert_eq!(expected.len(), BuiltinTheme::ALL.len());
        assert_eq!(BuiltinTheme::default().id(), None);
        for (family, stored_id, label_key) in expected {
            assert_eq!(family.stored_id(), stored_id);
            assert_eq!(family.label_key(), label_key);
            assert_eq!(BuiltinTheme::from_id(Some(stored_id)), Some(family));
        }
        assert_eq!(BuiltinTheme::from_id(Some("")), None);
        assert_eq!(BuiltinTheme::from_id(Some("builtin:nord")), None);
    }

    #[test]
    fn related_families_have_distinct_palettes_in_both_appearances() {
        for (index, family) in BuiltinTheme::ALL.iter().enumerate() {
            for other in &BuiltinTheme::ALL[..index] {
                for scheme in [Scheme::Light, Scheme::Dark] {
                    assert_ne!(family.palette(scheme), other.palette(scheme));
                }
            }
        }
    }
}
