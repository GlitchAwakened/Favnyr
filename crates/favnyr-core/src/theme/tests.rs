//! Regression references for the built-in appearance and palette resolution.

use super::*;

const CUSTOM_COLORS: &str = "
[colors]
background = '#102030'
surface = '#182838'
toolbar = '#203040'
foreground = '#C0D0E0'
accent_background = '#8090A0'
danger = '#A02040'
warning = '#F0C030'
tooltip_background = '#FAFAFA'
folder_red = '#B03040'
folder_blue = '#3050A0'
folder_slate = '#708090'
";

const EXPLICIT_DERIVED_COLORS: &str = "
[opacity]
hidden = 0.0
age = 1.0
[colors]
background = '#E0D0C0'
foreground = '#201810'
warning = '#F0C030'
track = '#12345678'
disk_normal = '#23456789'
disk_warning = '#3456789A'
disk_critical = '#456789AB'
accent_background = '#161616'
accent_foreground = '#D0D0D0'
toast_success = '#405060'
toast_info = '#506070'
toast_danger = '#607080'
folder_red = '#806040'
folder_red_foreground = '#F0F0F0'
";

// Writes a set of overrides as a theme file, which always declares both
// appearances and here gives each of them the same overrides.
fn theme_file(overrides: &str) -> String {
    let section = |variant: &str| {
        overrides
            .replace("[colors]", &format!("[{variant}.colors]"))
            .replace("[opacity]", &format!("[{variant}.opacity]"))
    };
    format!(
        "schema_version = 1\n[light]\n{}\n[dark]\n{}",
        section("light"),
        section("dark")
    )
}

// Stable test fingerprints cover every channel, scheme and opacity.
fn fingerprint(palette: &Palette) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in ColorRole::ALL
        .iter()
        .flat_map(|role| palette.color(*role).0)
        .chain(palette.hidden_opacity.to_bits().to_le_bytes())
        .chain(palette.age_opacity.to_bits().to_le_bytes())
        .chain([u8::from(palette.scheme == Scheme::Dark)])
    {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    hash
}

#[test]
fn theme_file_derivations_match_fixed_references() {
    for (scheme, expected) in [
        (Scheme::Light, [0x0ce016946f7e8ab5, 0xb47f386b59909c4e]),
        (Scheme::Dark, [0xfeba7d4861b68fee, 0xdf5cad96d38d08d6]),
    ] {
        for (overrides, expected) in [CUSTOM_COLORS, EXPLICIT_DERIVED_COLORS]
            .into_iter()
            .zip(expected)
        {
            let palettes = parse(&theme_file(overrides)).unwrap();
            assert_eq!(
                fingerprint(palettes.variant(scheme)),
                expected,
                "{scheme:?}"
            );
        }
    }
}

// The promise the themes directory makes: an embedded palette file is an
// ordinary theme file, and a copy of one reproduces the theme exactly.
#[test]
fn every_embedded_theme_is_also_a_valid_theme_file() {
    for &family in BuiltinTheme::ALL {
        assert_eq!(
            parse(family.source()).unwrap(),
            *family.palettes(),
            "{family:?}"
        );
    }
}

#[test]
fn the_base_family_requires_every_role_and_both_opacities() {
    let complete: toml::Value = toml::from_str(BuiltinTheme::Nord.source()).unwrap();
    for name in ["light", "dark"] {
        let mut missing_variant = complete.clone();
        missing_variant.as_table_mut().unwrap().remove(name);
        assert!(schema::parse_base(&missing_variant.to_string()).is_err());
        let colors = complete[name]["colors"].as_table().unwrap();
        for role in colors.keys() {
            let mut missing = complete.clone();
            missing[name]["colors"].as_table_mut().unwrap().remove(role);
            assert!(
                schema::parse_base(&missing.to_string()).is_err(),
                "{name}/{role}"
            );
        }
        for opacity in ["hidden", "age"] {
            let mut missing = complete.clone();
            missing[name]["opacity"]
                .as_table_mut()
                .unwrap()
                .remove(opacity);
            assert!(
                schema::parse_base(&missing.to_string()).is_err(),
                "{name}/{opacity}"
            );
        }
    }
}

#[test]
fn built_in_palette_cache_is_shared_and_returned_values_are_independent() {
    for &family in BuiltinTheme::ALL {
        let expected = family.palettes() as *const _ as usize;
        let threads: Vec<_> = (0..4)
            .map(|_| std::thread::spawn(move || family.palettes() as *const _ as usize))
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), expected);
        }
        let original = family.palette(Scheme::Dark);
        let mut changed = family.palette(Scheme::Dark);
        changed.hidden_opacity = original.hidden_opacity + 0.1;
        assert_ne!(changed, original);
        assert_eq!(family.palette(Scheme::Dark), original);
    }
    let original = builtin(Scheme::Dark);
    let mut changed = builtin(Scheme::Dark);
    changed.set(ColorRole::Surface, Rgba::from_u32(0x123456ff));
    changed.hidden_opacity = 0.0;
    assert_ne!(changed, original);
    assert_eq!(builtin(Scheme::Dark), original);
}

