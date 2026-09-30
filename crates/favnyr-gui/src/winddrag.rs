//! Native file drag to EXTERNAL applications — **Windows**.
//!
//! HYBRID approach: internal drag (between views / onto a folder) stays
//! handled by Slint (ghost, auto-scroll, menu). When the cursor LEAVES the
//! window during a drag, we switch here to a native OLE drag so that
//! external applications receive the files (shell `CF_HDROP` format).
//!
//! On the way out, the `IDataObject` is built by the shell
//! (`IShellFolder::GetUIObjectOf`) and `SHDoDragDrop` provides the default
//! `IDropSource` + the drag image. On the way in, our small `IDropTarget`
//! replaces winit's so we can receive `DragOver` and reuse Favnyr's
//! hover/menu.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};
use windows::Win32::Foundation::{HGLOBAL, HWND, POINTL};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows::Win32::System::Com::StructuredStorage::{IStorage, StgCreateDocfile};
use windows::Win32::System::Com::{
    CoTaskMemFree, DVASPECT_CONTENT, FORMATETC, IDataObject, IStream, STGC_DEFAULT, STGM_CREATE,
    STGM_READWRITE, STGM_SHARE_EXCLUSIVE, TYMED_HGLOBAL, TYMED_ISTORAGE, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
    OleInitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, CFSTR_SHELLIDLIST, DragQueryFileW, FILEDESCRIPTORW,
    FILEGROUPDESCRIPTORW, HDROP, IShellFolder, SHDoDragDrop, SHGetDesktopFolder,
    SHParseDisplayName,
};
use windows::core::{PCWSTR, Ref, implement};

/// Event from an incoming OLE drag. Favnyr replaces winit's minimal drop
/// target so it can also receive `DragOver` coordinates, essential for
/// real-time row hover.
pub enum IncomingFileDrag {
    Hover {
        screen_x: i32,
        screen_y: i32,
        copy: bool,
    },
    Leave,
    Drop {
        paths: Vec<PathBuf>,
        screen_x: i32,
        screen_y: i32,
        copy: bool,
        /// Present when Favnyr had to take ownership of source data before
        /// returning from OLE `Drop` (for example an email attachment or a
        /// temporary path exposed by an archive manager).
        staging: Option<DropStaging>,
    },
    /// The source advertised supported external data, but Favnyr could not
    /// take ownership of usable paths or bytes during `Drop`.
    ExternalDropFailed,
}

/// Favnyr-owned data that must stay alive until the asynchronous transfer
/// finishes. Hard-linked captures require a real copy at the destination so a
/// persistent third-party source can never share file identity with it.
pub struct DropStaging {
    pub temp_dir: PathBuf,
    pub copy_from_staging: bool,
}

type DropHandler = Box<dyn Fn(IncomingFileDrag)>;

#[implement(IDropTarget)]
struct FavnyrDropTarget {
    handler: DropHandler,
    /// Cheap format classification retained between `DragEnter` and `Drop`.
    /// No source-owned data is rendered until the user actually drops it.
    incoming_kind: Cell<IncomingDataKind>,
    /// Original `DoDragDrop` effect mask captured before live feedback replaces
    /// the in/out value with one selected effect.
    allowed_effects: Cell<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum IncomingDataKind {
    ShellPaths,
    ApplicationPaths,
    VirtualFiles,
    #[default]
    None,
}

impl IncomingDataKind {
    fn accepted(self) -> bool {
        !matches!(self, Self::None)
    }

    fn copy_only(self) -> bool {
        matches!(self, Self::ApplicationPaths | Self::VirtualFiles)
    }
}

impl FavnyrDropTarget {
    fn ctrl_down(keys: MODIFIERKEYS_FLAGS) -> bool {
        // MK_CONTROL, defined in winuser.h. The generated type doesn't provide
        // a dedicated constant across every windows-rs feature combination.
        keys.0 & 0x0008 != 0
    }

    fn choose_copy_effect(
        allowed: windows::Win32::System::Ole::DROPEFFECT,
        accepted: bool,
    ) -> windows::Win32::System::Ole::DROPEFFECT {
        if accepted && allowed.0 & DROPEFFECT_COPY.0 != 0 {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        }
    }

    /// Chooses the effect used only for live cursor feedback. Physical Shell
    /// paths advertise MOVE by default so the native cursor and Favnyr's pill
    /// agree with the menu that will open on release. Copy-only providers and
    /// an explicit Ctrl modifier remain COPY.
    fn choose_hover_effect(
        allowed: windows::Win32::System::Ole::DROPEFFECT,
        kind: IncomingDataKind,
        copy_requested: bool,
    ) -> windows::Win32::System::Ole::DROPEFFECT {
        // The deferred Favnyr menu takes ownership of path strings only after
        // OLE returns, so the final handoff must remain COPY. Reject a source
        // that cannot support that safe handoff instead of advertising MOVE
        // and then failing on release.
        if !kind.accepted() || allowed.0 & DROPEFFECT_COPY.0 == 0 {
            return DROPEFFECT_NONE;
        }
        if kind.copy_only() || copy_requested {
            return Self::choose_copy_effect(allowed, true);
        }
        if allowed.0 & DROPEFFECT_MOVE.0 != 0 {
            DROPEFFECT_MOVE
        } else {
            Self::choose_copy_effect(allowed, true)
        }
    }

    /// Updates OLE's live feedback and returns whether Favnyr should render
    /// the copy variant of its own drag indicator. `None` rejects the drop.
    fn set_hover_effect(
        effect: *mut windows::Win32::System::Ole::DROPEFFECT,
        allowed: windows::Win32::System::Ole::DROPEFFECT,
        kind: IncomingDataKind,
        copy_requested: bool,
    ) -> Option<bool> {
        if effect.is_null() {
            return None;
        }
        unsafe {
            *effect = Self::choose_hover_effect(allowed, kind, copy_requested);
            if *effect == DROPEFFECT_NONE {
                None
            } else {
                Some(*effect == DROPEFFECT_COPY)
            }
        }
    }

