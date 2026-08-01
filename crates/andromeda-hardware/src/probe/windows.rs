use std::process::Command;

use chrono::Utc;
use serde_json::Value;

use crate::{BootInfo, CpuInfo, HardwareIdentity, HardwareReport, MemoryInfo, OsFamily};

use super::{ProbeError, logical_cores};

pub(super) fn probe() -> Result<HardwareReport, ProbeError> {
    // The first statement forces UTF-8 stdout: Windows PowerShell 5.1
    // otherwise writes the OEM code page, and `serde_json::from_slice` would
    // fail outright on machines whose manufacturer/model strings contain
    // non-ASCII characters.
    //
    // TPM detection asks Win32_Tpm for its SpecVersion instead of trusting
    // Get-Tpm's TpmPresent, which is also true for TPM 1.2 chips. A null
    // answer (query threw, typically for lack of elevation) is reported as
    // unknown rather than false.
    let script = r"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$cs = Get-CimInstance Win32_ComputerSystem
$bios = Get-CimInstance Win32_BIOS
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$secureBoot = $null
try { $secureBoot = Confirm-SecureBootUEFI } catch {}
$tpm2 = $null
try {
  $tpmInfo = Get-CimInstance -Namespace 'root/cimv2/Security/MicrosoftTpm' -ClassName Win32_Tpm -ErrorAction Stop
  if ($null -eq $tpmInfo) { $tpm2 = $false }
  else { $tpm2 = (($tpmInfo.SpecVersion -split ',')[0].Trim() -eq '2.0') }
} catch {}
[pscustomobject]@{
  manufacturer = $cs.Manufacturer
  model = $cs.Model
  firmware = $bios.SMBIOSBIOSVersion
  cpu = $cpu.Name
  memory = [string]$cs.TotalPhysicalMemory
  secure_boot = $secureBoot
  tpm2 = $tpm2
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
    let secure_boot = value.get("secure_boot").and_then(Value::as_bool);
    let tpm2 = value.get("tpm2").and_then(Value::as_bool);
    let mut warnings = vec![
        "Run the Andromeda installer preflight for driver-level Windows device inventory.".into(),
    ];
    if secure_boot.is_none() {
        warnings.push(
            "Confirm-SecureBootUEFI requires an elevated PowerShell session; UEFI and Secure Boot state are unknown on unelevated runs (or on legacy BIOS firmware).".into(),
        );
    }
    if tpm2.is_none() {
        warnings.push(
            "The Win32_Tpm query requires an elevated PowerShell session; TPM 2.0 presence is unknown on unelevated runs.".into(),
        );
    }
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
            // Confirm-SecureBootUEFI only produces a boolean on UEFI
            // firmware (it throws "not supported on this platform" on
            // legacy BIOS), so ANY boolean answer - true or false - proves
            // the machine booted via UEFI; hence `map(|_| true)`. When the
            // answer is null the cmdlet failed (legacy BIOS or missing
            // elevation) and UEFI stays unknown rather than false.
            uefi: secure_boot.map(|_| true),
            secure_boot,
            tpm2,
            virtualization: value.get("virtualization").and_then(Value::as_bool),
        },
        devices: Vec::new(),
        warnings,
    })
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|item| !item.is_empty())
}
