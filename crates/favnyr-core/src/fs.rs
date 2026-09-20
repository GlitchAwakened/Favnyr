//! Listing, classification, sorting, formatting, and FS operations.
//!
//! This file covers **listing** a folder, **classification** by type,
//! **sorting**, and **formatting**. File operations
//! (copy / move / trash / properties) live in the
//! [`ops`] submodule.

pub mod ops;

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::Result;

/// Category of a file — drives icon selection on the Slint side.
///
/// The `u8` values are **stable**: they are serialized to Slint
/// (`int`) on the `FileRow.kind` side and used in ternary cascades
/// `entry.kind == 0 ? @image-url(...)`. **Do not reorder.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum FileKind {
    Folder = 0,
    File = 1,
    Application = 2,
    Archive = 3,
    Audio = 4,
    Document = 5,
    Image = 6,
    Video = 7,
    /// Configuration files / structured data (toml, yaml, json, ini…).
    Config = 8,
}

/// Extensions recognized as images, sorted to allow binary search.
/// This list is also the authority for the Windows gallery filter: a new
/// extension therefore never needs to be added in two places.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "af", "afdesign", "afphoto", "afpub", "arw", "avif", "bmp", "cr2", "cr3", "dds", "dng", "exr",
    "gif", "hdr", "heic", "heif", "ico", "jpeg", "jpg", "nef", "orf", "pam", "pbm", "pgm", "png",
    "pnm", "ppm", "psd", "qoi", "raf", "raw", "rw2", "srw", "svg", "tga", "tif", "tiff", "webp",
    "xcf",
];

/// `extension` must be normalized to lowercase, without a dot.
pub fn is_image_extension(extension: &str) -> bool {
    IMAGE_EXTENSIONS.binary_search(&extension).is_ok()
}

/// Extensions recognized as archives, sorted to allow binary search.
/// Single source of truth: it classifies rows AND pre-fills the extension
/// filter of the ready-made "extract" commands, so an archive format never
/// needs adding in two places.
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "7z", "ar", "bz2", "cab", "gz", "iso", "lz", "lzma", "lzo", "rar", "tar", "tbz2", "tgz", "txz",
    "xz", "zip", "zst",
];

/// `extension` must be normalized to lowercase, without a dot.
pub fn is_archive_extension(extension: &str) -> bool {
    ARCHIVE_EXTENSIONS.binary_search(&extension).is_ok()
}

impl FileKind {
    pub fn as_i32(self) -> i32 {
        self as u8 as i32
    }

    /// Can a file of this type be a launchable PROGRAM / SCRIPT (with
    /// dropped files as arguments)? Used to filter out false positives from
    /// the Unix executable bit: on a FAT/NTFS/exFAT mount (everything is
    /// `0777`) or for a file copied from Windows, an image/video/audio/document/
    /// archive/config can appear "executable" without being a program. Only a
    /// binary/package/script of a recognized type (`Application`) or a file of
    /// an UNRECOGNIZED type (`File` — typically a binary or a script WITHOUT an
    /// extension, common on Linux) are real candidates.
    pub fn can_be_program(self) -> bool {
        matches!(self, FileKind::Application | FileKind::File)
    }
}

/// An entry listed in a folder.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
    pub mtime_unix: Option<i64>,
    pub is_dir: bool,
    pub kind: FileKind,
    /// **Hidden** entry (Unix dotfile OR Windows HIDDEN attribute) — for a
    /// discreet visual marker when showing hidden items is enabled.
    pub hidden: bool,
    /// Symbolic link (or Windows junction). `is_dir` reflects the TARGET;
    /// this flag is used for the visual marker — a "folder + arrow" icon for
    /// a folder-link, to distinguish it from a real folder.
    pub is_symlink: bool,
    /// Unix executable bit computed during listing (avoids a second `stat`
    /// in the GUI for the "launch with dropped files" feedback).
    /// Always `false` on Windows, where extensions define this action.
    pub executable: bool,
}

/// Classifies a file from its extension (lower-case, without the dot).
/// For a folder, returns `Folder` regardless of the name.
pub fn classify_kind(extension: Option<&str>, is_dir: bool) -> FileKind {
    if is_dir {
        return FileKind::Folder;
    }
    let Some(ext) = extension else {
        return FileKind::File;
    };
    let ext = ext.to_ascii_lowercase();
    if is_image_extension(&ext) {
        return FileKind::Image;
    }
    if is_archive_extension(&ext) {
        return FileKind::Archive;
    }
    match ext.as_str() {
        // Applications / binaries
        "exe" | "msi" | "appimage" | "deb" | "rpm" | "flatpak" | "snap" | "apk" | "dmg" | "pkg"
        | "sh" | "bat" | "ps1" | "cmd" => FileKind::Application,

        // Audio
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "aac" | "m4a" | "wma" | "aiff" | "alac" => {
            FileKind::Audio
        }

        // Documents (text, word processing, presentations, spreadsheets)
        "txt" | "md" | "rst" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "tex" | "epub" | "djvu"
        | "xls" | "xlsx" | "ods" | "csv" | "tsv" | "ppt" | "pptx" | "odp" | "pages" | "numbers"
        | "key" => FileKind::Document,

        // Video
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg" | "ts"
        | "3gp" => FileKind::Video,

        // Configuration / structured data
        "toml" | "yaml" | "yml" | "json" | "json5" | "ini" | "cfg" | "conf" | "config" | "xml"
        | "plist" | "env" | "reg" => FileKind::Config,

        _ => FileKind::File,
    }
}

/// True if `md` carries the Windows `FILE_ATTRIBUTE_HIDDEN` attribute.
/// On other OSes: always false (hidden files there follow the `.` prefix
/// convention, handled separately). `const`-evaluated → optimized.
#[cfg(windows)]
fn is_hidden_attr(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
    md.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}
#[cfg(not(windows))]
fn is_hidden_attr(_md: &std::fs::Metadata) -> bool {
    false
}

/// Lists the entries of a folder.
///
/// `include_hidden = false` filters out **hidden** entries:
///   - Unix convention: name starting with `.` (on all OSes);
///   - Windows: `FILE_ATTRIBUTE_HIDDEN` attribute (Windows hidden files
///     generally don't use the `.` prefix).
///
/// Per-entry errors (permission, broken link…) are **silently ignored**
/// at the item level but logged: a partial listing is preferred over
/// giving up entirely because of one exotic file.
pub fn list_dir(path: &Path, include_hidden: bool) -> Result<Vec<Entry>> {
    Ok(list_dir_counted(path, include_hidden)?.0)
}

/// Like [`list_dir`], but ALSO returns the number of hidden entries present
/// in the folder — whether they're included (`include_hidden`) or not. Used to
/// discreetly signal to the user that hidden items exist (they
/// sometimes forget to check "show hidden files").
/// UNC server root (`\\HOST` **without** a share) → host name. `None` for any
/// other path (local, or UNC with a share `\\HOST\share`). Pure string
/// analysis (no I/O). Used to enumerate a network machine's shares.
pub fn unc_server_root(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let rest = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//"))?;
    let rest = rest.trim_end_matches(['\\', '/']);
    if rest.is_empty() || rest.contains('\\') || rest.contains('/') {
        return None; // just "\\", or there's already a share
    }
    Some(rest.to_string())
}

/// Is a path listable by Favnyr? A classic folder, OR (Windows) a UNC
/// server root whose shares we know how to enumerate. Allows navigating to
/// `\\HOST`, which `Path::is_dir()` refuses (it's not a folder in the FS sense).
///
pub fn is_listable(path: &Path) -> bool {
    path.is_dir() || (cfg!(windows) && unc_server_root(path).is_some())
}