    /// Final OLE handoff remains COPY because Favnyr opens its own action menu
    /// after `Drop` returns. This prevents the source from deleting anything
    /// before Favnyr executes the user's later Move/Copy/Link choice.
    fn set_copy_effect(
        effect: *mut windows::Win32::System::Ole::DROPEFFECT,
        allowed: windows::Win32::System::Ole::DROPEFFECT,
        accepted: bool,
    ) -> bool {
        if effect.is_null() {
            return false;
        }
        unsafe {
            *effect = Self::choose_copy_effect(allowed, accepted);
            *effect == DROPEFFECT_COPY
        }
    }

    fn effect_value(effect: *mut windows::Win32::System::Ole::DROPEFFECT) -> u32 {
        if effect.is_null() {
            0
        } else {
            unsafe { (*effect).0 }
        }
    }
}

impl IDropTarget_Impl for FavnyrDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        let kind = data
            .as_ref()
            .map(classify_incoming_data)
            .unwrap_or_default();
        self.incoming_kind.set(kind);
        let allowed = FavnyrDropTarget::effect_value(effect);
        self.allowed_effects.set(allowed);
        let copy_requested = FavnyrDropTarget::ctrl_down(keys);
        // One line per drag, not per move: `DragOver` is the chatty one. It
        // records that Favnyr's own target WAS consulted and what it made of
        // the source — the two questions a refused drag raises, and which
        // cannot be told apart from the outside, the pointer showing the
        // system's refusal either way.
        debug!(
            ?kind,
            allowed = format!("0x{allowed:X}"),
            copy_requested,
            "OLE drag entered Favnyr"
        );
        if let Some(copy) = FavnyrDropTarget::set_hover_effect(
            effect,
            windows::Win32::System::Ole::DROPEFFECT(allowed),
            kind,
            copy_requested,
        ) {
            (self.handler)(IncomingFileDrag::Hover {
                screen_x: point.x,
                screen_y: point.y,
                copy,
            });
        }
        Ok(())
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        let kind = self.incoming_kind.get();
        let allowed = windows::Win32::System::Ole::DROPEFFECT(self.allowed_effects.get());
        let copy_requested = FavnyrDropTarget::ctrl_down(keys);
        if let Some(copy) =
            FavnyrDropTarget::set_hover_effect(effect, allowed, kind, copy_requested)
        {
            (self.handler)(IncomingFileDrag::Hover {
                screen_x: point.x,
                screen_y: point.y,
                copy,
            });
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.incoming_kind.set(IncomingDataKind::None);
        self.allowed_effects.set(0);
        (self.handler)(IncomingFileDrag::Leave);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        let allowed_effect = self.allowed_effects.replace(0);
        let kind = self.incoming_kind.replace(IncomingDataKind::None);
        let mut paths = Vec::new();
        let mut staging = None;
        let copy_allowed = allowed_effect & DROPEFFECT_COPY.0 != 0;
        if copy_allowed && let Some(data) = data.as_ref() {
            match kind {
                IncomingDataKind::ShellPaths => {
                    paths = file_paths(data);
                }
                IncomingDataKind::ApplicationPaths => {
                    let rendered_paths = file_paths(data);
                    if let Some(captured) = capture_application_paths(&rendered_paths) {
                        paths = captured.paths;
                        staging = Some(DropStaging {
                            temp_dir: captured.temp_dir,
                            copy_from_staging: captured.used_hard_links,
                        });
                    }
                }
                IncomingDataKind::VirtualFiles => {
                    if let Some(materialized) = materialize_virtual_files(data) {
                        paths = materialized.paths;
                        staging = Some(DropStaging {
                            temp_dir: materialized.temp_dir,
                            copy_from_staging: false,
                        });
                    }
                }
                IncomingDataKind::None => {}
            }
            // Some providers advertise CF_HDROP but render it only
            // conditionally. Keep the already-working email-attachment route as a
            // fallback when usable paths were not returned at Drop time.
            if paths.is_empty()
                && staging.is_none()
                && has_virtual_files(data)
                && let Some(materialized) = materialize_virtual_files(data)
            {
                paths = materialized.paths;
                staging = Some(DropStaging {
                    temp_dir: materialized.temp_dir,
                    copy_from_staging: false,
                });
            }
        }
        let accepted = FavnyrDropTarget::set_copy_effect(
            effect,
            windows::Win32::System::Ole::DROPEFFECT(allowed_effect),
            !paths.is_empty(),
        );
        if accepted {
            let copy = kind.copy_only()
                || FavnyrDropTarget::ctrl_down(keys)
                || allowed_effect & DROPEFFECT_MOVE.0 == 0;
            (self.handler)(IncomingFileDrag::Drop {
                paths,
                screen_x: point.x,
                screen_y: point.y,
                copy,
                staging,
            });
        } else if copy_allowed && kind.copy_only() {
            (self.handler)(IncomingFileDrag::ExternalDropFailed);
        } else {
            (self.handler)(IncomingFileDrag::Leave);
        }
        Ok(())
    }
}

struct CapturedPathDrop {
    paths: Vec<PathBuf>,
    temp_dir: PathBuf,
    used_hard_links: bool,
}

