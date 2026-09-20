//! Portable devices (phones, cameras) shown in the sidebar — Linux.
//!
//! MTP is not a filesystem. Unless a FUSE backend has mounted the device,
//! nothing about it appears under any path `std::fs` can reach, which is why
//! the mount scan in the core never reports one. The USB stack announces it
//! all the same: an MTP-capable device exposes an interface whose description
//! string is `MTP`, and sysfs publishes that string next to the manufacturer
//! and product names.
//!
//! Detection is therefore a handful of small reads under `/sys` — no library,
//! no daemon, no round trip to the device — which is cheap enough to run
//! synchronously on the UI thread. The Windows counterpart has to talk to the
//! Shell and to WPD, so it scans in the background and caches its result;
//! nothing here needs that.
//!
//! Favnyr cannot browse the device, since MTP exposes no path: activating the
//! entry hands it to the desktop's own file manager, the same compromise the
//! Windows side makes.

use anyhow::{Result, anyhow};
use std::path::Path;

/// USB device tree published by the kernel.
const USB_DEVICES: &str = "/sys/bus/usb/devices";

/// Interface description string advertised by an MTP-capable device.
const MTP_INTERFACE: &str = "MTP";

/// URI listing every connected device, used when no per-device URI can be
/// built.
const MTP_ROOT: &str = "mtp:/";

/// Backend able to resolve a URI that names one exact device.
const GVFS_MTP_BACKEND: &str = "gvfsd-mtp";

/// Where distributions install the GVFS backends.
const GVFS_BACKEND_DIRS: &[&str] = &[
    "/usr/libexec",
    "/usr/libexec/gvfs",
    "/usr/lib/gvfs",
    "/usr/lib64/gvfs",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortableDevice {
    /// Display name, built from the USB descriptors. Left untranslated, like
    /// a volume label: it is what the hardware reports.
    pub name: String,
    /// Target handed to the desktop's file manager. Opaque on purpose: it is a
    /// URI, never a path a filesystem call may touch.
    pub uri: String,
}

/// Devices currently connected. Reads sysfs on every call — measured in
/// microseconds, so no cache stands between the caller and the truth.
pub fn devices() -> Vec<PortableDevice> {
    scan(Path::new(USB_DEVICES), gvfs_mtp_available())
}

/// Fingerprint of the visible list, mixed into the sidebar signature so that
/// plugging or unplugging a device refreshes the panel. It only changes when
/// the list does.
pub fn signature() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for device in devices() {
        device.name.hash(&mut hasher);
        device.uri.hash(&mut hasher);
    }
    hasher.finish()
}

/// Hands the device to the desktop's file manager. `uri` is passed as a single
/// argument, never through a shell.
pub fn open(uri: &str) -> Result<()> {
    if uri.trim().is_empty() {
        return Err(anyhow!("empty device URI"));
    }
    ::open::that_detached(uri).map_err(|err| anyhow!("opening portable device: {err}"))
}

/// Walks the USB tree and keeps the devices exposing an MTP interface.
fn scan(root: &Path, gvfs: bool) -> Vec<PortableDevice> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut nodes: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Only interface directories carry a description string. They are
        // named `<device>:<configuration>.<interface>`, so the part before the
        // colon names the device the interface belongs to.
        let Some((device, _)) = name.split_once(':') else {
            continue;
        };
        if !reads_as_mtp(&entry.path().join("interface")) {
            continue;
        }
        // A device may expose several interfaces; it is listed once.
        if !nodes.iter().any(|known| known == device) {
            nodes.push(device.to_string());
        }
    }
    // Directory order is not guaranteed by the kernel: sorting keeps the
    // sidebar stable from one scan to the next.
    nodes.sort();
    nodes
        .iter()
        .filter_map(|device| describe(root, device, gvfs))
        .collect()
}

/// Does this interface advertise MTP?
fn reads_as_mtp(interface: &Path) -> bool {
    let Ok(description) = std::fs::read_to_string(interface) else {
        return false;
    };
    // A device may advertise several functions in one string, so the
    // description is compared token by token rather than as a whole.
    description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token.eq_ignore_ascii_case(MTP_INTERFACE))
}

/// Builds the sidebar entry from the device's USB descriptors.
fn describe(root: &Path, device: &str, gvfs: bool) -> Option<PortableDevice> {
    let dir = root.join(device);
    let name = match (
        read_field(&dir, "manufacturer"),
        read_field(&dir, "product"),
    ) {
        // Many devices repeat the maker inside the model; joining blindly
        // would show it twice.
        (Some(vendor), Some(model)) if model.starts_with(&vendor) => model,
        (Some(vendor), Some(model)) => format!("{vendor} {model}"),
        (None, Some(model)) => model,
        (Some(vendor), None) => vendor,
        // Descriptor strings are optional; the numeric identity is mandatory
        // and still beats a nameless row.
        (None, None) => format!(
            "{}:{}",
            read_field(&dir, "idVendor")?,
            read_field(&dir, "idProduct")?
        ),
    };
    Some(PortableDevice {
        name,
        uri: uri_for(&dir, gvfs),
    })
}

