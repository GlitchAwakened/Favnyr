//! Main `favnyr` binary.
//!
//! Startup sequence:
//!   1. `tracing` logging.
//!   2. Creating the XDG directories.
//!   3. Loading (or creating) `config.toml`.
//!   4. Instantiating the Slint window + installing the bridge.
//!   5. Slint event loop.
//!   6. Persisting the window size on close.

// On Windows, in a **release** build, no black console window on launch (GUI
// app). In debug we keep the console to see logs/panics during dev.
// (No-op on other OSes.)
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use slint::ComponentHandle;
use tracing::{info, warn};

use favnyr_core::{Config, WorkspaceState, logging, paths};

mod actions;
mod archives;
mod background_work;
mod bridge;
mod clipboard;
mod detached_view;
mod file_drag;
mod file_uri;
mod i18n;
#[cfg(target_os = "linux")]
mod linportable;
mod openwith;
mod shellmenu;
#[cfg(test)]
mod test_support;
mod theme;
mod transfer;
#[cfg(test)]
mod widget_tests;
#[cfg(windows)]
mod winddrag;
mod winmsg;
#[cfg(windows)]
mod winportable;
#[cfg(windows)]
mod winshare;
#[cfg(windows)]
mod winthumb;
#[cfg(windows)]
mod winutil;

slint::include_modules!();

/// Actual constraints declared by `MainWindow` in the Slint file.
/// Keeping them here lets us clamp a restored size even before the
/// backend creates the native window.
const MIN_WINDOW_WIDTH: u32 = 720;
const MIN_WINDOW_HEIGHT: u32 = 420;

#[derive(Debug, PartialEq)]
enum StartupRequest {
    Normal,
    /// Internal protocol used only for tab tear-off.
    DetachedTab {
        dir: PathBuf,
        position: Option<(i32, i32)>,
        tab_bar_mode: u8,
    },
    /// Internal protocol used only for VIEW tear-off: every tab of the view,
    /// the first being the one that stays active.
    DetachedView {
        dirs: Vec<PathBuf>,
        position: Option<(i32, i32)>,
        tab_bar_mode: u8,
    },
    DetachedState(detached_view::DetachedView),
    /// Desktop launch: one folder per argument, opened as tabs in a new window.
    Folders(Vec<PathBuf>),
    InvalidLocation(OsString),
}

fn parse_startup_request(args: impl IntoIterator<Item = OsString>) -> StartupRequest {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return StartupRequest::Normal;
    };

    if first == detached_view::MARKER {
        return detached_view::parse(args)
            .map(StartupRequest::DetachedState)
            .unwrap_or(StartupRequest::Normal);
    }

    if first == "--detached-tab" {
        let Some(dir) = args
            .next()
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
        else {
            return StartupRequest::Normal;
        };
        let x = args
            .next()
            .and_then(|value| value.to_str().and_then(|value| value.parse().ok()));
        let y = args
            .next()
            .and_then(|value| value.to_str().and_then(|value| value.parse().ok()));
        let position = match (x, y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        };
        let tab_bar_mode = if position.is_some() {
            args.next()
                .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
                .filter(|mode| *mode <= 2)
                .unwrap_or(0)
        } else {
            0
        };
        return StartupRequest::DetachedTab {
            dir,
            position,
            tab_bar_mode,
        };
    }

    // View tear-off. Position and bar mode have a FIXED arity and come first;
    // the folders make up the rest, however many there are, so the list can be
    // read with no ambiguity. Its own marker: `--detached-tab` is untouched.
    if first == "--detached-view" {
        let number = |arg: Option<OsString>| {
            arg.and_then(|value| value.to_str().and_then(|value| value.parse::<i32>().ok()))
        };
        let (x, y, mode) = (
            number(args.next()),
            number(args.next()),
            number(args.next()),
        );
        let dirs: Vec<PathBuf> = args
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        // A malformed header, or not a single folder: a plain window is a
        // better answer than a window on nothing.
        let (Some(x), Some(y), Some(mode)) = (x, y, mode) else {
            return StartupRequest::Normal;
        };
        if dirs.is_empty() {
            return StartupRequest::Normal;
        }
        return StartupRequest::DetachedView {
            dirs,
            position: Some((x, y)),
            tab_bar_mode: u8::try_from(mode).unwrap_or(0).min(2),
        };
    }

    // `--` allows a folder name starting with an option marker. Unrecognized
    // options are rejected rather than opened as unrelated relative paths.
    let positional_only = first == "--";
    let mut dirs = Vec::new();
    for argument in std::iter::once(first)
        .chain(args)
        .skip(usize::from(positional_only))
    {
        if argument.is_empty()
            || (!positional_only && argument.as_encoded_bytes().starts_with(b"--"))
        {
            return StartupRequest::InvalidLocation(argument);
        }
        let dir = if file_uri::has_file_scheme(&argument) {
            let Some(dir) = file_uri::to_path(&argument) else {
                return StartupRequest::InvalidLocation(argument);
            };
            dir
        } else {
            PathBuf::from(argument)
        };
        dirs.push(dir);
    }
    if dirs.is_empty() {
        StartupRequest::Normal
    } else {
        StartupRequest::Folders(dirs)
    }
}

