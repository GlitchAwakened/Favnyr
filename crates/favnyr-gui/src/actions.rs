//! System actions (desktop integration) for the context menu — **cross-platform**.
//!
//! Integrations favor local crates and OS APIs:
//!   - `open_path` / `open_parent`: `open` crate (xdg-open / ShellExecute);
//!     `open_parent` also reveals the item (`explorer /select,` on Windows).
//!   - `open_terminal`: launches a terminal with the desired `cwd` (known list
//!     on Linux; Windows Terminal then `cmd` on Windows).
//!   - `copy_to_clipboard`: `arboard` crate (Win / X11 /
//!     Wayland), via a persistent instance (see note below).
//!
//! All actions are **fire-and-forget**: we log the error if one
//! occurs and don't propagate it, so the Slint UI stays responsive.

use std::cell::RefCell;
#[cfg(windows)]
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow};
use favnyr_core::fs as rfs;
use favnyr_core::openers::{Opener, TagContext};
#[cfg(not(windows))]
use favnyr_core::thumbnail::SEVEN_ZIP_PROGRAMS;
use tracing::{debug, info, warn};

#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const SHELL_VERB_OPEN: &[u16] = &[b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
#[cfg(windows)]
const SHELL_VERB_RUNAS: &[u16] = &[
    b'r' as u16,
    b'u' as u16,
    b'n' as u16,
    b'a' as u16,
    b's' as u16,
    0,
];

// The `arboard` clipboard must stay ALIVE to serve the selection on
// X11/Wayland (the content is served by the owning app). We therefore keep
// ONE instance per UI thread, reused on every copy/read, instead of
// creating a disposable one (which would lose the content on Wayland when dropped).
thread_local! {
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

fn with_clipboard<R>(
    f: impl FnOnce(&mut arboard::Clipboard) -> std::result::Result<R, arboard::Error>,
) -> Result<R> {
    CLIPBOARD.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(arboard::Clipboard::new().map_err(|e| anyhow!("init clipboard: {e}"))?);
        }
        // `unwrap` is safe: we just guaranteed `Some`.
        f(slot.as_mut().unwrap()).map_err(|e| anyhow!("clipboard: {e}"))
    })
}

// Reading the clipboard used to live here, for the URL bar's "Paste". That
// menu now goes through the field's own paste, which drops the text at the
// caret instead of replacing the whole line — so nothing reads the clipboard
// from this side any more. Writing it still does, just below.

/// Pushes `text` into the clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    debug!(len = text.len(), "copy to clipboard");
    with_clipboard(|c| c.set_text(text.to_owned()))
}

/// Windows primitive shared by the `open` and `runas` Shell verbs.
///
/// Centralizes UTF-16 encoding, optional parameters, and the working
/// directory. The calling code keeps interpreting the return code, since
/// a UAC cancellation (`runas`) doesn't carry the same message as an open failure.
#[cfg(windows)]
fn shell_execute(
    verb: &[u16],
    target: &OsStr,
    parameters: Option<&OsStr>,
    working_directory: Option<&Path>,
) -> isize {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> isize;
    }

    const SW_SHOWNORMAL: i32 = 1;
    let target_w: Vec<u16> = target.encode_wide().chain(std::iter::once(0)).collect();
    let parameters_w: Option<Vec<u16>> =
        parameters.map(|value| value.encode_wide().chain(std::iter::once(0)).collect());
    let parameters_ptr = parameters_w
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());
    let directory_w: Option<Vec<u16>> = working_directory
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| {
            path.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        });
    let directory_ptr = directory_w
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());

    // ShellExecute is fire-and-forget (independent child, like Explorer).
    unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            target_w.as_ptr(),
            parameters_ptr,
            directory_ptr,
            SW_SHOWNORMAL,
        )
    }
}

/// Opens a path or a URI with the Shell. `working_directory` is only
/// provided for physical paths: a URI like `ms-photos:` has no
/// working directory.
#[cfg(windows)]
fn shell_execute_open(target: &OsStr, working_directory: Option<&Path>) -> Result<()> {
    let result = shell_execute(SHELL_VERB_OPEN, target, None, working_directory);
    if result > 32 {
        Ok(())
    } else {
        Err(anyhow!(
            "opening {} failed (ShellExecute code {result})",
            target.to_string_lossy()
        ))
    }
}

/// Opens `path` with the OS's default application.
///
/// - **Windows**: `ShellExecuteW("open", …)` for the standard association of an
///   item: documents, `.bat`/`.cmd` scripts, `.exe`, and `.lnk`. An image
///   opened from a view first tries [`try_open_image_gallery`] to
///   convey the gallery context that `ShellExecuteW` doesn't carry.
/// - **Other OSes**: `open` crate (xdg-open).
#[cfg(windows)]
pub fn open_path(path: &Path) -> Result<Option<WatchedLaunch>> {
    info!(path = %path.display(), "open with default app (ShellExecute)");
    // `lpDirectory` points to the folder containing the target so that
    // executables and scripts can resolve their relative paths.
    //
    // Never a watch here: the shell performs the launch and keeps no handle
    // on the child, so there is nothing to wait for. The return type matches
    // the other platform rather than diverging over it.
    shell_execute_open(path.as_os_str(), path.parent()).map(|()| None)
}

#[cfg(windows)]
fn default_handler_app_id(extension: &str) -> Result<Option<String>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{ASSOCF_NONE, ASSOCSTR_APPID, AssocQueryStringW};
    use windows::core::{PCWSTR, PWSTR};

    if extension.is_empty() {
        return Ok(None);
    }

    let association = format!(".{}", extension.trim_start_matches('.'));
    let association_w: Vec<u16> = OsStr::new(&association)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut length = 0_u32;

    // The first call just requests the size. Its HRESULT normally
    // indicates the buffer is too small; only the size matters.
    let _ = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_APPID,
            PCWSTR(association_w.as_ptr()),
            PCWSTR::null(),
            None,
            &mut length,
        )
    };
    if length <= 1 {
        return Ok(None);
    }

    let mut output = vec![0_u16; length as usize];
    unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_APPID,
            PCWSTR(association_w.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(output.as_mut_ptr())),
            &mut length,
        )
    }
    .ok()
    .map_err(|error| anyhow!("Windows association for {association}: {error}"))?;

    let text_length = output
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(output.len());
    Ok(Some(String::from_utf16_lossy(&output[..text_length])))
}

#[cfg(windows)]
fn is_microsoft_photos_app_id(app_id: &str) -> bool {
    // The `8wekyb3d8bbwe` suffix is Microsoft's publisher identity: staying exact
    // prevents another similarly-named package from hijacking the reserved route.
    app_id
        .trim()
        .eq_ignore_ascii_case("Microsoft.Windows.Photos_8wekyb3d8bbwe!App")
}

#[cfg(windows)]
fn photos_gallery_target(path: &Path) -> Result<OsString> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    if !path.is_absolute() {
        return Err(anyhow!(
            "the Photos protocol requires an absolute path: {}",
            path.display()
        ));
    }

    // The parameter is a Windows path, not a URL segment: we keep
    // the separators and the "C:", then percent-encode any UTF-8 byte that could
    // change the query's structure (`%`, `#`, `&`, `+`, space, non-ASCII…).
    // Photos 2025.11030+ once again accepts this correct form, unlike
    // a few intermediate 2025 builds that are obsolete today.
    // An invalid UTF-16 sequence can't be faithfully represented in
    // a UTF-8 URI. We reject it here: the caller will fall back to `open_path`,
    // which passes the original `OsStr` to ShellExecuteW without any conversion.
    let path = path
        .to_str()
        .ok_or_else(|| anyhow!("non-Unicode path incompatible with the Photos protocol"))?;
    let mut target = String::with_capacity("ms-photos:viewer?fileName=".len() + path.len());
    target.push_str("ms-photos:viewer?fileName=");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b':' | b'\\' | b'/')
        {
            target.push(byte as char);
        } else {
            target.push('%');
            target.push(HEX[(byte >> 4) as usize] as char);
            target.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    Ok(OsString::from(target))
}

