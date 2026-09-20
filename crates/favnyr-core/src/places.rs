//! Locations for the "Places" sidebar — **cross-platform**.
//!
//! Three families:
//!   - **Shortcuts**: standard user folders (`dirs`) — Home, Desktop,
//!     Documents, Downloads, Pictures, Music, Videos.
//!   - **Drives**: mounted volumes (`sysinfo`) — `C:`/`D:`/USB on Windows;
//!     `/`, `/home`, `/run/media/$USER/*`… on Linux (pseudo-mounts filtered out).
//!   - **Trash**: a single location. On Linux it's *navigable*
//!     (`~/.local/share/Trash/files`); on Windows it's virtual → the GUI
//!     opens it in Explorer (the GUI decides based on `kind`).
//!
//! Display (i18n labels for fixed entries, icons) is decided on the GUI side
//! from the `kind`; the core only returns a raw `name` (mostly useful for
//! drives).

use std::path::{Path, PathBuf};

/// Category of a location — drives the icon and label on the GUI side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    /// Home folder (`$HOME` / `%USERPROFILE%`).
    Home,
    /// Standard user folder (Documents, Downloads…).
    Folder,
    /// Local mounted volume (drive, partition, removable device).
    Drive,
    /// Local volume the machine can see but has not mounted. It carries no
    /// path yet — only the device that would be mounted.
    Volume,
    /// Encrypted volume, still locked. It holds no mountable filesystem until
    /// it is unlocked, and unlocking is not something Favnyr offers: that
    /// would mean holding a passphrase.
    LockedVolume,
    /// Network location: mapped drive (`Z:` → `\\srv\part`), NFS/CIFS/SSHFS/GVFS
    /// mount (Linux), or WSL distribution (`\\wsl$\…`, Windows).
    Network,
    /// Trash.
    Trash,
}

/// A location listable in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub kind: PlaceKind,
    /// Target path (empty for the Windows trash, which has no real path).
    pub path: PathBuf,
    /// Raw displayable name (volume name / folder name). The GUI may
    /// replace it with an i18n label for `Home`/`Trash`.
    pub name: String,
    /// Removable device (USB, CD…) → offers to release it.
    pub removable: bool,
    /// Can the device be physically unplugged while the machine runs? Decides
    /// whether releasing it is announced as "you may unplug it" or merely as
    /// "released" — see [`is_hotplug_device`].
    pub hotplug: bool,
    /// Eject/disconnect target: `/dev/sdX1` (Linux), drive letter `X:`
    /// (local or mapped-network Windows drive), otherwise empty (WSL, GVFS…).
    pub device: String,
    /// Capacity of the volume in bytes, `0` when it could not be read.
    ///
    /// Zero is the "unknown" marker rather than an `Option`: an empty card
    /// reader, an optical drive with no disc and a share that never answered
    /// all mean the same thing to the interface — show no capacity — and a
    /// volume of genuinely zero bytes does not exist.
    pub total_bytes: u64,
    /// Bytes still writable **by this user**, `0` when unknown.
    ///
    /// Deliberately the user-facing figure, not the raw free count: a
    /// filesystem keeps a reserve (ext4 holds back 5% for root) and a share may
    /// impose a quota. The badge answers "can I still write here", so it must
    /// report what the caller could actually use.
    pub free_bytes: u64,
}

impl Place {
    /// Constructor for a simple location (not removable, no device) —
    /// shortcuts, trash, root.
    fn simple(kind: PlaceKind, path: PathBuf, name: String) -> Self {
        Place {
            kind,
            path,
            name,
            removable: false,
            hotplug: false,
            device: String::new(),
            // A shortcut and the trash sit on a volume that is listed on its
            // own line: repeating its capacity here would state the same fact
            // twice under two names, which reads as a contradiction rather
            // than as agreement.
            total_bytes: 0,
            free_bytes: 0,
        }
    }
}

/// Shortcuts to standard user folders (the ones that exist).
pub fn user_places() -> Vec<Place> {
    let mut out = Vec::new();
    let mut push = |dir: Option<PathBuf>, kind: PlaceKind| {
        if let Some(p) = dir
            && p.is_dir()
        {
            let name = file_name_of(&p);
            out.push(Place::simple(kind, p, name));
        }
    };
    push(dirs::home_dir(), PlaceKind::Home);
    push(dirs::desktop_dir(), PlaceKind::Folder);
    push(dirs::document_dir(), PlaceKind::Folder);
    push(dirs::download_dir(), PlaceKind::Folder);
    push(dirs::picture_dir(), PlaceKind::Folder);
    push(dirs::audio_dir(), PlaceKind::Folder);
    push(dirs::video_dir(), PlaceKind::Folder);
    out
}

/// LIGHTWEIGHT "signature" of the set of drives/mounts, to cheaply detect an
/// external change (subst, `net use`, USB, mount…) and refresh the sidebar
/// without constantly re-scanning.
///   - **Windows**: bitmask of mounted letters (`GetLogicalDrives`,
///     instantaneous) → catches a drive appearing/disappearing.
///   - **Linux**: hash of `/proc/self/mountinfo` (mounts) + of the GVFS
///     folder (user network shares) → catches (un)mounts.
///
/// Two calls returning the SAME value ⇒ nothing has changed.
pub fn drives_signature() -> u64 {
    #[cfg(windows)]
    {
        windrives::logical_drives_mask() as u64
    }
    #[cfg(target_os = "linux")]
    {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Ok(s) = std::fs::read_to_string("/proc/self/mountinfo") {
            s.hash(&mut h);
        }
        // GVFS (user mounts, outside mountinfo): entry names.
        if let Ok(rd) = std::fs::read_dir(gvfs_root()) {
            let mut entries: Vec<String> = rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            entries.sort();
            entries.hash(&mut h);
        }
        h.finish()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        0
    }
}