/// "Parent" of a UNC SHARE ROOT: `\\HOST\share` → `\\HOST` (the
/// server root, navigable via share enumeration. `None` for any
/// other path — notably a share subfolder (`\\HOST\share\x`), for which
/// `Path::parent()` works normally. Fills the gap left by `Path::parent()`,
/// which returns `None` on a share root.
pub fn unc_share_parent(path: &Path) -> Option<PathBuf> {
    use std::path::{Component, Prefix};
    let mut comps = path.components();
    let Some(Component::Prefix(p)) = comps.next() else {
        return None;
    };
    let server = match p.kind() {
        Prefix::UNC(server, _) | Prefix::VerbatimUNC(server, _) => server.to_string_lossy(),
        _ => return None,
    };
    // Only the share root (nothing after prefix + root).
    if comps.any(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(PathBuf::from(format!(r"\\{server}")))
}

/// Network UNC path (`\\server…`)? Excludes **local** namespaces
/// `\\?\` (verbatim) and `\\.\` (device). Used to only offer the system
/// login on real network paths.
pub fn is_unc_path(path: &Path) -> bool {
    match path.to_str() {
        Some(s) => {
            (s.starts_with(r"\\") || s.starts_with("//"))
                && !s.starts_with(r"\\?\")
                && !s.starts_with(r"\\.\")
        }
        None => false,
    }
}

// ---------- What the user types in the address bar ----------

/// Turns an address typed by the user into a path.
///
/// Two conveniences, each written the way its own platform writes it:
///   - `~` and `~/…` for the home folder, on every platform;
///   - `%VAR%` on Windows, `$VAR` and `${VAR}` elsewhere.
///
/// **A name that is not a defined variable is left exactly as typed.** That one
/// rule covers the three cases that matter: a real variable expands, a typo
/// reaches the caller untouched so the usual "not listable" message can name
/// it, and a folder whose name merely carries a percent sign or a dollar is
/// never mangled.
///
/// Case follows the platform for free: a variable name is matched without
/// regard to case on Windows and with it elsewhere, because that is how the
/// system itself answers.
///
/// A folder CAN legitimately be named after a variable. Expanding wins, as it
/// does in the system's own file manager; such a folder stays reachable by
/// navigating into it rather than by typing its name.
pub fn expand_typed_path(raw: &str) -> PathBuf {
    // The tilde goes first, as a shell does it: it is only special at the very
    // start, and what a variable expands to must not be re-read for one.
    match strip_home_prefix(raw) {
        Some((home, rest)) => {
            let rest = expand_variables(rest);
            if rest.is_empty() {
                home
            } else {
                home.join(rest)
            }
        }
        None => PathBuf::from(expand_variables(raw)),
    }
}

/// Splits a leading `~` off, returning the home folder and what followed it.
/// Both separators are accepted on Windows, where a user types either.
fn strip_home_prefix(raw: &str) -> Option<(PathBuf, &str)> {
    let home = || dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    if raw == "~" {
        return Some((home(), ""));
    }
    let rest = raw.strip_prefix("~/").or_else(|| {
        if cfg!(windows) {
            raw.strip_prefix(r"~\")
        } else {
            None
        }
    })?;
    Some((home(), rest))
}

/// Replaces every `%NAME%` that names a defined variable. A `%` that opens
/// nothing, or opens a name the environment does not know, stays where it is —
/// which is also what leaves `%%` alone.
#[cfg(windows)]
fn expand_variables(text: &str) -> String {
    expand_variables_with(text, |name| std::env::var(name).ok())
}

/// The rules above, with the environment handed in: what a variable resolves to
/// is the only thing here that depends on the machine, so injecting it is what
/// makes the parsing testable.
#[cfg(windows)]
fn expand_variables_with(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        // The name runs to the next `%`. Anything may sit between the two but a
        // `%` itself: two real variables carry parentheses
        // (`%ProgramFiles(x86)%`), so letters alone would not do.
        match after.find('%').map(|close| (close, &after[..close])) {
            Some((close, name)) if !name.is_empty() => match lookup(name) {
                Some(value) => {
                    out.push_str(&value);
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replaces every `$NAME` and `${NAME}` that names a defined variable. A `$`
/// followed by something else, or by a name the environment does not know,
/// stays where it is.
#[cfg(not(windows))]
fn expand_variables(text: &str) -> String {
    expand_variables_with(text, |name| std::env::var(name).ok())
}

/// The rules above, with the environment handed in: what a variable resolves to
/// is the only thing here that depends on the machine, so injecting it is what
/// makes the parsing testable.
#[cfg(not(windows))]
fn expand_variables_with(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar + 1..];
        let braced = after.strip_prefix('{');
        let found = match braced {
            // `${NAME}`: the braces say where the name ends, so it may hold
            // anything, which is the reason the form exists.
            Some(body) => body
                .find('}')
                .map(|close| (&body[..close], &body[close + 1..])),
            // `$NAME`: the name ends at the first character a variable name
            // cannot hold, which is how `$USER/Documents` finds `USER`.
            None => {
                let end = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                Some((&after[..end], &after[end..]))
            }
        };
        match found {
            Some((name, tail)) if !name.is_empty() => match lookup(name) {
                Some(value) => {
                    out.push_str(&value);
                    rest = tail;
                }
                None => {
                    out.push('$');
                    rest = after;
                }
            },
            _ => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Lists the shares of a UNC server root as `Entry` items (folders) —
/// `\\HOST` becomes "browsable" like in Explorer. `Err(PermissionDenied)`
/// if authentication is required (→ GUI-side login prompt), `Err(NotConnected)`
/// if unreachable → "unavailable" banner.
#[cfg(windows)]
fn list_server_shares(server: &str) -> Result<(Vec<Entry>, usize)> {
    use crate::places::NetShares;
    match crate::places::net_shares(server) {
        NetShares::Ok(shares) => {
            let entries = shares
                .into_iter()
                .map(|sh| Entry {
                    path: PathBuf::from(format!(r"\\{server}\{sh}")),
                    name: sh,
                    size_bytes: None,
                    mtime_unix: None,
                    is_dir: true,
                    kind: FileKind::Folder,
                    hidden: false,
                    is_symlink: false,
                    executable: false,
                })
                .collect();
            Ok((entries, 0))
        }
        NetShares::AuthNeeded => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "network authentication required",
        )
        .into()),
        NetShares::Unreachable => Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "network server unreachable",
        )
        .into()),
    }
}

pub fn list_dir_counted(path: &Path, include_hidden: bool) -> Result<(Vec<Entry>, usize)> {
    // UNC server root (`\\HOST`): its SMB SHARES are enumerated (std::fs can
    // only list a single share, not a server). Shares are displayed as folders.
    #[cfg(windows)]
    {
        if let Some(server) = unc_server_root(path) {
            return list_server_shares(&server);
        }
    }
    let read = std::fs::read_dir(path)?;
    let mut out = Vec::with_capacity(64);
    let mut hidden_count = 0usize;
    for entry_res in read {
        let dir_entry = match entry_res {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, path = %path.display(), "read_dir item failed");
                continue;
            }
        };

        let name = dir_entry.file_name().to_string_lossy().into_owned();
        // Dotfiles (Unix convention) — fast filter, no `stat`. The dotfile is
        // counted as hidden HERE, which preserves the original optimization
        // (the `stat` is skipped when hidden items aren't shown).
        let is_dotfile = name.starts_with('.');
        if is_dotfile {
            hidden_count += 1;
            if !include_hidden {
                continue;
            }
        }

        let full_path = dir_entry.path();

        let md_res = dir_entry.metadata();
        // HIDDEN attribute (Windows): read from the already-loaded `metadata`
        // (no extra `stat`). No-op on Linux/macOS.
        let hidden_attr = md_res.as_ref().map(is_hidden_attr).unwrap_or(false);
        // A non-dotfile hidden by attribute → counted here (`&& !is_dotfile`
        // avoids double-counting a dotfile that would ALSO have the hidden attribute).
        if hidden_attr && !is_dotfile {
            hidden_count += 1;
        }
        if !include_hidden && hidden_attr {
            continue;
        }
        // `hidden` = dotfile OR HIDDEN attribute → visual marker on the GUI side.
        let hidden = is_dotfile || hidden_attr;

        // `DirEntry::metadata()` does NOT follow symbolic links → a link to a
        // FOLDER would report `is_dir=false` and Favnyr would open it in the
        // system file manager instead of navigating into it. The TARGET is
        // therefore resolved (via `fs::metadata`, which follows) ONLY for
        // links — the cost of an extra `stat` reserved for this rare case.
        // Broken link → target not found → treated as a non-folder.
        let is_symlink = dir_entry
            .file_type()
            .map(|t| t.is_symlink())
            .unwrap_or(false);
        let (is_dir, size_bytes, mtime_unix, executable) = match md_res {
            Ok(md) => {
                // A link describes itself, not what it points at: its own
                // length is the size of the stored path and its own timestamp
                // is when the link was made. On Unix its permissions are
                // usually 0777 as well. Every displayed field therefore reads
                // the FOLLOWED metadata, so a link to a file reports that
                // file's size, date and executability rather than the link's.
                // A broken link has no target to follow and keeps its own.
                let followed_md = is_symlink
                    .then(|| std::fs::metadata(&full_path).ok())
                    .flatten();
                let effective_md = followed_md.as_ref().unwrap_or(&md);
                let is_dir = effective_md.is_dir();
                let size = if is_dir {
                    None
                } else {
                    Some(effective_md.len())
                };
                let mtime = effective_md
                    .modified()
                    .ok()
                    .and_then(|st| st.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64);
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    !is_dir
                        && (!is_symlink || followed_md.is_some())
                        && effective_md.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                (is_dir, size, mtime, executable)
            }
            Err(err) => {
                tracing::debug!(error = %err, path = %full_path.display(), "metadata failed");
                // The entry is kept but without size/mtime, assuming a file.
                (false, None, None, false)
            }
        };

        let ext = full_path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_owned());
        let kind = classify_kind(ext.as_deref(), is_dir);

        out.push(Entry {
            name,
            path: full_path,
            size_bytes,
            mtime_unix,
            is_dir,
            kind,
            hidden,
            is_symlink,
            executable,
        });
    }
    Ok((out, hidden_count))
}

// ----- Sorting ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortColumn {
    Name,
    Path,
    Size,
    Modified,
    /// Modification age — sorts on the same field as `Modified`
    /// (mtime), it's just the "age" column (rendered with a hot→cold color gradient).
    Age,
    /// File extension — sorts by type, then by name when extensions are equal.
    Ext,
}

impl SortColumn {
    pub fn code(self) -> &'static str {
        match self {
            SortColumn::Name => "name",
            SortColumn::Path => "path",
            SortColumn::Size => "size",
            SortColumn::Modified => "modified",
            SortColumn::Age => "age",
            SortColumn::Ext => "ext",
        }
    }

    pub fn from_code(s: &str) -> Option<SortColumn> {
        Some(match s {
            "name" => SortColumn::Name,
            "path" => SortColumn::Path,
            "size" => SortColumn::Size,
            "modified" => SortColumn::Modified,
            "age" => SortColumn::Age,
            "ext" => SortColumn::Ext,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn flip(self) -> Self {
        match self {
            SortOrder::Asc => SortOrder::Desc,
            SortOrder::Desc => SortOrder::Asc,
        }
    }
}

/// Grouping by type, orthogonal to the sort criterion (column + direction).
/// - `FoldersFirst` : folders on top, then files (default mode).
/// - `FilesFirst`   : files on top, then folders.
/// - `Mixed`        : no grouping, everything is sorted together by the criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupMode {
    FoldersFirst,
    FilesFirst,
    Mixed,
}

impl GroupMode {
    pub fn code(self) -> &'static str {
        match self {
            GroupMode::FoldersFirst => "folders",
            GroupMode::FilesFirst => "files",
            GroupMode::Mixed => "mixed",
        }
    }

    pub fn from_code(s: &str) -> Option<GroupMode> {
        Some(match s {
            "folders" => GroupMode::FoldersFirst,
            "files" => GroupMode::FilesFirst,
            "mixed" => GroupMode::Mixed,
            _ => return None,
        })
    }
}

/// Sorts in place according to the criterion (`column` + `order`) and the
/// grouping by type (`group`). Grouping takes **priority** over the criterion:
/// in `FoldersFirst`/`FilesFirst` mode, one group always precedes the other
/// regardless of sort direction; the criterion only breaks ties within a
/// group. In `Mixed` mode, folders and files are sorted together by the
/// criterion alone.
///
/// Sort by name: case-insensitive (consistent with the visual sort).
pub fn sort(entries: &mut [Entry], column: SortColumn, order: SortOrder, group: GroupMode) {
    use std::cmp::Reverse;
    let asc = matches!(order, SortOrder::Asc);
    // Grouping rank (folders vs files): it splits the list into two blocks the
    // criterion never crosses, and — unlike the criterion — it is NEVER reversed
    // by the direction (folders stay on top in FoldersFirst even when sorting
    // descending). `Mixed` puts everything in one block.
    let rank = |e: &Entry| -> u8 {
        match group {
            GroupMode::FoldersFirst => u8::from(!e.is_dir),
            GroupMode::FilesFirst => u8::from(e.is_dir),
            GroupMode::Mixed => 0,
        }
    };
    // Name/Ext sort case-insensitively. Lowercasing a name INSIDE a comparison
    // sort re-allocates it on every comparison (~N·log N times); caching the key
    // lowercases each name once per entry instead. Only the criterion is
    // reversed for a descending sort (via `Reverse`), never the grouping rank.
    match column {
        SortColumn::Name if asc => entries.sort_by_cached_key(|e| (rank(e), e.name.to_lowercase())),
        SortColumn::Name => {
            entries.sort_by_cached_key(|e| (rank(e), Reverse(e.name.to_lowercase())))
        }
        // Type sort: extension (lowercase, dotfiles included), then name so the
        // order stays stable within one type.
        SortColumn::Ext if asc => entries.sort_by_cached_key(|e| {
            (
                rank(e),
                ops::ext_of(&e.name).to_string(),
                e.name.to_lowercase(),
            )
        }),
        SortColumn::Ext => entries.sort_by_cached_key(|e| {
            (
                rank(e),
                Reverse((ops::ext_of(&e.name).to_string(), e.name.to_lowercase())),
            )
        }),
        // Size / date / path: these comparators allocate nothing, so a direct
        // comparison sort is already optimal — only the grouping is kept apart
        // from the (possibly reversed) criterion.
        SortColumn::Size | SortColumn::Modified | SortColumn::Age | SortColumn::Path => {
            entries.sort_by(|a, b| {
                rank(a).cmp(&rank(b)).then_with(|| {
                    let ord = match column {
                        SortColumn::Size => {
                            a.size_bytes.unwrap_or(0).cmp(&b.size_bytes.unwrap_or(0))
                        }
                        SortColumn::Modified | SortColumn::Age => {
                            a.mtime_unix.unwrap_or(0).cmp(&b.mtime_unix.unwrap_or(0))
                        }
                        _ => a.path.cmp(&b.path),
                    };
                    if asc { ord } else { ord.reverse() }
                })
            });
        }
    }
}

// ----- Formatting ----------------------------------------------------------

/// Wording of a formatted size, supplied by the CALLER.
///
/// This crate holds no translations: the algorithm lives here, the vocabulary
/// lives with the interface. That also keeps the two locale-dependent pieces
/// together — a language that writes `Ko` also writes `1,0`, and separating
/// them is how the decimal mark ended up applied to French alone while
/// Spanish, German and Italian kept an English point.
#[derive(Debug, Clone, Copy)]
pub struct SizeUnits<'a> {
    /// Units by increasing power of 1024: byte, kilo, mega, giga, tera.
    pub steps: [&'a str; 5],
    /// Mark between the integer and the decimal part.
    pub decimal: char,
}

impl SizeUnits<'_> {
    /// Renders `value` with `decimals` places, using the locale's mark.
    fn number(&self, value: f64, decimals: usize) -> String {
        let s = format!("{value:.decimals$}");
        if self.decimal == '.' {
            s
        } else {
            s.replace('.', &self.decimal.to_string())
        }
    }
}