fn restore_session(config: Config) -> bridge::AppState {
    match WorkspaceState::load(&paths::workspace_path()) {
        Ok(Some(ws)) => {
            info!(panels = ws.panels.len(), "workspace restored");
            bridge::AppState::from_workspace(config, ws)
        }
        Ok(None) => bridge::AppState::new(config),
        Err(err) => {
            warn!(error = %err, "workspace invalid, starting fresh");
            bridge::AppState::new(config)
        }
    }
}

/// Applies the UI's minimum bounds and, when known, the maximum
/// dimensions of the client area that fits on the target monitor.
///
/// The maximum is expressed in LOGICAL pixels, like `Config::window_*`.
fn clamp_restored_size(requested: (u32, u32), maximum: Option<(u32, u32)>) -> (u32, u32) {
    let requested = (
        requested.0.max(MIN_WINDOW_WIDTH),
        requested.1.max(MIN_WINDOW_HEIGHT),
    );
    let Some((max_w, max_h)) = maximum else {
        return requested;
    };

    // On a screen exceptionally smaller than Slint's constraints,
    // the minimum size still takes priority (the UI can't shrink
    // any further anyway).
    (
        requested.0.min(max_w.max(MIN_WINDOW_WIDTH)),
        requested.1.min(max_h.max(MIN_WINDOW_HEIGHT)),
    )
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct WindowsWorkArea {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    scale: f32,
    decoration_width: i32,
    decoration_height: i32,
}

#[cfg(windows)]
impl WindowsWorkArea {
    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }

    /// Maximum logical client size that also lets the native decorations
    /// (title bar + borders) fit within the work area.
    fn max_logical_client_size(self) -> (u32, u32) {
        let scale = self.scale.max(0.5);
        let client_w = (self.width() - self.decoration_width).max(1);
        let client_h = (self.height() - self.decoration_height).max(1);
        (
            (client_w as f32 / scale).floor().max(1.0) as u32,
            (client_h as f32 / scale).floor().max(1.0) as u32,
        )
    }

    fn logical_to_physical(self, logical: (u32, u32)) -> (u32, u32) {
        let scale = self.scale.max(0.5);
        (
            (logical.0 as f32 * scale).floor().max(1.0) as u32,
            (logical.1 as f32 * scale).floor().max(1.0) as u32,
        )
    }
}