/// Attempts the gallery activation reserved for Microsoft Photos.
///
/// Microsoft Photos on Windows 11 now ignores `NeighboringFilesQuery`,
/// despite a successful WinRT activation. When Photos is genuinely the
/// default handler for the extension, its official protocol restores
/// navigation within the folder. Returns `false` without launching anything for
/// any other viewer: the caller then falls back to the standard OS open.
#[cfg(windows)]
pub fn try_open_image_gallery(path: &Path, extension: &str) -> Result<bool> {
    let Some(app_id) = default_handler_app_id(extension)? else {
        return Ok(false);
    };
    if !is_microsoft_photos_app_id(&app_id) {
        return Ok(false);
    }

    let target = photos_gallery_target(path)?;
    info!(path = %path.display(), app_id, "open image in Microsoft Photos gallery");
    shell_execute_open(target.as_os_str(), None)?;
    Ok(true)
}

/// Is `path` a program this desktop would refuse to start for us?
///
/// The executable bit alone is not enough: a FAT/NTFS/exFAT mount reports
/// everything as `0777`, so an image copied from Windows would qualify. The
/// same filter as the listing is applied — only a recognized binary/script, or
/// a file of unrecognized type (a binary without an extension, common here).
///
/// `.desktop` is deliberately left out even though it looks executable: it is a
/// launcher description, not a program, and only the desktop environment knows
/// how to act on it. Running it as a script would do the wrong thing.
#[cfg(not(windows))]
fn is_launchable_program(path: &Path) -> bool {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if extension.as_deref() == Some("desktop") {
        return false;
    }
    if !favnyr_core::fs::classify_kind(extension.as_deref(), false).can_be_program() {
        return false;
    }
    has_exec_bit(path)
}

/// Executable bit set on a regular file. Follows links, like the listing does:
/// a shortcut to a binary is launchable, and it is the target's bit that
/// decides.
#[cfg(not(windows))]
fn has_exec_bit(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|md| md.is_file() && md.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Is `path` a desktop launcher?
///
/// The executable bit is deliberately NOT required here, unlike for a native
/// program. It was at first, by analogy, and that was wrong: the desktop's own
/// rule for a launcher never looks at it — it reads the entry and starts what
/// it names. Requiring it made every launcher that did not happen to carry the
/// bit fall through to the generic opener, which hands a launcher back as the
/// text file it is made of, in an editor.
///
/// Whether the entry is actually usable is left to the launch itself: an
/// unreadable or `Exec`-less file fails there, and the caller then opens it
/// like any other document.
#[cfg(not(windows))]
fn is_desktop_launcher(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("desktop"))
}

/// Opens `path` the way the desktop would, and hands back a watch when
/// Favnyr ran a program itself.
///
/// Only that case can be watched: anything handed to the desktop is launched
/// by something else, which keeps the handle. The caller waits on the watch
/// off the interface thread, or drops it to stop caring.
#[cfg(not(windows))]
pub fn open_path(path: &Path) -> Result<Option<WatchedLaunch>> {
    // A launcher is acted upon, not opened. Asking the desktop to open one
    // gives back its source text in an editor: KIO only runs an entry when the
    // caller opted in, exactly as it does for a binary, so the same delegation
    // has to be skipped. Favnyr reads the entry itself and starts what it
    // names.
    if is_desktop_launcher(path) {
        match crate::openwith::launch_desktop_file(path) {
            Ok(()) => {
                info!(path = %path.display(), "launch desktop entry");
                return Ok(None);
            }
            // Carries the `.desktop` name but nothing to start: no `Exec`, no
            // `URL`, or unreadable. It is then just a file, and opening it the
            // ordinary way is more useful than reporting a failure.
            Err(err) => {
                debug!(error = %err, path = %path.display(),
                    "not a usable desktop entry, opening it as a document");
            }
        }
    }
    // A program is started directly instead of being handed to the desktop.
    //
    // Handing it over does not work on KDE: the request reaches KIO's
    // `OpenUrlJob`, which runs an executable only when the calling application
    // asked for it through `setRunExecutables(true)`. Dolphin does; the
    // `xdg-open` path does not, and the launch comes back refused with "For
    // security reasons, launching executables is not allowed in this context."
    // That flag belongs to the KDE-side API, so no argument passed to
    // `xdg-open` can reach it — the delegation itself is what has to be
    // skipped, and only for this case.
    if is_launchable_program(path) {
        info!(path = %path.display(), "run program directly");
        // Its own folder, as if started from a terminal there, so a program
        // reading a file next to itself finds it whatever folder Favnyr was
        // started from.
        //
        // This is the one launch Favnyr performs itself, so it is the one
        // whose outcome it can report. A script that dies on its first line
        // used to vanish without a word.
        return spawn_watched_in(path, path.parent()).map(Some);
    }
    info!(path = %path.display(), "open with default app");
    open::that_detached(path)
        .map(|()| None)
        .map_err(|e| anyhow!("opening {}: {e}", path.display()))
}

/// Opens the **parent** folder of `path` and, if possible, **selects**
/// the item (Windows: `explorer /select,`). If `path` is a folder, it's
/// opened directly.
pub fn open_parent(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("explorer");
        if path.is_dir() {
            // A folder is opened directly to show its contents.
            cmd.arg(path);
        } else {
            // FILE → we REVEAL it: `explorer /select,<path>` opens the
            // containing folder AND highlights the file. The path MUST be
            // passed as a RAW argument and quoted AFTER the comma: with `.arg(...)`,
            // `std` wraps the whole `/select,C:\...` token in quotes as soon as there's
            // a space → explorer no longer recognizes `/select,` and opens
            // "Documents" by default instead. `raw_arg` writes exactly
            // `/select,"<path>"`.
            cmd.raw_arg(format!("/select,\"{}\"", path.display()));
        }
        // (explorer often returns a nonzero code even on success → we don't test
        // the status, we just launch it.)
        cmd.spawn().map_err(|e| anyhow!("spawn explorer: {e}"))?;
        info!(path = %path.display(), is_dir = path.is_dir(), "reveal/open in explorer");
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow!("no parent for {}", path.display()))?
        };
        info!(target = %target.display(), "opening parent folder");
        open::that_detached(&target).map_err(|e| anyhow!("opening {}: {e}", target.display()))
    }
}

/// Launches an **opener** ("Open with") on `paths`, detached and WITHOUT a shell
/// (security: `argv` as a list, never a concatenated string). `{file}` (or
/// no tag at all) → one instance per file / all paths queued together.
pub fn run_opener(opener: &Opener, paths: &[PathBuf]) -> Result<()> {
    // App based on an OS association handler (Windows UWP/Store, Linux
    // `.desktop`): no exe to launch via `Command` → we delegate to the OS, one
    // file at a time (the current file's extension resolves the right handler).
    if let Some(assoc) = &opener.assoc {
        if paths.is_empty() {
            return Err(anyhow!("no file to open with {}", opener.label));
        }
        for p in paths {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            crate::openwith::launch(assoc, &ext, p)?;
        }
        return Ok(());
    }
    // Resolve exactly as the editor validates it. Keeping the stored text
    // untouched lets environment changes apply to later launches.
    let Some(program) = launchable_program_path(&opener.program) else {
        return Err(anyhow!("program not found: {}", opener.program));
    };
    if paths.is_empty() {
        return spawn_detached(&program, opener.args.clone(), opener.elevated);
    }
    if opener.expands_list() {
        // A single instance covering the whole selection: `{files}` expands
        // where it stands. The other tags describe the first item — a selection
        // always comes from one folder, so `{dir}` and `{dirname}` designate
        // the folder shared by every path.
        let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let ctx = TagContext::from_path(&paths[0]);
        spawn_detached(
            &program,
            opener.render_for_batch(&ctx, &refs),
            opener.elevated,
        )
    } else if opener.has_tag() {
        // One instance per file (the template references the file).
        for p in paths {
            spawn_detached(
                &program,
                opener.render_for_file(&TagContext::from_path(p)),
                opener.elevated,
            )?;
        }
        Ok(())
    } else {
        // A single instance: fixed arguments + all paths queued.
        let mut argv = opener.args.clone();
        argv.extend(paths.iter().map(|p| p.display().to_string()));
        spawn_detached(&program, argv, opener.elevated)
    }
}