/// Formats a size in bytes with the most appropriate unit.
/// Base-1024 convention (binary), one decimal beyond KB.
pub fn format_size(bytes: u64, units: SizeUnits<'_>) -> String {
    if bytes < 1024 {
        return format!("{bytes} {}", units.steps[0]);
    }
    let mut value = bytes as f64;
    let mut idx = 0usize;
    while value >= 1024.0 && idx + 1 < units.steps.len() {
        value /= 1024.0;
        idx += 1;
    }
    // One decimal beyond the byte step.
    format!("{} {}", units.number(value, 1), units.steps[idx])
}

/// How comfortable the remaining space on a volume is: `0` roomy, `1` low,
/// `2` critical. Used to colour the sidebar capacity badge.
///
/// A share alone is not enough: "90% used" leaves 400 GB on a 4 TB archive,
/// which is no problem at all. So each level pairs a ratio with a ceiling above
/// which no percentage justifies an alarm:
///
///   - the ratios (10% / 20%) are the classic filesystem health marks: ext4
///     reserves 5% for root, and both ext4 and NTFS lose allocation locality
///     below a tenth free, as do SSDs their spare area;
///   - the ceilings (128 / 256 GiB) keep a large archive quiet: 700 GB free is
///     not a warning, whatever fraction of the disk it represents.
///
/// The ratio is what makes this scale from a 3 GB USB stick to an 8 TB archive.
/// An earlier version also had absolute floors (< 4 / 16 GiB free = alarm), but
/// a small removable drive can never hold that much: a 3 GB stick two-thirds
/// EMPTY still has under 4 GiB free and lit up "critical" (red). The ratio
/// already flags a genuinely full small drive (a few % free), so the floors only
/// mis-fired — always on tiny drives, and redundant on large ones.
pub fn free_space_level(free: u64, total: u64) -> i32 {
    const GIB: u64 = 1024 * 1024 * 1024;
    if total == 0 {
        return 0;
    }
    if free < total / 10 && free < 128 * GIB {
        return 2;
    }
    if free < total / 5 && free < 256 * GIB {
        return 1;
    }
    0
}

