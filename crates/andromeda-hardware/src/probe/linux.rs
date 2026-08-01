use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::{
    BootInfo, CpuInfo, DeviceInfo, HardwareIdentity, HardwareReport, MemoryInfo, OsFamily,
};

use super::{ProbeError, logical_cores};

pub(super) fn probe() -> Result<HardwareReport, ProbeError> {
    fs::metadata("/sys")?;
    let devices = collect_devices("/sys/bus/pci/devices", "pci")
        .into_iter()
        .chain(collect_devices("/sys/bus/usb/devices", "usb"))
        .collect();
    Ok(HardwareReport {
        schema_version: HardwareReport::CURRENT_SCHEMA_VERSION,
        collected_at: Utc::now(),
        os_family: OsFamily::Linux,
        identity: HardwareIdentity {
            manufacturer: read_trimmed("/sys/class/dmi/id/sys_vendor"),
            model: read_trimmed("/sys/class/dmi/id/product_name"),
            board: read_trimmed("/sys/class/dmi/id/board_name"),
            firmware_version: read_trimmed("/sys/class/dmi/id/bios_version"),
        },
        cpu: CpuInfo {
            architecture: std::env::consts::ARCH.into(),
            model: cpu_model(),
            logical_cores: logical_cores(),
        },
        memory: MemoryInfo {
            bytes: memory_bytes(),
        },
        boot: BootInfo {
            uefi: Some(Path::new("/sys/firmware/efi").exists()),
            secure_boot: secure_boot_state(),
            tpm2: Path::new("/sys/class/tpm/tpm0").exists(),
            virtualization: Path::new("/dev/kvm").exists(),
        },
        devices,
        warnings: vec![
            "A probe report is evidence, not a support promise; match it against a signed HCM."
                .into(),
        ],
    })
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn cpu_model() -> Option<String> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    cpuinfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        matches!(key.trim(), "model name" | "Hardware" | "Processor")
            .then(|| value.trim().to_owned())
    })
}

fn memory_bytes() -> Option<u64> {
    let memory = fs::read_to_string("/proc/meminfo").ok()?;
    let line = memory.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

fn secure_boot_state() -> Option<bool> {
    let entries = fs::read_dir("/sys/firmware/efi/efivars").ok()?;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("SecureBoot-") {
            let data = fs::read(entry.path()).ok()?;
            return data.get(4).map(|value| *value == 1);
        }
    }
    None
}

fn collect_devices(root: &str, bus: &str) -> Vec<DeviceInfo> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .take(512)
        .map(|entry| {
            let path = entry.path();
            DeviceInfo {
                bus: bus.into(),
                address: entry.file_name().into_string().ok(),
                vendor_id: read_trimmed(path.join(vendor_file(bus))),
                product_id: read_trimmed(path.join(product_file(bus))),
                class: read_trimmed(path.join("class")),
                driver: driver_name(&path),
                name: read_trimmed(path.join("product")),
            }
        })
        .collect()
}

fn vendor_file(bus: &str) -> &str {
    if bus == "usb" { "idVendor" } else { "vendor" }
}

fn product_file(bus: &str) -> &str {
    if bus == "usb" { "idProduct" } else { "device" }
}

fn driver_name(path: &Path) -> Option<String> {
    let target: PathBuf = fs::read_link(path.join("driver")).ok()?;
    target.file_name()?.to_str().map(str::to_owned)
}
