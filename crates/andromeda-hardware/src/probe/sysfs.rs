//! sysfs device inventory collection with an injectable root path.
//!
//! The collectors take the sysfs directory as a parameter so unit tests can
//! exercise the real on-disk layout (including the USB interface layout) with
//! a mock tree on any Unix host.

use std::fs;
use std::path::{Path, PathBuf};

use crate::DeviceInfo;

/// Upper bound on inventoried entries per bus; prevents unbounded reports on
/// pathological sysfs trees. A warning is appended when the cap is hit.
const DEVICE_CAP: usize = 512;

pub(super) fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn sorted_entry_names(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

pub(super) fn collect_pci_devices(root: &Path, warnings: &mut Vec<String>) -> Vec<DeviceInfo> {
    let names = sorted_entry_names(root);
    if names.len() > DEVICE_CAP {
        warnings.push(format!(
            "pci device inventory was truncated at {DEVICE_CAP} entries; the report is incomplete"
        ));
    }
    names
        .into_iter()
        .take(DEVICE_CAP)
        .map(|name| {
            let path = root.join(&name);
            DeviceInfo {
                bus: "pci".into(),
                address: Some(name),
                vendor_id: read_trimmed(path.join("vendor")),
                product_id: read_trimmed(path.join("device")),
                subsystem_vendor_id: read_trimmed(path.join("subsystem_vendor")),
                subsystem_product_id: read_trimmed(path.join("subsystem_device")),
                revision: read_trimmed(path.join("revision")),
                class: read_trimmed(path.join("class")),
                driver: driver_name(&path),
                name: read_trimmed(path.join("product")),
                modalias: read_trimmed(path.join("modalias")),
            }
        })
        .collect()
}

/// Collects USB *function* (interface) entries.
///
/// Real sysfs exposes no `class` attribute for USB entries: devices carry
/// `bDeviceClass` (which is `00` for composite devices and `ef` for
/// miscellaneous ones) and each interface such as `1-1:1.0` carries the
/// authoritative `bInterfaceClass` plus its own driver binding (usbhid,
/// uvcvideo, ...). Support diagnosis therefore inventories interfaces, with
/// vendor/product identity taken from the parent device directory. Hubs and
/// device-level entries are skipped as structural noise.
pub(super) fn collect_usb_interfaces(root: &Path, warnings: &mut Vec<String>) -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    let mut truncated = false;
    for name in sorted_entry_names(root) {
        // Interface directories are named `<device>:<config>.<interface>`.
        let Some((device_name, _)) = name.split_once(':') else {
            continue;
        };
        let interface_path = root.join(&name);
        let Some(class) = read_trimmed(interface_path.join("bInterfaceClass")) else {
            continue;
        };
        // Hub interfaces (class 09) include every root hub; they carry no
        // user-visible function and would only add noise.
        if class.eq_ignore_ascii_case("09") {
            continue;
        }
        let device_path = root.join(device_name);
        let Some(vendor_id) = read_trimmed(device_path.join("idVendor")) else {
            continue;
        };
        if devices.len() >= DEVICE_CAP {
            truncated = true;
            break;
        }
        devices.push(DeviceInfo {
            bus: "usb".into(),
            address: Some(name),
            vendor_id: Some(vendor_id),
            product_id: read_trimmed(device_path.join("idProduct")),
            subsystem_vendor_id: None,
            subsystem_product_id: None,
            revision: None,
            class: Some(class),
            driver: driver_name(&interface_path),
            name: read_trimmed(device_path.join("product")),
            modalias: read_trimmed(interface_path.join("modalias")),
        });
    }
    if truncated {
        warnings.push(format!(
            "usb interface inventory was truncated at {DEVICE_CAP} entries; the report is incomplete"
        ));
    }
    devices
}