/// Formats a used/total pair for the capacity gauge, both figures sharing the
/// **total's** unit so they can be compared at a glance: `26.0 / 57.3 GB`.
///
/// [`format_size`] cannot do this — called twice it picks each number's own
/// unit, so a nearly empty disk would read "900 Mo/1,8 To" and force the reader
/// to convert before judging anything. Same base-1024 convention and same
/// localized unit names, so the gauge agrees with the size column.
///
/// Used rather than free, and it must stay that way: the bar beside these
/// figures grows as the volume fills, so a figure counting DOWN while the bar
/// counts up left the reader with two contradictory readings of one fact. The
/// free space has its own, unambiguous place in the hover hint.
pub fn format_used_total(used: u64, total: u64, units: SizeUnits<'_>) -> String {
    // Unit of the total: the larger of the two, so the pair stays comparable.
    let mut scale = 1.0_f64;
    let mut idx = 0usize;
    while (total as f64) / scale >= 1024.0 && idx + 1 < units.steps.len() {
        scale *= 1024.0;
        idx += 1;
    }
    let decimals = if idx == 0 { 0 } else { 1 };
    // Spaces around the slash: at this size the two figures ran into the
    // separator and read as one long number.
    format!(
        "{} / {} {}",
        units.number(used as f64 / scale, decimals),
        units.number(total as f64 / scale, decimals),
        units.steps[idx]
    )
}

/// A single bounded walk over `path` computing BOTH its recursive **max mtime**
/// (the most recent modification date among the folder and its descendants, up
/// to `mtime_depth` levels) AND its recursive **total size** (sum of descendant
/// files' bytes, up to `size_depth` levels). `depth = 1` = direct children, `2`
/// = grandchildren, …; a depth of `0` DISABLES that metric (its result is
/// `None`). Doing both in one traversal costs a single `stat` per entry instead
/// of two — the reason the two "folder date"/"folder size" options share it.
///
/// Symbolic links are NOT followed (`DirEntry::metadata`) → no loops or runaway
/// cost; only regular FILES add to the size. Errors (permission, broken link)
/// are ignored: a partial result beats none.
pub fn recursive_folder_stats(
    path: &Path,
    mtime_depth: u32,
    size_depth: u32,
) -> (Option<i64>, Option<u64>) {
    fn to_unix(md: &std::fs::Metadata) -> Option<i64> {
        md.modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() as i64)
    }
    fn walk(
        dir: &Path,
        mtime_left: u32,
        size_left: u32,
        max_mtime: &mut Option<i64>,
        total_size: &mut Option<u64>,
    ) {
        if mtime_left == 0 && size_left == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let Ok(md) = entry.metadata() else {
                continue; // does not follow symbolic links
            };
            if mtime_left > 0
                && let Some(m) = to_unix(&md)
                && max_mtime.is_none_or(|cur| m > cur)
            {
                *max_mtime = Some(m);
            }
            if size_left > 0
                && md.is_file()
                && let Some(total) = total_size.as_mut()
            {
                *total = total.saturating_add(md.len());
            }
            if md.is_dir() {
                walk(
                    &entry.path(),
                    mtime_left.saturating_sub(1),
                    size_left.saturating_sub(1),
                    max_mtime,
                    total_size,
                );
            }
        }
    }
    // Bases: the folder's own mtime; a directory has no intrinsic size, so the
    // size accumulator starts at 0 and only files add to it.
    let mut max_mtime = if mtime_depth > 0 {
        std::fs::metadata(path).ok().as_ref().and_then(to_unix)
    } else {
        None
    };
    let mut total_size = if size_depth > 0 { Some(0) } else { None };
    walk(
        path,
        mtime_depth,
        size_depth,
        &mut max_mtime,
        &mut total_size,
    );
    (max_mtime, total_size)
}

