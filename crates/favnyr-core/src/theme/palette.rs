use super::{ColorRole, Scheme};
use serde::{Deserialize, Deserializer};

/// RGBA channels, with alpha last in the file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const fn from_u32(rgba: u32) -> Self {
        Self(rgba.to_be_bytes())
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self([
            self.0[0],
            self.0[1],
            self.0[2],
            (alpha * 255.0).round() as u8,
        ])
    }

    /// Composites in sRGB to match ordinary UI surfaces.
    pub fn over(self, background: Self) -> Self {
        let a = f32::from(self.0[3]) / 255.0;
        let mut result = [0, 0, 0, 255];
        for (i, channel) in result[..3].iter_mut().enumerate() {
            *channel =
                (f32::from(self.0[i]) * a + f32::from(background.0[i]) * (1.0 - a)).round() as u8;
        }
        Self(result)
    }

    fn luminance(self) -> f32 {
        let mut linear = [0.0; 3];
        for (i, value) in linear.iter_mut().enumerate() {
            let c = f32::from(self.0[i]) / 255.0;
            *value = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
    }

    /// Chooses an opaque text color once, when a custom fill is resolved.
    pub fn contrasting_ink(self) -> Self {
        let l = self.luminance();
        if (l + 0.05) / 0.05 >= 1.05 / (l + 0.05) {
            Self::from_u32(0x000000ff)
        } else {
            Self::from_u32(0xffffffff)
        }
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let hex = value.strip_prefix('#').unwrap_or("");
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(serde::de::Error::custom("expected #RRGGBB or #RRGGBBAA"));
        }
        let value = u32::from_str_radix(hex, 16).map_err(serde::de::Error::custom)?;
        Ok(Self::from_u32(if hex.len() == 6 {
            (value << 8) | 255
        } else {
            value
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub scheme: Scheme,
    pub(crate) colors: [Rgba; ColorRole::ALL.len()],
    pub hidden_opacity: f32,
    pub age_opacity: f32,
}

impl Palette {
    pub fn color(&self, role: ColorRole) -> Rgba {
        self.colors[role as usize]
    }

    pub(crate) fn set(&mut self, role: ColorRole, value: Rgba) {
        self.colors[role as usize] = value;
    }
}

/// The two appearances a theme always carries.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemePalettes {
    pub light: Palette,
    pub dark: Palette,
}

impl ThemePalettes {
    pub fn variant(&self, scheme: Scheme) -> &Palette {
        match scheme {
            Scheme::Light => &self.light,
            Scheme::Dark => &self.dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::BuiltinTheme;

    fn contrast_ratio(ink: Rgba, fill: Rgba) -> f32 {
        let a = ink.luminance();
        let b = fill.luminance();
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn extended_builtin_text_remains_legible_on_surfaces_and_semantic_fills() {
        use ColorRole::*;
        for (family, scheme) in BuiltinTheme::ALL
            .iter()
            .copied()
            .filter(|family| *family != BuiltinTheme::Nord)
            .flat_map(|family| [Scheme::Light, Scheme::Dark].map(|scheme| (family, scheme)))
        {
            let palette = family.palette(scheme);
            let mut pairs: Vec<_> = [
                Background,
                Surface,
                Toolbar,
                AlternateBackground,
                ToastInfo,
                ToastSuccess,
                ToastDanger,
            ]
            .into_iter()
            .map(|fill| (Foreground, fill, Background))
            .collect();
            pairs.extend([
                (AccentForeground, AccentBackground, Background),
                (DangerForeground, Danger, Background),
                (TooltipForeground, TooltipBackground, Background),
                (DiskNormalForeground, DiskNormal, Toolbar),
                (DiskWarningForeground, DiskWarning, Toolbar),
                (DiskCriticalForeground, DiskCritical, Toolbar),
                (FolderRedForeground, FolderRed, Surface),
                (FolderGreenForeground, FolderGreen, Surface),
                (FolderTealForeground, FolderTeal, Surface),
                (FolderBlueForeground, FolderBlue, Surface),
                (FolderVioletForeground, FolderViolet, Surface),
                (FolderPinkForeground, FolderPink, Surface),
                (FolderSlateForeground, FolderSlate, Surface),
            ]);
            for (ink, fill, surface) in pairs {
                let contrast = contrast_ratio(
                    palette.color(ink),
                    palette.color(fill).over(palette.color(surface)),
                );
                assert!(
                    contrast >= 4.5,
                    "{family:?} {scheme:?} {ink:?}/{fill:?}: {contrast}"
                );
            }
        }
    }

    #[test]
    fn additional_palettes_keep_metadata_legible_on_idle_rows() {
        use ColorRole::*;
        for family in [
            BuiltinTheme::SonokaiAtlantis,
            BuiltinTheme::RosePine,
            BuiltinTheme::GruvboxHard,
        ] {
            for scheme in [Scheme::Light, Scheme::Dark] {
                let palette = family.palette(scheme);
                let surface = palette.color(Surface);
                for role in [
                    Extension, AgeHour, AgeHours, AgeDay, AgeWeek, AgeMonth, AgeYear, AgeOlder,
                ] {
                    let ink = palette.color(role);
                    // Age labels sit on their own translucent color; extensions
                    // are drawn directly on the row surface.
                    let fill = if role == Extension {
                        surface
                    } else {
                        ink.with_alpha(palette.age_opacity).over(surface)
                    };
                    let contrast = contrast_ratio(ink, fill);
                    assert!(
                        contrast >= 4.5,
                        "{family:?} {scheme:?} {role:?}: {contrast}"
                    );
                }
            }
        }
    }
}