/// Mounted volumes "visible to the user": LOCAL drives
/// (`PlaceKind::Drive`) **and** network locations (`PlaceKind::Network`) —
/// mapped drives, NFS/CIFS/SSHFS/GVFS mounts, WSL distributions. The
/// GUI splits the two families into "Drives" / "Network" sections.
pub fn drives() -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();

    // ----- Linux (and other non-Windows): mounts via `sysinfo` -----
    // Windows does NOT use `sysinfo` here: `GetLogicalDrives` (below) already
    // sees ALL letters — including mapped network, SUBST, VHD — and properly
    // computes type/removable/device. Avoiding the double pass removes any
    // ambiguity on `device`/`removable`.
    #[cfg(not(windows))]
    {
        use sysinfo::Disks;
        let disks = Disks::new_with_refreshed_list();
        for d in disks.list() {
            let mount = d.mount_point().to_path_buf();
            let fs = d.file_system().to_string_lossy().to_string();
            let is_net = is_network_fs(&fs);
            // A mount is kept if it's network (regardless of its path) OR
            // if it passes the "user-facing" filter for local mounts.
            if !is_net && !is_user_facing_mount(&mount, &fs) {
                continue;
            }
            // Deduplicate by mount point (sysinfo can list duplicates).
            if out.iter().any(|p| p.path == mount) {
                continue;
            }
            let name = drive_label(&mount, &d.name().to_string_lossy());
            out.push(Place {
                kind: if is_net {
                    PlaceKind::Network
                } else {
                    PlaceKind::Drive
                },
                path: mount.clone(),
                name,
                // Removable → ejectable. udisks heuristic: mount under
                // /run/media|/media. (Network is never "removable".)
                removable: !is_net && is_removable_mount(&mount),
                // Whether the cable can be pulled — a different question from
                // the one above, and the one the release message answers.
                hotplug: !is_net && is_hotplug_device(&d.name().to_string_lossy()),
                // Eject device: `/dev/sdX1` (= `Disk::name()` on Linux).
                device: if is_net {
                    String::new()
                } else {
                    d.name().to_string_lossy().to_string()
                },
                // Free of charge: the listing above already refreshed these,
                // so the `statvfs` behind them is a cost this scan pays
                // whether or not anything reads the result. Shares are left
                // unmeasured all the same — the figure would describe the
                // server, not what this user may write.
                total_bytes: if is_net { 0 } else { d.total_space() },
                free_bytes: if is_net { 0 } else { d.available_space() },
            });
        }
    }

    // ----- Windows: all letters via the Win32 API -----
    #[cfg(windows)]
    for p in windrives::logical_drives() {
        if !out.iter().any(|e| e.path == p.path) {
            out.push(p);
        }
    }

    // Additional network locations WITHOUT a classic mount point:
    // GVFS (Linux) and WSL distributions (Windows).
    out.extend(network_extra());
    // Stable sort: first by family (local before network), then by path —
    // keeps sections grouped regardless of discovery order.
    out.sort_by(|a, b| {
        (a.kind == PlaceKind::Network)
            .cmp(&(b.kind == PlaceKind::Network))
            .then_with(|| a.path.cmp(&b.path))
    });
    // Guarantees the root "/" comes first: some systems (atomic / overlay /
    // composefs) don't report it via `sysinfo` → without this the Drives
    // section could be empty. (Windows always lists its letters.)
    //
    // It carries NO capacity, deliberately. On such a system the root is a
    // small read-only overlay — measured at 45 MB, entirely full — so a gauge
    // there would report the size of the overlay rather than of any storage
    // the user can write to. A figure that is technically exact and
    // practically meaningless is worse than none: the volumes that do hold the
    // user's data are listed on their own lines, with their own gauges.
    #[cfg(not(windows))]
    if !out.iter().any(|p| p.path == Path::new("/")) {
        out.insert(
            0,
            Place::simple(PlaceKind::Drive, PathBuf::from("/"), String::new()),
        );
    }
    out
}

/// Does the path live on a NETWORK filesystem? Used to spare the network
/// expensive convenience work (e.g. recursive folder mtime, which walks the
/// whole tree — Explorer does nothing like that on a share).
/// Detection WITHOUT touching the network:
///   - **Windows**: UNC path, or `DRIVE_REMOTE` mapped letter
///     (`GetDriveTypeW` reads the LOCAL mount table — instantaneous);
///   - **Linux**: network fstype (`is_network_fs`) of the path's most
///     specific mount point (`/proc/self/mountinfo`, local read).
pub fn is_network_path(path: &Path) -> bool {
    if crate::fs::is_unc_path(path) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(p)) = path.components().next()
            && let Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) = p.kind()
        {
            return windrives::drive_is_remote(letter as char);
        }
        false
    }
    #[cfg(target_os = "linux")]
    {
        // Mount point with the longest prefix → its fstype.
        let Ok(info) = std::fs::read_to_string("/proc/self/mountinfo") else {
            return false;
        };
        let mut best: Option<(usize, bool)> = None; // (mount point length, network?)
        for line in info.lines() {
            // Format: … field[4] = mount point … "-" fstype source options
            let mut parts = line.split(' ');
            let Some(mount) = parts.nth(4) else { continue };
            let Some(fs) = line.split(" - ").nth(1).and_then(|r| r.split(' ').next()) else {
                continue;
            };
            if path.starts_with(mount) && best.is_none_or(|(l, _)| mount.len() > l) {
                best = Some((mount.len(), is_network_fs(fs)));
            }
        }
        best.map(|(_, net)| net).unwrap_or(false)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Result of enumerating a network server's shares.
pub enum NetShares {
    /// Visible shares (possibly empty).
    Ok(Vec<String>),
    /// Access denied / failed logon → offer a system login.
    AuthNeeded,
    /// Server unreachable (offline, invalid host, firewall…).
    Unreachable,
}

/// VISIBLE SMB shares of a server (`server` = host name without `\\`). Allows
/// "browsing" a `\\HOST` machine (like Explorer), where `std::fs` can only
/// list a single share. Windows only (`Unreachable` elsewhere).
pub fn net_shares(server: &str) -> NetShares {
    #[cfg(windows)]
    {
        windrives::shares_of(server)
    }
    #[cfg(not(windows))]
    {
        let _ = server;
        NetShares::Unreachable
    }
}

/// Establishes an authenticated network connection to `resource` (`\\HOST\share`
/// or `\\HOST\IPC$`), showing the **Windows credentials dialog** if needed.
/// `true` if connected (or already). Windows only. Blocking (modal).
pub fn net_connect_prompt(resource: &str) -> bool {
    #[cfg(windows)]
    {
        windrives::connect_prompt(resource)
    }
    #[cfg(not(windows))]
    {
        let _ = resource;
        false
    }
}

/// Buses whose devices can be unplugged while the machine is running.
#[cfg(target_os = "linux")]
const HOTPLUG_BUSES: &[&str] = &["/usb", "/mmc", "/firewire", "/pcmcia"];

/// Can this device be physically unplugged?
///
/// NOT "is it a stick rather than a disk". Measured on real hardware, the
/// rotational flag classifies the opposite way — an internal SSD reports 0
/// while a USB drive can report 1 — and the removable-media flag also marks an
/// optical drive. What a message needs to answer is whether the user may pull
/// the cable, and that is a property of the BUS, not of the storage medium.
///
/// This is the criterion the system itself applies: udisks separates
/// `power-off-drive` from `power-off-drive-system` on the same basis, so
/// following it keeps Favnyr and the system from contradicting each other.
///
/// Read from sysfs, where a device's real path traverses its bus: a plain
/// symlink resolution, no subprocess and no dependency. `false` on any other
/// platform, and whenever the device cannot be resolved.
pub fn is_hotplug_device(device: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Some(name) = Path::new(device).file_name() else {
            return false;
        };
        let Ok(resolved) = std::fs::canonicalize(Path::new("/sys/class/block").join(name)) else {
            return false;
        };
        path_traverses_hotplug_bus(&resolved.to_string_lossy())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        false
    }
}

/// Rule behind [`is_hotplug_device`], isolated from the filesystem.
#[cfg(target_os = "linux")]
fn path_traverses_hotplug_bus(resolved: &str) -> bool {
    HOTPLUG_BUSES.iter().any(|bus| resolved.contains(bus))
}

