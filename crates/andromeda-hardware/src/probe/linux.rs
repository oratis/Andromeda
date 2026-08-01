use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::{BootInfo, CpuInfo, HardwareIdentity, HardwareReport, MemoryInfo, OsFamily};

use super::sysfs::{collect_pci_devices, collect_usb_interfaces, read_trimmed};
use super::{ProbeError, logical_cores};

pub(super) fn probe() -> Result<HardwareReport, ProbeError> {
    fs::metadata("/sys")?;
    let mut warnings = vec![
        "A probe report is evidence, not a support promise; match it against a signed HCM.".into(),
    ];
    let devices = collect_pci_devices(Path::new("/sys/bus/pci/devices"), &mut warnings)
        .into_iter()
        .chain(collect_usb_interfaces(
            Path::new("/sys/bus/usb/devices"),
            &mut warnings,
        ))
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
            tpm2: tpm2_state(),
            virtualization: Some(Path::new("/dev/kvm").exists()),
        },
        devices,
        warnings,
    })
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

/// TPM 1.2 chips also appear under `/sys/class/tpm`, so device presence alone
/// does not prove TPM 2.0. When the kernel does not expose the version the
/// state stays unknown instead of collapsing to `false`.
fn tpm2_state() -> Option<bool> {
    if !Path::new("/sys/class/tpm/tpm0").exists() {
        return Some(false);
    }
    read_trimmed("/sys/class/tpm/tpm0/tpm_version_major").map(|version| version == "2")
}