/// Launches `program` with `args` (detached). Exposed for the "Open
/// with" picker, which resolves the program and arguments itself.
pub fn spawn_program(program: &Path, args: &[String]) -> Result<()> {
    spawn_detached(program, args.to_vec(), false)
}

/// [`spawn_program`] started in `cwd`, for a launcher that names the folder its
/// application expects to run from (`Path=` in a desktop entry).
#[cfg_attr(windows, allow(dead_code))]
pub fn spawn_program_in(program: &Path, args: &[String], cwd: Option<&Path>) -> Result<()> {
    spawn_detached_in(program, args.to_vec(), false, cwd)
}

/// Launches an opener on a folder from a command pinned to the view's
/// background, e.g. "open a shell here". In this context, `{dir}` and `{file}`
/// both equal the current folder (in `TagContext::from_path`, `{dir}` would be
/// the PARENT — counter-intuitive for an "here" command). OS association apps:
/// delegated to the OS on the folder.
pub fn run_opener_dir(opener: &Opener, dir: &Path) -> Result<()> {
    if let Some(assoc) = &opener.assoc {
        return crate::openwith::launch(assoc, "", dir);
    }
    let Some(program) = launchable_program_path(&opener.program) else {
        return Err(anyhow!("program not found: {}", opener.program));
    };
    let mut ctx = TagContext::from_path(dir);
    ctx.dir = ctx.file.clone();
    spawn_detached(&program, opener.render_for_file(&ctx), opener.elevated)
}

/// Launches one window with all tabs, their settings and the active index.
/// Positions are physical screen coordinates. Paths travel as native arguments
/// independently of the serialized settings. A failed spawn leaves the source
/// untouched, whether the caller is detaching a single tab or a whole view.
pub fn spawn_detached_view(
    view: &favnyr_core::workspace::ViewState,
    dirs: &[PathBuf],
    at: Option<(i32, i32)>,
) -> Result<()> {
    let args = crate::detached_view::arguments(view, dirs, at)?;
    let exe = std::env::current_exe().map_err(|e| anyhow!("Favnyr executable not found: {e}"))?;
    info!(tabs = dirs.len(), at = ?at, "spawn new Favnyr instance (view tear-off)");
    let mut cmd = Command::new(&exe);
    cmd.args(args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| anyhow!("new Favnyr instance: {e}"))
}

/// Detached spawn (stdio cut off; on Windows, no ghost console).
///
/// `elevated` (Windows only): launches via `ShellExecuteW("runas", …)` →
/// UAC prompt. On other OSes the flag is ignored (elevating a graphical
/// app isn't a standard mechanism there): normal launch.
fn spawn_detached(program: &Path, argv: Vec<String>, elevated: bool) -> Result<()> {
    spawn_detached_in(program, argv, elevated, None)
}

/// [`spawn_detached`] with a working directory for the child. Split out rather
/// than added to every call site, which has no folder to impose and passes
/// `None`.
fn spawn_detached_in(
    program: &Path,
    argv: Vec<String>,
    elevated: bool,
    cwd: Option<&Path>,
) -> Result<()> {
    info!(program = %program.display(), argc = argv.len(), elevated, "run opener");
    #[cfg(windows)]
    if elevated {
        return shell_execute_runas(program, &argv);
    }
    let mut cmd = detached_command(program, &argv, cwd);
    cmd.stderr(Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| anyhow!("launching {}: {e}", program.display()))
}

/// The command both spawn paths start from: no input, no output, and no
/// startup token inherited. Only the handling of standard error differs
/// between them, so that is the one thing left to the caller.
fn detached_command(program: &Path, argv: &[String], cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(argv).stdin(Stdio::null()).stdout(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // Startup-notification tokens are valid for ONE launch: the desktop hands
    // one to Favnyr, and a child inheriting it would have its own startup
    // credited to Favnyr's window. Unset everywhere, they do not exist on
    // Windows, where removing them is a no-op.
    cmd.env_remove("DESKTOP_STARTUP_ID")
        .env_remove("XDG_ACTIVATION_TOKEN");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A launch whose outcome is still being waited for.
pub struct WatchedLaunch {
    child: std::process::Child,
    started: std::time::Instant,
    program: PathBuf,
}

/// What a program said on its way out.
pub struct EarlyFailure {
    /// Exit status, absent when a signal ended the process.
    pub code: Option<i32>,
    /// The tail of its standard error, already trimmed. May be empty.
    pub message: String,
}

/// A failure reported after this long is no longer read as an answer to the
/// launch. A document editor closed with an error two hours later must not
/// raise a banner out of nowhere.
const EARLY_FAILURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(4);

/// How much of the standard error is kept. The end of it, because that is
/// where a program puts the reason it is giving up.
const KEPT_STDERR_BYTES: usize = 2 * 1024;

impl WatchedLaunch {
    /// Blocks until the program exits, then reports a failure only if it came
    /// quickly enough to be understood as an answer to the launch.
    ///
    /// Reading to the end of the pipe is what waits here, rather than polling.
    /// It has to be read at all: a pipe nobody drains fills its kernel buffer
    /// and **blocks the child**, so a chatty program would be frozen by the
    /// very mechanism meant to report on it. Closing the read end instead
    /// would be worse, since writing to a closed pipe raises SIGPIPE, whose
    /// default action is to kill the program.
    ///
    /// The cost is one thread per running child, parked in a read that uses no
    /// processor time and a fixed amount of memory.
    pub fn wait_for_early_failure(mut self) -> Option<EarlyFailure> {
        use std::io::Read;

        let mut kept: Vec<u8> = Vec::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        kept.extend_from_slice(&chunk[..read]);
                        // Trimmed as it grows so a program writing for hours
                        // costs a bounded buffer and not its whole output.
                        if kept.len() > KEPT_STDERR_BYTES {
                            let cut = kept.len() - KEPT_STDERR_BYTES;
                            kept.drain(..cut);
                        }
                    }
                }
            }
        }

        let status = self.child.wait().ok()?;
        let elapsed = self.started.elapsed();
        if status.success() {
            debug!(program = %self.program.display(), ?elapsed, "program finished");
            return None;
        }
        let message = String::from_utf8_lossy(&kept).trim().to_string();
        if elapsed > EARLY_FAILURE_WINDOW {
            // Worth a line at the default level even though no banner is raised.
            info!(program = %self.program.display(), ?elapsed, code = ?status.code(),
                reason = %one_line(&message),
                "program failed, too late to be shown as a launch failure");
            return None;
        }
        warn!(program = %self.program.display(), code = ?status.code(),
            reason = %one_line(&message),
            "program failed right after being launched");
        Some(EarlyFailure {
            code: status.code(),
            message,
        })
    }
}

/// Flattens captured output into something a log file can hold.
///
/// A log is read and filtered one line per event. Pasting a program's whole
/// standard error into a field breaks that: forty lines of build noise turn a
/// single failure into forty entries for anything reading the file. The
/// newlines become separators and the whole is capped, since the detail
/// beyond the last few lines has never explained anything.
fn one_line(message: &str) -> String {
    const MAX: usize = 400;
    let mut flat = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    if flat.len() > MAX {
        // Cut from the front: the reason sits at the end.
        let start = flat
            .char_indices()
            .nth(flat.chars().count().saturating_sub(MAX))
            .map_or(0, |(i, _)| i);
        flat = format!("… {}", &flat[start..]);
    }
    flat
}