/// Captures application-provided CF_HDROP paths before returning from OLE
/// `Drop`. 7-Zip, for example, deletes its extraction directory immediately
/// after `DoDragDrop` returns, before Favnyr's deferred transfer can start.
fn capture_application_paths(paths: &[PathBuf]) -> Option<CapturedPathDrop> {
    if paths.is_empty() {
        return None;
    }
    let dir = create_drop_dir("favnyr-path-dnd")?;
    let mut out = Vec::with_capacity(paths.len());
    let mut used_hard_links = false;
    for (index, source) in paths.iter().enumerate() {
        let Some(name) = source.file_name() else {
            warn!(path = %source.display(), "application drop path has no file name");
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        };
        // Separate roots preserve same-named items from different source
        // folders; Favnyr's existing conflict resolver handles them later.
        let item_dir = dir.join(index.to_string());
        if let Err(err) = std::fs::create_dir(&item_dir) {
            warn!(error = %err, path = %item_dir.display(), "creating application drop item directory failed");
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        let captured = item_dir.join(name);
        match capture_path_tree(source, &captured) {
            Ok(item_used_hard_links) => used_hard_links |= item_used_hard_links,
            Err(err) => {
                warn!(error = %err, path = %source.display(), "capturing application drop path failed");
                let _ = std::fs::remove_dir_all(&dir);
                return None;
            }
        }
        out.push(captured);
    }
    Some(CapturedPathDrop {
        paths: out,
        temp_dir: dir,
        used_hard_links,
    })
}

fn create_drop_dir(prefix: &str) -> Option<PathBuf> {
    for _ in 0..32 {
        let serial = DROP_SERIAL.with(|counter| {
            let value = counter.get().wrapping_add(1);
            counter.set(value);
            value
        });
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{serial}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Some(dir),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                warn!(error = %err, path = %dir.display(), "creating drop directory failed");
                return None;
            }
        }
    }
    warn!(prefix, "could not allocate a unique drop directory");
    None
}

/// Recreates a tree without following Windows reparse points. Regular files
/// use hard links first so the OLE callback stays independent of file size;
/// cross-volume and unsupported filesystems fall back to a physical copy.
fn capture_path_tree(source: &Path, destination: &Path) -> std::io::Result<bool> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "reparse points are not supported in transient file drops",
        ));
    }
    if metadata.is_dir() {
        std::fs::create_dir(destination)?;
        let mut used_hard_links = false;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            used_hard_links |=
                capture_path_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(used_hard_links)
    } else if metadata.is_file() {
        match std::fs::hard_link(source, destination) {
            Ok(()) => Ok(true),
            Err(_) => {
                std::fs::copy(source, destination)?;
                Ok(false)
            }
        }
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "unsupported filesystem object in transient file drop",
        ))
    }
}

thread_local! {
    /// Explicitly keeps our COM object alive. OLE also holds a reference
    /// between RegisterDragDrop and window destruction.
    static DROP_TARGET: RefCell<Option<IDropTarget>> = const { RefCell::new(None) };
}

/// Replaces the very minimal file drop target installed by winit. The latter
/// only reports "file entered/left/dropped", without the DragOver position;
/// Favnyr needs that position to target the panel, row, and executable. Must
/// be called after the HWND is created, on the UI thread.
pub fn init_drop_target(hwnd: isize, handler: impl Fn(IncomingFileDrag) + 'static) -> bool {
    if hwnd == 0 {
        return false;
    }
    DROP_TARGET.with(|slot| {
        if slot.borrow().is_some() {
            return true;
        }
        unsafe {
            if let Err(err) = OleInitialize(None) {
                warn!(error = %err, "initializing OLE for Favnyr drop target failed");
                return false;
            }
            // winit already registers a CF_HDROP target. It isn't exposed to
            // Slint and doesn't provide DragOver: we cleanly replace it.
            let _ = RevokeDragDrop(HWND(hwnd as *mut c_void));
        }
        let target: IDropTarget = FavnyrDropTarget {
            handler: Box::new(handler),
            incoming_kind: Cell::new(IncomingDataKind::None),
            allowed_effects: Cell::new(0),
        }
        .into();
        match unsafe { RegisterDragDrop(HWND(hwnd as *mut c_void), &target) } {
            Ok(()) => {
                *slot.borrow_mut() = Some(target);
                debug!(hwnd, "Favnyr OLE drop target registered");
                true
            }
            Err(err) => {
                warn!(error = %err, "register Favnyr OLE drop target failed");
                false
            }
        }
    })
}

/// Re-registers the already-created Favnyr target after the native window has
/// completed its first event-loop iterations. Slint's declarative startup
/// timer can expire while winit is still finalizing its own CF_HDROP target;
/// reusing the same COM object once the loop is stable keeps Favnyr's richer
/// target authoritative without adding any recurring work.
pub fn rebind_drop_target(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    DROP_TARGET.with(|slot| {
        let Some(target) = slot.borrow().as_ref().cloned() else {
            warn!(
                hwnd,
                "Favnyr OLE drop target was unavailable for delayed registration"
            );
            return false;
        };
        let revoke = unsafe { RevokeDragDrop(HWND(hwnd as *mut c_void)) };
        let register = unsafe { RegisterDragDrop(HWND(hwnd as *mut c_void), &target) };
        if let Err(err) = revoke {
            debug!(error = %err, hwnd, "revoke before delayed drop-target registration failed");
        }
        match register {
            Ok(()) => {
                debug!(hwnd, "Favnyr OLE drop target re-registered after startup");
                true
            }
            Err(err) => {
                warn!(error = %err, hwnd, "delayed Favnyr OLE drop-target registration failed");
                false
            }
        }
    })
}

/// Classifies the formats without rendering their data. In particular,
/// calling `GetData(CF_HDROP)` from `DragEnter` can make archive managers
/// extract files merely because the pointer crossed the window.
fn classify_incoming_data(data: &IDataObject) -> IncomingDataKind {
    let has_hdrop = has_hdrop(data);
    // The stream answer is a signal only from a provider that reads the
    // medium it is asked about. See `provider_reads_the_medium`.
    let synthesized_hdrop = has_stream_hdrop(data) && provider_reads_the_medium(data);
    let has_shell_id_list = has_shell_id_list(data);
    let has_virtual_files = has_virtual_files(data);
    classify_formats(
        has_hdrop,
        synthesized_hdrop,
        has_shell_id_list,
        has_virtual_files,
    )
}

