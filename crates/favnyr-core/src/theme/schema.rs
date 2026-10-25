use super::palette::ThemePalettes;
use super::{BuiltinTheme, ColorRole, Palette, Rgba, Scheme};
use serde::Deserialize;
use std::collections::BTreeMap;

pub const MAX_THEME_BYTES: usize = 64 * 1024;

// A single format for the embedded themes and for the files a user adds, so
// one can be copied into the other unchanged. Both appearances are required:
// a theme carrying only one would leave the appearance selector with nothing
// to switch to, and the light or dark choice would silently do nothing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    schema_version: u32,
    light: Variant,
    dark: Variant,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    #[serde(default)]
    colors: BTreeMap<ColorRole, Rgba>,
    #[serde(default)]
    opacity: Opacity,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Opacity {
    hidden: Option<f32>,
    age: Option<f32>,
}

/// Reads a theme file. Every role and opacity left out inherits the base
/// family, so a complete theme can be as short as its two appearance tables.
pub fn parse(text: &str) -> Result<ThemePalettes, String> {
    read(text, Some(BuiltinTheme::Nord.palettes()))
}

/// Reads the base family, which has nothing to inherit from.
pub(crate) fn parse_base(text: &str) -> Result<ThemePalettes, String> {
    read(text, None)
}

fn read(text: &str, baseline: Option<&ThemePalettes>) -> Result<ThemePalettes, String> {
    // Measured before parsing, so an oversized file is refused without
    // building anything from it.
    if text.len() > MAX_THEME_BYTES {
        return Err("theme exceeds the size limit".into());
    }
    let spec: Spec = toml::from_str(text).map_err(|e| e.to_string())?;
    if spec.schema_version != 1 {
        return Err("unsupported theme schema version".into());
    }
    Ok(ThemePalettes {
        light: variant(Scheme::Light, spec.light, baseline.map(|p| &p.light))?,
        dark: variant(Scheme::Dark, spec.dark, baseline.map(|p| &p.dark))?,
    })
}

fn variant(scheme: Scheme, spec: Variant, baseline: Option<&Palette>) -> Result<Palette, String> {
    validate_values(&spec.colors, &spec.opacity)?;
    if let Some(baseline) = baseline {
        return Ok(resolve(baseline, &spec.colors, &spec.opacity));
    }
    // The base family cannot inherit from itself; every value is required.
    let mut colors = [Rgba([0; 4]); ColorRole::ALL.len()];
    for &role in ColorRole::ALL {
        colors[role as usize] = *spec
            .colors
            .get(&role)
            .ok_or_else(|| format!("missing {scheme:?} color: {role:?}"))?;
    }
    Ok(Palette {
        scheme,
        colors,
        hidden_opacity: spec
            .opacity
            .hidden
            .ok_or_else(|| format!("missing {scheme:?} hidden opacity"))?,
        age_opacity: spec
            .opacity
            .age
            .ok_or_else(|| format!("missing {scheme:?} age opacity"))?,
    })
}

fn validate_values(colors: &BTreeMap<ColorRole, Rgba>, opacity: &Opacity) -> Result<(), String> {
    for value in [opacity.hidden, opacity.age].into_iter().flatten() {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("opacity must be finite and between zero and one".into());
        }
    }
    for (&role, &color) in colors {
        if color.0[3] != 255
            && !matches!(
                role,
                ColorRole::Track
                    | ColorRole::DiskNormal
                    | ColorRole::DiskWarning
                    | ColorRole::DiskCritical
                    | ColorRole::ModalOverlay
            )
        {
            return Err(format!("transparency is not allowed for {role:?}"));
        }
    }
    Ok(())
}

