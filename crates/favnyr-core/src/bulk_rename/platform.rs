use std::io;
use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pub directory: bool,
    link: bool,
    size: u64,
    modified: Option<SystemTime>,
    object: (u64, u64),
}

pub(super) fn identity(path: &Path) -> io::Result<Identity> {
    let metadata = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    let object = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let object = windows_identity(path)?;
    #[cfg(not(any(unix, windows)))]
    let object = (0, 0);
    Ok(Identity {
        directory: metadata.is_dir(),
        link: metadata.is_symlink(),
        size: if metadata.is_dir() { 0 } else { metadata.len() },
        modified: if metadata.is_dir() {
            None
        } else {
            metadata.modified().ok()
        },
        // A rename can inherit a previously cached creation time on NTFS.
        // Volume and file identifiers track ownership independently of that date.
        object,
    })
}

#[cfg(windows)]
fn windows_identity(path: &Path) -> io::Result<(u64, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    #[repr(C)]
    #[derive(Default)]
    struct Info {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    // Query the entry itself, including directories and links, without content access.
    let file = std::fs::OpenOptions::new()
        .access_mode(0x80)
        .share_mode(7)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)?;
    let mut info = Info::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        u64::from(info.volume),
        (u64::from(info.index_high) << 32) | u64::from(info.index_low),
    ))
}

/// The existence check and rename are one OS operation; never fall back to overwrite.
#[cfg(windows)]
pub(super) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        let mut text: Vec<u16> = path.as_os_str().encode_wide().collect();
        if text.contains(&0) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        text.push(0);
        Ok(text)
    };
    let from = wide(from)?;
    let to = wide(to)?;
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renameat2(
            old_dir: i32,
            old: *const std::ffi::c_char,
            new_dir: i32,
            new: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::ErrorKind::InvalidInput)?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::ErrorKind::InvalidInput)?;
    // AT_FDCWD and RENAME_NOREPLACE. Unsupported filesystems fail safely.
    if unsafe { renameat2(-100, from.as_ptr(), -100, to.as_ptr(), 1) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn rename(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