/// Cheap fingerprint of the block-device topology: the names the kernel
/// publishes under `/sys/class/block`.
///
/// Complementary to [`drives_signature`], which follows MOUNTS. Plugging a disk
/// in changes this one and not that one, and a volume that never gets mounted
/// would otherwise announce itself nowhere. It is a directory listing and
/// nothing else, so it costs microseconds and can be polled freely.
pub fn block_signature() -> u64 {
    #[cfg(target_os = "linux")]
    {
        use std::hash::{Hash, Hasher};
        let Ok(entries) = std::fs::read_dir("/sys/class/block") else {
            return 0;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        names.hash(&mut hasher);
        hasher.finish()
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// Volumes the machine can see but has NOT mounted.
///
/// [`drives()`] reports mounted filesystems, which is why a disk stays
/// invisible until something else mounts it. This answers the other question —
/// what *could* be mounted — and is deliberately kept out of `drives()`: it
/// costs a subprocess, so the caller decides when to pay, and
/// [`block_signature`] tells it when the answer could have changed.
///
/// Empty on any failure: a machine without the tool keeps exactly the
/// behaviour it had before.
pub fn unmounted_volumes() -> Vec<Place> {
    #[cfg(target_os = "linux")]
    {
        let Some(text) = block_inventory() else {
            return Vec::new();
        };
        let rows = parse_block_rows(&text);
        let mut out: Vec<Place> = rows
            .iter()
            .filter_map(|row| volume_kind(row, &rows).map(|kind| volume_place(row, kind)))
            .collect();
        // Kernel order is not guaranteed; the device name keeps the sidebar
        // stable between two scans.
        out.sort_by(|a, b| a.device.cmp(&b.device));
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Network locations that don't show up as a "disk" mount point:
/// GVFS shares (Linux) and WSL distributions (Windows).
fn network_extra() -> Vec<Place> {
    #[cfg(target_os = "linux")]
    {
        gvfs_mounts()
    }
    #[cfg(windows)]
    {
        windrives::wsl_distros()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

/// Enumeration of Windows drives via the Win32 API (complement to `sysinfo`).
///
/// `GetLogicalDrives` returns a bitmask of mounted letters (bit 0 = A:
/// … bit 25 = Z:) — it also sees **mapped network** drives, **SUBST** and
/// **mounted VHD** that `sysinfo` ignores. Each letter is enriched with its
/// type (`GetDriveTypeW`) and its volume label (`GetVolumeInformationW`).
///
/// A direct FFI to `kernel32`, linked by default under `windows-msvc`, avoids
/// an extra dependency.
#[cfg(windows)]
mod windrives {
    use super::{Place, PlaceKind};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(lp_root_path_name: *const u16) -> u32;
        /// Capacity of a volume. The FIRST output is what the *caller* may
        /// still write, which a quota can make smaller than the volume's own
        /// free count — that is the figure a user cares about.
        fn GetDiskFreeSpaceExW(
            lp_directory_name: *const u16,
            lp_free_bytes_available_to_caller: *mut u64,
            lp_total_number_of_bytes: *mut u64,
            lp_total_number_of_free_bytes: *mut u64,
        ) -> i32;
        /// Suppresses the modal the system would otherwise raise on a drive
        /// with no media — the "There is no disk in the drive" box, which a
        /// background probe must never be able to trigger.
        fn SetThreadErrorMode(new_mode: u32, old_mode: *mut u32) -> i32;
        fn GetVolumeInformationW(
            lp_root_path_name: *const u16,
            lp_volume_name_buffer: *mut u16,
            n_volume_name_size: u32,
            lp_volume_serial_number: *mut u32,
            lp_maximum_component_length: *mut u32,
            lp_file_system_flags: *mut u32,
            lp_file_system_name_buffer: *mut u16,
            n_file_system_name_size: u32,
        ) -> i32;
        // Querying a volume's bus (USB? → ejectable), see `bus_ejectable`.
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sa: *mut core::ffi::c_void,
            disp: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        fn DeviceIoControl(
            h: isize,
            ctl: u32,
            inbuf: *mut core::ffi::c_void,
            insz: u32,
            outbuf: *mut core::ffi::c_void,
            outsz: u32,
            ret: *mut u32,
            ov: *mut core::ffi::c_void,
        ) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const INVALID_HANDLE: isize = -1;
    const FILE_SHARE_RW: u32 = 0x0000_0003;
    const OPEN_EXISTING: u32 = 3;
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002d_1400;

    // Target of a mapped network drive (`Z:` → `\\server\share`) + enumeration
    // of a `\\HOST` server's SMB shares. `mpr.dll` is a system DLL, which
    // avoids an extra crate.
    #[link(name = "mpr")]
    unsafe extern "system" {
        fn WNetGetConnectionW(local: *const u16, remote: *mut u16, len: *mut u32) -> u32;
        fn WNetOpenEnumW(
            scope: u32,
            ty: u32,
            usage: u32,
            netres: *mut NetResourceW,
            handle: *mut isize,
        ) -> u32;
        fn WNetEnumResourceW(
            handle: isize,
            count: *mut u32,
            buf: *mut core::ffi::c_void,
            bufsize: *mut u32,
        ) -> u32;
        fn WNetCloseEnum(handle: isize) -> u32;
        // Authenticated connection with system PROMPT (network login).
        fn WNetAddConnection2W(
            netres: *mut NetResourceW,
            password: *const u16,
            user: *const u16,
            flags: u32,
        ) -> u32;
    }
    // Win32 error codes "authentication required" vs "unreachable".
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_ALREADY_ASSIGNED: u32 = 85;
    const ERROR_INVALID_PASSWORD: u32 = 86;
    const ERROR_SESSION_CREDENTIAL_CONFLICT: u32 = 1219;
    const ERROR_LOGON_FAILURE: u32 = 1326;
    const CONNECT_INTERACTIVE: u32 = 0x0000_0008;
    const CONNECT_PROMPT: u32 = 0x0000_0010;

    fn is_auth_error(rc: u32) -> bool {
        matches!(
            rc,
            ERROR_ACCESS_DENIED
                | ERROR_INVALID_PASSWORD
                | ERROR_SESSION_CREDENTIAL_CONFLICT
                | ERROR_LOGON_FAILURE
        )
    }

    /// Establishes an authenticated connection to `resource` (`\\HOST\share` or
    /// `\\HOST\IPC$`), showing the **Windows credentials dialog** if
    /// necessary (`CONNECT_PROMPT`). `true` if connected (or already connected),
    /// `false` if cancelled / failed. Blocking (modal dialog).
    pub fn connect_prompt(resource: &str) -> bool {
        let mut remote_w = wide(resource);
        let mut nr = NetResourceW {
            dw_scope: 0,
            dw_type: RESOURCETYPE_DISK,
            dw_display_type: 0,
            dw_usage: 0,
            lp_local_name: std::ptr::null_mut(),
            lp_remote_name: remote_w.as_mut_ptr(),
            lp_comment: std::ptr::null_mut(),
            lp_provider: std::ptr::null_mut(),
        };
        let rc = unsafe {
            WNetAddConnection2W(
                &mut nr,
                std::ptr::null(),
                std::ptr::null(),
                CONNECT_INTERACTIVE | CONNECT_PROMPT,
            )
        };
        matches!(rc, NO_ERROR | ERROR_ALREADY_ASSIGNED)
    }
    // `NETRESOURCEW` (Win32): 4 DWORDs then 4 pointers → `#[repr(C)]`.
    #[repr(C)]
    struct NetResourceW {
        dw_scope: u32,
        dw_type: u32,
        dw_display_type: u32,
        dw_usage: u32,
        lp_local_name: *mut u16,
        lp_remote_name: *mut u16,
        lp_comment: *mut u16,
        lp_provider: *mut u16,
    }
    const RESOURCE_GLOBALNET: u32 = 0x0000_0002;
    const RESOURCETYPE_DISK: u32 = 0x0000_0001;
    const NO_ERROR: u32 = 0;
    const ERROR_MORE_DATA: u32 = 234;

    /// Reads a NUL-terminated UTF-16 string from a pointer (remote share).
    unsafe fn pwstr_to_string(p: *const u16) -> String {
        unsafe {
            if p.is_null() {
                return String::new();
            }
            let mut len = 0isize;
            while *p.offset(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len as usize))
        }
    }

    /// Enumerates a server's VISIBLE disk shares (`server` = host name,
    /// without `\\`). Ignores hidden administrative shares (`C$`, `IPC$`…). A
    /// BLOCKING call (network timeout) — like `read_dir` on a remote path.
    /// Distinguishes "access denied" (→ offer a login) from "unreachable".
    pub fn shares_of(server: &str) -> super::NetShares {
        use super::NetShares;
        let mut remote_w = wide(&format!(r"\\{server}"));
        let mut nr = NetResourceW {
            dw_scope: RESOURCE_GLOBALNET,
            dw_type: RESOURCETYPE_DISK,
            dw_display_type: 0,
            dw_usage: 0,
            lp_local_name: std::ptr::null_mut(),
            lp_remote_name: remote_w.as_mut_ptr(),
            lp_comment: std::ptr::null_mut(),
            lp_provider: std::ptr::null_mut(),
        };
        let mut handle: isize = 0;
        let rc = unsafe {
            WNetOpenEnumW(
                RESOURCE_GLOBALNET,
                RESOURCETYPE_DISK,
                0,
                &mut nr,
                &mut handle,
            )
        };
        if rc != NO_ERROR {
            return if is_auth_error(rc) {
                NetShares::AuthNeeded
            } else {
                NetShares::Unreachable
            };
        }
        let mut shares = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut count: u32 = 0xFFFF_FFFF; // as many as the buffer can hold
            let mut bufsize: u32 = buf.len() as u32;
            let rc = unsafe {
                WNetEnumResourceW(
                    handle,
                    &mut count,
                    buf.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut bufsize,
                )
            };
            if rc == ERROR_MORE_DATA {
                buf.resize(bufsize as usize, 0);
                continue;
            }
            if rc != NO_ERROR {
                break; // ERROR_NO_MORE_ITEMS (259) or end
            }
            let items = unsafe {
                std::slice::from_raw_parts(buf.as_ptr() as *const NetResourceW, count as usize)
            };
            for it in items {
                let remote = unsafe { pwstr_to_string(it.lp_remote_name) };
                // `\\HOST\share` → last segment = share name.
                if let Some(name) = remote.rsplit('\\').next()
                    && !name.is_empty()
                    && !name.ends_with('$')
                {
                    shares.push(name.to_string());
                }
            }
        }
        unsafe { WNetCloseEnum(handle) };
        super::NetShares::Ok(shares)
    }

    // Enumeration of WSL distributions via the registry (advapi32, system DLL).
    type Hkey = isize;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(hkey: Hkey, sub: *const u16, opts: u32, sam: u32, out: *mut Hkey) -> i32;
        fn RegEnumKeyExW(
            hkey: Hkey,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_len: *mut u32,
            last_write: *mut u64,
        ) -> i32;
        fn RegQueryValueExW(
            hkey: Hkey,
            value: *const u16,
            reserved: *mut u32,
            ty: *mut u32,
            data: *mut u8,
            data_len: *mut u32,
        ) -> i32;
        fn RegCloseKey(hkey: Hkey) -> i32;
    }
    // `((HKEY)(LONG)0x80000001)` sign-extended to pointer size.
    const HKEY_CURRENT_USER: Hkey = 0x8000_0001u32 as i32 as Hkey;
    const KEY_READ: u32 = 0x2_0019;

    // Drive types relevant to the user (UNKNOWN=0 and NO_ROOT_DIR=1 are
    // excluded).
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4; // mapped network
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Bitmask of mounted letters (bit 0 = A: … 25 = Z:) — instantaneous.
    /// Used as a "signature" to detect a drive appearing/disappearing.
    pub fn logical_drives_mask() -> u32 {
        unsafe { GetLogicalDrives() }
    }

    /// Capacity of a volume as `(total, free_for_this_user)`, `(0, 0)` when it
    /// cannot be read.
    ///
    /// A drive with no media — an empty card reader slot, an optical drive
    /// standing open — is the hazard here: left to itself the call raises a
    /// modal asking for a disc, and it can stall while the hardware is polled.
    /// `SEM_FAILCRITICALERRORS` turns that into a plain failure, and the mode
    /// is restored right after so nothing else inherits it.
    fn volume_capacity(root_w: &[u16]) -> (u64, u64) {
        const SEM_FAILCRITICALERRORS: u32 = 0x0001;
        let mut previous_mode: u32 = 0;
        let changed =
            unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut previous_mode) } != 0;

        let (mut free_for_caller, mut total) = (0u64, 0u64);
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                root_w.as_ptr(),
                &mut free_for_caller,
                &mut total,
                std::ptr::null_mut(),
            )
        } != 0;

        if changed {
            unsafe { SetThreadErrorMode(previous_mode, std::ptr::null_mut()) };
        }
        if ok { (total, free_for_caller) } else { (0, 0) }
    }

    /// Is the letter a mapped NETWORK drive? `GetDriveTypeW` reads the local
    /// mount table (no network access, instantaneous).
    pub fn drive_is_remote(letter: char) -> bool {
        let root_w = wide(&format!("{letter}:\\"));
        unsafe { GetDriveTypeW(root_w.as_ptr()) == DRIVE_REMOTE }
    }

    pub fn logical_drives() -> Vec<Place> {
        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root = format!("{letter}:\\");
            let root_w = wide(&root);
            let dtype = unsafe { GetDriveTypeW(root_w.as_ptr()) };
            if !matches!(
                dtype,
                DRIVE_REMOVABLE | DRIVE_FIXED | DRIVE_REMOTE | DRIVE_CDROM | DRIVE_RAMDISK
            ) {
                continue;
            }
            let letter_str = format!("{letter}:");
            let is_net = dtype == DRIVE_REMOTE;
            // Mapped network: label "\\server\share (P:)" (resolved target),
            // otherwise fallback "P:". Local: "Label (P:)" or "P:".
            let name = if is_net {
                match mapped_remote(&letter_str) {
                    Some(r) => format!("{r} ({letter_str})"),
                    None => letter_str.clone(),
                }
            } else {
                match volume_label(&root_w) {
                    Some(l) if !l.is_empty() => format!("{l} ({letter_str})"),
                    _ => letter_str.clone(),
                }
            };
            // Local volumes only. A mapped network letter is skipped for the
            // same reason a share is on the other platform: the call leaves the
            // machine and can stall, and the answer describes the server.
            let (total_bytes, free_bytes) = if is_net {
                (0, 0)
            } else {
                volume_capacity(&root_w)
            };
            // Ejectable if removable/CD, OR if it's a "fixed" disk but on
            // an external bus (USB/1394/SD/MMC): Windows classifies many
            // USB flash drives/SSDs as DRIVE_FIXED, hence the bus check.
            // Computed once — the bus check is an IOCTL, and two fields read it.
            let ejectable = match dtype {
                DRIVE_REMOVABLE | DRIVE_CDROM => true,
                DRIVE_FIXED => bus_ejectable(letter),
                _ => false, // network, ramdisk
            };
            out.push(Place {
                kind: if is_net {
                    PlaceKind::Network
                } else {
                    PlaceKind::Drive
                },
                path: PathBuf::from(&root),
                name,
                total_bytes,
                free_bytes,
                removable: ejectable,
                // An optical drive is the one case that parts company with the
                // line above: its disc pops out, but the drive itself stays
                // bolted in — so releasing it is not an invitation to unplug.
                hotplug: ejectable && dtype != DRIVE_CDROM,
                // Eject/disconnect target: the letter (`P:`).
                device: letter_str,
            });
        }
        out
    }

    /// Is a "fixed" disk actually **ejectable**? The volume's bus is queried
    /// via `IOCTL_STORAGE_QUERY_PROPERTY`: USB / 1394 / SD / MMC (or
    /// removable media) ⇒ yes. Covers USB flash drives & SSDs that Windows
    /// files under `DRIVE_FIXED`. Best-effort: `false` if opening/IOCTL fails.
    fn bus_ejectable(letter: char) -> bool {
        let path = wide(&format!(r"\\.\{letter}:"));
        // 0 access rights: the property IOCTL is FILE_ANY_ACCESS.
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_RW,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if h == INVALID_HANDLE {
            return false;
        }
        // STORAGE_PROPERTY_QUERY { PropertyId = 0 (StorageDeviceProperty),
        // QueryType = 0 (PropertyStandardQuery), AdditionalParameters }.
        let query: [u32; 3] = [0, 0, 0];
        let mut buf = [0u8; 512];
        let mut ret = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                h,
                IOCTL_STORAGE_QUERY_PROPERTY,
                query.as_ptr() as *mut core::ffi::c_void,
                std::mem::size_of_val(&query) as u32,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
                &mut ret,
                std::ptr::null_mut(),
            )
        };
        unsafe { CloseHandle(h) };
        if ok == 0 || ret < 32 {
            return false;
        }
        // STORAGE_DEVICE_DESCRIPTOR : RemovableMedia @10 (BOOLEAN), BusType @28 (DWORD).
        let removable_media = buf[10] != 0;
        let bus_type = u32::from_ne_bytes([buf[28], buf[29], buf[30], buf[31]]);
        // External buses: 1394=0x4, USB=0x7, SD=0xC, MMC=0xD.
        removable_media || matches!(bus_type, 0x4 | 0x7 | 0xC | 0xD)
    }

    /// Target of a mapped network drive (`Z:` → `\\server\share`) via
    /// `WNetGetConnectionW`. `None` if not mapped / unavailable.
    fn mapped_remote(letter: &str) -> Option<String> {
        let local = wide(letter); // "Z:" (without backslash)
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let rc = unsafe { WNetGetConnectionW(local.as_ptr(), buf.as_mut_ptr(), &mut len) };
        if rc != 0 {
            return None; // NO_ERROR = 0
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
    }

    /// WSL distributions (including `docker-desktop`) exposed as `\\wsl$\<distro>`.
    /// Read from the registry (`HKCU\…\Lxss\<guid>\DistributionName`) — fast,
    /// without launching `wsl.exe` (no console flash, no service latency).
    pub fn wsl_distros() -> Vec<Place> {
        let mut out = Vec::new();
        let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Lxss");
        let mut lxss: Hkey = 0;
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_READ, &mut lxss) } != 0 {
            return out; // WSL not installed
        }
        let mut idx = 0u32;
        loop {
            let mut name = [0u16; 128];
            let mut nlen = name.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    lxss,
                    idx,
                    name.as_mut_ptr(),
                    &mut nlen,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                break; // ERROR_NO_MORE_ITEMS
            }
            idx += 1;
            let subname: Vec<u16> = name[..nlen as usize]
                .iter()
                .copied()
                .chain(std::iter::once(0))
                .collect();
            let mut hsub: Hkey = 0;
            if unsafe { RegOpenKeyExW(lxss, subname.as_ptr(), 0, KEY_READ, &mut hsub) } != 0 {
                continue;
            }
            if let Some(distro) = reg_read_sz(hsub, "DistributionName") {
                out.push(Place {
                    kind: PlaceKind::Network,
                    path: PathBuf::from(format!(r"\\wsl$\{distro}")),
                    name: distro,
                    removable: false,
                    hotplug: false,
                    device: String::new(),
                    // Reached through a network path: same reasoning as a share.
                    total_bytes: 0,
                    free_bytes: 0,
                });
            }
            unsafe { RegCloseKey(hsub) };
        }
        unsafe { RegCloseKey(lxss) };
        out
    }

    /// Reads a `REG_SZ` (string) value. `None` if absent / different type.
    fn reg_read_sz(hkey: Hkey, value: &str) -> Option<String> {
        let vw = wide(value);
        let mut buf = [0u16; 260];
        let mut len = (buf.len() * 2) as u32; // bytes
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                vw.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut u8,
                &mut len,
            )
        };
        if rc != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
    }

    fn volume_label(root_w: &[u16]) -> Option<String> {
        let mut buf = [0u16; 261]; // MAX_PATH + 1
        let ok = unsafe {
            GetVolumeInformationW(
                root_w.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            return None; // volume inaccessible (e.g. disconnected network)
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// The trash (a single location). On Linux: navigable. On Windows:
/// empty path → the GUI opens it in Explorer.
pub fn trash_place() -> Place {
    #[cfg(windows)]
    {
        Place::simple(PlaceKind::Trash, PathBuf::new(), String::new())
    }
    #[cfg(not(windows))]
    {
        let path = dirs::data_dir()
            .map(|d| d.join("Trash").join("files"))
            .unwrap_or_default();
        Place::simple(PlaceKind::Trash, path, String::new())
    }
}

// ----- Helpers -----

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string_lossy().to_string())
}

/// A drive's label (its **name**, not its path):
///   - **Windows**: `"Windows (C:)"` (volume label + letter);
///   - **Linux**: volume label (`/dev/disk/by-label`) → otherwise the mount
///     point's name (udisks removable mounts carry the label) → otherwise
///     empty for the root `/` (the GUI then sets an i18n "System" label).
///
/// `disk_name` = `Disk::name()` = **device path** (`/dev/sda1`…) on
/// Linux — hence the need to resolve it. (Windows builds its label directly
/// in `windrives`, without going through here.)
#[cfg(not(windows))]
fn drive_label(mount: &Path, disk_name: &str) -> String {
    #[cfg(target_os = "linux")]
    if let Some(label) = linux_volume_label(Path::new(disk_name))
        && !label.is_empty()
    {
        return label;
    }
    let _ = disk_name;
    if mount != Path::new("/") {
        let base = file_name_of(mount);
        if !base.is_empty() && base != "/" {
            return base;
        }
    }
    // Root without a label → the GUI will show an i18n "System" label.
    String::new()
}

/// Resolves a device's volume label via `/dev/disk/by-label/*`
/// (symlinks `LABEL → ../../<device>`). `None` if not found.
#[cfg(target_os = "linux")]
fn linux_volume_label(device: &Path) -> Option<String> {
    let target = std::fs::canonicalize(device).ok()?;
    let dir = std::fs::read_dir("/dev/disk/by-label").ok()?;
    for entry in dir.flatten() {
        if let Ok(resolved) = std::fs::canonicalize(entry.path())
            && resolved == target
        {
            return Some(udev_unescape(&entry.file_name().to_string_lossy()));
        }
    }
    None
}

/// Decodes udev escaping of `by-label` names (`\xNN` hex, e.g. `\x20`
/// for space). Rebuilds as bytes then UTF-8 (accented labels handled correctly).
#[cfg(target_os = "linux")]
fn udev_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1] == b'x'
            && let Ok(code) = u8::from_str_radix(&s[i + 2..i + 4], 16)
        {
            out.push(code);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A **network** filesystem? (remote mount → "Network" section).
/// Covers common protocols and FUSE backends (`fuse.sshfs`, GVFS…).
#[cfg(not(windows))]
fn is_network_fs(fs: &str) -> bool {
    const NET_FS: &[&str] = &[
        "nfs",
        "nfs4",
        "cifs",
        "smbfs",
        "smb3",
        "9p",
        "afs",
        "ncpfs",
        "coda",
        "davfs",
        "ceph",
        "glusterfs",
        "fuse.sshfs",
        "fuse.gvfsd-fuse",
        "fuse.rclone",
        "fuse.s3fs",
        "fuse.davfs2",
    ];
    NET_FS.iter().any(|n| fs.eq_ignore_ascii_case(n))
}

/// Removable mount (USB, card, external disk) → ejectable.
/// udisks heuristic: removable devices are auto-mounted under
/// `/run/media/<user>/…` (Fedora/Arch) or `/media/…` (Debian/Ubuntu).
#[cfg(not(windows))]
fn is_removable_mount(mount: &Path) -> bool {
    let s = mount.to_string_lossy();
    s.starts_with("/run/media/") || s.starts_with("/media/")
}

/// Shares mounted by **GVFS** (GNOME/Nautilus) under
/// `/run/user/<uid>/gvfs/<backend>:<params>` — SMB, SFTP, MTP, DAV… They do
/// NOT show up as "disk" mounts (a single FUSE mount point), so the folder
/// is listed instead.
#[cfg(target_os = "linux")]
fn gvfs_mounts() -> Vec<Place> {
    let root = gvfs_root();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let raw = e.file_name().to_string_lossy().to_string();
            out.push(Place {
                kind: PlaceKind::Network,
                path,
                name: gvfs_pretty_name(&raw),
                removable: false,
                hotplug: false,
                device: String::new(),
                // Never measured: asking a share for its size is a round trip
                // that can stall for seconds, and the answer would be the
                // server's free space, not the user's quota on it.
                total_bytes: 0,
                free_bytes: 0,
            });
        }
    }
    out
}

