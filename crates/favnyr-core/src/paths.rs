//! Locating Favnyr's data, config, and cache folders — **cross-platform**
//! via the `dirs` crate:
//!   - **Linux**: XDG spec (`$XDG_DATA_HOME` → `~/.local/share`,
//!     `$XDG_CONFIG_HOME` → `~/.config`, `$XDG_CACHE_HOME` → `~/.cache`).
//!   - **Windows**: `%APPDATA%` (config + data, *Roaming*) and `%LOCALAPPDATA%`
//!     (cache).
//!
//! `dirs::*_dir()` returns `None` only if no base can be found
//! (e.g. `HOME`/`%APPDATA%` missing); we then fall back to the temp folder
//! so we never panic.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Name of the application subdirectory under each base.
const APP: &str = "favnyr";

/// Joins `APP` to the given base, falling back to the temp folder if the
/// base can't be found (degenerate case).
fn app_dir(base: Option<PathBuf>) -> PathBuf {
    base.unwrap_or_else(std::env::temp_dir).join(APP)
}

/// Data: Linux `$XDG_DATA_HOME/favnyr` (default `~/.local/share/favnyr`);
/// Windows `%APPDATA%\favnyr`.
pub fn data_dir() -> PathBuf {
    app_dir(dirs::data_dir())
}

/// Config: Linux `$XDG_CONFIG_HOME/favnyr` (default `~/.config/favnyr`);
/// Windows `%APPDATA%\favnyr`.
pub fn config_dir() -> PathBuf {
    app_dir(dirs::config_dir())
}

/// Cache: Linux `$XDG_CACHE_HOME/favnyr` (default `~/.cache/favnyr`);
/// Windows `%LOCALAPPDATA%\favnyr`.
pub fn cache_dir() -> PathBuf {
    app_dir(dirs::cache_dir())
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn workspace_path() -> PathBuf {
    data_dir().join("workspace.toml")
}

/// Folder for **named** workspaces (one `.toml` file per workspace).
/// Distinct from the automatic `workspace.toml` (current session state).
pub fn workspaces_dir() -> PathBuf {
    data_dir().join("workspaces")
}

/// SINGLE file for the favorites tree (shared/common storage).
pub fn favorites_path() -> PathBuf {
    data_dir().join("favorites.toml")
}

/// SINGLE file for per-path annotations (folder colour, note). Data rather
/// than config: it is keyed by path and grows with use, like favorites.toml.
pub fn annotations_path() -> PathBuf {
    data_dir().join("annotations.toml")
}

/// File for "openers" (Open with: programs + custom commands). Data,
/// not config: a growing collection (cf. favorites.toml).
pub fn openers_path() -> PathBuf {
    data_dir().join("openers.toml")
}

pub fn log_path() -> PathBuf {
    cache_dir().join("favnyr.log")
}

/// Creates all the XDG directories if they don't already exist.
pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(config_dir())?;
    std::fs::create_dir_all(cache_dir())?;
    Ok(())
}

