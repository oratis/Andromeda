use std::process::Command;

use chrono::Utc;
use serde_json::Value;

use crate::{BootInfo, CpuInfo, HardwareIdentity, HardwareReport, MemoryInfo, OsFamily};

use super::{ProbeError, logical_cores};

pub(super) fn probe() -> Result<HardwareReport, ProbeError> {
    let script = r"
$cs = Get-CimInstance Win32_ComputerSystem
$bios = Get-CimInstance Win32_BIOS
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$secureBoot = $null
try { $secureBoot = Confirm-SecureBootUEFI } catch {}
$tpm = $false
try { $tpm = (Get-Tpm).TpmPresent } catch {}
[pscustomobject]@{
  manufacturer = $cs.Manufacturer
  model = $cs.Model
  firmware = $bios.SMBIOSBIOSVersion
  cpu = $cpu.Name
  memory = [string]$cs.TotalPhysicalMemory
  secure_boot = $secureBoot
  tpm2 = $tpm
  virtualization = [bool]$cs.HypervisorPresent
} | ConvertTo-Json -Compress
";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()?;
    if !output.status.success() {
        return Err(ProbeError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    Ok(HardwareReport {
        schema_version: HardwareReport::CURRENT_SCHEMA_VERSION,
        collected_at: Utc::now(),
        os_family: OsFamily::Windows,
        identity: HardwareIdentity {
            manufacturer: string(&value, "manufacturer"),
            model: string(&value, "model"),
            board: None,
            firmware_version: string(&value, "firmware"),
        },
        cpu: CpuInfo {
            architecture: std::env::consts::ARCH.into(),
            model: string(&value, "cpu"),
            logical_cores: logical_cores(),
        },
        memory: MemoryInfo {
            bytes: string(&value, "memory").and_then(|item| item.parse().ok()),
        },
        boot: BootInfo {
            uefi: value
                .get("secure_boot")
                .and_then(Value::as_bool)
                .map(|_| true),
            secure_boot: value.get("secure_boot").and_then(Value::as_bool),
            tpm2: value.get("tpm2").and_then(Value::as_bool).unwrap_or(false),
            virtualization: value
                .get("virtualization")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        devices: Vec::new(),
        warnings: vec![
            "Run the Andromeda installer preflight for driver-level Windows device inventory."
                .into(),
        ],
    })
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|item| !item.is_empty())
}