/// Turns a GVFS mount name into something readable: `smb-share:server=nas,share=media`
/// → `media (nas)`; `sftp:host=host.tld,user=me` → `host.tld (sftp)`. Kept
/// language-neutral on purpose (no embedded word like "on"/"at"): per this
/// module's own convention, the core only returns raw, un-translated names.
#[cfg(target_os = "linux")]
fn gvfs_pretty_name(raw: &str) -> String {
    let (backend, rest) = raw.split_once(':').unwrap_or((raw, ""));
    let mut params = std::collections::HashMap::new();
    for kv in rest.split(',') {
        if let Some((k, v)) = kv.split_once('=') {
            params.insert(k, v);
        }
    }
    let server = params.get("server").or_else(|| params.get("host")).copied();
    let share = params.get("share").copied();
    match (backend, share, server) {
        ("smb-share", Some(sh), Some(sv)) => format!("{sh} ({sv})"),
        (_, _, Some(sv)) => {
            let proto = backend.trim_end_matches("-share");
            format!("{sv} ({proto})")
        }
        _ => raw.to_string(),
    }
}

/// Filesystems that name a volume nobody browses: system bookkeeping, or a
/// container whose real volumes appear on their own rows once assembled.
#[cfg(target_os = "linux")]
const HIDDEN_VOLUME_FS: &[&str] = &[
    "swap",
    "LVM2_member",
    "linux_raid_member",
    "isw_raid_member",
    "ddf_raid_member",
    "squashfs",
];