fn resolve(baseline: &Palette, colors: &BTreeMap<ColorRole, Rgba>, opacity: &Opacity) -> Palette {
    let mut palette = baseline.clone();
    for (&role, &color) in colors {
        palette.set(role, color);
    }

    // Recompute only affected derived colors, preserving the built-in palette
    // exactly when a file overrides an unrelated role.
    let source = palette.clone();
    let supplied = |role| colors.contains_key(&role);
    let mut derive = |target, sources: &[ColorRole], value: Rgba| {
        if !supplied(target) && sources.iter().any(|r| supplied(*r)) {
            palette.set(target, value);
        }
    };
    use ColorRole::*;
    let dark = baseline.scheme == Scheme::Dark;
    if dark {
        derive(
            Track,
            &[Foreground],
            source.color(Foreground).with_alpha(0.12),
        );
        derive(
            DiskNormal,
            &[Foreground],
            source.color(Foreground).with_alpha(0.30),
        );
    }
    derive(
        DiskWarning,
        &[Warning],
        source
            .color(Warning)
            .with_alpha(if dark { 1.0 } else { 0.42 }),
    );
    derive(
        DiskCritical,
        &[Danger],
        source
            .color(Danger)
            .with_alpha(if dark { 1.0 } else { 0.42 }),
    );
    for (target, tint) in [
        (ToastSuccess, Success),
        (ToastInfo, AccentBackground),
        (ToastDanger, Danger),
    ] {
        derive(
            target,
            &[tint, Background],
            source
                .color(tint)
                .with_alpha(0.10)
                .over(source.color(Background)),
        );
    }
    // Reborrow after the source-color derivations, so ink sees the final fill.
    for (fill, ink, surface) in [
        (AccentBackground, AccentForeground, Background),
        (Danger, DangerForeground, Background),
        (TooltipBackground, TooltipForeground, Background),
        (DiskNormal, DiskNormalForeground, Toolbar),
        (DiskWarning, DiskWarningForeground, Toolbar),
        (DiskCritical, DiskCriticalForeground, Toolbar),
        (FolderRed, FolderRedForeground, Surface),
        (FolderGreen, FolderGreenForeground, Surface),
        (FolderTeal, FolderTealForeground, Surface),
        (FolderBlue, FolderBlueForeground, Surface),
        (FolderViolet, FolderVioletForeground, Surface),
        (FolderPink, FolderPinkForeground, Surface),
        (FolderSlate, FolderSlateForeground, Surface),
    ] {
        if !supplied(ink)
            && (palette.color(fill) != baseline.color(fill)
                || supplied(surface)
                || supplied(Background)
                || supplied(Foreground))
        {
            palette.set(
                ink,
                palette
                    .color(fill)
                    .over(palette.color(surface))
                    .contrasting_ink(),
            );
        }
    }
    palette.hidden_opacity = opacity.hidden.unwrap_or(palette.hidden_opacity);
    palette.age_opacity = opacity.age.unwrap_or(palette.age_opacity);
    palette
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortest_theme_file_is_the_base_family() {
        let palettes = parse("schema_version = 1\n[light]\n[dark]").unwrap();
        assert_eq!(palettes, *BuiltinTheme::Nord.palettes());
    }

    // The short theme the documentation offers as a starting point, so it
    // cannot drift away from what the parser accepts.
    #[test]
    fn the_documented_example_is_accepted() {
        let palettes = parse(
            "schema_version = 1\n\n[light.colors]\naccent_background = \"#1F7A54\"\n\n[dark.colors]\naccent_background = \"#A0D6BC\"\n",
        )
        .unwrap();
        assert_eq!(
            palettes.light.color(ColorRole::AccentBackground),
            Rgba::from_u32(0x1f7a54ff)
        );
        assert_eq!(
            palettes.dark.color(ColorRole::AccentBackground),
            Rgba::from_u32(0xa0d6bcff)
        );
        // Everything else is the base family, including the ink the new fills
        // need, which is recomputed for each of them.
        assert_eq!(
            palettes.light.color(ColorRole::Background),
            BuiltinTheme::Nord
                .palettes()
                .light
                .color(ColorRole::Background)
        );
        for variant in [&palettes.light, &palettes.dark] {
            assert_eq!(
                variant.color(ColorRole::AccentForeground),
                variant
                    .color(ColorRole::AccentBackground)
                    .over(variant.color(ColorRole::Background))
                    .contrasting_ink()
            );
        }
    }

    #[test]
    fn accent_ink_depends_on_the_fill_not_the_base() {
        let palettes =
            parse("schema_version = 1\n[light]\n[dark.colors]\naccent_background = '#101010'")
                .unwrap();
        assert_eq!(
            palettes.dark.color(ColorRole::AccentForeground),
            Rgba::from_u32(0xffffffff)
        );
        // An explicit ink is kept whatever the fill would have produced.
        let palettes = parse(
            "schema_version = 1\n[dark]\n[light.colors]\naccent_background = '#EEEEEE'\naccent_foreground = '#202020'",
        )
        .unwrap();
        assert_eq!(
            palettes.light.color(ColorRole::AccentForeground),
            Rgba::from_u32(0x202020ff)
        );
    }

    #[test]
    fn transparent_gauges_keep_rgba_channel_order() {
        let palettes =
            parse("schema_version = 1\n[dark]\n[light.colors]\ntrack = '#12345678'").unwrap();
        assert_eq!(
            palettes.light.color(ColorRole::Track).0,
            [0x12, 0x34, 0x56, 0x78]
        );
    }

    #[test]
    fn derived_gauge_ink_uses_the_composited_fill() {
        let palettes = parse(
            "schema_version = 1\n[dark]\n[light.colors]\nwarning = '#000000'\ntoolbar = '#000000'",
        )
        .unwrap();
        assert_eq!(
            palettes.light.color(ColorRole::DiskWarningForeground),
            Rgba::from_u32(0xffffffff)
        );
    }

    #[test]
    fn malformed_or_incomplete_files_are_rejected() {
        for text in [
            // Both appearances are mandatory, in either direction.
            "schema_version = 1\n[light]",
            "schema_version = 1\n[dark]",
            // The earlier single-appearance shape, which `base` selected.
            "schema_version = 1\nbase = 'dark'\n[colors]\nsurface = '#203040'",
            // Missing or unsupported version.
            "[light]\n[dark]",
            "schema_version = 2\n[light]\n[dark]",
            // Typos are refused rather than ignored, in roles and in tables.
            "schema_version = 1\n[light.colors]\nunknown = '#123456'\n[dark]",
            "schema_version = 1\n[light]\n[dark]\nunexpected = true",
            "schema_version = 1\n[light]\n[dark.opacity]\nextra = 0.5",
            // Color syntax, and transparency where it is not supported.
            "schema_version = 1\n[light.colors]\nsurface = '#12345600'\n[dark]",
            "schema_version = 1\n[light.colors]\nforeground = '123456'\n[dark]",
            "schema_version = 1\n[light.colors]\nforeground = '#12345Z'\n[dark]",
            // Opacities outside the unit range.
            "schema_version = 1\n[light.opacity]\nhidden = nan\n[dark]",
            "schema_version = 1\n[light.opacity]\nage = 1.1\n[dark]",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
        let oversized = " ".repeat(MAX_THEME_BYTES + 1);
        assert!(parse(&oversized).is_err());
        assert!(parse_base(&oversized).is_err());
    }
}
