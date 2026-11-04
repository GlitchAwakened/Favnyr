//! Detached-window startup carries display settings separately from native
//! paths, so Unix filenames never pass through a lossy UTF-8 conversion.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Result, bail};
use favnyr_core::workspace::ViewState;

pub const MARKER: &str = "--detached-view-state";

/// Conservative native argument budget, including quoting on Windows.
/// Oversized batches stay in their source window instead of spawning partially.
pub fn validate_command(exe: &std::path::Path, args: &[OsString]) -> Result<()> {
    #[cfg(windows)]
    fn units(value: &std::ffi::OsStr) -> usize {
        use std::os::windows::ffi::OsStrExt;
        value
            .encode_wide()
            .count()
            .saturating_mul(2)
            .saturating_add(3)
    }
    #[cfg(not(windows))]
    fn units(value: &std::ffi::OsStr) -> usize {
        value.as_encoded_bytes().len().saturating_add(1)
    }
    let size = args.iter().fold(units(exe.as_os_str()), |n, arg| {
        n.saturating_add(units(arg))
    });
    let limit = if cfg!(windows) { 32_767 } else { 65_536 };
    if size >= limit {
        bail!("detached view exceeds the native argument budget");
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
pub struct DetachedView {
    pub view: ViewState,
    pub dirs: Vec<PathBuf>,
    pub position: Option<(i32, i32)>,
}

/// Builds arguments for `Command`, never a shell command line. Clearing the
/// textual paths also avoids paying twice for long folder names in the payload.
pub fn arguments(
    view: &ViewState,
    dirs: &[PathBuf],
    position: Option<(i32, i32)>,
) -> Result<Vec<OsString>> {
    if dirs.is_empty()
        || dirs.len() != view.tabs.len()
        || dirs.iter().any(|dir| dir.as_os_str().is_empty())
    {
        bail!("a detached view needs one native path per tab");
    }
    let mut settings = view.clone();
    for tab in &mut settings.tabs {
        tab.path.clear();
    }
    let (x, y) = position
        .map(|(x, y)| (x.to_string(), y.to_string()))
        .unwrap_or_default();
    let mut args = vec![
        MARKER.into(),
        toml::to_string(&settings)?.into(),
        x.into(),
        y.into(),
    ];
    args.extend(dirs.iter().map(|dir| dir.as_os_str().to_owned()));
    Ok(args)
}

/// The marker has already been consumed. Reject incomplete payloads as a
/// whole rather than silently associating settings with the wrong folder.
pub fn parse(mut args: impl Iterator<Item = OsString>) -> Option<DetachedView> {
    let mut view: ViewState = toml::from_str(args.next()?.to_str()?).ok()?;
    let x = args.next()?;
    let y = args.next()?;
    let position = if x.is_empty() && y.is_empty() {
        None
    } else {
        Some((x.to_str()?.parse().ok()?, y.to_str()?.parse().ok()?))
    };
    let dirs: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if dirs.is_empty()
        || dirs.len() != view.tabs.len()
        || dirs.iter().any(|dir| dir.as_os_str().is_empty())
    {
        return None;
    }
    view.sanitize();
    Some(DetachedView {
        view,
        dirs,
        position,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use favnyr_core::workspace::ItemArrangement;

    #[test]
    fn argument_budget_rejects_oversized_batches_without_lossy_path_conversion() {
        let exe = std::path::Path::new("app");
        assert!(validate_command(exe, &["folder01".into(), "folder02".into()]).is_ok());
        assert!(validate_command(exe, &["x".repeat(65_536).into()]).is_err());
        assert!(validate_command(exe, &vec!["folder01".into(); 10_000]).is_err());
    }

    fn sample() -> ViewState {
        toml::from_str(
            r#"
active_tab = 1
tab_bar_mode = 2
vbar_width = 160.0
[[tabs]]
path = "folder01"
preview = false
zoom = 5
item_arrangement = "grid"
grid_zoom = 3
[[tabs]]
path = "folder02"
preview = true
zoom = 4
"#,
        )
        .unwrap()
    }

    #[test]
    fn settings_and_native_paths_round_trip_without_reordering() {
        let view = sample();
        let dirs = vec![
            PathBuf::from("folder01/my folder 'é'"),
            PathBuf::from("--folder02"),
        ];
        for position in [None, Some((-200, 400))] {
            let args = arguments(&view, &dirs, position).unwrap();
            assert_eq!(args[0], MARKER);
            assert_eq!(&args[4..], &[dirs[0].as_os_str(), dirs[1].as_os_str()]);
            let decoded = parse(args.into_iter().skip(1)).unwrap();
            assert_eq!(decoded.dirs, dirs);
            assert_eq!(decoded.position, position);
            assert_eq!(decoded.view.active_tab, 1);
            assert_eq!(decoded.view.tab_bar_mode, 2);
            assert_eq!(decoded.view.vbar_width, 160.0);
            assert_eq!(decoded.view.tabs[0].item_arrangement, ItemArrangement::Grid);
            assert!(!decoded.view.tabs[0].preview);
            assert_eq!(decoded.view.tabs[0].zoom, Some(5));
            assert_eq!(decoded.view.tabs[0].grid_zoom, Some(3));
            assert!(decoded.view.tabs.iter().all(|tab| tab.path.is_empty()));
            assert!(decoded.view.tabs[1].preview);
        }
    }

    #[test]
    fn incomplete_or_malformed_payloads_are_rejected_as_a_whole() {
        let view = sample();
        let dirs = vec![PathBuf::from("folder01"), PathBuf::from("folder02")];
        assert!(arguments(&view, &[], None).is_err());
        assert!(arguments(&view, &dirs[..1], None).is_err());
        let args = arguments(&view, &dirs, Some((10, 20))).unwrap();
        for len in 1..args.len() {
            assert!(parse(args[1..len].iter().cloned()).is_none());
        }
        for (index, replacement) in [(1, "bad toml"), (2, "bad"), (3, ""), (4, "")] {
            let mut invalid = args.clone();
            invalid[index] = replacement.into();
            assert!(parse(invalid.into_iter().skip(1)).is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_paths_keep_non_utf8_bytes_during_detachment() {
        use std::os::unix::ffi::OsStringExt;
        let dirs = vec![
            PathBuf::from(OsString::from_vec(b"folder01/\xff".to_vec())),
            PathBuf::from("folder02"),
        ];
        let args = arguments(&sample(), &dirs, None).unwrap();
        assert_eq!(parse(args.into_iter().skip(1)).unwrap().dirs, dirs);
    }
}