/// Does this provider actually look at the storage medium it is asked about?
///
/// It is asked for a file list carried as a compound file, which nothing can
/// answer honestly: a list of paths is not structured storage. A provider that
/// inspects the request refuses it. One that accepts is answering on the format
/// alone and ignoring the medium — the Shell does exactly that, accepting every
/// value including GDI handles — so nothing it says about a medium carries any
/// information, and reading its answer as a signal misclassifies it.
fn provider_reads_the_medium(data: &IDataObject) -> bool {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_ISTORAGE.0 as u32,
    };
    unsafe { data.QueryGetData(&format).is_err() }
}

fn classify_formats(
    has_paths: bool,
    synthesized_hdrop: bool,
    has_shell_id_list: bool,
    has_virtual_files: bool,
) -> IncomingDataKind {
    if has_paths {
        if has_shell_id_list && !synthesized_hdrop {
            IncomingDataKind::ShellPaths
        } else {
            // Standard CF_HDROP uses HGLOBAL. Some application data objects
            // additionally advertise IStream and synthesize both CF_HDROP and
            // Shell IDList Array from temporary paths (PeaZip's public
            // TDropFileSource implementation is one example). CIDA is not a
            // lifetime guarantee in that case, so capture before Drop returns.
            IncomingDataKind::ApplicationPaths
        }
    } else if has_virtual_files {
        IncomingDataKind::VirtualFiles
    } else {
        IncomingDataKind::None
    }
}

fn has_hdrop(data: &IDataObject) -> bool {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe { data.QueryGetData(&format).is_ok() }
}

/// Detects a non-standard, application-provided CF_HDROP representation.
/// Microsoft's CF_HDROP contract uses TYMED_HGLOBAL; accepting IStream as well
/// marks paths that are synthesized rather than a normal Shell filesystem
/// selection — but only from a provider that reads the medium at all, which is
/// why the caller pairs this with `provider_reads_the_medium`. QueryGetData
/// never renders them.
fn has_stream_hdrop(data: &IDataObject) -> bool {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_ISTREAM.0 as u32,
    };
    unsafe { data.QueryGetData(&format).is_ok() }
}

/// The Shell IDList Array is a positive marker for data objects produced by
/// Explorer and Favnyr's own Shell-based outgoing drag. Those paths remain on
/// the existing Move/Copy/Link route.
fn has_shell_id_list(data: &IDataObject) -> bool {
    let format = FORMATETC {
        cfFormat: clip_format(CFSTR_SHELLIDLIST),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe { data.QueryGetData(&format).is_ok() }
}

/// Extracts and COPIES the CF_HDROP paths from an IDataObject. `ReleaseStgMedium`
/// always releases the medium after reading; no Shell data leaks into the
/// application state.
fn file_paths(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut medium = match unsafe { data.GetData(&format) } {
        Ok(medium) => medium,
        Err(err) => {
            debug!(error = %err, "CF_HDROP could not be rendered during Drop");
            return Vec::new();
        }
    };
    if medium.tymed != TYMED_HGLOBAL.0 as u32 {
        warn!(
            tymed = medium.tymed,
            "CF_HDROP source returned an unexpected storage medium"
        );
        unsafe {
            ReleaseStgMedium(&mut medium);
        }
        return Vec::new();
    }
    let mut out = Vec::new();
    unsafe {
        let hdrop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        if out.try_reserve(count as usize).is_err() {
            warn!(count, "CF_HDROP path list is too large to allocate");
        } else {
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                if len == 0 {
                    continue;
                }
                let mut wide = vec![0u16; len + 1];
                if DragQueryFileW(hdrop, i, Some(&mut wide)) > 0 {
                    out.push(PathBuf::from(String::from_utf16_lossy(&wide[..len])));
                }
            }
        }
        ReleaseStgMedium(&mut medium);
    }
    out
}

// ---- "Virtual files" (email attachments, zip entries, browser images…) ----
// These aren't on disk: `CFSTR_FILEDESCRIPTORW` names them, `CFSTR_FILECONTENTS`
// carries their bytes (usually an `IStream`). We materialize them into a temp
// folder during `Drop`, and the normal drop pipeline then MOVES them into the
// target folder. A guard removes the complete staging tree after the move.