/// Filesystem a locked encrypted container reports. It holds nothing mountable
/// until unlocked, so it is classified apart rather than hidden.
#[cfg(target_os = "linux")]
const ENCRYPTED_FS: &str = "crypto_LUKS";

/// One row of the block inventory, reduced to the fields the sidebar needs.
#[cfg(target_os = "linux")]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct BlockRow {
    name: String,
    /// Kernel name of the device this one sits on, empty for a whole disk.
    parent: String,
    /// `disk`, `part`, `loop`, `dm`…
    kind: String,
    fs_type: String,
    mountpoint: String,
    removable: bool,
    size: u64,
    label: String,
    /// Partition type as named by the table, e.g. the firmware partition.
    part_type: String,
}

/// Reads the block inventory.
///
/// Byte sizes and a fixed locale on purpose: the human-readable size is
/// localised down to its decimal mark, and would not parse the same on two
/// machines.
#[cfg(target_os = "linux")]
fn block_inventory() -> Option<String> {
    let output = std::process::Command::new("lsblk")
        .env("LC_ALL", "C")
        .args([
            "-b",
            "-P",
            "-o",
            "NAME,PKNAME,TYPE,FSTYPE,MOUNTPOINT,RM,SIZE,LABEL,PARTTYPENAME",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Splits one `KEY="value"` line. Values are quoted, and an embedded quote is
/// backslash-escaped — a label is free text, so that case is real.
#[cfg(target_os = "linux")]
fn parse_pairs(line: &str) -> Vec<(String, String)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = line[key_start..i].to_string();
        i += 1;
        if i >= bytes.len() || bytes[i] != b'"' {
            break;
        }
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            // Skip whatever the backslash protects, quote included.
            i += if bytes[i] == b'\\' { 2 } else { 1 };
        }
        let value = line[value_start..i.min(bytes.len())].replace("\\\"", "\"");
        out.push((key, value));
        i += 1;
    }
    out
}