pub(super) fn driver_name(path: &Path) -> Option<String> {
    let target: PathBuf = fs::read_link(path.join("driver")).ok()?;
    target.file_name()?.to_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Returns `<base>/devices` as the scanned root; driver directories are
    /// created under the sibling `<base>/drivers`, mirroring real sysfs where
    /// `/sys/bus/<bus>/devices` only contains device entries.
    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir()
            .join(format!(
                "andromeda-sysfs-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ))
            .join("devices");
        fs::create_dir_all(&path).expect("create mock sysfs root");
        path
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(path, contents).expect("write attribute");
    }

    fn bind_driver(entry: &Path, driver: &str) {
        let driver_dir = entry
            .parent()
            .expect("devices dir")
            .parent()
            .expect("bus root")
            .join("drivers")
            .join(driver);
        fs::create_dir_all(&driver_dir).expect("create driver dir");
        fs::create_dir_all(entry).expect("create device dir");
        std::os::unix::fs::symlink(driver_dir, entry.join("driver")).expect("driver symlink");
    }

    #[test]
    fn pci_devices_are_read_from_the_standard_attributes() {
        let root = temp_root("pci");
        let device = root.join("0000:01:00.0");
        write(&device.join("vendor"), "0x8086\n");
        write(&device.join("device"), "0x1521\n");
        write(&device.join("class"), "0x020000\n");
        write(&device.join("subsystem_vendor"), "0x1043\n");
        write(&device.join("subsystem_device"), "0x85f0\n");
        write(&device.join("revision"), "0x03\n");
        bind_driver(&device, "e1000e");

        let mut warnings = Vec::new();
        let devices = collect_pci_devices(&root, &mut warnings);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].vendor_id.as_deref(), Some("0x8086"));
        assert_eq!(devices[0].class.as_deref(), Some("0x020000"));
        assert_eq!(devices[0].subsystem_vendor_id.as_deref(), Some("0x1043"));
        assert_eq!(devices[0].driver.as_deref(), Some("e1000e"));
        assert!(warnings.is_empty());
        let _ = fs::remove_dir_all(root.parent().expect("base"));
    }

    #[test]
    fn pci_truncation_appends_a_warning() {
        let root = temp_root("pci-cap");
        for index in 0..(DEVICE_CAP + 3) {
            write(
                &root.join(format!("0000:{index:04}.0")).join("vendor"),
                "0x8086\n",
            );
        }
        let mut warnings = Vec::new();
        let devices = collect_pci_devices(&root, &mut warnings);
        assert_eq!(devices.len(), DEVICE_CAP);
        assert!(warnings.iter().any(|warning| warning.contains("truncated")));
        let _ = fs::remove_dir_all(root.parent().expect("base"));
    }

    /// Regression for the real sysfs USB layout: device directories expose
    /// `bDeviceClass` (not `class`), composite devices report `00`/`ef`, and
    /// only interface directories carry `bInterfaceClass` plus the function
    /// driver. The collector must classify by interface, not device.
    #[test]
    fn usb_functions_are_collected_at_the_interface_level() {
        let root = temp_root("usb");
        // Root hub device and its hub interface: pure noise.
        let hub = root.join("usb1");
        write(&hub.join("idVendor"), "1d6b\n");
        write(&hub.join("idProduct"), "0002\n");
        write(&hub.join("bDeviceClass"), "09\n");
        write(&root.join("1-0:1.0").join("bInterfaceClass"), "09\n");
        // Composite webcam: device has bDeviceClass ef and NO `class` file.
        let camera = root.join("1-1");
        write(&camera.join("idVendor"), "046d\n");
        write(&camera.join("idProduct"), "085e\n");
        write(&camera.join("bDeviceClass"), "ef\n");
        write(&camera.join("product"), "BRIO Webcam\n");
        let video = root.join("1-1:1.0");
        write(&video.join("bInterfaceClass"), "0e\n");
        write(&video.join("modalias"), "usb:v046Dp085E*ic0Eisc01ip00*\n");
        bind_driver(&video, "uvcvideo");
        let audio = root.join("1-1:1.2");
        write(&audio.join("bInterfaceClass"), "01\n");
        // No driver symlink: this function is driverless.

        let mut warnings = Vec::new();
        let devices = collect_usb_interfaces(&root, &mut warnings);
        assert_eq!(devices.len(), 2, "hub and device-level entries are noise");
        assert!(
            devices
                .iter()
                .all(|device| device.address.as_deref().is_some_and(|a| a.contains(':')))
        );

        let video = devices
            .iter()
            .find(|device| device.class.as_deref() == Some("0e"))
            .expect("video function");
        assert_eq!(video.vendor_id.as_deref(), Some("046d"));
        assert_eq!(video.product_id.as_deref(), Some("085e"));
        assert_eq!(video.driver.as_deref(), Some("uvcvideo"));
        assert_eq!(video.name.as_deref(), Some("BRIO Webcam"));

        let audio = devices
            .iter()
            .find(|device| device.class.as_deref() == Some("01"))
            .expect("audio function");
        assert_eq!(audio.driver, None);
        assert!(warnings.is_empty());
        let _ = fs::remove_dir_all(root.parent().expect("base"));
    }

    #[test]
    fn usb_interfaces_without_a_parent_device_are_skipped() {
        let root = temp_root("usb-orphan");
        write(&root.join("9-9:1.0").join("bInterfaceClass"), "0e\n");
        let mut warnings = Vec::new();
        assert!(collect_usb_interfaces(&root, &mut warnings).is_empty());
        let _ = fs::remove_dir_all(root.parent().expect("base"));
    }
}
