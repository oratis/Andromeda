use chrono::Utc;

use crate::{BootInfo, CpuInfo, HardwareIdentity, HardwareReport, MemoryInfo, OsFamily};

use super::{ProbeError, logical_cores};

pub(super) fn probe() -> Result<HardwareReport, ProbeError> {
    Ok(HardwareReport {
        schema_version: HardwareReport::CURRENT_SCHEMA_VERSION,
        collected_at: Utc::now(),
        os_family: OsFamily::Other,
        identity: HardwareIdentity {
            manufacturer: None,
            model: None,
            board: None,
            firmware_version: None,
        },
        cpu: CpuInfo {
            architecture: std::env::consts::ARCH.into(),
            model: None,
            logical_cores: logical_cores(),
        },
        memory: MemoryInfo { bytes: None },
        boot: BootInfo {
            // Nothing can be verified on an unknown platform; report
            // unknown rather than false.
            uefi: None,
            secure_boot: None,
            tpm2: None,
            virtualization: None,
        },
        devices: Vec::new(),
        warnings: vec!["This operating system does not yet have a detailed probe backend.".into()],
    })
}