/// Turns the inventory text into rows, ignoring anything unparsable.
#[cfg(target_os = "linux")]
fn parse_block_rows(text: &str) -> Vec<BlockRow> {
    let mut rows = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut row = BlockRow::default();
        for (key, value) in parse_pairs(line) {
            match key.as_str() {
                "NAME" => row.name = value,
                "PKNAME" => row.parent = value,
                "TYPE" => row.kind = value,
                "FSTYPE" => row.fs_type = value,
                "MOUNTPOINT" => row.mountpoint = value,
                "RM" => row.removable = value == "1",
                "SIZE" => row.size = value.parse().unwrap_or(0),
                "LABEL" => row.label = value,
                "PARTTYPENAME" => row.part_type = value,
                _ => {}
            }
        }
        if !row.name.is_empty() {
            rows.push(row);
        }
    }
    rows
}

/// How this row should appear in the sidebar, if at all. The whole policy
/// lives here so the two families cannot drift apart.
#[cfg(target_os = "linux")]
fn volume_kind(row: &BlockRow, all: &[BlockRow]) -> Option<PlaceKind> {
    // Already mounted: it is `drives()` that reports it, with its capacity.
    if !row.mountpoint.is_empty() {
        return None;
    }
    let has_child = || all.iter().any(|other| other.parent == row.name);
    // An encrypted container. Once unlocked it grows a child holding the real
    // filesystem, and THAT child is the volume — the container itself then has
    // nothing left to offer. Still locked, it is worth showing: knowing the
    // disk is there is most of what the user was missing.
    if row.fs_type == ENCRYPTED_FS {
        return (!has_child()).then_some(PlaceKind::LockedVolume);
    }
    // Nothing to mount without a recognised filesystem.
    if row.fs_type.is_empty() || HIDDEN_VOLUME_FS.contains(&row.fs_type.as_str()) {
        return None;
    }
    // The firmware partition is not a place anyone browses.
    if row.part_type.contains("EFI") {
        return None;
    }
    match row.kind.as_str() {
        // A partition, or the volume exposed by an unlocked container.
        "part" | "crypt" => Some(PlaceKind::Volume),
        // A whole device carrying a filesystem — a stick written without a
        // partition table. Once it IS partitioned, its partitions are the
        // volumes and the disk itself is not one.
        "disk" => (!has_child()).then_some(PlaceKind::Volume),
        _ => None,
    }
}