/// Returns the work area and DPI metrics of the primary monitor, or
/// of the monitor containing the drop point of a detached tab.
#[cfg(windows)]
fn windows_work_area(
    detached_pos: Option<(i32, i32)>,
    fallback_scale: f32,
) -> Option<WindowsWorkArea> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
        MonitorFromPoint,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        SM_CXFRAME, SM_CXPADDEDBORDER, SM_CYCAPTION, SM_CYFRAME,
    };

    let (point, flags) = match detached_pos {
        Some((x, y)) => (POINT { x, y }, MONITOR_DEFAULTTONEAREST),
        None => (POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
    };
    let monitor = unsafe { MonitorFromPoint(point, flags) };
    if monitor.0.is_null() {
        return None;
    }

    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return None;
    }

    let mut dpi_x = 0_u32;
    let mut dpi_y = 0_u32;
    let dpi = if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .is_ok()
        && dpi_x > 0
    {
        dpi_x
    } else {
        (fallback_scale.max(0.5) * 96.0).round() as u32
    };
    let scale = dpi as f32 / 96.0;

    // Metrics for the MONITOR's DPI, not just the primary screen's.
    // This matters when detaching between two screens with
    // different scale factors.
    let (caption, frame_x, frame_y, padded_border) = unsafe {
        (
            GetSystemMetricsForDpi(SM_CYCAPTION, dpi),
            GetSystemMetricsForDpi(SM_CXFRAME, dpi),
            GetSystemMetricsForDpi(SM_CYFRAME, dpi),
            GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi),
        )
    };
    let decoration_width = 2 * (frame_x + padded_border).max(0);
    let decoration_height = caption.max(0) + 2 * (frame_y + padded_border).max(0);
    let work = monitor_info.rcWork;
    if work.right <= work.left || work.bottom <= work.top {
        return None;
    }

    Some(WindowsWorkArea {
        left: work.left,
        top: work.top,
        right: work.right,
        bottom: work.bottom,
        scale,
        decoration_width,
        decoration_height,
    })
}

#[cfg(windows)]
fn fit_window_axis(desired: i32, start: i32, end: i32, outer_size: i32) -> i32 {
    let latest_start = end.saturating_sub(outer_size);
    if latest_start < start {
        start
    } else {
        desired.clamp(start, latest_start)
    }
}

#[cfg(windows)]
thread_local! {
    static CONSOLE_WINDOW: std::cell::RefCell<Option<slint::Weak<MainWindow>>> = const { std::cell::RefCell::new(None) };
}

/// Routes console termination through the same guard as the window close button.
/// A repeated signal before the event loop responds still forces termination.
/// Without an attached console, this handler stays inactive.
#[cfg(windows)]
fn install_console_ctrl_handler(window: &MainWindow) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static QUITTING: AtomicBool = AtomicBool::new(false);
    CONSOLE_WINDOW.with(|slot| *slot.borrow_mut() = Some(window.as_weak()));

    unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
        if QUITTING.swap(true, Ordering::SeqCst) {
            // A second signal, or a loop that doesn't respond, forces an
            // immediate termination.
            terminate_process_now(130);
        }
        // Keep the event loop alive when file operations need confirmation.
        let _ = slint::invoke_from_event_loop(|| {
            CONSOLE_WINDOW.with(|slot| {
                if let Some(window) = slot.borrow().as_ref().and_then(slint::Weak::upgrade) {
                    window.invoke_request_close();
                }
            });
            QUITTING.store(false, Ordering::SeqCst);
        });
        1 // TRUE: event handled (we don't run the default behavior).
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(handler), 1);
    }
}

