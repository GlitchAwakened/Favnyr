//! Mounting a volume the machine sees but has not mounted — **Linux**.
//!
//! The counterpart of [`crate::eject`], and it goes through the same tool:
//! `udisksctl`, shipped with udisks2 on any desktop. No dependency is added.
//!
//! **Favnyr never handles a password.** Mounting is a question of permission,
//! not of secret: the daemon asks polkit whether this session may proceed, and
//! polkit runs its own agent — a separate process belonging to the desktop —
//! when it needs to authenticate. The passphrase travels from that agent to
//! polkit and never enters this one. All this call sees is "mounted" or
//! "refused". A removable volume usually needs no authentication at all; a
//! system-internal one usually does.
//!
//! The call BLOCKS for as long as an agent keeps its prompt open, so it must
//! never run on the UI thread.
//!
//! Like [`crate::eject`], failures are **data only**, with no natural-language
//! text: the GUI renders each variant in the user's language.

use std::path::PathBuf;

/// Data-only failure reason for [`mount`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountError {
    /// Empty or otherwise unusable device identifier.
    UnknownDevice,
    /// `udisksctl` is not installed — no udisks2 on this machine.
    ToolNotFound,
    /// Permission refused, or no agent answered the authentication request.
    NotAuthorized,
    /// Someone mounted it in the meantime: the listing was simply stale.
    AlreadyMounted,
    /// The tool ran and failed for another reason. `detail` is its own stderr
    /// when available (arbitrary text, outside Favnyr's control).
    ToolFailed { detail: Option<String> },
    /// The tool reported success but named no mount point — nothing to open.
    NoMountPoint,
    /// No implementation for this OS.
    PlatformUnsupported,
}

/// Mounts `device` (a `Place::device` value, e.g. `/dev/sdb1`) and returns
/// where it landed. Blocking; see the module notes.
pub fn mount(device: &str) -> Result<PathBuf, MountError> {
    #[cfg(target_os = "linux")]
    {
        imp::mount(device)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        Err(MountError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{MountError, PathBuf};
    use std::process::Command;

    pub fn mount(device: &str) -> Result<MountPoint, MountError> {
        if device.trim().is_empty() {
            return Err(MountError::UnknownDevice);
        }
        let output = Command::new("udisksctl")
            // The tool speaks the session's language; a fixed locale keeps its
            // output parsable. The failure cause is read from the D-Bus error
            // name rather than the prose anyway, which is sturdier still.
            .env("LC_ALL", "C")
            .args(["mount", "-b", device])
            .output()
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => MountError::ToolNotFound,
                _ => MountError::ToolFailed { detail: None },
            })?;
        if !output.status.success() {
            return Err(classify_failure(&String::from_utf8_lossy(&output.stderr)));
        }
        parse_mount_point(&String::from_utf8_lossy(&output.stdout)).ok_or(MountError::NoMountPoint)
    }

    type MountPoint = PathBuf;

    /// Reads the mount point out of a success line, which reads
    /// `Mounted /dev/… at /some/path` — with or without a trailing period
    /// depending on the version.
    pub(super) fn parse_mount_point(stdout: &str) -> Option<PathBuf> {
        let line = stdout.lines().find(|line| line.contains(" at "))?;
        // Split at the FIRST separator: the device name comes before it and
        // never contains a space, whereas the mount point that follows may
        // well contain " at " inside a folder name.
        let path = line.split_once(" at ")?.1.trim().trim_end_matches('.');
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    /// Sorts a failure by the D-Bus error name the daemon reports. Those names
    /// are stable and language-independent, unlike the sentence beside them.
    pub(super) fn classify_failure(stderr: &str) -> MountError {
        if stderr.contains("NotAuthorized") {
            return MountError::NotAuthorized;
        }
        if stderr.contains("AlreadyMounted") {
            return MountError::AlreadyMounted;
        }
        let detail = stderr.trim();
        MountError::ToolFailed {
            detail: (!detail.is_empty()).then(|| detail.to_string()),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::MountError;
    use super::imp::{classify_failure, parse_mount_point};
    use std::path::PathBuf;

    #[test]
    fn the_mount_point_is_read_from_the_success_line() {
        assert_eq!(
            parse_mount_point("Mounted /dev/sda1 at /run/media/user/Volume01.\n"),
            Some(PathBuf::from("/run/media/user/Volume01"))
        );
    }

    #[test]
    fn a_success_line_without_a_trailing_period_reads_the_same() {
        assert_eq!(
            parse_mount_point("Mounted /dev/sda1 at /run/media/user/Volume01\n"),
            Some(PathBuf::from("/run/media/user/Volume01"))
        );
    }

    #[test]
    fn a_mount_point_containing_the_separator_keeps_its_whole_path() {
        // The separator also appears inside the folder name, so only cutting
        // at the FIRST one keeps the whole path.
        assert_eq!(
            parse_mount_point("Mounted /dev/sda1 at /run/media/user/look at me"),
            Some(PathBuf::from("/run/media/user/look at me"))
        );
    }

    #[test]
    fn an_unexpected_success_output_names_no_mount_point() {
        assert_eq!(parse_mount_point("something else entirely"), None);
        assert_eq!(parse_mount_point(""), None);
    }

    #[test]
    fn a_refused_authorisation_is_recognised_by_its_error_name() {
        assert_eq!(
            classify_failure(
                "Error mounting /dev/sda1: GDBus.Error:\
                 org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: \
                 Not authorized to perform operation"
            ),
            MountError::NotAuthorized
        );
    }

    #[test]
    fn a_volume_mounted_meanwhile_is_not_reported_as_a_failure_cause() {
        assert_eq!(
            classify_failure(
                "Error mounting /dev/sda1: GDBus.Error:\
                 org.freedesktop.UDisks2.Error.AlreadyMounted: \
                 Device is already mounted"
            ),
            MountError::AlreadyMounted
        );
    }

    #[test]
    fn any_other_failure_carries_the_tools_own_words() {
        assert_eq!(
            classify_failure("  wire fell out  "),
            MountError::ToolFailed {
                detail: Some("wire fell out".to_string())
            }
        );
        assert_eq!(
            classify_failure("   "),
            MountError::ToolFailed { detail: None }
        );
    }
}