/// Starts `program` keeping its standard error, so an immediate failure can be
/// reported instead of vanishing.
///
/// Spawning happens here, on the caller's thread, so a program that cannot be
/// started at all is an error straight away. Only the waiting is left to the
/// caller, which must do it off the interface thread.
///
/// Unix only: on Windows every launch is handed to the shell, which keeps the
/// handle, so there is never a child of ours to watch.
#[cfg(not(windows))]
fn spawn_watched_in(program: &Path, cwd: Option<&Path>) -> Result<WatchedLaunch> {
    let mut cmd = detached_command(program, &[], cwd);
    cmd.stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| anyhow!("launching {}: {e}", program.display()))?;
    Ok(WatchedLaunch {
        child,
        started: std::time::Instant::now(),
        program: program.to_path_buf(),
    })
}

/// Launches executable `exe` with `files` as ARGUMENTS (the "drag a file
/// onto a .cmd/.bat/.exe" pattern). Not elevated.
///
/// Windows: `ShellExecuteW("open", …)` → handles `.cmd`/`.bat` (via their
/// `cmd.exe /c` association) just like `.exe`, which `CreateProcess`/`Command`
/// does NOT do for a script. Other OSes: detached `Command::new(exe).args(files)`
/// (the executable — shebang script or binary — receives the paths).
#[cfg(windows)]
pub fn run_with_files(exe: &Path, files: &[PathBuf]) -> Result<()> {
    info!(exe = %exe.display(), argc = files.len(), "run executable with dropped files");
    let mut params = String::new();
    for (i, f) in files.iter().enumerate() {
        if i > 0 {
            params.push(' ');
        }
        win_append_arg(&mut params, &f.to_string_lossy());
    }
    // Working directory = the executable's folder (like a double-click).
    let r = shell_execute(
        SHELL_VERB_OPEN,
        exe.as_os_str(),
        Some(OsStr::new(&params)),
        exe.parent(),
    );
    if r > 32 {
        Ok(())
    } else {
        Err(anyhow!(
            "launching {} failed (ShellExecute code {r})",
            exe.display()
        ))
    }
}
#[cfg(not(windows))]
pub fn run_with_files(exe: &Path, files: &[PathBuf]) -> Result<()> {
    info!(exe = %exe.display(), argc = files.len(), "run executable with dropped files");
    let mut cmd = Command::new(exe);
    for f in files {
        cmd.arg(f);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| anyhow!("launching {}: {e}", exe.display()))
}

/// Opens `path` ELEVATED (UAC prompt) — the context menu's "Run as
/// administrator". On an `.exe`: "run as administrator"; on
/// a document, fails cleanly if the type has no registered "runas" verb.
/// Windows only (the menu entry is hidden elsewhere).
#[cfg(windows)]
pub fn open_elevated(path: &Path) -> Result<()> {
    shell_execute_runas(path, &[])
}
#[cfg(not(windows))]
pub fn open_elevated(path: &Path) -> Result<()> {
    let _ = path;
    Err(anyhow!("elevation: Windows only"))
}

/// Launches `program` ELEVATED (UAC prompt) via `ShellExecuteW` with the
/// "runas" verb — Windows only. `Command`/`CreateProcess` can't elevate;
/// only the Shell can. The arguments are reassembled into ONE command line
/// using `CommandLineToArgvW`'s quoting (backslashes/quotes) — the
/// target program thus receives the SAME `argv` as with a direct spawn.
#[cfg(windows)]
fn shell_execute_runas(program: &Path, argv: &[String]) -> Result<()> {
    const SE_ERR_ACCESSDENIED: isize = 5; // includes cancellation of the UAC prompt
    info!(program = %program.display(), argc = argv.len(), "run opener elevated (runas)");
    let mut params = String::new();
    for (i, a) in argv.iter().enumerate() {
        if i > 0 {
            params.push(' ');
        }
        win_append_arg(&mut params, a);
    }
    // Fire-and-forget, like `open_path`: working directory inherited from Favnyr
    // (parity with the non-elevated spawn, which also doesn't set the cwd).
    let r = shell_execute(
        SHELL_VERB_RUNAS,
        program.as_os_str(),
        (!argv.is_empty()).then(|| OsStr::new(&params)),
        None,
    );
    if r > 32 {
        Ok(())
    } else if r == SE_ERR_ACCESSDENIED {
        Err(anyhow!("elevation refused (UAC) for {}", program.display()))
    } else {
        Err(anyhow!(
            "admin launch of {} failed (ShellExecute code {r})",
            program.display()
        ))
    }
}

/// Appends `arg` to a Windows command line using `CommandLineToArgvW`'s
/// quoting rules (quotes if space/tab/quote; backslashes doubled
/// before a quote or the end of a token). Ported from `std::sys::…::append_arg`.
#[cfg(windows)]
fn win_append_arg(cmd: &mut String, arg: &str) {
    let quote = arg.is_empty() || arg.contains([' ', '\t', '"']);
    if quote {
        cmd.push('"');
    }
    let mut backslashes: usize = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                // N backslashes + the quote → 2N+1 backslashes then the `"`.
                for _ in 0..=backslashes {
                    cmd.push('\\');
                }
            }
            backslashes = 0;
        }
        cmd.push(c);
    }
    if quote {
        // Trailing backslashes doubled before the closing quote.
        for _ in 0..backslashes {
            cmd.push('\\');
        }
        cmd.push('"');
    }
}

/// Opens the OS's native **"Open with"** picker for `path`.
///
/// Windows: standard Shell dialog ("How do you want to open this
/// file?") via `rundll32 shell32.dll,OpenAs_RunDLL <path>`. The path is
/// passed as a **raw argument** (`raw_arg`) to prevent `std` from wrapping it
/// in quotes — `OpenAs_RunDLL` takes the entire rest of the command line
/// as-is (quotes would break opening paths containing spaces).
///
/// Other OSes: not supported (no universal picker) — the corresponding
/// menu entry is hidden outside Windows.
#[cfg(windows)]
pub fn open_with(path: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    info!(path = %path.display(), "open-with dialog");
    Command::new("rundll32.exe")
        .raw_arg(format!("shell32.dll,OpenAs_RunDLL {}", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow!("spawn rundll32 (open with): {e}"))
}
#[cfg(not(windows))]
pub fn open_with(_path: &Path) -> Result<()> {
    Err(anyhow!("\"Open with\" is not supported on this platform"))
}

/// Shows Windows' **native properties dialog** for `path` (General /
/// Security / Details / Previous Versions tabs…). Via `ShellExecuteExW`
/// with the "properties" verb and `SEE_MASK_INVOKEIDLIST`. A direct FFI to
/// shell32 avoids an extra dependency. Only exists on Windows:
/// other OSes have no portable native properties dialog → the caller
/// falls back to the custom properties panel.
#[cfg(windows)]
pub fn show_native_properties(path: &Path) -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    // Win32 layout of SHELLEXECUTEINFOW (x64). The last field before
    // `h_process` is the hIcon/hMonitor union (a HANDLE → a pointer).
    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut core::ffi::c_void,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: *mut core::ffi::c_void,
        lp_id_list: *mut core::ffi::c_void,
        lp_class: *const u16,
        hkey_class: *mut core::ffi::c_void,
        dw_hot_key: u32,
        h_icon_or_monitor: *mut core::ffi::c_void,
        h_process: *mut core::ffi::c_void,
    }

    // SEE_MASK_INVOKEIDLIST: allows the "properties" verb (goes through the IDList).
    const SEE_MASK_INVOKEIDLIST: u32 = 0x0000_000C;
    const SW_SHOW: i32 = 5;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }

    let verb: Vec<u16> = OsStr::new("properties")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let file: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: zero-initialized structure then filled in per Win32 docs;
    // `verb`/`file` (NUL-terminated) stay alive for the duration of the call.
    let mut info: ShellExecuteInfoW = unsafe { std::mem::zeroed() };
    info.cb_size = std::mem::size_of::<ShellExecuteInfoW>() as u32;
    info.f_mask = SEE_MASK_INVOKEIDLIST;
    info.lp_verb = verb.as_ptr();
    info.lp_file = file.as_ptr();
    info.n_show = SW_SHOW;

    info!(path = %path.display(), "native properties dialog");
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return Err(anyhow!(
            "ShellExecuteExW(properties) failed for {}",
            path.display()
        ));
    }
    Ok(())
}