/// Writes `contents` so the file is never observed half-written: the bytes go
/// to a sibling temporary, reach the device, then take the target's place in a
/// single rename. A power cut, a full disk, a drive pulled out mid-save, or a
/// second instance writing the same file therefore leaves either the previous
/// content or the new one — never a truncated file.
///
/// That guarantee is what the stores rest on: a file they cannot parse comes
/// back as an EMPTY store, and the next save would write that emptiness over
/// the only copy of the user's favourites, notes or programs.
///
/// The temporary is a SIBLING on purpose: a rename is only atomic within one
/// filesystem, and a temporary in the system temp folder — often another
/// volume — would quietly degrade into a copy.
///
/// A symbolic link at `path` is followed, so a settings file kept elsewhere and
/// linked into place keeps receiving its updates. A HARD link cannot be honoured
/// the same way: replacing the name is what makes the write atomic, and that
/// necessarily separates the two names. Nothing is lost when it happens — the
/// other name keeps the previous content — but the two stop tracking each other.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    // A settings file is often a LINK into a dotfiles repository or a shared
    // folder. Writing used to follow that link and update what it points at; a
    // rename would instead take the link's place with a plain file, and the
    // real one would quietly stop being fed. The link is therefore resolved
    // first, so the replacement happens at the other end of it — where the
    // previous behaviour wrote.
    let resolved = match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => std::fs::canonicalize(path).ok(),
        _ => None,
    };
    let path = resolved.as_deref().unwrap_or(path);

    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(dir)?;

    // The process id keeps two instances from choosing the same temporary.
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(".{}.tmp", std::process::id()));
    let tmp = dir.join(name);

    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        // Pushing the bytes to the device before the rename publishes them is
        // what makes the result survive a power cut. Best effort, though: some
        // network filesystems refuse the flush outright, and failing the whole
        // save there would be the worse trade — the content is written either
        // way, and the rename that follows is still all-or-nothing.
        if let Err(err) = file.sync_all() {
            tracing::debug!(error = %err, path = %tmp.display(), "flush before rename failed");
        }
        Ok(())
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }

    // `rename` replaces an existing file on both platforms.
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// Moves a file that cannot be parsed out of the way, keeping it under a
/// `.corrupt` sibling.
///
/// A store that fails to load starts empty so the application still opens —
/// but the next save would then write that emptiness over the user's only
/// copy. Setting the file aside first turns a silent, permanent loss into
/// something recoverable by hand, and leaves a line in the log saying so.
pub fn preserve_unreadable(path: &Path) {
    let mut name = std::ffi::OsString::from(path.file_name().unwrap_or_default());
    name.push(".corrupt");
    let kept = path.with_file_name(name);
    match std::fs::rename(path, &kept) {
        Ok(()) => tracing::warn!(
            path = %path.display(),
            kept = %kept.display(),
            "unreadable file kept aside; starting from an empty one"
        ),
        Err(err) => tracing::warn!(
            error = %err,
            path = %path.display(),
            "unreadable file could not be kept aside"
        ),
    }
}

#[cfg(test)]
mod tests {

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "favnyr-paths-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_replaces_the_content_and_leaves_no_temporary() {
        let dir = scratch("write");
        let file = dir.join("store.toml");
        write_atomic(
            &file,
            "first = 1
",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "first = 1
"
        );

        // A second, SHORTER write must not leave any tail of the first: the
        // rename replaces the file rather than overwriting it in place.
        write_atomic(
            &file, "b = 2
",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "b = 2
"
        );

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "store.toml")
            .collect();
        assert!(leftovers.is_empty(), "temporary left behind: {leftovers:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_atomic_creates_the_folder_it_writes_into() {
        let dir = scratch("mkdir");
        let file = dir.join("deeper").join("store.toml");
        write_atomic(
            &file, "a = 1
",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "a = 1
"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Keeping a settings file elsewhere and linking it into place is ordinary
    /// on Unix (dotfile managers do exactly this). Replacing the link with a
    /// plain file would leave the real one frozen on its last content, with
    /// nothing to show for it.
    #[cfg(unix)]
    #[test]
    fn write_atomic_writes_through_a_symlink_instead_of_replacing_it() {
        let dir = scratch("symlink");
        let real = dir.join("elsewhere.toml");
        let link = dir.join("config.toml");
        std::fs::write(
            &real, "old = 1
",
        )
        .unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_atomic(
            &link, "new = 2
",
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "new = 2
",
            "the file the link points at is the one updated"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link is still a link"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preserve_unreadable_keeps_the_file_instead_of_losing_it() {
        let dir = scratch("preserve");
        let file = dir.join("favorites.toml");
        std::fs::write(&file, "not valid toml {{{").unwrap();

        preserve_unreadable(&file);

        assert!(!file.exists(), "the unreadable file moved out of the way");
        let kept = dir.join("favorites.toml.corrupt");
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            "not valid toml {{{",
            "its content is still there, byte for byte"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
    use super::*;

    #[test]
    fn paths_end_in_app_subdir() {
        assert!(data_dir().ends_with(APP));
        assert!(config_dir().ends_with(APP));
        assert!(cache_dir().ends_with(APP));
        assert!(config_path().ends_with("config.toml"));
        assert!(workspace_path().ends_with("workspace.toml"));
    }
}