/// Builds the sidebar entry for a volume that is not mounted.
#[cfg(target_os = "linux")]
fn volume_place(row: &BlockRow, kind: PlaceKind) -> Place {
    Place {
        kind,
        // No path: nothing is mounted yet. Like the Windows trash, the entry is
        // addressed by what it is rather than by where it lives.
        path: PathBuf::new(),
        name: if row.label.is_empty() {
            row.name.clone()
        } else {
            row.label.clone()
        },
        removable: row.removable,
        hotplug: is_hotplug_device(&row.name),
        device: format!("/dev/{}", row.name),
        total_bytes: row.size,
        // Only a mounted filesystem can say how much of it is left.
        free_bytes: 0,
    }
}

/// Directory where GVFS exposes its mounts.
///
/// The session variable is authoritative when it is set; the conventional path
/// is the fallback. Both the listing and the change signature go through here
/// so they always look at the same place: they used to resolve it differently,
/// and a session without that variable left the signature blind to a mount
/// appearing — the sidebar then never refreshed on its own.
#[cfg(target_os = "linux")]
fn gvfs_root() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty());
    gvfs_root_in(runtime.as_deref().map(Path::new), unsafe { libc_getuid() })
}

/// Resolution rule of [`gvfs_root`], isolated from the environment.
#[cfg(target_os = "linux")]
fn gvfs_root_in(runtime: Option<&Path>, uid: u32) -> PathBuf {
    match runtime {
        Some(dir) => dir.join("gvfs"),
        None => PathBuf::from(format!("/run/user/{uid}/gvfs")),
    }
}