/// Reads one sysfs attribute, treating blank as absent.
fn read_field(dir: &Path, file: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(file))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The desktop decides how a device is addressed, and the two stacks in use
/// disagree: GVFS resolves one device per URI from its USB coordinates, while
/// a desktop whose MTP support lives inside its own I/O layer exposes a single
/// root listing them all. Naming the exact device is preferable, so that form
/// is used whenever the backend which understands it is installed.
fn uri_for(device: &Path, gvfs: bool) -> String {
    if gvfs
        && let Some(bus) = read_field(device, "busnum").and_then(|v| v.parse::<u32>().ok())
        && let Some(number) = read_field(device, "devnum").and_then(|v| v.parse::<u32>().ok())
    {
        return format!("mtp://[usb:{bus:03},{number:03}]/");
    }
    MTP_ROOT.to_string()
}

/// Is a GVFS MTP backend installed?
fn gvfs_mtp_available() -> bool {
    GVFS_BACKEND_DIRS
        .iter()
        .any(|dir| Path::new(dir).join(GVFS_MTP_BACKEND).exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Builds an isolated fake USB tree.
    fn tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "favnyr-usb-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Writes one sysfs attribute, trailing newline included as the kernel
    /// does.
    fn attribute(root: &Path, node: &str, file: &str, value: &str) {
        let dir = root.join(node);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), format!("{value}\n")).unwrap();
    }

    /// Device exposing one MTP interface, with the usual descriptors.
    fn plug(root: &Path, node: &str, vendor: &str, model: &str) {
        attribute(root, &format!("{node}:1.0"), "interface", MTP_INTERFACE);
        attribute(root, node, "manufacturer", vendor);
        attribute(root, node, "product", model);
        attribute(root, node, "busnum", "1");
        attribute(root, node, "devnum", "5");
    }

    #[test]
    fn an_mtp_device_is_named_from_its_descriptors() {
        let root = tree("named");
        plug(&root, "1-1", "Vendor01", "Model01");

        let found = scan(&root, false);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Vendor01 Model01");
        assert_eq!(found[0].uri, MTP_ROOT);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_product_already_carrying_the_vendor_is_not_named_twice() {
        let root = tree("dedup-name");
        plug(&root, "1-1", "Vendor01", "Vendor01 Model01");

        assert_eq!(scan(&root, false)[0].name, "Vendor01 Model01");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_device_without_an_mtp_interface_is_ignored() {
        let root = tree("not-mtp");
        attribute(&root, "1-1:1.0", "interface", "Mass Storage");
        attribute(&root, "1-1", "product", "Model01");
        // A description merely containing the letters is not an MTP interface.
        attribute(&root, "1-2:1.0", "interface", "MTPX");
        attribute(&root, "1-2", "product", "Model02");

        assert!(scan(&root, false).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_device_advertising_several_functions_is_still_detected() {
        let root = tree("multi-function");
        attribute(&root, "1-1:1.0", "interface", "MTP,ADB");
        attribute(&root, "1-1", "product", "Model01");

        assert_eq!(scan(&root, false).len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn several_interfaces_of_one_device_yield_a_single_entry() {
        let root = tree("dedup-device");
        plug(&root, "1-1", "Vendor01", "Model01");
        attribute(&root, "1-1:1.1", "interface", MTP_INTERFACE);

        assert_eq!(scan(&root, false).len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn devices_are_listed_in_a_stable_order() {
        let root = tree("order");
        plug(&root, "1-2", "Vendor02", "Model02");
        plug(&root, "1-1", "Vendor01", "Model01");

        let names: Vec<String> = scan(&root, false).into_iter().map(|d| d.name).collect();

        assert_eq!(names, vec!["Vendor01 Model01", "Vendor02 Model02"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_gvfs_system_addresses_the_exact_device() {
        let root = tree("gvfs-uri");
        plug(&root, "1-1", "Vendor01", "Model01");

        assert_eq!(scan(&root, true)[0].uri, "mtp://[usb:001,005]/");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_device_without_usb_coordinates_falls_back_to_the_root_uri() {
        let root = tree("no-coordinates");
        attribute(&root, "1-1:1.0", "interface", MTP_INTERFACE);
        attribute(&root, "1-1", "product", "Model01");

        assert_eq!(scan(&root, true)[0].uri, MTP_ROOT);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_device_without_descriptor_strings_falls_back_to_its_numeric_identity() {
        let root = tree("numeric");
        attribute(&root, "1-1:1.0", "interface", MTP_INTERFACE);
        attribute(&root, "1-1", "idVendor", "1d6b");
        attribute(&root, "1-1", "idProduct", "0002");

        assert_eq!(scan(&root, false)[0].name, "1d6b:0002");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_missing_tree_yields_no_device() {
        assert!(scan(Path::new("/nonexistent-usb-tree"), false).is_empty());
    }

    #[test]
    fn an_empty_uri_is_refused() {
        assert!(open("   ").is_err());
    }
}