/// Encodes `path` as a strict `file://` URI (RFC 3986) for D-Bus: anything
/// that isn't "unreserved" (`A-Z a-z 0-9 - . _ ~`) or the `/` separator is
/// percent-encoded. Deliberately STRICTER than `openers::file_uri` (which
/// only encodes spaces, good enough for editors): the GVariant literal
/// passed to `gdbus` must stay unambiguous even with `'`, `"`, `#`, or `?`.
#[cfg(all(unix, not(target_os = "macos")))]
fn dbus_file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(b as char)
            }
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

/// True if a file manager exposing `org.freedesktop.FileManager1`
/// is reachable (already running or activatable on demand via D-Bus). A
/// **fast** check (~6 ms measured) that queries the bus daemon itself — definitely
/// not `ShowItemProperties`, which only returns once the dialog is closed.
#[cfg(all(unix, not(target_os = "macos")))]
fn file_manager1_available() -> bool {
    // `ListActivatableNames` covers installed handlers (`.service`
    // file), `ListNames` those already running.
    ["ListActivatableNames", "ListNames"].iter().any(|method| {
        Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.DBus",
                "--object-path",
                "/org/freedesktop/DBus",
                "--method",
            ])
            .arg(format!("org.freedesktop.DBus.{method}"))
            .output()
            .ok()
            .filter(|out| out.status.success())
            .is_some_and(|out| {
                String::from_utf8_lossy(&out.stdout).contains("org.freedesktop.FileManager1")
            })
    })
}

/// **Desktop-native** properties dialog on Linux, via the freedesktop
/// interface `org.freedesktop.FileManager1.ShowItemProperties` — implemented
/// by the file managers of the common desktops (KDE, GNOME, Cinnamon, MATE…). We go
/// through `gdbus` (glib2, present on all common desktops): **no
/// dependency added**, compliant with the cargo-deny policy.
///
/// Two EMPIRICALLY VERIFIED constraints dictate this design:
///  - the call **blocks until the dialog closes** (observed with
///    Dolphin) → the process is launched without waiting for it, and a detached
///    thread reaps it (no zombie, no UI freeze);
///  - no handler is guaranteed (minimal session, bare WM) → a fast
///    pre-check, and `Err` otherwise so the caller falls back to the internal panel.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn show_native_properties(path: &Path) -> Result<()> {
    if !file_manager1_available() {
        return Err(anyhow!(
            "org.freedesktop.FileManager1 unavailable (no desktop file manager)"
        ));
    }
    // GVariant literal: array of URIs (the URI is percent-encoded → safe within
    // quotes), then the startup identifier, empty.
    let uris = format!("[\"{}\"]", dbus_file_uri(path));
    info!(path = %path.display(), "native properties dialog (FileManager1)");
    let child = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItemProperties",
        ])
        .arg(&uris)
        .arg("")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("spawning gdbus (properties): {e}"))?;
    // The child lives as long as the dialog is open: we reap it separately
    // to avoid leaving a zombie, without ever blocking the UI thread.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// Platforms with no usable native properties dialog (macOS): the failure
/// is reported to the user, like for any missing desktop integration.
#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
pub fn show_native_properties(_path: &Path) -> Result<()> {
    Err(anyhow!(
        "native properties dialog not supported on this platform"
    ))
}

/// Opens the Windows Recycle Bin in Explorer. The Recycle Bin
/// is a *virtual* folder there (no real path) → we go through the shell.
#[cfg(windows)]
pub fn open_trash() -> Result<()> {
    Command::new("explorer")
        .arg("shell:RecycleBinFolder")
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow!("spawn explorer (recycle bin): {e}"))
}

/// Builds the silent launch of the Windows Terminal alias. The program is
/// named rather than assumed, so a copy chosen by browsing for it is started
/// with the same option as the one found on the `PATH`.
#[cfg(windows)]
fn windows_terminal_command(program: &str, cwd: &Path) -> Command {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new(program);
    command
        .arg("-d")
        .arg(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // `wt.exe` in WindowsApps is a very brief console alias/launcher.
        // Hiding it doesn't affect the actual Windows Terminal GUI window.
        .creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(windows)]
fn command_prompt_command(cwd: &Path) -> Command {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new("cmd.exe");
    command
        // Favnyr is a GUI application with no parent console: we explicitly
        // create THE final interactive console, at the right cwd.
        .current_dir(cwd)
        .creation_flags(CREATE_NEW_CONSOLE);
    command
}

/// Builds the launch of a terminal named in the settings.
///
/// Windows Terminal takes the working directory as an option, because it
/// hands the request to an instance that is already running and whose own
/// directory is elsewhere. The others are started in the directory itself.
#[cfg(windows)]
fn chosen_terminal_command(program: &str, cwd: &Path) -> Command {
    use std::os::windows::process::CommandExt;

    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    if name == "wt.exe" {
        return windows_terminal_command(program, cwd);
    }
    let mut command = Command::new(program);
    command.current_dir(cwd);
    // A shell is a console program and Favnyr, being graphical, has no
    // console to hand down, so one is created for it. A graphical terminal
    // opens a window of its own and is given none.
    if matches!(name.as_str(), "cmd.exe" | "powershell.exe" | "pwsh.exe") {
        command.creation_flags(CREATE_NEW_CONSOLE);
    } else {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Several terminals ship a small console launcher next to their
            // graphical program, and browsing for one may well land on it.
            // Windows allocates a console for a console-subsystem image that
            // is given none, which flashes a window before the terminal
            // appears; the decision comes from the image header and these
            // flags, so redirecting the handles above does not prevent it. A
            // genuinely graphical program has no console either way, so this
            // costs it nothing.
            .creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Opens a terminal with `cwd` set to the folder containing `path`.
///
/// `preferred` is the terminal chosen in the settings, empty for none. It is
/// scoped to this application: nothing here consults or changes what the
/// system opens elsewhere. A terminal that cannot be started any more, having
/// been uninstalled since it was chosen, gives way to the automatic choice,
/// because a shortcut that opens another terminal beats one that does nothing.
pub fn open_terminal(path: &Path, preferred: &str) -> Result<()> {
    let cwd = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("no parent for {}", path.display()))?
    };
    let chosen = if preferred.is_empty() {
        None
    } else if terminal_is_available(preferred) {
        Some(preferred)
    } else {
        warn!(
            terminal = %preferred,
            "the terminal chosen in the settings was not found, falling back"
        );
        None
    };

    #[cfg(windows)]
    {
        if let Some(program) = chosen {
            match chosen_terminal_command(program, &cwd).spawn() {
                Ok(_) => {
                    info!(terminal = %program, cwd = %cwd.display(), "opening the chosen terminal");
                    return Ok(());
                }
                Err(error) => {
                    warn!(error = %error, terminal = %program, "chosen terminal failed to start");
                }
            }
        }
        // Windows Terminal (`wt`) first: clean window, `-d` = cwd.
        match windows_terminal_command("wt.exe", &cwd).spawn() {
            Ok(_) => {
                info!(cwd = %cwd.display(), "opening Windows Terminal");
                return Ok(());
            }
            Err(error) => {
                debug!(error = %error, "Windows Terminal unavailable; falling back to cmd");
            }
        }
        // Direct fallback: the old `cmd /C start … cmd /K` chain created a
        // first intermediate cmd window visible for a few milliseconds, then the
        // final terminal. A single `cmd.exe` with `CREATE_NEW_CONSOLE` is enough.
        command_prompt_command(&cwd)
            .spawn()
            .map(|_| ())
            .map_err(|e| anyhow!("spawn cmd: {e}"))
    }
    #[cfg(not(windows))]
    {
        let term = match chosen {
            Some(program) => program.to_string(),
            None => pick_terminal()
                .ok_or_else(|| anyhow!("no terminal detected ($TERMINAL or known candidates)"))?,
        };
        info!(terminal = %term, cwd = %cwd.display(), "opening a terminal");
        Command::new(&term)
            .args(working_directory_args(&term, &cwd))
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| anyhow!("spawn '{term}' in '{}': {e}", cwd.display()))
    }
}

