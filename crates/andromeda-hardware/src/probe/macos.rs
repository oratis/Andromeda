use std::process::Command;

use chrono::Utc;

use crate::{BootInfo, CpuInfo, HardwareIdentity, HardwareReport, MemoryInfo, OsFamily};

use super::{ProbeError, logical_cores};

pub(super) fn probe() -> Result<HardwareReport, ProbeError> {
    let architecture = std::env::consts::ARCH.to_owned();
    let apple_silicon = architecture == "aarch64";
    let model = sysctl_string("hw.model")
        .ok_or_else(|| ProbeError::Command("sysctl hw.model returned no value".into()))?;
    Ok(HardwareReport {
        schema_version: HardwareReport::CURRENT_SCHEMA_VERSION,
        collected_at: Utc::now(),
        os_family: OsFamily::Macos,
        identity: HardwareIdentity {
            manufacturer: Some("Apple Inc.".into()),
            model: Some(model),
            board: None,
            firmware_version: None,
        },
        cpu: CpuInfo {
            architecture,
            model: sysctl_string("machdep.cpu.brand_string")
                .or_else(|| apple_silicon.then(|| "Apple silicon".into())),
            logical_cores: logical_cores(),
        },
        memory: MemoryInfo {
            bytes: sysctl_string("hw.memsize").and_then(|value| value.parse().ok()),
        },
        boot: BootInfo {
            uefi: Some(!apple_silicon),
            secure_boot: None,
            tpm2: false,
            virtualization: true,
        },
        devices: Vec::new(),
        warnings: vec![
            "macOS does not expose Apple boot policy as a TPM/Secure Boot equivalent; HCM must verify the platform-specific boot provider.".into(),
            "Detailed Mac device support requires an exact Asahi or Intel/T2 model manifest.".into(),
        ],
    })
}

fn sysctl_string(key: &str) -> Option<String> {
    let output = Command::new("/usr/sbin/sysctl")
        .args(["-n", key])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}