// `getuid(2)` — direct FFI to libc (already linked) → no dependency.
#[cfg(target_os = "linux")]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// Should this mount point be shown to the user? Pseudo-filesystems and
/// irrelevant system mounts are excluded.
#[cfg(not(windows))]
fn is_user_facing_mount(mount: &Path, fs: &str) -> bool {
    // Pseudo / ephemeral filesystems → hidden. (NETWORK filesystems do NOT
    // pass through here: `drives()` handles them upstream via `is_network_fs`.)
    const HIDDEN_FS: &[&str] = &[
        "squashfs",
        "tmpfs",
        "devtmpfs",
        "overlay",
        "proc",
        "sysfs",
        "cgroup",
        "cgroup2",
        "ramfs",
        "autofs",
        "fuse.portal",
        "fuse.gvfsd-fuse",
    ];
    if HIDDEN_FS.iter().any(|h| fs.eq_ignore_ascii_case(h)) {
        return false;
    }
    if mount == Path::new("/") {
        return true;
    }
    let s = mount.to_string_lossy();
    // System mounts → hidden; common user mounts → kept.
    if s.starts_with("/boot")
        || s.starts_with("/snap")
        || s.starts_with("/var/lib/docker")
        || s.starts_with("/var/snap")
        || s == "/dev"
    {
        return false;
    }
    s == "/home"
        || s.starts_with("/home/")
        || s.starts_with("/run/media/")
        || s.starts_with("/media/")
        || s.starts_with("/mnt/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_places_includes_existing_home() {
        let places = user_places();
        // The home folder always exists in a test environment.
        if dirs::home_dir().map(|h| h.is_dir()).unwrap_or(false) {
            assert!(places.iter().any(|p| p.kind == PlaceKind::Home));
        }
        // All returned paths exist and are folders.
        for p in &places {
            assert!(
                p.path.is_dir(),
                "{} should be a directory",
                p.path.display()
            );
        }
    }

    #[test]
    fn drives_are_deduplicated_and_grouped() {
        let d = drives();
        // No duplicate mount point.
        for i in 1..d.len() {
            assert_ne!(d[i - 1].path, d[i].path, "no duplicate mount point");
        }
        // Only local or network drives.
        assert!(
            d.iter()
                .all(|p| matches!(p.kind, PlaceKind::Drive | PlaceKind::Network))
        );
        // Local ones come before network (grouped sections), and each family
        // is sorted by path.
        let first_net = d.iter().position(|p| p.kind == PlaceKind::Network);
        if let Some(k) = first_net {
            assert!(
                d[k..].iter().all(|p| p.kind == PlaceKind::Network),
                "all network places must be grouped at the end"
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn network_fs_and_removable_detection() {
        assert!(is_network_fs("nfs4"));
        assert!(is_network_fs("cifs"));
        assert!(is_network_fs("fuse.sshfs"));
        assert!(!is_network_fs("ext4"));
        assert!(!is_network_fs("vfat"));
        assert!(is_removable_mount(Path::new("/run/media/u/USB")));
        assert!(is_removable_mount(Path::new("/media/u/Stick")));
        assert!(!is_removable_mount(Path::new("/")));
        assert!(!is_removable_mount(Path::new("/home/u")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gvfs_names_are_prettified() {
        assert_eq!(
            gvfs_pretty_name("smb-share:server=nas,share=media"),
            "media (nas)"
        );
        assert_eq!(
            gvfs_pretty_name("sftp:host=host.tld,user=me"),
            "host.tld (sftp)"
        );
        // Unknown format → returned as-is.
        assert_eq!(gvfs_pretty_name("weird-thing"), "weird-thing");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_gvfs_root_follows_the_session_when_it_declares_one() {
        assert_eq!(
            gvfs_root_in(Some(Path::new("/run/user/4242")), 7),
            PathBuf::from("/run/user/4242/gvfs")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_gvfs_root_falls_back_to_the_conventional_path() {
        assert_eq!(gvfs_root_in(None, 7), PathBuf::from("/run/user/7/gvfs"));
    }

    /// Inventory row with only the fields a test cares about.
    #[cfg(target_os = "linux")]
    fn row(name: &str, kind: &str, fs: &str, mount: &str) -> BlockRow {
        BlockRow {
            name: name.to_string(),
            kind: kind.to_string(),
            fs_type: fs.to_string(),
            mountpoint: mount.to_string(),
            ..BlockRow::default()
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_quoted_pair_line_is_split_into_its_fields() {
        let pairs = parse_pairs(r#"NAME="sda1" TYPE="part" SIZE="1024" LABEL="Volume01""#);
        assert_eq!(
            pairs,
            vec![
                ("NAME".to_string(), "sda1".to_string()),
                ("TYPE".to_string(), "part".to_string()),
                ("SIZE".to_string(), "1024".to_string()),
                ("LABEL".to_string(), "Volume01".to_string()),
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_label_may_carry_an_escaped_quote() {
        let pairs = parse_pairs(r#"NAME="sda1" LABEL="a\"b" TYPE="part""#);
        assert_eq!(pairs[1].1, r#"a"b"#);
        // The row after the escape is still read: the scan did not lose its place.
        assert_eq!(pairs[2], ("TYPE".to_string(), "part".to_string()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_row_is_built_with_its_size_in_bytes() {
        let rows = parse_block_rows(
            r#"NAME="sda1" PKNAME="sda" TYPE="part" FSTYPE="ext4" MOUNTPOINT="" RM="1" SIZE="2048" LABEL="Volume01" PARTTYPENAME="Linux filesystem""#,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size, 2048);
        assert!(rows[0].removable);
        assert_eq!(rows[0].parent, "sda");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_unmounted_partition_with_a_filesystem_is_offered() {
        let rows = vec![row("sda1", "part", "ext4", "")];
        assert_eq!(volume_kind(&rows[0], &rows), Some(PlaceKind::Volume));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_already_mounted_partition_is_left_to_the_mount_scan() {
        let rows = vec![row("sda1", "part", "ext4", "/mnt/somewhere")];
        assert_eq!(volume_kind(&rows[0], &rows), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_partition_without_a_filesystem_is_not_offered() {
        let rows = vec![row("sda1", "part", "", "")];
        assert_eq!(volume_kind(&rows[0], &rows), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bookkeeping_filesystems_are_not_offered() {
        for fs in ["swap", "LVM2_member", "linux_raid_member", "squashfs"] {
            let rows = vec![row("sda1", "part", fs, "")];
            assert_eq!(volume_kind(&rows[0], &rows), None, "{fs} should be hidden");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_firmware_partition_is_not_offered() {
        let mut efi = row("sda1", "part", "vfat", "");
        efi.part_type = "EFI System".to_string();
        let rows = vec![efi];
        assert_eq!(volume_kind(&rows[0], &rows), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_whole_disk_carrying_a_filesystem_is_offered() {
        let rows = vec![row("sdb", "disk", "vfat", "")];
        assert_eq!(volume_kind(&rows[0], &rows), Some(PlaceKind::Volume));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_partitioned_disk_yields_its_partitions_not_itself() {
        let mut child = row("sdb1", "part", "ext4", "");
        child.parent = "sdb".to_string();
        let rows = vec![row("sdb", "disk", "vfat", ""), child];
        assert_eq!(volume_kind(&rows[0], &rows), None);
        assert_eq!(volume_kind(&rows[1], &rows), Some(PlaceKind::Volume));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_volume_is_named_by_its_label_and_falls_back_to_its_device() {
        let mut labelled = row("sda1", "part", "ext4", "");
        labelled.label = "Volume01".to_string();
        assert_eq!(volume_place(&labelled, PlaceKind::Volume).name, "Volume01");
        assert_eq!(
            volume_place(&row("sda1", "part", "ext4", ""), PlaceKind::Volume).name,
            "sda1"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_volume_carries_its_device_and_no_path() {
        let place = volume_place(&row("sda1", "part", "ext4", ""), PlaceKind::Volume);
        assert_eq!(place.kind, PlaceKind::Volume);
        assert_eq!(place.device, "/dev/sda1");
        assert_eq!(place.path, PathBuf::new());
        // No gauge is possible before the kernel has the filesystem.
        assert_eq!(place.free_bytes, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_locked_encrypted_container_is_shown_as_locked() {
        let rows = vec![row("sda1", "part", "crypto_LUKS", "")];
        assert_eq!(volume_kind(&rows[0], &rows), Some(PlaceKind::LockedVolume));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_unlocked_container_steps_aside_for_the_volume_it_exposes() {
        let mut opened = row("dm-0", "crypt", "ext4", "");
        opened.parent = "sda1".to_string();
        let rows = vec![row("sda1", "part", "crypto_LUKS", ""), opened];
        // The container has nothing left to offer once it has been opened…
        assert_eq!(volume_kind(&rows[0], &rows), None);
        // …and what it exposes is an ordinary volume, mountable like any other.
        assert_eq!(volume_kind(&rows[1], &rows), Some(PlaceKind::Volume));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_unlocked_container_whose_volume_is_mounted_shows_neither() {
        let mut opened = row("dm-0", "crypt", "ext4", "/mnt/somewhere");
        opened.parent = "sda1".to_string();
        let rows = vec![row("sda1", "part", "crypto_LUKS", ""), opened];
        assert_eq!(volume_kind(&rows[0], &rows), None);
        assert_eq!(volume_kind(&rows[1], &rows), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_hotplug_bus_is_recognised_in_a_resolved_device_path() {
        // Shapes taken from a real sysfs resolution.
        assert!(path_traverses_hotplug_bus(
            "/sys/devices/pci0000:00/0000:00:14.0/usb4/4-1/4-1:1.0/host8/target8:0:0/8:0:0:0/block/sdi"
        ));
        assert!(path_traverses_hotplug_bus(
            "/sys/devices/platform/soc/mmc_host/mmc0/mmc0:0001/block/mmcblk0"
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_internal_bus_is_not_a_hotplug_one() {
        // The same disk family, wired to the motherboard: releasing it must not
        // claim the cable can be pulled.
        assert!(!path_traverses_hotplug_bus(
            "/sys/devices/pci0000:00/0000:00:17.0/ata6/host5/target5:0:0/5:0:0:0/block/sdd"
        ));
        assert!(!path_traverses_hotplug_bus(
            "/sys/devices/pci0000:00/0000:00:1d.0/nvme/nvme0/nvme0n1"
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_locked_container_keeps_its_size_and_its_device() {
        let mut locked = row("sda1", "part", "crypto_LUKS", "");
        locked.size = 4096;
        let place = volume_place(&locked, PlaceKind::LockedVolume);
        assert_eq!(place.kind, PlaceKind::LockedVolume);
        assert_eq!(place.device, "/dev/sda1");
        assert_eq!(place.total_bytes, 4096);
    }

    #[test]
    fn trash_place_kind() {
        assert_eq!(trash_place().kind, PlaceKind::Trash);
    }

    #[cfg(not(windows))]
    #[test]
    fn drives_always_include_root() {
        assert!(
            drives().iter().any(|p| p.path == Path::new("/")),
            "the / root must always be present"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn filters_pseudo_filesystems() {
        assert!(!is_user_facing_mount(Path::new("/proc"), "proc"));
        assert!(!is_user_facing_mount(
            Path::new("/snap/core/123"),
            "squashfs"
        ));
        assert!(is_user_facing_mount(Path::new("/"), "ext4"));
        assert!(is_user_facing_mount(Path::new("/run/media/u/USB"), "vfat"));
    }
}
