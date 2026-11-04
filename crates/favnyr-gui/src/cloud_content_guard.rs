//! Limits automatic content reads to files whose contents are locally available.
//! Explicit user actions such as opening or copying a file are unaffected.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentStatus {
    Local,
    #[cfg(windows)]
    Remote,
    #[cfg(windows)]
    Unavailable,
}

#[cfg(not(windows))]
pub fn automatic_content_status(_path: &Path) -> ContentStatus {
    ContentStatus::Local
}

pub fn may_read_automatically(path: &Path) -> bool {
    automatic_content_status(path) == ContentStatus::Local
}

#[cfg(windows)]
pub fn automatic_content_status(path: &Path) -> ContentStatus {
    automatic_content_status_inner(path, 0)
}

#[cfg(windows)]
fn automatic_content_status_inner(path: &Path, link_depth: usize) -> ContentStatus {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindFirstFileW, WIN32_FIND_DATAW,
    };
    use windows::Win32::System::SystemServices::IO_REPARSE_TAG_SYMLINK;
    use windows::core::PCWSTR;

    if link_depth >= 8 {
        return ContentStatus::Unavailable;
    }
    // Directory enumeration returns attributes and the reparse tag without
    // opening the file's data stream. Do not use an on-disk-only search here:
    // remote placeholders still need to remain visible in the file list.
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut data = WIN32_FIND_DATAW::default();
    let Ok(handle) = (unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data) }) else {
        return ContentStatus::Unavailable;
    };
    let _ = unsafe { FindClose(handle) };
    let status = content_status(data.dwFileAttributes, data.dwReserved0);
    if status != ContentStatus::Local {
        return status;
    }
    if data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        && data.dwReserved0 == IO_REPARSE_TAG_SYMLINK
    {
        // The link itself can be local while its target is a remote placeholder.
        let Ok(target) = std::fs::read_link(path) else {
            return ContentStatus::Unavailable;
        };
        let target = if target.is_absolute() {
            target
        } else {
            path.parent().unwrap_or(Path::new(".")).join(target)
        };
        return automatic_content_status_inner(&target, link_depth + 1);
    }
    ContentStatus::Local
}

#[cfg(windows)]
fn content_status(attributes: u32, reparse_tag: u32) -> ContentStatus {
    use windows::Win32::Storage::CloudFilters::CfGetPlaceholderStateFromAttributeTag;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    if favnyr_core::fs::content_is_remote_attributes(attributes) {
        return ContentStatus::Remote;
    }

    if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
        return ContentStatus::Local;
    }
    let tag = reparse_tag;
    let state = unsafe { CfGetPlaceholderStateFromAttributeTag(attributes, tag) };
    placeholder_content_status(state, tag)
}

#[cfg(windows)]
fn placeholder_content_status(
    state: windows::Win32::Storage::CloudFilters::CF_PLACEHOLDER_STATE,
    tag: u32,
) -> ContentStatus {
    use windows::Win32::Storage::CloudFilters::{
        CF_PLACEHOLDER_STATE_INVALID, CF_PLACEHOLDER_STATE_NO_STATES, CF_PLACEHOLDER_STATE_PARTIAL,
        CF_PLACEHOLDER_STATE_PARTIALLY_ON_DISK,
    };
    use windows::Win32::System::SystemServices::{IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_CLOUD_MASK};

    let cloud_tag = tag != 0 && tag & !IO_REPARSE_TAG_CLOUD_MASK == IO_REPARSE_TAG_CLOUD;
    if state == CF_PLACEHOLDER_STATE_INVALID {
        return if cloud_tag {
            ContentStatus::Unavailable
        } else {
            ContentStatus::Local
        };
    }
    if state.contains(CF_PLACEHOLDER_STATE_PARTIAL)
        || state.contains(CF_PLACEHOLDER_STATE_PARTIALLY_ON_DISK)
    {
        return ContentStatus::Remote;
    }

    // A malformed or unrecognized cloud placeholder is not evidence that its
    // data is available. Ordinary reparse points, including local symlinks,
    // remain eligible when no recall attribute is present.
    if cloud_tag && state == CF_PLACEHOLDER_STATE_NO_STATES {
        return ContentStatus::Unavailable;
    }
    ContentStatus::Local
}

#[cfg(all(test, windows))]
fn content_is_local(attributes: u32, reparse_tag: u32) -> bool {
    content_status(attributes, reparse_tag) == ContentStatus::Local
}

#[cfg(all(test, windows))]
fn placeholder_content_is_local(
    state: windows::Win32::Storage::CloudFilters::CF_PLACEHOLDER_STATE,
    tag: u32,
) -> bool {
    placeholder_content_status(state, tag) == ContentStatus::Local
}

#[cfg(all(test, windows))]
mod tests {
    use super::{
        ContentStatus, content_is_local, content_status, may_read_automatically,
        placeholder_content_is_local, placeholder_content_status,
    };
    use std::path::Path;
    use windows::Win32::Storage::CloudFilters::{
        CF_PLACEHOLDER_STATE_INVALID, CF_PLACEHOLDER_STATE_PARTIAL,
        CF_PLACEHOLDER_STATE_PLACEHOLDER,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    use windows::Win32::System::SystemServices::{IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_SYMLINK};

    #[test]
    fn recall_attributes_prevent_automatic_content_reads() {
        for attribute in [
            FILE_ATTRIBUTE_OFFLINE,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        ] {
            assert!(!content_is_local(attribute.0, 0));
            assert_eq!(content_status(attribute.0, 0), ContentStatus::Remote);
        }
    }

    #[test]
    fn ordinary_files_and_local_symlinks_remain_eligible() {
        assert!(content_is_local(FILE_ATTRIBUTE_NORMAL.0, 0));
        assert!(content_is_local(
            FILE_ATTRIBUTE_REPARSE_POINT.0,
            IO_REPARSE_TAG_SYMLINK
        ));
    }

    #[test]
    fn cloud_placeholders_without_local_content_are_remote() {
        assert!(!content_is_local(
            FILE_ATTRIBUTE_REPARSE_POINT.0,
            IO_REPARSE_TAG_CLOUD
        ));
        assert_eq!(
            content_status(FILE_ATTRIBUTE_REPARSE_POINT.0, IO_REPARSE_TAG_CLOUD),
            ContentStatus::Remote
        );
        assert_eq!(
            placeholder_content_status(CF_PLACEHOLDER_STATE_INVALID, IO_REPARSE_TAG_CLOUD),
            ContentStatus::Unavailable
        );
    }

    #[test]
    fn partial_cloud_content_is_skipped_but_complete_content_is_eligible() {
        let complete = CF_PLACEHOLDER_STATE_PLACEHOLDER;
        assert!(placeholder_content_is_local(complete, IO_REPARSE_TAG_CLOUD));
        assert!(!placeholder_content_is_local(
            complete | CF_PLACEHOLDER_STATE_PARTIAL,
            IO_REPARSE_TAG_CLOUD
        ));
        assert_eq!(
            placeholder_content_status(
                complete | CF_PLACEHOLDER_STATE_PARTIAL,
                IO_REPARSE_TAG_CLOUD
            ),
            ContentStatus::Remote
        );
    }

    #[test]
    fn local_source_file_is_available_for_automatic_reads() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cloud_content_guard.rs");
        assert!(may_read_automatically(&source));
    }
}