fn main() -> Result<()> {
    let t_start = std::time::Instant::now();
    logging::init();
    paths::ensure_dirs().context("creating XDG directories")?;

    let mut config = Config::load_or_default(&paths::config_path()).unwrap_or_else(|err| {
        warn!(error = %err, "config invalid, using defaults");
        Config::default()
    });

    // Public arguments are folder paths or local file URIs, one per tab.
    // Parsing performs no filesystem probes.
    let startup = parse_startup_request(std::env::args_os().skip(1));
    let mut invalid_location = None;
    let (startup_dirs, detached_pos, startup_tab_bar, detached_settings) = match startup {
        StartupRequest::Normal => (None, None, 0, None),
        StartupRequest::Folders(dirs) => {
            (Some(dirs), None, config.default_tab_bar_mode.min(2), None)
        }
        StartupRequest::InvalidLocation(location) => {
            warn!(location = %location.to_string_lossy(), "invalid startup location");
            invalid_location = Some(location);
            (None, None, 0, None)
        }
        // A torn-off tab is a torn-off view holding one tab: the two protocols
        // meet here and everything downstream sees the same shape.
        StartupRequest::DetachedTab {
            dir,
            position,
            tab_bar_mode,
        } => (Some(vec![dir]), position, tab_bar_mode, None),
        StartupRequest::DetachedView {
            dirs,
            position,
            tab_bar_mode,
        } => (Some(dirs), position, tab_bar_mode, None),
        StartupRequest::DetachedState(packet) => (
            Some(packet.dirs),
            packet.position,
            packet.view.tab_bar_mode,
            Some(packet.view),
        ),
    };
    // Desktop launchers can pass "." with the target folder as their working
    // directory. Absolutize without canonicalizing or blocking on network I/O.
    let startup_dirs = startup_dirs.map(|dirs| {
        dirs.into_iter()
            .map(|dir| {
                std::path::absolute(&dir).unwrap_or_else(|err| {
                    warn!(error = %err, path = %dir.display(), "resolving startup folder failed");
                    dir
                })
            })
            .collect::<Vec<_>>()
    });

    let window = MainWindow::new().context("Slint MainWindow::new failed")?;

    // Restores the persisted size without ever letting the outer window
    // exceed the target monitor's work area. Config values are
    // LOGICAL; the work area and Windows decorations are PHYSICAL.
    {
        let requested = (config.window_width, config.window_height);
        let minimum_restored = clamp_restored_size(requested, None);

        #[cfg(windows)]
        let restored = {
            if let Some(work) = windows_work_area(detached_pos, window.window().scale_factor()) {
                let restored = clamp_restored_size(requested, Some(work.max_logical_client_size()));
                let (physical_w, physical_h) = work.logical_to_physical(restored);
                // Before the native window is created, Slint doesn't
                // necessarily know the final DPI yet. Giving it a LOGICAL size
                // avoids a double conversion when opening on a
                // secondary screen; physical pixels stay reserved for the placement
                // calculation below.
                window.window().set_size(slint::LogicalSize::new(
                    restored.0 as f32,
                    restored.1 as f32,
                ));

                let outer_w = physical_w as i32 + work.decoration_width;
                let outer_h = physical_h as i32 + work.decoration_height;
                let (desired_x, desired_y) = detached_pos
                    .map(|(px, py)| (px - 50, py - 14))
                    .unwrap_or_else(|| {
                        (
                            work.left + (work.width() - outer_w) / 2,
                            work.top + (work.height() - outer_h) / 2,
                        )
                    });
                let x = fit_window_axis(desired_x, work.left, work.right, outer_w);
                let y = fit_window_axis(desired_y, work.top, work.bottom, outer_h);
                window
                    .window()
                    .set_position(slint::PhysicalPosition::new(x, y));
                restored
            } else {
                // Very unlikely fallback (no Win32 monitor available): the
                // cross-platform behavior stays usable.
                window.window().set_size(slint::LogicalSize::new(
                    minimum_restored.0 as f32,
                    minimum_restored.1 as f32,
                ));
                if let Some((px, py)) = detached_pos {
                    window
                        .window()
                        .set_position(slint::PhysicalPosition::new(px - 50, py - 14));
                }
                minimum_restored
            }
        };

        #[cfg(not(windows))]
        let restored = {
            window.window().set_size(slint::LogicalSize::new(
                minimum_restored.0 as f32,
                minimum_restored.1 as f32,
            ));
            // Tab tear-off: physical screen position provided by the backend.
            if let Some((px, py)) = detached_pos {
                window
                    .window()
                    .set_position(slint::PhysicalPosition::new(px - 50, py - 14));
            }
            minimum_restored
        };

        // A normal instance sanitizes any size incompatible with the
        // current monitor. A detached instance stays ephemeral and never modifies this
        // global preference.
        if startup_dirs.is_none()
            && (config.window_width != restored.0 || config.window_height != restored.1)
        {
            info!(
                requested_width = requested.0,
                requested_height = requested.1,
                restored_width = restored.0,
                restored_height = restored.1,
                "clamping persisted window size to monitor work area"
            );
            config.window_width = restored.0;
            config.window_height = restored.1;
            if let Err(err) = config.save(&paths::config_path()) {
                warn!(error = %err, "saving clamped window size failed");
            }
        }
    }

    // Folder launches and detached instances start on the requested folders.
    // Otherwise: restores the workspace (panels + tabs) if present, or falls
    // back to the default state (one panel, one tab on $HOME).
    let state = if let Some(dirs) = startup_dirs.as_ref().filter(|dirs| !dirs.is_empty()) {
        info!(tabs = dirs.len(), dir = %dirs[0].display(), "starting folder instance");
        let s = bridge::AppState::new_at(config, dirs[0].clone(), startup_tab_bar);
        if let Some(view) = detached_settings {
            s.adopt_detached_view(&view, dirs);
        } else {
            s.adopt_tabs(&dirs[1..]);
        }
        // A folder window must never replace another instance's shared workspace.
        s.set_ephemeral();
        s
    } else {
        restore_session(config)
    };
    bridge::install(&window, state.clone());
    if let Some(location) = invalid_location {
        bridge::show_invalid_startup_location_notice(&window, &state, &location.to_string_lossy());
    }

    // Ctrl+C / Ctrl+Break from the terminal → clean exit (dev). Installed
    // right before the loop (the event loop exists → `invoke_from_event_loop` OK).
    #[cfg(windows)]
    install_console_ctrl_handler(&window);

    info!(
        init_ms = t_start.elapsed().as_millis(),
        "starting Slint event loop"
    );
    window.run().context("Slint event loop failed")?;

    // Persistence on close: window size (config) + panel/tab layout
    // (workspace). SKIPPED for a folder launch or detached instance: it is
    // ephemeral and would otherwise overwrite the main instance's config/workspace
    // (both files are shared).
    // ALSO skipped if the instance was emptied by transferring all its tabs to
    // another instance: otherwise we'd overwrite the shared workspace (empty
    // state), and the tab — already on the other instance — would be "recreated" on
    // the next launch.
    if startup_dirs.is_none() && !bridge::suppress_workspace_persist() {
        bridge::persist_window_size(&window, &state);
        state.persist_workspace();
    }

    // On Windows, the GUI stack's CRT/DllMain teardown (winit/FemtoVG and
    // AccessKit/UI-Automation COM) can hang. Since the state is already persisted,
    // the forced termination avoids leaving a residual process. Normal return
    // on other platforms.
    finish()
}