/// Terminals the settings offer, in the order the list shows them. The same
/// list drives the automatic choice, which takes the first one present.
///
/// They are names resolved through the `PATH`, which is where terminals are
/// installed here, so what the list offers is what the machine really has.
#[cfg(not(windows))]
const TERMINAL_CANDIDATES: &[&str] = &[
    "kgx",            // GNOME Console
    "konsole",        // KDE
    "gnome-terminal", // legacy GNOME
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "xfce4-terminal",
    "lxterminal",
    "tilix",
    "xterm",
];

/// Terminals the settings offer, in the order Favnyr already tries them.
///
/// Only what Windows itself provides is named. Other terminals install
/// outside the `PATH`, so looking for them by name would find nothing and
/// offering them by name would promise something that cannot be started:
/// they are chosen by browsing for the program instead. `wt.exe` is listed
/// although it is absent from older systems, exactly as the automatic choice
/// already tries it before falling back.
#[cfg(windows)]
const TERMINAL_CANDIDATES: &[&str] = &["wt.exe", "powershell.exe", "cmd.exe"];

/// The terminals to list in the settings. Empty is a valid answer, and means
/// the automatic choice is the only one on offer.
#[cfg(not(windows))]
pub fn detected_terminals() -> Vec<String> {
    TERMINAL_CANDIDATES
        .iter()
        .filter(|c| which(c))
        .map(|c| (*c).to_string())
        .collect()
}

#[cfg(windows)]
pub fn detected_terminals() -> Vec<String> {
    TERMINAL_CANDIDATES
        .iter()
        .map(|c| (*c).to_string())
        .collect()
}

/// True when the terminal named in the settings can still be started.
///
/// A terminal chosen by browsing for it is a full path, and that path must
/// still lead to a file: this is what notices one uninstalled since.
#[cfg(not(windows))]
fn terminal_is_available(program: &str) -> bool {
    if program.contains('/') {
        Path::new(program).is_file()
    } else {
        which(program)
    }
}

/// Same, for a platform where a bare name is not looked up in the `PATH`.
/// The list only offers terminals the system provides, so such a name is
/// taken on trust and a launch that fails falls back as it always did.
#[cfg(windows)]
fn terminal_is_available(program: &str) -> bool {
    let path = Path::new(program);
    !path.is_absolute() || path.is_file()
}

// ----- Internal helpers (Linux only) -----

/// The argument that names the working directory, for terminals that ignore
/// the one their process was started in.
///
/// A single-instance terminal does not open the window itself: it hands the
/// request to a server already running elsewhere, usually over D-Bus, and that
/// server has its own working directory. Setting `current_dir` then reaches a
/// process that only forwards the message, and the window opens in the home
/// folder. Naming the directory in an argument travels with the request.
///
/// Terminals absent from this list are left alone: they honour the directory
/// they are started in, and an argument they do not know would make them
/// refuse to start at all.
#[cfg(not(windows))]
fn working_directory_args(term: &str, cwd: &Path) -> Vec<std::ffi::OsString> {
    use std::ffi::OsString;

    // $TERMINAL may hold a full path, and the table is keyed by program name.
    let name = Path::new(term)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(term);
    match name {
        "gnome-terminal" | "kgx" | "xfce4-terminal" | "tilix" | "lxterminal" => {
            let mut arg = OsString::from("--working-directory=");
            arg.push(cwd.as_os_str());
            vec![arg]
        }
        "konsole" => vec![OsString::from("--workdir"), cwd.as_os_str().to_os_string()],
        _ => Vec::new(),
    }
}

#[cfg(not(windows))]
pub(crate) fn pick_terminal() -> Option<String> {
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.is_empty() && which(&t) {
            return Some(t);
        } else if !t.is_empty() {
            tracing::warn!(term = %t, "$TERMINAL set but not found, falling back");
        }
    }
    TERMINAL_CANDIDATES
        .iter()
        .find(|c| which(c))
        .map(|c| (*c).to_string())
}

#[cfg(not(windows))]
fn which(cmd: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(cmd);
        candidate.is_file() || candidate.is_symlink()
    })
}

/// First of `candidates` that can actually be launched, or `None`.
///
/// Drives the ready-made commands offered in the settings: they are only
/// listed when their tool is really installed, so the list never advertises
/// something the machine cannot run.
///
/// A single list serves both platforms. Bare names resolve through PATH
/// outside Windows and fail the file test on it; absolute Windows paths do the
/// reverse. Each entry is therefore silently skipped where it makes no sense.
pub fn resolve_program(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|c| program_is_valid(c))
        .map(|c| (*c).to_string())
}

/// Expands the platform's user-facing path syntax without changing the value
/// persisted in the custom command.
pub(crate) fn expand_program_path(program: &str) -> PathBuf {
    rfs::expand_typed_path(program.trim())
}

/// Resolves a custom command's executable to the exact path passed to the OS.
/// Outside Windows, a bare command may deliberately remain relative so
/// [`Command`] can resolve it through `PATH`.
fn launchable_program_path(program: &str) -> Option<PathBuf> {
    let expanded = expand_program_path(program);
    if expanded.as_os_str().is_empty() {
        return None;
    }
    if expanded.is_file() {
        return Some(expanded);
    }
    #[cfg(not(windows))]
    if let Some(command) = expanded.to_str()
        && !command.contains('/')
        && which(command)
    {
        return Some(expanded);
    }
    None
}

/// Is an opener's "Program" field launchable? An existing file path after
/// expanding `~` and platform environment variables, or — especially on Linux
/// — a simple command resolvable through `PATH` (`viewer`, `editor`…).
pub fn program_is_valid(program: &str) -> bool {
    launchable_program_path(program).is_some()
}

/// Is `ffmpeg` available to generate video thumbnails? On Windows,
/// video goes through the native shell (never `ffmpeg`) → always "yes"
/// (no warning to show). On Linux, resolved via `PATH`.
pub fn ffmpeg_available() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(not(windows))]
    {
        which("ffmpeg")
    }
}

#[derive(Clone, Debug, Default)]
pub struct ToolInfo {
    pub available: bool,
    pub version: String,
}

#[derive(Clone, Debug, Default)]
pub struct OptionalToolsInfo {
    pub ffmpeg: ToolInfo,
    pub poppler: ToolInfo,
    pub seven_zip: ToolInfo,
    pub flatpak: bool,
}

/// Detects the optional Linux command-line tools only when the expandable
/// settings card requests it. Windows uses native preview providers and does
/// not display this card.
pub fn optional_tools_info() -> OptionalToolsInfo {
    #[cfg(windows)]
    {
        OptionalToolsInfo::default()
    }
    #[cfg(not(windows))]
    {
        let ffmpeg = ffmpeg_available();
        let poppler_program = resolve_program(&["pdftoppm", "pdftocairo"]);
        let seven_zip_program = resolve_program(SEVEN_ZIP_PROGRAMS);
        OptionalToolsInfo {
            ffmpeg: ToolInfo {
                available: ffmpeg,
                version: ffmpeg.then(ffmpeg_version).flatten().unwrap_or_default(),
            },
            poppler: tool_info(poppler_program.as_deref(), &["-v"]),
            seven_zip: tool_info(seven_zip_program.as_deref(), &["i"]),
            flatpak: in_flatpak(),
        }
    }
}