/// Recursive **max mtime** up to `depth` levels — the mtime-only case of
/// [`recursive_folder_stats`]. `depth = 0` = the folder's own mtime.
pub fn recursive_max_mtime(path: &Path, depth: u32) -> Option<i64> {
    // `recursive_folder_stats` treats an mtime depth of 0 as "disabled" (`None`),
    // but this function's historical contract is that depth 0 = the folder's own
    // mtime — handled directly here.
    if depth == 0 {
        return std::fs::metadata(path)
            .ok()
            .and_then(|md| md.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
    }
    recursive_folder_stats(path, depth, 0).0
}

/// ISO-like format, locale-independent: `YYYY-MM-DD HH:MM`. It stays
/// readable in every language without depending on `chrono`.
///
/// `offset_secs` shifts the Unix (UTC) timestamp before decomposition: `0` = UTC,
/// a local offset (set by the GUI) = local time. The timezone is a user
/// setting — the core stays agnostic by receiving the already-resolved offset.
pub fn format_mtime(unix: i64, offset_secs: i64) -> String {
    let unix = unix + offset_secs;
    let days_from_epoch = unix.div_euclid(86_400);
    let secs_in_day = unix.rem_euclid(86_400);
    let hh = (secs_in_day / 3600) as u32;
    let mm = ((secs_in_day % 3600) / 60) as u32;

    let (y, mo, d) = civil_from_days(days_from_epoch);
    format!("{y:04}-{mo:02}-{d:02} {hh:02}:{mm:02}")
}

/// Compact units for the "age" column depending on the language:
/// `(minute, day, month, year, "just now")`. The hour uses `h` and
/// minutes the apostrophe `'` (compact and language-neutral, for example "2 h 50'").
#[derive(Debug, Clone, Copy)]
pub struct AgeUnits<'a> {
    pub minute: &'a str,
    pub day: &'a str,
    pub month: &'a str,
    pub year: &'a str,
    /// Whole wording for "less than a minute ago" — a sentence, not a unit.
    pub now: &'a str,
}

/// Formats the **age** (now − mtime) compactly, readable at a glance:
/// `42 min`, `2 h 50'`, `25d`, `3mo`, `2y`. `now_unix` and
/// `mtime_unix` are Unix seconds.
pub fn format_age(mtime_unix: i64, now_unix: i64, units: AgeUnits<'_>) -> String {
    let secs = (now_unix - mtime_unix).max(0);
    if secs < 60 {
        return units.now.to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins} {}", units.minute);
    }
    let hours = mins / 60;
    if hours < 24 {
        let m = mins % 60;
        return format!("{hours} h {m:02}'");
    }
    let days = hours / 24;
    if days < 31 {
        return format!("{days}{}", units.day);
    }
    if days < 365 {
        let mo = days / 30;
        return format!("{mo}{}", units.month);
    }
    let years = days / 365;
    format!("{years}{}", units.year)
}

/// "Heat" index of the age, for the hot→cold color gradient of the
/// "age" column: `0` = very recent (hot) … `6` = old (cold).
pub fn age_bucket(mtime_unix: i64, now_unix: i64) -> i32 {
    let secs = (now_unix - mtime_unix).max(0);
    let hours = secs / 3600;
    let days = hours / 24;
    if hours < 1 {
        0
    } else if hours < 6 {
        1
    } else if days < 1 {
        2
    } else if days < 7 {
        3
    } else if days < 30 {
        4
    } else if days < 365 {
        5
    } else {
        6
    }
}