// Fixed color references detect drift independently of the embedded theme data.
const BASE_COLORS: &[(ColorRole, u32, u32)] = &[
    (ColorRole::Background, 0x1b2230ff, 0xe3e9f1ff),
    (ColorRole::Surface, 0x2a3242ff, 0xf8fafcff),
    (ColorRole::Toolbar, 0x222a39ff, 0xe9eef5ff),
    (ColorRole::AlternateBackground, 0x3a455cff, 0xd9e2f0ff),
    (ColorRole::Foreground, 0xeceff4ff, 0x202733ff),
    (ColorRole::Border, 0x46536eff, 0xccd6e4ff),
    (ColorRole::AccentBackground, 0x7ccbe1ff, 0x1f5292ff),
    (ColorRole::AccentForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::Danger, 0xc4757cff, 0xbf616aff),
    (ColorRole::DangerForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::Success, 0xa3be8cff, 0x7a9a5eff),
    (ColorRole::Warning, 0xeab464ff, 0xd08019ff),
    (ColorRole::Extension, 0x8ec7e6ff, 0x3168c4ff),
    (ColorRole::Track, 0xeceff41f, 0xc0c8cdff),
    (ColorRole::DiskNormal, 0xeceff44d, 0xa3adb5ff),
    (ColorRole::DiskWarning, 0xeab464ff, 0xd080196b),
    (ColorRole::DiskCritical, 0xd08d92ff, 0xbf616a6b),
    (ColorRole::DiskNormalForeground, 0xeceff4ff, 0x202733ff),
    (ColorRole::DiskWarningForeground, 0x1b2230ff, 0x202733ff),
    (ColorRole::DiskCriticalForeground, 0x1b2230ff, 0x202733ff),
    (ColorRole::ToastSuccess, 0x1e2a1cff, 0xeef4e8ff),
    (ColorRole::ToastInfo, 0x1c2530ff, 0xe9eff8ff),
    (ColorRole::ToastDanger, 0x2a1c20ff, 0xfbe9eaff),
    (ColorRole::TooltipBackground, 0x1a1a1aff, 0x1a1a1aff),
    (ColorRole::TooltipForeground, 0xffffffff, 0xffffffff),
    (ColorRole::Shadow, 0x000000ff, 0x000000ff),
    (ColorRole::ModalOverlay, 0x00000073, 0x00000073),
    (ColorRole::FileApplication, 0xa3be8cff, 0xa3be8cff),
    (ColorRole::FileArchive, 0xd08770ff, 0xd08770ff),
    (ColorRole::FileAudio, 0xb48eadff, 0xb48eadff),
    (ColorRole::FileDocument, 0x81a1c1ff, 0x81a1c1ff),
    (ColorRole::FileImage, 0xbf616aff, 0xbf616aff),
    (ColorRole::FileVideo, 0x8fbcbbff, 0x8fbcbbff),
    (ColorRole::FileRegistry, 0x8b95a7ff, 0x8b95a7ff),
    (ColorRole::FolderDefault, 0xd9a441ff, 0xd9a441ff),
    (ColorRole::FolderRed, 0xe9948cff, 0xa85853ff),
    (ColorRole::FolderGreen, 0x79c289ff, 0x3c854fff),
    (ColorRole::FolderTeal, 0x53c2c5ff, 0x028387ff),
    (ColorRole::FolderBlue, 0x81b2edff, 0x4775acff),
    (ColorRole::FolderViolet, 0xb6a1e6ff, 0x7a65a6ff),
    (ColorRole::FolderPink, 0xdc94bdff, 0x9c5a80ff),
    (ColorRole::FolderSlate, 0xa8afbaff, 0x6d737eff),
    (ColorRole::AgeHour, 0xf0786aff, 0xc0392bff),
    (ColorRole::AgeHours, 0xef9a4fff, 0xb06317ff),
    (ColorRole::AgeDay, 0xe3bd4eff, 0x8a6d12ff),
    (ColorRole::AgeWeek, 0x9fc06aff, 0x54802fff),
    (ColorRole::AgeMonth, 0x5fb39aff, 0x2a7a66ff),
    (ColorRole::AgeYear, 0x6ba0c4ff, 0x3a6797ff),
    (ColorRole::AgeOlder, 0x8b95a7ff, 0x586780ff),
    (ColorRole::FolderRedForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::FolderGreenForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::FolderTealForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::FolderBlueForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::FolderVioletForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::FolderPinkForeground, 0x1b2230ff, 0xffffffff),
    (ColorRole::FolderSlateForeground, 0x1b2230ff, 0xffffffff),
];

#[test]
fn base_theme_matches_color_and_opacity_references_exactly() {
    assert_eq!(BASE_COLORS.len(), ColorRole::ALL.len());
    for (scheme, hidden, age) in [
        (Scheme::Light, 0.58_f32, 0.16_f32),
        (Scheme::Dark, 0.55_f32, 0.20_f32),
    ] {
        let palette = BuiltinTheme::Nord.palette(scheme);
        assert_eq!(palette.scheme, scheme);
        for ((role, dark, light), expected_role) in BASE_COLORS.iter().zip(ColorRole::ALL) {
            assert_eq!(role, expected_role);
            let expected = if scheme == Scheme::Dark {
                *dark
            } else {
                *light
            };
            assert_eq!(
                palette.color(*role),
                Rgba::from_u32(expected),
                "{scheme:?}/{role:?}"
            );
        }
        assert_eq!(palette.hidden_opacity.to_bits(), hidden.to_bits());
        assert_eq!(palette.age_opacity.to_bits(), age.to_bits());
        assert_eq!(builtin(scheme), palette);
    }
}