thread_local! {
    static DROP_SERIAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

struct MaterializedFileDrop {
    paths: Vec<PathBuf>,
    temp_dir: PathBuf,
}

/// Registered clipboard-format id for the given name. `RegisterClipboardFormatW`
/// is idempotent (same id for a given name), so this is cheap to call.
fn clip_format(name: PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

/// True when the drag offers a "virtual file" descriptor (a file not on disk).
fn has_virtual_files(data: &IDataObject) -> bool {
    let format = FORMATETC {
        cfFormat: clip_format(CFSTR_FILEDESCRIPTORW),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe { data.QueryGetData(&format).is_ok() }
}

/// Materializes the drag's virtual files into a fresh temp folder and returns
/// their real paths (`None` on failure). Must run during `Drop`, while the
/// `IDataObject` is still valid.
fn materialize_virtual_files(data: &IDataObject) -> Option<MaterializedFileDrop> {
    let names = read_file_descriptor_names(data)?;
    if names.is_empty() {
        warn!("virtual file descriptor contained no items");
        return None;
    }
    let dir = create_drop_dir("favnyr-dnd")?;
    let mut out = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        // A descriptor may carry a relative path (zip subfolders): keep only the
        // final component so the file always stays inside our temp folder.
        let leaf = name.rsplit(['\\', '/']).next().unwrap_or(name);
        if leaf.is_empty() || leaf == "." || leaf == ".." {
            warn!(index, "virtual file descriptor contained an invalid name");
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        // Each item gets its own staging subdirectory. This preserves duplicate
        // attachment names so the existing conflict resolver can arbitrate them.
        let item_dir = dir.join(index.to_string());
        if let Err(err) = std::fs::create_dir(&item_dir) {
            warn!(error = %err, path = %item_dir.display(), "creating virtual item directory failed");
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        let path = item_dir.join(leaf);
        if let Err(err) = write_file_contents(data, index as i32, &path) {
            warn!(error = %err, index, name, "materializing virtual file failed");
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        out.push(path);
    }
    Some(MaterializedFileDrop {
        paths: out,
        temp_dir: dir,
    })
}

/// Reads the file NAMES from the drag's `FILEGROUPDESCRIPTORW`.
fn read_file_descriptor_names(data: &IDataObject) -> Option<Vec<String>> {
    let format = FORMATETC {
        cfFormat: clip_format(CFSTR_FILEDESCRIPTORW),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else {
        warn!("reading virtual file descriptors failed");
        return None;
    };
    let out = unsafe {
        let hglobal = medium.u.hGlobal;
        let byte_len = GlobalSize(hglobal);
        let base = GlobalLock(hglobal) as *const u8;
        if base.is_null() {
            None
        } else {
            let descriptor_offset = std::mem::offset_of!(FILEGROUPDESCRIPTORW, fgd);
            let descriptor_size = std::mem::size_of::<FILEDESCRIPTORW>();
            let count = if byte_len >= descriptor_offset {
                std::ptr::read_unaligned(base as *const u32) as usize
            } else {
                usize::MAX
            };
            let available = byte_len
                .saturating_sub(descriptor_offset)
                .checked_div(descriptor_size)
                .unwrap_or(0);
            let result = if count > available {
                warn!(
                    count,
                    byte_len, "virtual file descriptor block was truncated"
                );
                None
            } else {
                let first = base.add(descriptor_offset) as *const FILEDESCRIPTORW;
                let mut names = Vec::with_capacity(count);
                for i in 0..count {
                    // FILEDESCRIPTORW is packed: copy the name array through a
                    // raw pointer instead of creating an unaligned reference.
                    let name_ptr = std::ptr::addr_of!((*first.add(i)).cFileName);
                    let name: [u16; 260] = std::ptr::read_unaligned(name_ptr);
                    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                    names.push(String::from_utf16_lossy(&name[..len]));
                }
                Some(names)
            };
            let _ = GlobalUnlock(hglobal);
            result
        }
    };
    unsafe {
        ReleaseStgMedium(&mut medium);
    }
    out
}

/// Requests one supported storage medium for an indexed `FILECONTENTS` item.
/// Individual requests come first because some real-world OLE providers reject
/// a standards-compliant bitmask even though they support one of its members.
///
/// All three media the format allows are tried. A provider picks the one that
/// matches its own storage: a mail client hands over an attachment as a byte
/// stream, but a whole message as a compound file — that one only ever answers
/// to `TYMED_ISTORAGE`, and asking for streams alone is refused outright.
fn file_contents_medium(
    data: &IDataObject,
    index: i32,
) -> Option<windows::Win32::System::Com::STGMEDIUM> {
    let mut failures = Vec::with_capacity(4);
    for requested in [
        TYMED_ISTREAM.0 as u32,
        TYMED_ISTORAGE.0 as u32,
        TYMED_HGLOBAL.0 as u32,
        (TYMED_ISTREAM.0 | TYMED_ISTORAGE.0 | TYMED_HGLOBAL.0) as u32,
    ] {
        let format = FORMATETC {
            cfFormat: clip_format(CFSTR_FILECONTENTS),
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: index,
            tymed: requested,
        };
        match unsafe { data.GetData(&format) } {
            Ok(medium) => return Some(medium),
            Err(err) => failures.push(format!("0x{requested:X}: {err}")),
        }
    }
    warn!(index, errors = %failures.join("; "), "requesting virtual file contents failed");
    None
}

/// Streams one virtual file straight to disk. This keeps memory use bounded
/// even for large email attachments.
fn write_file_contents(data: &IDataObject, index: i32, path: &Path) -> Result<(), String> {
    let Some(mut medium) = file_contents_medium(data, index) else {
        return Err("the source did not provide a supported storage medium".into());
    };
    let result = unsafe {
        if medium.tymed == TYMED_ISTREAM.0 as u32 {
            (*medium.u.pstm)
                .as_ref()
                .ok_or_else(|| "the source returned a null IStream".to_string())
                .and_then(|stream| write_istream(stream, path))
        } else if medium.tymed == TYMED_ISTORAGE.0 as u32 {
            (*medium.u.pstg)
                .as_ref()
                .ok_or_else(|| "the source returned a null IStorage".to_string())
                .and_then(|storage| write_istorage(storage, path))
        } else if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            write_hglobal(medium.u.hGlobal, path)
        } else {
            Err(format!(
                "the source returned unsupported TYMED 0x{:X}",
                medium.tymed
            ))
        }
    };
    unsafe {
        ReleaseStgMedium(&mut medium);
    }
    result
}

/// Writes an `IStream` to a file in fixed-size chunks.
fn write_istream(stream: &IStream, path: &Path) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|err| err.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let mut read: u32 = 0;
        let status = unsafe {
            stream.Read(
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u32,
                Some(&mut read),
            )
        };
        if read as usize > buf.len() {
            return Err("IStream returned an invalid byte count".into());
        }
        if read > 0 {
            file.write_all(&buf[..read as usize])
                .map_err(|err| err.to_string())?;
        }
        if status.is_err() {
            return Err(format!("IStream::Read failed with {status:?}"));
        }
        if read == 0 {
            return Ok(());
        }
    }
}

/// Writes an `IStorage` to a compound file. A structured-storage item is a
/// whole file system in miniature rather than a byte sequence, so it is copied
/// as such: the destination is created as a compound file and the source
/// duplicates its own streams and substorages into it. This is what produces a
/// mail message the rest of the desktop can reopen.
fn write_istorage(storage: &IStorage, path: &Path) -> Result<(), String> {
    let wide_path = wide(path);
    let destination = unsafe {
        StgCreateDocfile(
            PCWSTR(wide_path.as_ptr()),
            STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE,
            None,
        )
    }
    .map_err(|err| format!("StgCreateDocfile failed: {err}"))?;
    unsafe { storage.CopyTo(None, None, &destination) }
        .map_err(|err| format!("IStorage::CopyTo failed: {err}"))?;
    unsafe { destination.Commit(STGC_DEFAULT.0 as u32) }
        .map_err(|err| format!("IStorage::Commit failed: {err}"))
}

/// Writes the bytes held by an `HGLOBAL` without an intermediate allocation.
fn write_hglobal(hglobal: HGLOBAL, path: &Path) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|err| err.to_string())?;
    let size = unsafe { GlobalSize(hglobal) };
    if size == 0 {
        return Ok(());
    }
    let ptr = unsafe { GlobalLock(hglobal) } as *const u8;
    if ptr.is_null() {
        return Err("GlobalLock failed for virtual file contents".into());
    }
    let result = file
        .write_all(unsafe { std::slice::from_raw_parts(ptr, size) })
        .map_err(|err| err.to_string());
    unsafe {
        let _ = GlobalUnlock(hglobal);
    }
    result
}

/// NUL-terminated wide string (kept alive by the caller for as long as the
/// pointer is in use).
fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
fn wide_str(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Starts a native OLE drag of `paths` (all assumed to be in the SAME folder —
/// which is the case for a selection coming from a view). External apps
/// receive the files as `CF_HDROP` (COPY effect). Blocking (modal loop of
/// `SHDoDragDrop`) until dropped/cancelled. `true` if a drop occurred.
/// Fire-and-forget: any error is logged, never propagated (no panic).
pub fn drag_files(paths: &[PathBuf]) -> bool {
    match unsafe { drag_files_inner(paths) } {
        Ok(dropped) => dropped,
        Err(err) => {
            warn!(error = %err, "native OLE drag failed");
            false
        }
    }
}

unsafe fn drag_files_inner(paths: &[PathBuf]) -> windows::core::Result<bool> {
    unsafe {
        if paths.is_empty() {
            return Ok(false);
        }
        // Common parent folder (the selection comes from a single view).
        let Some(parent_dir) = paths[0].parent() else {
            return Ok(false);
        };

        // `SHDoDragDrop`/OLE require OLE init on the UI thread. Idempotent
        // (S_FALSE if already initialized); we ignore failure (already in MTA →
        // we try anyway).
        let _ = OleInitialize(None);

        let desktop: IShellFolder = SHGetDesktopFolder()?;

        // Absolute PIDL of the parent folder → IShellFolder of the parent.
        let parent_w = wide(parent_dir);
        let mut parent_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(parent_w.as_ptr()), None, &mut parent_pidl, 0, None)?;
        // On failure `parent_pidl` (already allocated) must still be freed — a
        // bare `?` here leaked it.
        let parent: IShellFolder = match desktop.BindToObject(parent_pidl, None) {
            Ok(p) => p,
            Err(e) => {
                CoTaskMemFree(Some(parent_pidl as *const c_void));
                return Err(e);
            }
        };

        // Child (simple) PIDL of each file, relative to the parent.
        let mut child_pidls: Vec<*mut ITEMIDLIST> = Vec::with_capacity(paths.len());
        for p in paths {
            let Some(name) = p.file_name() else { continue };
            let name_w = wide_str(name);
            let mut cpidl: *mut ITEMIDLIST = std::ptr::null_mut();
            // pcheaten=None, pdwattributes=null (attributes not requested).
            if parent
                .ParseDisplayName(
                    HWND::default(),
                    None,
                    PCWSTR(name_w.as_ptr()),
                    None,
                    &mut cpidl,
                    std::ptr::null_mut(),
                )
                .is_ok()
                && !cpidl.is_null()
            {
                child_pidls.push(cpidl);
            }
        }

        // Capture the outcome instead of `?`-returning: the shell-allocated
        // PIDLs below must be freed even when GetUIObjectOf / SHDoDragDrop fail
        // (both previously leaked parent_pidl AND every child PIDL).
        let dropped: windows::core::Result<bool> = if child_pidls.is_empty() {
            Ok(false)
        } else {
            let ptrs: Vec<*const ITEMIDLIST> = child_pidls.iter().map(|p| *p as *const _).collect();
            // IDataObject built by the shell (CF_HDROP + shell formats) — no
            // homegrown COM. `pdsrc = None` → SHDoDragDrop provides IDropSource +
            // the drag image.
            match parent.GetUIObjectOf(HWND::default(), &ptrs, None) {
                Ok(data) => {
                    let data: IDataObject = data;
                    match SHDoDragDrop(None, &data, None, DROPEFFECT_COPY) {
                        Ok(effect) => {
                            debug!(effect = effect.0, "SHDoDragDrop returned");
                            Ok(effect.0 != 0)
                        }
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            }
        };

        // Free the PIDLs (shell-allocated → CoTaskMem) on every path.
        for c in &child_pidls {
            CoTaskMemFree(Some(*c as *const c_void));
        }
        CoTaskMemFree(Some(parent_pidl as *const c_void));
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static FILE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn incoming_format_classification_preserves_each_route() {
        assert_eq!(
            classify_formats(true, false, true, false),
            IncomingDataKind::ShellPaths
        );
        assert_eq!(
            classify_formats(true, false, false, false),
            IncomingDataKind::ApplicationPaths
        );
        assert_eq!(
            classify_formats(true, false, false, true),
            IncomingDataKind::ApplicationPaths
        );
        assert_eq!(
            classify_formats(true, true, true, false),
            IncomingDataKind::ApplicationPaths
        );
        assert_eq!(
            classify_formats(false, false, false, true),
            IncomingDataKind::VirtualFiles
        );
        assert_eq!(
            classify_formats(false, false, true, false),
            IncomingDataKind::None
        );
    }

    #[test]
    fn copy_effect_never_exceeds_the_source_mask() {
        use windows::Win32::System::Ole::DROPEFFECT_LINK;

        assert_eq!(
            FavnyrDropTarget::choose_copy_effect(DROPEFFECT_COPY | DROPEFFECT_MOVE, true),
            DROPEFFECT_COPY
        );
        assert_eq!(
            FavnyrDropTarget::choose_copy_effect(DROPEFFECT_MOVE | DROPEFFECT_LINK, true),
            DROPEFFECT_NONE
        );
        assert_eq!(
            FavnyrDropTarget::choose_copy_effect(DROPEFFECT_COPY, false),
            DROPEFFECT_NONE
        );
    }

    #[test]
    fn shell_hover_prefers_move_while_copy_only_sources_stay_copy() {
        let both = DROPEFFECT_COPY | DROPEFFECT_MOVE;
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(both, IncomingDataKind::ShellPaths, false),
            DROPEFFECT_MOVE
        );
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(both, IncomingDataKind::ShellPaths, true),
            DROPEFFECT_COPY
        );
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(
                DROPEFFECT_COPY,
                IncomingDataKind::ShellPaths,
                false,
            ),
            DROPEFFECT_COPY
        );
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(
                DROPEFFECT_MOVE,
                IncomingDataKind::ShellPaths,
                false,
            ),
            DROPEFFECT_NONE
        );
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(both, IncomingDataKind::ApplicationPaths, false),
            DROPEFFECT_COPY
        );
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(
                DROPEFFECT_MOVE,
                IncomingDataKind::ApplicationPaths,
                false,
            ),
            DROPEFFECT_NONE
        );
        assert_eq!(
            FavnyrDropTarget::choose_hover_effect(both, IncomingDataKind::None, false),
            DROPEFFECT_NONE
        );
    }

    #[test]
    fn application_capture_survives_source_removal_and_preserves_trees() {
        let _lock = FILE_TEST_LOCK.lock().expect("file test lock poisoned");
        let source_root = create_drop_dir("favnyr-path-dnd").expect("create source root");
        let first_root = source_root.join("first");
        let second_root = source_root.join("second");
        std::fs::create_dir_all(first_root.join("album")).expect("create first tree");
        std::fs::create_dir_all(&second_root).expect("create second tree");
        std::fs::write(first_root.join("album").join("same.txt"), b"first")
            .expect("write first source");
        std::fs::write(second_root.join("same.txt"), b"second").expect("write second source");

        let captured =
            capture_application_paths(&[first_root.join("album"), second_root.join("same.txt")])
                .expect("capture application paths");
        std::fs::remove_dir_all(&source_root).expect("remove source tree");

        assert_eq!(captured.paths.len(), 2);
        assert_eq!(
            std::fs::read(captured.paths[0].join("same.txt")).expect("read captured tree"),
            b"first"
        );
        assert_eq!(
            std::fs::read(&captured.paths[1]).expect("read captured file"),
            b"second"
        );
        std::fs::remove_dir_all(&captured.temp_dir).expect("remove captured tree");
    }

    #[test]
    fn failed_application_capture_rolls_back_its_staging_tree() {
        let _lock = FILE_TEST_LOCK.lock().expect("file test lock poisoned");
        let source_root = create_drop_dir("favnyr-path-dnd").expect("create source root");
        let valid = source_root.join("valid.txt");
        std::fs::write(&valid, b"valid").expect("write valid source");
        let before = application_drop_directories();

        assert!(capture_application_paths(&[valid, source_root.join("missing.txt")]).is_none());
        assert_eq!(application_drop_directories(), before);
        std::fs::remove_dir_all(source_root).expect("remove source root");
    }

    /// A structured-storage item must survive the copy as a structured-storage
    /// item: nested substorages included, and reopenable by any decoder. This
    /// is the shape a mail message arrives in, where a byte-for-byte stream
    /// copy would produce an unusable file.
    #[test]
    fn a_compound_file_is_copied_with_its_whole_tree() {
        use windows::Win32::System::Com::STGM_READ;
        use windows::Win32::System::Com::StructuredStorage::StgOpenStorage;

        let _guard = FILE_TEST_LOCK.lock().expect("file test lock");
        let root = std::env::temp_dir().join(format!("favnyr-stg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create test root");
        let source_path = root.join("source.bin");
        let target_path = root.join("target.bin");

        let source_wide = wide(&source_path);
        let source: IStorage = unsafe {
            StgCreateDocfile(
                PCWSTR(source_wide.as_ptr()),
                STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE,
                None,
            )
        }
        .expect("create source compound file");
        write_stream(&source, "stream01", b"top level payload");
        let nested = unsafe {
            source.CreateStorage(
                PCWSTR(wide_str(std::ffi::OsStr::new("folder01")).as_ptr()),
                STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE,
                0,
                0,
            )
        }
        .expect("create substorage");
        write_stream(&nested, "stream02", b"nested payload");
        unsafe { nested.Commit(STGC_DEFAULT.0 as u32) }.expect("commit substorage");
        unsafe { source.Commit(STGC_DEFAULT.0 as u32) }.expect("commit source");

        write_istorage(&source, &target_path).expect("copy the compound file");
        drop(nested);
        drop(source);

        let target_wide = wide(&target_path);
        let reopened: IStorage = unsafe {
            StgOpenStorage(
                PCWSTR(target_wide.as_ptr()),
                None,
                STGM_READ | STGM_SHARE_EXCLUSIVE,
                None,
                0,
            )
        }
        .expect("reopen the copy as a compound file");
        assert_eq!(read_stream(&reopened, "stream01"), b"top level payload");
        let nested_copy = unsafe {
            reopened.OpenStorage(
                PCWSTR(wide_str(std::ffi::OsStr::new("folder01")).as_ptr()),
                None,
                STGM_READ | STGM_SHARE_EXCLUSIVE,
                std::ptr::null_mut(),
                0,
            )
        }
        .expect("reopen the substorage");
        assert_eq!(read_stream(&nested_copy, "stream02"), b"nested payload");
        drop(nested_copy);
        drop(reopened);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn write_stream(storage: &IStorage, name: &str, bytes: &[u8]) {
        let wide_name = wide_str(std::ffi::OsStr::new(name));
        let stream = unsafe {
            storage.CreateStream(
                PCWSTR(wide_name.as_ptr()),
                STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE,
                0,
                0,
            )
        }
        .expect("create stream");
        let mut written: u32 = 0;
        unsafe {
            stream.Write(
                bytes.as_ptr() as *const c_void,
                bytes.len() as u32,
                Some(&mut written),
            )
        }
        .ok()
        .expect("write stream");
        assert_eq!(written as usize, bytes.len());
        unsafe { stream.Commit(STGC_DEFAULT) }.expect("commit stream");
    }

    fn read_stream(storage: &IStorage, name: &str) -> Vec<u8> {
        use windows::Win32::System::Com::STGM_READ;
        let wide_name = wide_str(std::ffi::OsStr::new(name));
        let stream = unsafe {
            storage.OpenStream(
                PCWSTR(wide_name.as_ptr()),
                None,
                STGM_READ | STGM_SHARE_EXCLUSIVE,
                0,
            )
        }
        .expect("open stream");
        let mut out = vec![0u8; 256];
        let mut read: u32 = 0;
        unsafe {
            stream.Read(
                out.as_mut_ptr() as *mut c_void,
                out.len() as u32,
                Some(&mut read),
            )
        }
        .ok()
        .expect("read stream");
        out.truncate(read as usize);
        out
    }

    /// The Shell's own data object must be recognised as a Shell selection.
    /// It decides the whole route: a Shell selection keeps the Move/Copy/Link
    /// menu, where a synthesized one is captured and copied. The object is
    /// built exactly as Explorer builds it, so the answer is measured on the
    /// real thing rather than assumed from the formats it is expected to carry.
    #[test]
    fn a_shell_selection_is_recognised_as_one() {
        let _guard = FILE_TEST_LOCK.lock().expect("file test lock");
        let root = std::env::temp_dir().join(format!("favnyr-shellobj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("folder01")).expect("create test folder");
        std::fs::write(root.join("my_file.txt"), b"payload").expect("create test file");

        let data = shell_data_object(&root, &["folder01", "my_file.txt"]);
        assert_eq!(classify_incoming_data(&data), IncomingDataKind::ShellPaths);
        assert!(has_hdrop(&data));
        assert!(has_shell_id_list(&data));
        // The Shell answers on the format alone: it accepts a file list carried
        // as a compound file, which is impossible. Its answer about streams is
        // therefore not a signal, and taking it for one sent every Explorer
        // selection down the capture route — losing its Move/Copy/Link menu.
        assert!(
            has_stream_hdrop(&data),
            "the Shell is expected to accept a stream it does not use"
        );
        assert!(
            !provider_reads_the_medium(&data),
            "the Shell is expected to ignore the requested medium"
        );

        drop(data);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The `IDataObject` the Shell produces for a selection inside `parent`.
    fn shell_data_object(parent: &Path, names: &[&str]) -> IDataObject {
        unsafe {
            let _ = OleInitialize(None);
            let desktop: IShellFolder = SHGetDesktopFolder().expect("desktop folder");
            let parent_w = wide(parent);
            let mut parent_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(parent_w.as_ptr()), None, &mut parent_pidl, 0, None)
                .expect("parse the parent folder");
            let folder: IShellFolder = desktop
                .BindToObject(parent_pidl, None)
                .expect("bind the parent folder");
            let mut children: Vec<*mut ITEMIDLIST> = Vec::new();
            for name in names {
                let name_w = wide_str(std::ffi::OsStr::new(name));
                let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
                folder
                    .ParseDisplayName(
                        HWND::default(),
                        None,
                        PCWSTR(name_w.as_ptr()),
                        None,
                        &mut child,
                        std::ptr::null_mut(),
                    )
                    .expect("parse a child name");
                children.push(child);
            }
            let ptrs: Vec<*const ITEMIDLIST> = children.iter().map(|p| *p as *const _).collect();
            let data: IDataObject = folder
                .GetUIObjectOf(HWND::default(), &ptrs, None)
                .expect("build the Shell data object");
            for child in &children {
                CoTaskMemFree(Some(*child as *const c_void));
            }
            CoTaskMemFree(Some(parent_pidl as *const c_void));
            data
        }
    }

    fn application_drop_directories() -> HashSet<OsString> {
        let prefix = format!("favnyr-path-dnd-{}-", std::process::id());
        std::fs::read_dir(std::env::temp_dir())
            .expect("read temp directory")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&prefix)
                    .then(|| entry.file_name())
            })
            .collect()
    }
}