#[cfg(not(windows))]
fn tool_info(program: Option<&str>, args: &[&str]) -> ToolInfo {
    let Some(program) = program else {
        return ToolInfo::default();
    };
    let version = Command::new(program)
        .args(args)
        .output()
        .ok()
        .and_then(|output| {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            version_token(&text)
        })
        .unwrap_or_default();
    ToolInfo {
        available: true,
        version,
    }
}

#[cfg(not(windows))]
fn version_token(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.');
        let token = token.strip_prefix(['n', 'v']).unwrap_or(token);
        (token.starts_with(|c: char| c.is_ascii_digit()) && token.contains('.')).then(|| {
            token
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .to_string()
        })
    })
}

/// `ffmpeg` version cleaned up for display ("8.1.2"), read via
/// `ffmpeg -version`. Tolerates distro package prefixes/suffixes
/// ("n7.0.2" → "7.0.2", "4.4.2-0ubuntu…" → "4.4.2"). `None` if absent.
#[cfg(not(windows))]
fn ffmpeg_version() -> Option<String> {
    // Windows never uses ffmpeg → no spawn (settings also open on
    // Windows and trigger re-detection).
    #[cfg(windows)]
    {
        None
    }
    #[cfg(not(windows))]
    {
        let out = Command::new("ffmpeg").arg("-version").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let tok = stdout
            .lines()
            .next()?
            .strip_prefix("ffmpeg version ")?
            .split_whitespace()
            .next()?;
        // Strips a leading "n" (Arch: "n7.0.2") then cuts at the 1st "-".
        let tok = tok
            .strip_prefix('n')
            .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
            .unwrap_or(tok);
        let clean = tok.split('-').next().unwrap_or(tok);
        (!clean.is_empty()).then(|| clean.to_string())
    }
}

/// Is Favnyr running inside a **Flatpak** sandbox? Optional tools are then
/// resolved inside its runtime rather than from the host.
#[cfg(not(windows))]
fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists()
}

/// Offset (seconds) between LOCAL time and UTC, current DST included — to
/// ADD to a Unix (UTC) timestamp to get local time. Used for the
/// "Modified" column. 0 on failure (falls back to UTC).
///
/// Windows: `GetTimeZoneInformation` (bias + active seasonal bias), raw FFI
/// (no extra `windows` feature). Other OSes: libc's `localtime_r`
/// and its `tm_gmtoff` field (glibc/musl/BSD extension).
#[cfg(windows)]
pub fn local_utc_offset_secs() -> i64 {
    #[repr(C)]
    struct SysTime {
        year: u16,
        month: u16,
        dow: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        ms: u16,
    }
    #[repr(C)]
    struct Tzi {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SysTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SysTime,
        daylight_bias: i32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTimeZoneInformation(info: *mut Tzi) -> u32;
    }
    const TIME_ZONE_ID_STANDARD: u32 = 1;
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    const TIME_ZONE_ID_INVALID: u32 = 0xFFFF_FFFF;
    unsafe {
        let mut tzi: Tzi = std::mem::zeroed();
        let r = GetTimeZoneInformation(&mut tzi);
        if r == TIME_ZONE_ID_INVALID {
            return 0;
        }
        let seasonal = match r {
            TIME_ZONE_ID_DAYLIGHT => tzi.daylight_bias,
            TIME_ZONE_ID_STANDARD => tzi.standard_bias,
            _ => 0,
        };
        // Win32 doc: UTC = local + (Bias + seasonal bias) minutes.
        // → offset to add to UTC to get local = −(Bias + seasonal).
        -((tzi.bias + seasonal) as i64) * 60
    }
}

#[cfg(not(windows))]
pub fn local_utc_offset_secs() -> i64 {
    // libc's `struct tm` (x86_64): 9 × int, then `tm_gmtoff` (long, offset 40
    // after padding) and `tm_zone` (ptr). `#[repr(C)]` reproduces this layout.
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const i8,
    }
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, result: *mut Tm) -> *mut Tm;
    }
    unsafe {
        let now = time(std::ptr::null_mut());
        let mut tm: Tm = std::mem::zeroed();
        if localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
        tm.gmtoff
    }
}

#[cfg(test)]
mod opener_program_path_tests {
    use super::expand_program_path;
    #[cfg(not(windows))]
    use super::version_token;
    use std::path::PathBuf;

    #[test]
    fn custom_programs_reuse_typed_path_expansion() {
        let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
        assert_eq!(expand_program_path("~/bin/tool"), home.join("bin/tool"));
        assert_eq!(expand_program_path("  tool  "), PathBuf::from("tool"));

        #[cfg(not(windows))]
        if let Ok(value) = std::env::var("HOME") {
            assert_eq!(
                expand_program_path("$HOME/bin/tool"),
                PathBuf::from(value).join("bin/tool")
            );
        }
        #[cfg(windows)]
        if let Ok(value) = std::env::var("TEMP") {
            assert_eq!(
                expand_program_path(r"%TEMP%\bin\tool.exe"),
                PathBuf::from(value).join("bin").join("tool.exe")
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn optional_tool_versions_are_read_from_their_native_output() {
        assert_eq!(
            version_token("ffmpeg version n7.0.2-static"),
            Some("7.0.2".into())
        );
        assert_eq!(
            version_token("pdftoppm version 25.06.0"),
            Some("25.06.0".into())
        );
        assert_eq!(version_token("7-Zip (z) 24.09 (x64)"), Some("24.09".into()));
        assert_eq!(version_token("version unavailable"), None);
    }
}

#[cfg(all(test, windows))]
mod image_activation_tests {
    use super::{is_microsoft_photos_app_id, photos_gallery_target};

    #[test]
    fn photos_handler_detection_is_specific_and_case_insensitive() {
        assert!(is_microsoft_photos_app_id(
            "Microsoft.Windows.Photos_8wekyb3d8bbwe!App"
        ));
        assert!(is_microsoft_photos_app_id(
            "microsoft.windows.photos_8WEKYB3D8BBWE!app"
        ));
        assert!(!is_microsoft_photos_app_id(
            "Contoso.Windows.Photos_8wekyb3d8bbwe!App"
        ));
        assert!(!is_microsoft_photos_app_id(
            "Microsoft.Paint_8wekyb3d8bbwe!App"
        ));
        assert!(!is_microsoft_photos_app_id(
            "Microsoft.Windows.Photos_fakepublisher!App"
        ));
    }

    #[test]
    fn photos_gallery_protocol_encodes_query_delimiters_and_unicode() {
        let target =
            photos_gallery_target(std::path::Path::new(r"C:\Photos café\image + #1 %20.png"))
                .unwrap();
        assert_eq!(
            target.to_string_lossy(),
            r"ms-photos:viewer?fileName=C:\Photos%20caf%C3%A9\image%20%2B%20%231%20%2520.png"
        );
    }

    #[test]
    fn photos_gallery_protocol_rejects_relative_and_non_unicode_paths() {
        use std::os::windows::ffi::OsStringExt;

        assert!(photos_gallery_target(std::path::Path::new("relative.png")).is_err());

        let invalid = std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xd800,
            b'.' as u16,
            b'p' as u16,
            b'n' as u16,
            b'g' as u16,
        ]);
        assert!(photos_gallery_target(std::path::Path::new(&invalid)).is_err());
    }
}

#[cfg(test)]
mod log_line_tests {
    use super::one_line;

    #[test]
    fn a_multi_line_failure_becomes_one_log_entry() {
        // Forty lines in a log field turn one failure into forty entries for
        // whatever reads the file.
        let flat = one_line("line 0\nline 1\n\nfatal: no such file\n");
        assert!(!flat.contains('\n'), "{flat}");
        assert_eq!(flat, "line 0 / line 1 / fatal: no such file");
    }