/// On Windows, immediately terminates the process without going through the
/// GUI stack's CRT/DllMain teardown. On other systems, returns normally.
#[cfg(windows)]
fn finish() -> Result<()> {
    terminate_process_now(0)
}
#[cfg(not(windows))]
fn finish() -> Result<()> {
    Ok(())
}

/// Forces the current process to end via `TerminateProcess`, without running the
/// CRT shutdown or the `DllMain`s. Only call this after the state has been persisted.
#[cfg(windows)]
fn terminate_process_now(code: u32) -> ! {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn TerminateProcess(handle: isize, exit_code: u32) -> i32;
    }
    unsafe {
        TerminateProcess(GetCurrentProcess(), code);
    }
    // TerminateProcess is asynchronous: we wait for the process to actually die.
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_size_keeps_a_normal_window_unchanged() {
        assert_eq!(
            clamp_restored_size((1100, 700), Some((1904, 993))),
            (1100, 700)
        );
    }

    #[test]
    fn restored_size_clamps_dimensions_saved_while_maximized() {
        // A size coming from a maximized window is bounded to the
        // normal available client area.
        assert_eq!(
            clamp_restored_size((1920, 1009), Some((1904, 993))),
            (1904, 993)
        );
    }

    #[test]
    fn restored_size_enforces_the_slint_minimum() {
        assert_eq!(
            clamp_restored_size((1, 2), Some((1904, 993))),
            (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
        );
    }

    #[test]
    fn startup_bare_arguments_are_independent_folder_paths() {
        let arguments = ["folder01", "my folder", "folder02"];
        assert_eq!(
            parse_startup_request(arguments.map(OsString::from)),
            StartupRequest::Folders(arguments.map(PathBuf::from).to_vec())
        );
    }

    #[test]
    fn startup_folder_arguments_keep_spaces_and_argument_boundaries() {
        let arguments = ["/folder01/my folder", "/folder02/other folder"];
        assert_eq!(
            parse_startup_request(arguments.map(OsString::from)),
            StartupRequest::Folders(arguments.map(PathBuf::from).to_vec())
        );
    }

    #[test]
    fn startup_relative_paths_include_the_desktop_working_directory_marker() {
        for path in [
            ".",
            "..",
            "folder01",
            "./folder01",
            "folder01/",
            "folder01/folder02",
        ] {
            assert_eq!(
                parse_startup_request([OsString::from(path)]),
                StartupRequest::Folders(vec![PathBuf::from(path)])
            );
        }
    }

    #[test]
    fn startup_unknown_options_need_the_positional_separator() {
        let argument = OsString::from("--folder01");
        assert_eq!(
            parse_startup_request([argument.clone()]),
            StartupRequest::InvalidLocation(argument.clone())
        );
        assert_eq!(
            parse_startup_request([OsString::from("--"), argument.clone()]),
            StartupRequest::Folders(vec![PathBuf::from(argument)])
        );
        assert_eq!(
            parse_startup_request([OsString::from("--"), OsString::from("--detached-tab")]),
            StartupRequest::Folders(vec![PathBuf::from("--detached-tab")])
        );
    }

    #[test]
    fn startup_empty_arguments_are_not_folder_paths() {
        assert_eq!(parse_startup_request([]), StartupRequest::Normal);
        assert_eq!(
            parse_startup_request([OsString::from("--")]),
            StartupRequest::Normal
        );
        assert_eq!(
            parse_startup_request([OsString::new()]),
            StartupRequest::InvalidLocation(OsString::new())
        );
    }

    #[test]
    fn startup_malformed_file_uri_reports_invalid_location() {
        for argument in [
            "file://remote-host/folder01",
            "file:folder01",
            "file:///folder01/%GG",
            "file:///folder01/%00",
        ] {
            assert_eq!(
                parse_startup_request([OsString::from(argument)]),
                StartupRequest::InvalidLocation(argument.into())
            );
        }
        assert_eq!(
            parse_startup_request([
                OsString::from("/folder01"),
                OsString::from("file:///folder02/%"),
            ]),
            StartupRequest::InvalidLocation("file:///folder02/%".into())
        );
    }

    #[cfg(windows)]
    #[test]
    fn startup_windows_native_paths_and_drive_uris_open_folders() {
        for path in [r"C:\folder01\my folder", r"\\device01\folder01\my folder"] {
            assert_eq!(
                parse_startup_request([OsString::from(path)]),
                StartupRequest::Folders(vec![PathBuf::from(path)])
            );
        }
        assert_eq!(
            parse_startup_request([
                OsString::from("file:///C:/folder01/my%20folder"),
                OsString::from("FILE://LOCALHOST/D:/folder02/my%23folder"),
            ]),
            StartupRequest::Folders(vec![
                PathBuf::from(r"C:\folder01\my folder"),
                PathBuf::from(r"D:\folder02\my#folder"),
            ])
        );
    }

    #[cfg(unix)]
    #[test]
    fn startup_unix_paths_and_uris_preserve_non_utf8_filename_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"/folder01/my file\xFF".to_vec());
        assert_eq!(
            parse_startup_request([path.clone()]),
            StartupRequest::Folders(vec![PathBuf::from(path.clone())])
        );
        assert_eq!(
            parse_startup_request([
                OsString::from("FILE://LOCALHOST/folder01/my%20file%FF"),
                OsString::from("file:///folder02/my%2520file+%C3%A9"),
            ]),
            StartupRequest::Folders(vec![
                PathBuf::from(path),
                PathBuf::from("/folder02/my%20file+é"),
            ])
        );
    }

    #[test]
    fn detached_tab_protocol_carries_its_folder_position_and_bar_mode() {
        assert_eq!(
            parse_startup_request([
                OsString::from("--detached-tab"),
                OsString::from("C:\\folder_01"),
                OsString::from("120"),
                OsString::from("240"),
                OsString::from("2"),
            ]),
            StartupRequest::DetachedTab {
                dir: PathBuf::from("C:\\folder_01"),
                position: Some((120, 240)),
                tab_bar_mode: 2,
            }
        );
    }

    #[test]
    fn detached_state_protocol_dispatches_settings_and_native_paths() {
        let view = toml::from_str(
            "tabs = [{ path = 'folder01', item_arrangement = 'grid', preview = false, zoom = 5, grid_zoom = 2 }]"
        ).unwrap();
        let dirs = vec![PathBuf::from("folder01")];
        let args = detached_view::arguments(&view, &dirs, Some((-100, 200))).unwrap();
        let StartupRequest::DetachedState(packet) = parse_startup_request(args) else {
            panic!("detached settings must use the internal startup path");
        };
        assert_eq!(packet.dirs, dirs);
        assert_eq!(packet.position, Some((-100, 200)));
        assert_eq!(
            packet.view.tabs[0].item_arrangement,
            favnyr_core::workspace::ItemArrangement::Grid
        );
        assert_eq!(packet.view.tabs[0].grid_zoom, Some(2));
        assert!(!packet.view.tabs[0].preview);
        assert_eq!(
            parse_startup_request([OsString::from(detached_view::MARKER), OsString::from("bad")]),
            StartupRequest::Normal
        );
    }

    /// A whole view: the header has a fixed arity and comes first, the folders
    /// make up the rest. The FIRST folder is the tab that stays active.
    #[test]
    fn detached_view_protocol_carries_every_tab_in_order() {
        assert_eq!(
            parse_startup_request([
                OsString::from("--detached-view"),
                OsString::from("120"),
                OsString::from("240"),
                OsString::from("1"),
                OsString::from("/folder_01"),
                OsString::from("/folder_02"),
                OsString::from("/folder_03"),
            ]),
            StartupRequest::DetachedView {
                dirs: vec![
                    PathBuf::from("/folder_01"),
                    PathBuf::from("/folder_02"),
                    PathBuf::from("/folder_03"),
                ],
                position: Some((120, 240)),
                tab_bar_mode: 1,
            }
        );
    }

    /// A marker with nothing behind it opens an ordinary window rather than a
    /// window on nothing.
    #[test]
    fn detached_view_without_a_folder_falls_back_to_normal() {
        assert_eq!(
            parse_startup_request([
                OsString::from("--detached-view"),
                OsString::from("10"),
                OsString::from("20"),
                OsString::from("0"),
            ]),
            StartupRequest::Normal
        );
    }

    /// Same answer for a header that cannot be read: better an ordinary window
    /// than a guess about where it belongs.
    #[test]
    fn detached_view_with_an_unreadable_header_falls_back_to_normal() {
        assert_eq!(
            parse_startup_request([
                OsString::from("--detached-view"),
                OsString::from("left"),
                OsString::from("20"),
                OsString::from("0"),
                OsString::from("/folder_01"),
            ]),
            StartupRequest::Normal
        );
    }

    /// An out-of-range bar mode is clamped rather than refused: the folders are
    /// what matter, the chrome can fall back to the default.
    #[test]
    fn detached_view_clamps_an_impossible_tab_bar_mode() {
        assert_eq!(
            parse_startup_request([
                OsString::from("--detached-view"),
                OsString::from("0"),
                OsString::from("0"),
                OsString::from("9"),
                OsString::from("/folder_01"),
            ]),
            StartupRequest::DetachedView {
                dirs: vec![PathBuf::from("/folder_01")],
                position: Some((0, 0)),
                tab_bar_mode: 2,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn detached_window_position_stays_inside_negative_monitor_coordinates() {
        assert_eq!(fit_window_axis(-2500, -1920, 0, 1100), -1920);
        assert_eq!(fit_window_axis(-50, -1920, 0, 1100), -1100);
    }
}