/// Converts days-since-1970 → (year, month, day) (Howard Hinnant's
/// algorithm, public domain). Avoids the dependency on `chrono` for this
/// minimal need.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {

    /// The environment is handed in, so these assert the PARSING rather than
    /// whatever the machine running them happens to define.
    fn fake_env(name: &str) -> Option<String> {
        match name {
            "APPDATA" => Some(r"C:\Users\someone\AppData\Roaming".to_owned()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_owned()),
            "HOME" => Some("/home/someone".to_owned()),
            "USER" => Some("someone".to_owned()),
            _ => None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_variables_expand_only_when_they_exist() {
        let expand = |text: &str| expand_variables_with(text, fake_env);

        assert_eq!(
            expand(r"%APPDATA%\Favnyr"),
            r"C:\Users\someone\AppData\Roaming\Favnyr"
        );
        // Two real variables carry parentheses, so a letters-only name would
        // have missed them.
        assert_eq!(
            expand(r"%ProgramFiles(x86)%\Tool"),
            r"C:\Program Files (x86)\Tool"
        );
        // Anywhere in the path, not just at the start.
        assert_eq!(
            expand(r"C:\x\%APPDATA%"),
            r"C:\x\C:\Users\someone\AppData\Roaming"
        );

        // Untouched: an unknown name reaches the caller so it can say so, and a
        // percent sign that names nothing is just a character in a folder name.
        assert_eq!(expand("%NOT_A_VARIABLE%"), "%NOT_A_VARIABLE%");
        assert_eq!(expand("Report 100% final"), "Report 100% final");
        assert_eq!(expand("50%-75%"), "50%-75%");
        assert_eq!(expand("%%"), "%%");
        assert_eq!(expand("%unterminated"), "%unterminated");
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_variables_expand_in_both_spellings() {
        let expand = |text: &str| expand_variables_with(text, fake_env);

        assert_eq!(expand("$HOME/Documents"), "/home/someone/Documents");
        assert_eq!(expand("${HOME}/Documents"), "/home/someone/Documents");
        // Mid-path, which is the form a shell user reaches for.
        assert_eq!(expand("/home/$USER/Documents"), "/home/someone/Documents");
        // The braces are what allow a name to be followed by a letter.
        assert_eq!(expand("${USER}name"), "someonename");
        assert_eq!(expand("$USERname"), "$USERname");

        // Untouched: unknown names, and a dollar that opens nothing.
        assert_eq!(expand("$NOT_A_VARIABLE/x"), "$NOT_A_VARIABLE/x");
        assert_eq!(expand("price $ 5"), "price $ 5");
        assert_eq!(expand("${unterminated"), "${unterminated");
    }

    #[test]
    fn a_plain_path_is_returned_unchanged() {
        // Nothing to expand must mean nothing altered, on either platform.
        let typed = if cfg!(windows) {
            r"C:\Users\someone\Documents"
        } else {
            "/home/someone/Documents"
        };
        assert_eq!(expand_typed_path(typed), PathBuf::from(typed));
    }

    #[test]
    fn the_home_shorthand_still_works() {
        let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
        assert_eq!(expand_typed_path("~"), home);
        assert_eq!(expand_typed_path("~/Documents"), home.join("Documents"));
        #[cfg(windows)]
        assert_eq!(expand_typed_path(r"~\Documents"), home.join("Documents"));
        // Only at the very start: a tilde inside a name is a name.
        assert_eq!(expand_typed_path("backup~1"), PathBuf::from("backup~1"));
    }
    use super::*;

    /// Wording the interface supplies for English and French. Mirrored here so
    /// the formatting assertions keep testing the exact rendered strings.
    const EN: SizeUnits<'static> = SizeUnits {
        steps: ["B", "KB", "MB", "GB", "TB"],
        decimal: '.',
    };
    const FR: SizeUnits<'static> = SizeUnits {
        steps: ["o", "Ko", "Mo", "Go", "To"],
        decimal: ',',
    };
    const FR_AGE: AgeUnits<'static> = AgeUnits {
        minute: "min",
        day: "j",
        month: "mois",
        year: "an",
        now: "à l'instant",
    };

    #[test]
    fn classify_folder_overrides_extension() {
        assert_eq!(classify_kind(Some("zip"), true), FileKind::Folder);
    }

    #[test]
    fn unc_server_root_detects_bare_server_only() {
        use std::path::Path;
        // Server root (no share) → host name.
        assert_eq!(unc_server_root(Path::new(r"\\NAS")), Some("NAS".into()));
        assert_eq!(unc_server_root(Path::new(r"\\NAS\")), Some("NAS".into()));
        assert_eq!(
            unc_server_root(Path::new(r"\\192.168.1.50")),
            Some("192.168.1.50".into())
        );
        // With a share OR a local path OR just "\\" → not a server root.
        assert_eq!(unc_server_root(Path::new(r"\\NAS\media")), None);
        assert_eq!(unc_server_root(Path::new(r"\\NAS\media\sub")), None);
        assert_eq!(unc_server_root(Path::new(r"C:\Users")), None);
        assert_eq!(unc_server_root(Path::new(r"\\")), None);
    }

    #[cfg(windows)]
    #[test]
    fn unc_share_parent_only_at_share_root() {
        use std::path::Path;
        // Share root → server root.
        assert_eq!(
            unc_share_parent(Path::new(r"\\NAS\media")),
            Some(PathBuf::from(r"\\NAS"))
        );
        assert_eq!(
            unc_share_parent(Path::new(r"\\NAS\media\")),
            Some(PathBuf::from(r"\\NAS"))
        );
        // Share subfolder → None (Path::parent() is enough).
        assert_eq!(unc_share_parent(Path::new(r"\\NAS\media\sub")), None);
        // Local paths / server root: None.
        assert_eq!(unc_share_parent(Path::new(r"C:\Users")), None);
        assert_eq!(unc_share_parent(Path::new(r"\\NAS")), None);
    }

    #[test]
    fn is_unc_path_only_true_for_network_shares() {
        use std::path::Path;
        assert!(is_unc_path(Path::new(r"\\NAS\media")));
        assert!(is_unc_path(Path::new(r"\\NAS")));
        // LOCAL namespaces (verbatim / device) → not network.
        assert!(!is_unc_path(Path::new(r"\\?\C:\x")));
        assert!(!is_unc_path(Path::new(r"\\.\PhysicalDrive0")));
        assert!(!is_unc_path(Path::new(r"C:\Users")));
    }

    #[test]
    fn classify_known_extensions() {
        // Both lists are searched by bisection: unsorted, they would silently
        // stop matching some of their own entries.
        assert!(IMAGE_EXTENSIONS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(ARCHIVE_EXTENSIONS.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(classify_kind(Some("PNG"), false), FileKind::Image);
        for ext in ["af", "afdesign", "afphoto", "afpub"] {
            assert_eq!(classify_kind(Some(ext), false), FileKind::Image, "{ext}");
        }
        // Every archive extension still classifies as one after the move out of
        // the match arm into a bisected list.
        for ext in ARCHIVE_EXTENSIONS {
            assert_eq!(classify_kind(Some(ext), false), FileKind::Archive, "{ext}");
        }
        assert_eq!(classify_kind(Some("toml"), false), FileKind::Config);
        assert_eq!(classify_kind(Some("mp3"), false), FileKind::Audio);
        assert_eq!(classify_kind(Some("mp4"), false), FileKind::Video);
        assert_eq!(
            classify_kind(Some("AppImage"), false),
            FileKind::Application
        );
        assert_eq!(classify_kind(Some("zip"), false), FileKind::Archive);
        assert_eq!(classify_kind(Some("pdf"), false), FileKind::Document);
    }

    #[test]
    fn classify_unknown_extension_is_file() {
        assert_eq!(classify_kind(Some("xyz123"), false), FileKind::File);
        assert_eq!(classify_kind(None, false), FileKind::File);
    }

    #[test]
    fn the_capacity_warning_weighs_the_share_against_the_amount() {
        const GIB: u64 = 1024 * 1024 * 1024;
        const TIB: u64 = 1024 * GIB;

        // Roomy.
        assert_eq!(free_space_level(38 * GIB, 64 * GIB), 0);
        assert_eq!(free_space_level(180 * GIB, 460 * GIB), 0);
        // A share alone would condemn this one: 8.75% free, but 700 GiB is
        // nobody's emergency. The ceiling is what keeps a big archive quiet.
        assert_eq!(free_space_level(700 * GIB, 8 * TIB), 0);
        // Small removable drives, mostly EMPTY, must read as roomy — the whole
        // point of dropping the absolute floors. A 3 GB stick two-thirds free
        // never reaches 4 GiB free; a 16 GB stick a quarter free never reaches
        // 16 GiB. Both used to light up red / amber.
        assert_eq!(free_space_level(2 * GIB, 3 * GIB), 0); // 67% free
        assert_eq!(free_space_level(4 * GIB, 16 * GIB), 0); // 25% free

        // Low: the share, not any floor, is what flags these.
        assert_eq!(free_space_level(12 * GIB, 64 * GIB), 1); // 18.75%
        assert_eq!(free_space_level(200 * GIB, 8 * TIB), 1);

        // Critical.
        assert_eq!(free_space_level(5 * GIB, 64 * GIB), 2); // 7.8%
        assert_eq!(free_space_level(50 * GIB, TIB), 2); // 5%
        assert_eq!(free_space_level(100 * GIB, 8 * TIB), 2); // under the ceiling
        // A large disk with almost nothing left: the share alone (0.6%) is well
        // under a tenth, so it is caught without any floor.
        assert_eq!(free_space_level(3 * GIB, 500 * GIB), 2);
        // A genuinely full SMALL drive is still flagged — the share scales down
        // with it: 0.5 GB on an 8 GB stick is 6%.
        assert_eq!(free_space_level(GIB / 2, 8 * GIB), 2);

        // Unmeasurable volume: no capacity, so nothing to warn about.
        assert_eq!(free_space_level(0, 0), 0);
    }

    #[test]
    fn a_capacity_pair_shares_one_unit_so_the_two_figures_compare() {
        const GIB: u64 = 1024 * 1024 * 1024;
        const MIB: u64 = 1024 * 1024;
        const TIB: u64 = 1024 * GIB;

        // Both figures in the total's unit — calling `format_size` twice would
        // have picked a unit per number.
        assert_eq!(format_used_total(26 * GIB, 64 * GIB, EN), "26.0 / 64.0 GB");
        assert_eq!(format_used_total(26 * GIB, 64 * GIB, FR), "26,0 / 64,0 Go");

        // A barely used volume keeps the total's unit and reads near zero,
        // which is what it means. No special case is needed now that the
        // figure counts UP alongside the bar instead of down against it.
        assert_eq!(format_used_total(900 * MIB, 2 * TIB, EN), "0.0 / 2.0 TB");

        // Kept honest against the size column: same base-1024 convention, so a
        // drive sold as "64 GB" reads the same here as everywhere else.
        assert_eq!(format_size(64_000_000_000, EN), "59.6 GB");
        assert_eq!(
            format_used_total(32_000_000_000, 64_000_000_000, EN),
            "29.8 / 59.6 GB"
        );

        // Tiny volume: bytes carry no decimal.
        assert_eq!(format_used_total(200, 900, EN), "200 / 900 B");
    }

    #[test]
    fn only_programs_and_untyped_files_can_be_launched() {
        // Real candidates for "launch with these files".
        assert!(FileKind::Application.can_be_program()); // .sh, .appimage, binary
        assert!(FileKind::File.can_be_program()); // binary/script WITHOUT an extension
        // Data: never a program, EVEN with the executable bit (FAT/NTFS
        // 0777 mount) → fixes the incorrect "Open with this program" label
        // on an image/document/etc. on Linux.
        assert!(!FileKind::Image.can_be_program());
        assert!(!FileKind::Video.can_be_program());
        assert!(!FileKind::Audio.can_be_program());
        assert!(!FileKind::Document.can_be_program());
        assert!(!FileKind::Archive.can_be_program());
        assert!(!FileKind::Config.can_be_program());
        assert!(!FileKind::Folder.can_be_program());
    }

    #[test]
    fn format_age_buckets_and_text() {
        const H: i64 = 3600;
        const D: i64 = 86_400;
        let now = 1_000_000_000;
        // < 1 min → "à l'instant".
        assert_eq!(format_age(now - 30, now, FR_AGE), "à l'instant");
        assert_eq!(age_bucket(now - 30, now), 0);
        // minutes.
        assert_eq!(format_age(now - 42 * 60, now, FR_AGE), "42 min");
        // hours + minutes ("2 h 05'").
        assert_eq!(format_age(now - (2 * H + 5 * 60), now, FR_AGE), "2 h 05'");
        // days.
        assert_eq!(format_age(now - 25 * D, now, FR_AGE), "25j");
        assert_eq!(age_bucket(now - 25 * D, now), 4);
        // mtime in the future (clock) → clamped to 0 → "à l'instant".
        assert_eq!(format_age(now + 9999, now, FR_AGE), "à l'instant");
    }

    #[test]
    fn age_bucket_is_monotonic_cold() {
        const D: i64 = 86_400;
        let now = 1_000_000_000;
        // The older it is, the bigger the bucket (colder).
        assert!(age_bucket(now - 30, now) < age_bucket(now - 10 * D, now));
        assert_eq!(age_bucket(now - 400 * D, now), 6);
    }

    #[test]
    fn format_size_units() {
        assert_eq!(format_size(0, EN), "0 B");
        assert_eq!(format_size(512, EN), "512 B");
        assert_eq!(format_size(1024, EN), "1.0 KB");
        assert_eq!(format_size(1536, EN), "1.5 KB");
        assert_eq!(format_size(1024 * 1024, EN), "1.0 MB");
    }

    #[test]
    fn format_size_fr_uses_o_and_comma() {
        assert_eq!(format_size(1024, FR), "1,0 Ko");
        assert_eq!(format_size(1_500_000_000, FR), "1,4 Go");
    }

    #[test]
    fn format_mtime_iso() {
        // Known boundaries (offset 0 = UTC):
        assert_eq!(format_mtime(0, 0), "1970-01-01 00:00");
        assert_eq!(format_mtime(86_400, 0), "1970-01-02 00:00");
        // 2000-01-01 00:00:00 UTC = 946_684_800
        assert_eq!(format_mtime(946_684_800, 0), "2000-01-01 00:00");
        // Format is always `YYYY-MM-DD HH:MM` (16 chars).
        assert_eq!(format_mtime(1_780_948_604, 0).len(), 16);
        // Local offset applied: +2 h → same clock shifted by 2 h.
        assert_eq!(format_mtime(0, 2 * 3600), "1970-01-01 02:00");
        // Negative offset crossing midnight.
        assert_eq!(format_mtime(0, -3600), "1969-12-31 23:00");
    }

    #[test]
    fn sort_folders_first_then_name_asc() {
        let mut v = vec![
            make_entry("zeta.txt", false),
            make_entry("alpha-dir", true),
            make_entry("Bravo.png", false),
            make_entry("aaa-dir", true),
        ];
        sort(
            &mut v,
            SortColumn::Name,
            SortOrder::Asc,
            GroupMode::FoldersFirst,
        );
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["aaa-dir", "alpha-dir", "Bravo.png", "zeta.txt"]);
    }

    #[test]
    fn sort_files_first_then_name_asc() {
        let mut v = vec![
            make_entry("zeta.txt", false),
            make_entry("alpha-dir", true),
            make_entry("Bravo.png", false),
            make_entry("aaa-dir", true),
        ];
        sort(
            &mut v,
            SortColumn::Name,
            SortOrder::Asc,
            GroupMode::FilesFirst,
        );
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        // Files (alpha) then folders (alpha).
        assert_eq!(names, ["Bravo.png", "zeta.txt", "aaa-dir", "alpha-dir"]);
    }

    #[test]
    fn sort_mixed_interleaves_by_name_asc() {
        let mut v = vec![
            make_entry("zeta.txt", false),
            make_entry("alpha-dir", true),
            make_entry("Bravo.png", false),
            make_entry("aaa-dir", true),
        ];
        sort(&mut v, SortColumn::Name, SortOrder::Asc, GroupMode::Mixed);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        // No grouping: everything is sorted together (case-insensitive).
        assert_eq!(names, ["aaa-dir", "alpha-dir", "Bravo.png", "zeta.txt"]);
    }

    #[test]
    fn sort_name_desc_keeps_folders_on_top() {
        let mut v = vec![
            make_entry("zeta.txt", false),
            make_entry("alpha-dir", true),
            make_entry("Bravo.png", false),
            make_entry("aaa-dir", true),
        ];
        sort(
            &mut v,
            SortColumn::Name,
            SortOrder::Desc,
            GroupMode::FoldersFirst,
        );
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        // Descending reverses the names WITHIN each block, but not the grouping:
        // folders stay on top.
        assert_eq!(names, ["alpha-dir", "aaa-dir", "zeta.txt", "Bravo.png"]);
    }

    #[test]
    fn sort_ext_groups_by_extension_then_name() {
        let mut v = vec![
            make_entry("b.txt", false),
            make_entry("a.txt", false),
            make_entry("C.png", false),
            make_entry("a.png", false),
        ];
        sort(&mut v, SortColumn::Ext, SortOrder::Asc, GroupMode::Mixed);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        // Grouped by extension (png before txt), then by name (case-insensitive).
        assert_eq!(names, ["a.png", "C.png", "a.txt", "b.txt"]);
    }

    #[test]
    fn sort_size_desc_keeps_folders_on_top() {
        let mut v = vec![
            sized_entry("big.bin", 10_000_000),
            sized_entry("small.bin", 10),
            make_entry("dir", true),
        ];
        sort(
            &mut v,
            SortColumn::Size,
            SortOrder::Desc,
            GroupMode::FoldersFirst,
        );
        assert!(v[0].is_dir, "folder must remain on top regardless of size");
        assert_eq!(v[1].name, "big.bin");
        assert_eq!(v[2].name, "small.bin");
    }

    #[test]
    fn list_dir_reads_temp() {
        let tmp = std::env::temp_dir().join(format!("favnyr-fs-test-{}", nano()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), b"hello").unwrap();
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join(".hidden"), b"x").unwrap();

        let mut entries = list_dir(&tmp, false).unwrap();
        sort(
            &mut entries,
            SortColumn::Name,
            SortOrder::Asc,
            GroupMode::FoldersFirst,
        );
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.txt"]);

        let entries_hidden = list_dir(&tmp, true).unwrap();
        assert_eq!(entries_hidden.len(), 3);

        // Hidden counter: returned regardless of `include_hidden`.
        let (visible, hidden_n) = list_dir_counted(&tmp, false).unwrap();
        assert_eq!(visible.len(), 2); // sub + a.txt
        assert_eq!(hidden_n, 1); // .hidden
        let (all, hidden_n2) = list_dir_counted(&tmp, true).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(hidden_n2, 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_dir_follows_symlink_to_dir() {
        // A symbolic link to a FOLDER must be classified `is_dir = true`
        // (otherwise Favnyr would open it in the file manager instead of navigating).
        let tmp = std::env::temp_dir().join(format!("favnyr-symlink-{}", nano()));
        std::fs::create_dir_all(tmp.join("realdir")).unwrap();
        let link = tmp.join("linkdir");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(tmp.join("realdir"), &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(tmp.join("realdir"), &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;
        // Link creation is often refused without privilege (Windows) → only
        // assert if the link was actually created (keeps the test non-flaky in restricted CI).
        if made {
            let (entries, _) = list_dir_counted(&tmp, false).unwrap();
            let e = entries
                .iter()
                .find(|e| e.name == "linkdir")
                .expect("linkdir listed");
            assert!(e.is_dir, "a link to a directory must have is_dir = true");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_dir_reports_target_size_and_date_for_a_symlink() {
        // A link stores a path, so its own length is a handful of bytes and its
        // own timestamp is when it was created. Showing those would describe
        // the link instead of the file the user sees listed.
        let tmp = std::env::temp_dir().join(format!("favnyr-symlink-size-{}", nano()));
        std::fs::create_dir_all(&tmp).unwrap();
        let target = tmp.join("payload.bin");
        let payload = vec![7u8; 4096];
        std::fs::write(&target, &payload).unwrap();
        let link = tmp.join("payload-link.bin");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;

        // Creating a link is refused without privilege on Windows; asserting
        // only when one exists keeps the test meaningful and non-flaky.
        if made {
            let (entries, _) = list_dir_counted(&tmp, false).unwrap();
            let linked = entries
                .iter()
                .find(|e| e.name == "payload-link.bin")
                .expect("link listed");
            let real = entries
                .iter()
                .find(|e| e.name == "payload.bin")
                .expect("target listed");
            assert!(linked.is_symlink, "the entry is still marked as a link");
            assert_eq!(
                linked.size_bytes,
                Some(payload.len() as u64),
                "a link must report the size of its target"
            );
            assert_eq!(
                linked.mtime_unix, real.mtime_unix,
                "a link must report the date of its target"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn list_dir_reports_target_executable_bits_including_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let tmp = std::env::temp_dir().join(format!("favnyr-executable-{}", nano()));
        std::fs::create_dir_all(&tmp).unwrap();
        let executable = tmp.join("tool");
        let plain = tmp.join("plain");
        std::fs::write(&executable, b"#!/bin/sh\n").unwrap();
        std::fs::write(&plain, b"text\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&executable, tmp.join("tool-link")).unwrap();
        symlink(&plain, tmp.join("plain-link")).unwrap();
        symlink(tmp.join("missing"), tmp.join("broken-link")).unwrap();

        let entries = list_dir(&tmp, false).unwrap();
        let executable_of = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.executable)
                .unwrap()
        };
        assert!(executable_of("tool"));
        assert!(executable_of("tool-link"));
        assert!(!executable_of("plain"));
        assert!(!executable_of("plain-link"));
        assert!(!executable_of("broken-link"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn recursive_max_mtime_respects_depth() {
        use std::time::{Duration, UNIX_EPOCH};
        let tmp = std::env::temp_dir().join(format!("favnyr-rmtime-{}", nano()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("a.txt"), b"x").unwrap();
        let deep = tmp.join("sub").join("deep.txt");
        std::fs::write(&deep, b"y").unwrap();

        // Explicit FUTURE mtime on the deep file (level 2).
        let future: i64 = 4_000_000_000; // ~2096, > any "current" mtime
        std::fs::File::options()
            .write(true)
            .open(&deep)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(future as u64))
            .unwrap();

        // depth 2 reaches deep.txt; depth 1 (direct children) does not.
        assert_eq!(recursive_max_mtime(&tmp, 2), Some(future));
        let d1 = recursive_max_mtime(&tmp, 1).unwrap();
        assert!(d1 < future, "depth 1 must NOT reach level 2");
        // depth 0 = the folder's own mtime only (≤ depth 1).
        let d0 = recursive_max_mtime(&tmp, 0).unwrap();
        assert!(d0 <= d1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn recursive_folder_stats_sums_size_by_depth() {
        let tmp = std::env::temp_dir().join(format!("favnyr-fsize-{}", nano()));
        std::fs::create_dir_all(tmp.join("sub").join("deep")).unwrap();
        std::fs::write(tmp.join("a.txt"), [0u8; 10]).unwrap(); // level 1
        std::fs::write(tmp.join("sub").join("b.txt"), [0u8; 20]).unwrap(); // level 2
        std::fs::write(tmp.join("sub").join("deep").join("c.txt"), [0u8; 30]).unwrap(); // level 3

        // Size accumulates by depth; folders themselves add nothing.
        assert_eq!(recursive_folder_stats(&tmp, 0, 1).1, Some(10));
        assert_eq!(recursive_folder_stats(&tmp, 0, 2).1, Some(30));
        assert_eq!(recursive_folder_stats(&tmp, 0, 3).1, Some(60));
        // A depth of 0 disables that metric.
        assert_eq!(recursive_folder_stats(&tmp, 0, 0).1, None);
        assert_eq!(recursive_folder_stats(&tmp, 0, 2).0, None);
        // Unified: one walk returns both, and each half matches its single-metric call.
        let (mtime, size) = recursive_folder_stats(&tmp, 2, 2);
        assert!(mtime.is_some(), "mtime computed");
        assert_eq!(size, Some(30), "size at depth 2");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Benchmark for listing 10,000 entries with a 500 ms target. Creates the
    /// folder on the fly, measures `list_dir + sort`, prints the time, and
    /// checks the target. Ignored by default, since creating the files is slow:
    /// run with `cargo test -p favnyr-core bench_list_10k -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_list_10k() {
        let tmp = std::env::temp_dir().join(format!("favnyr-bench-{}", nano()));
        std::fs::create_dir_all(&tmp).unwrap();
        for i in 0..10_000 {
            std::fs::write(tmp.join(format!("file_{i}.txt")), b"").unwrap();
        }

        let t0 = std::time::Instant::now();
        let mut entries = list_dir(&tmp, false).unwrap();
        let listed = t0.elapsed();
        let t1 = std::time::Instant::now();
        sort(
            &mut entries,
            SortColumn::Name,
            SortOrder::Asc,
            GroupMode::FoldersFirst,
        );
        let sorted = t1.elapsed();
        let total = t0.elapsed();

        println!(
            "bench_list_10k: {} entries | list_dir={:?} sort={:?} total={:?}",
            entries.len(),
            listed,
            sorted,
            total
        );
        assert_eq!(entries.len(), 10_000);
        assert!(
            total.as_millis() < 500,
            "listing 10k should be < 500ms, measured: {total:?}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn make_entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(name),
            size_bytes: if is_dir { None } else { Some(0) },
            mtime_unix: None,
            is_dir,
            kind: if is_dir {
                FileKind::Folder
            } else {
                FileKind::File
            },
            hidden: false,
            is_symlink: false,
            executable: false,
        }
    }

    fn sized_entry(name: &str, size: u64) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(name),
            size_bytes: Some(size),
            mtime_unix: None,
            is_dir: false,
            kind: FileKind::File,
            hidden: false,
            is_symlink: false,
            executable: false,
        }
    }

    fn nano() -> u64 {
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