    #[test]
    fn a_very_long_output_keeps_its_end() {
        let message: String = (0..200).map(|i| format!("line {i} of noise\n")).collect();
        let flat = one_line(&(message + "fatal: no such file"));
        assert!(flat.len() <= 420, "{} characters", flat.len());
        // The reason is at the end, so that is the part worth keeping.
        assert!(flat.ends_with("fatal: no such file"), "{flat}");
        assert!(flat.starts_with('…'), "{flat}");
        assert!(!flat.contains("line 0 of noise"), "{flat}");
    }

    #[test]
    fn a_silent_program_produces_nothing() {
        assert_eq!(one_line(""), "");
        assert_eq!(one_line("\n  \n"), "");
    }

    #[test]
    fn cutting_never_splits_a_character() {
        // A path or a message in any language must not be cut mid-codepoint,
        // which would panic on the slice.
        let message = "é".repeat(500);
        let flat = one_line(&message);
        assert!(flat.chars().count() <= 402, "{}", flat.chars().count());
    }
}

#[cfg(all(test, not(windows)))]
mod unix_terminal_tests {
    use super::working_directory_args;
    use std::path::Path;

    fn args_of(term: &str) -> Vec<String> {
        working_directory_args(term, Path::new("/folder01/folder02"))
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn single_instance_terminals_are_told_the_directory() {
        for term in [
            "gnome-terminal",
            "kgx",
            "xfce4-terminal",
            "tilix",
            "lxterminal",
        ] {
            assert_eq!(
                args_of(term),
                vec!["--working-directory=/folder01/folder02"],
                "{term}"
            );
        }
        assert_eq!(args_of("konsole"), vec!["--workdir", "/folder01/folder02"]);
    }

    #[test]
    fn a_terminal_outside_the_table_is_given_no_argument() {
        // An option it does not know would stop it from starting at all, and
        // these honour the directory they are launched in.
        for term in [
            "kitty",
            "alacritty",
            "foot",
            "wezterm",
            "xterm",
            "unknown-term",
        ] {
            assert!(args_of(term).is_empty(), "{term}");
        }
    }

    #[test]
    fn a_full_path_in_the_variable_still_matches_the_table() {
        // $TERMINAL may hold a path rather than a bare program name.
        assert_eq!(
            args_of("/usr/bin/konsole"),
            vec!["--workdir", "/folder01/folder02"]
        );
    }
}

#[cfg(all(test, windows))]
mod terminal_tests {
    use super::{chosen_terminal_command, command_prompt_command, windows_terminal_command};
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn terminal_commands_do_not_use_an_intermediate_cmd_start() {
        let cwd = Path::new(r"C:\Work folder");

        let wt = windows_terminal_command("wt.exe", cwd);
        assert_eq!(wt.get_program(), OsStr::new("wt.exe"));
        assert_eq!(
            wt.get_args().collect::<Vec<_>>(),
            [OsStr::new("-d"), cwd.as_os_str()]
        );

        let cmd = command_prompt_command(cwd);
        assert_eq!(cmd.get_program(), OsStr::new("cmd.exe"));
        assert_eq!(cmd.get_args().count(), 0);
        assert_eq!(cmd.get_current_dir(), Some(cwd));
    }

    /// How a terminal named in the settings reaches its branch. The choice is
    /// keyed on the file name, so a program designated by its full path has to
    /// land where its bare name would, and the path itself must be handed to
    /// the system unchanged rather than reduced to that name.
    ///
    /// Only two outcomes are observable on a built command: the directory
    /// given as an option, or given as the process directory. Which of the
    /// latter also creates a console cannot be read back, `Command` exposing
    /// no accessor for its creation flags.
    #[test]
    fn a_chosen_terminal_is_routed_by_its_file_name() {
        let cwd = Path::new(r"C:\Work folder");
        let as_option = [OsStr::new("-d"), cwd.as_os_str()];

        // The directory has to travel as an option here, because the window is
        // opened by an instance already running elsewhere. Matching ignores
        // case, as the file system does.
        for program in ["wt.exe", "WT.EXE", r"C:\Program Files\folder01\wt.exe"] {
            let built = chosen_terminal_command(program, cwd);
            assert_eq!(built.get_program(), OsStr::new(program), "{program}");
            assert_eq!(built.get_args().collect::<Vec<_>>(), as_option, "{program}");
        }

        // Everything else is started in the directory itself, and receives no
        // option: one it does not know would make it refuse to start.
        for program in [
            r"C:\Windows\System32\cmd.exe",
            "powershell.exe",
            r"C:\Program Files\folder01\myterm.exe",
        ] {
            let built = chosen_terminal_command(program, cwd);
            assert_eq!(built.get_program(), OsStr::new(program), "{program}");
            assert_eq!(built.get_args().count(), 0, "{program}");
            assert_eq!(built.get_current_dir(), Some(cwd), "{program}");
        }
    }
}

#[cfg(all(test, unix))]
mod program_tests {
    use super::{is_desktop_launcher, is_launchable_program};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "favnyr-launchable-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"#!/bin/sh\ntrue\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn only_real_programs_bypass_the_desktop_opener() {
        let dir = scratch();

        // A binary without an extension with its bit set: the everyday case
        // this bypass exists for.
        assert!(is_launchable_program(&write(&dir, "sample-app", 0o755)));
        assert!(is_launchable_program(&write(&dir, "run.sh", 0o755)));

        // Same file without the bit: it is not a program, the desktop opener
        // stays in charge of finding it an editor.
        assert!(!is_launchable_program(&write(&dir, "notes", 0o644)));

        // A data file on a FAT/NTFS mount reports 0777. Executing it would be
        // absurd, so the type is what decides, not the bit.
        for data in ["photo.jpg", "clip.mp4", "report.pdf", "archive.zip"] {
            assert!(
                !is_launchable_program(&write(&dir, data, 0o777)),
                "{data} must never be run"
            );
        }

        // A launcher description, not a program: only the desktop knows how to
        // act on it, so it keeps going through the normal opener.
        assert!(!is_launchable_program(&write(&dir, "app.desktop", 0o755)));

        // A folder is never a program, whatever it is called.
        let folder = dir.join("tools.sh");
        std::fs::create_dir(&folder).unwrap();
        assert!(!is_launchable_program(&folder));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_launcher_is_recognised_whether_or_not_it_carries_the_executable_bit() {
        let dir = scratch();

        // The regression this guards. Requiring the bit sent a launcher that
        // did not carry it to the generic opener, which showed the entry's
        // source text in an editor instead of starting the application. The
        // desktop's own rule never consults the bit for a launcher.
        assert!(is_desktop_launcher(&write(
            &dir,
            "Sample App 5.desktop",
            0o644
        )));
        assert!(is_desktop_launcher(&write(&dir, "game.desktop", 0o755)));
        // Matched on the extension alone, whatever its case.
        assert!(is_desktop_launcher(&write(&dir, "App.DESKTOP", 0o644)));

        // Anything else is not a launcher, bit or no bit.
        assert!(!is_desktop_launcher(&write(&dir, "notes.txt", 0o644)));
        assert!(!is_desktop_launcher(&write(&dir, "runner", 0o755)));

        // And a launcher is never mistaken for a native program: running the
        // entry as a script would do the wrong thing, whoever calls.
        assert!(!is_launchable_program(&write(&dir, "run.desktop", 0o755)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_link_is_judged_on_the_binary_it_points_at() {
        let dir = scratch();
        let real = write(&dir, "engine", 0o755);
        let link = dir.join("engine-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(is_launchable_program(&link));

        // A link to something that is not executable stays a plain document.
        let plain = write(&dir, "readme", 0o644);
        let plain_link = dir.join("readme-link");
        std::os::unix::fs::symlink(&plain, &plain_link).unwrap();
        assert!(!is_launchable_program(&plain_link));

        std::fs::remove_dir_all(&dir).ok();
    }
}
