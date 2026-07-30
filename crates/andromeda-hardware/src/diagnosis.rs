use crate::{DeviceInfo, HardwareReport, OsFamily};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareReadiness {
    Ready,
    NeedsReview,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCategory {
    Storage,
    Network,
    Graphics,
    Audio,
    UsbController,
    Input,
    Camera,
    Wireless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSupport {
    Ready,
    Limited,
    MissingDriver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFinding {
    pub bus: String,
    pub address: Option<String>,
    pub category: DeviceCategory,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub driver: Option<String>,
    pub support: DeviceSupport,
    pub boot_critical: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareDiagnosis {
    pub schema_version: u32,
    pub report_schema_version: u32,
    pub readiness: HardwareReadiness,
    pub relevant_devices: usize,
    pub ready_devices: usize,
    pub limited_devices: usize,
    pub missing_driver_devices: usize,
    pub boot_critical_missing: usize,
    #[serde(default)]
    pub findings: Vec<DeviceFinding>,
    #[serde(default)]
    pub recommendations: Vec<String>,
}

impl HardwareDiagnosis {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

#[must_use]
pub fn diagnose_report(report: &HardwareReport) -> HardwareDiagnosis {
    let findings: Vec<_> = report.devices.iter().filter_map(classify_device).collect();
    let ready_devices = count_support(&findings, DeviceSupport::Ready);
    let limited_devices = count_support(&findings, DeviceSupport::Limited);
    let missing_driver_devices = count_support(&findings, DeviceSupport::MissingDriver);
    let boot_critical_missing = findings
        .iter()
        .filter(|finding| finding.boot_critical && finding.support == DeviceSupport::MissingDriver)
        .count();

    let readiness = if boot_critical_missing > 0 {
        HardwareReadiness::Blocked
    } else if findings.is_empty() || limited_devices > 0 || missing_driver_devices > 0 {
        HardwareReadiness::NeedsReview
    } else {
        HardwareReadiness::Ready
    };

    HardwareDiagnosis {
        schema_version: HardwareDiagnosis::CURRENT_SCHEMA_VERSION,
        report_schema_version: report.schema_version,
        readiness,
        relevant_devices: findings.len(),
        ready_devices,
        limited_devices,
        missing_driver_devices,
        boot_critical_missing,
        recommendations: recommendations(report, &findings),
        findings,
    }
}

fn count_support(findings: &[DeviceFinding], support: DeviceSupport) -> usize {
    findings
        .iter()
        .filter(|finding| finding.support == support)
        .count()
}

fn classify_device(device: &DeviceInfo) -> Option<DeviceFinding> {
    let category = if device.bus.eq_ignore_ascii_case("pci") {
        classify_pci(device.class.as_deref()?)
    } else if device.bus.eq_ignore_ascii_case("usb") {
        classify_usb(device.class.as_deref()?)
    } else {
        None
    }?;
    let boot_critical = matches!(
        category,
        DeviceCategory::Storage
            | DeviceCategory::Network
            | DeviceCategory::Graphics
            | DeviceCategory::UsbController
    );
    let nvidia_graphics = category == DeviceCategory::Graphics
        && device
            .vendor_id
            .as_deref()
            .is_some_and(|vendor| id_matches(vendor, "10de"));
    let nouveau = device
        .driver
        .as_deref()
        .is_some_and(|driver| driver.eq_ignore_ascii_case("nouveau"));
    let support = if device.driver.is_none() {
        DeviceSupport::MissingDriver
    } else if nvidia_graphics && nouveau {
        DeviceSupport::Limited
    } else {
        DeviceSupport::Ready
    };
    let summary = match support {
        DeviceSupport::Ready => "A kernel driver is bound to this device.".into(),
        DeviceSupport::Limited => {
            "The open NVIDIA driver is bound; power, compute and game performance require model-specific qualification.".into()
        }
        DeviceSupport::MissingDriver => {
            "No bound kernel driver was detected; do not certify this machine until the driver and firmware path is proven.".into()
        }
    };

    Some(DeviceFinding {
        bus: device.bus.clone(),
        address: device.address.clone(),
        category,
        vendor_id: device.vendor_id.clone(),
        product_id: device.product_id.clone(),
        driver: device.driver.clone(),
        support,
        boot_critical,
        summary,
    })
}

fn classify_pci(class: &str) -> Option<DeviceCategory> {
    let class = parse_hex(class)?;
    let base = (class >> 16) & 0xff;
    let subclass = (class >> 8) & 0xff;
    match (base, subclass) {
        (0x01, _) => Some(DeviceCategory::Storage),
        (0x02, _) => Some(DeviceCategory::Network),
        (0x03, _) => Some(DeviceCategory::Graphics),
        (0x04, _) => Some(DeviceCategory::Audio),
        (0x0c, 0x03) => Some(DeviceCategory::UsbController),
        (0x0d, _) => Some(DeviceCategory::Wireless),
        _ => None,
    }
}

fn classify_usb(class: &str) -> Option<DeviceCategory> {
    match parse_hex(class)? & 0xff {
        0x01 => Some(DeviceCategory::Audio),
        0x03 => Some(DeviceCategory::Input),
        0x0e => Some(DeviceCategory::Camera),
        0xe0 => Some(DeviceCategory::Wireless),
        _ => None,
    }
}

fn parse_hex(value: &str) -> Option<u32> {
    u32::from_str_radix(value.trim().trim_start_matches("0x"), 16).ok()
}

fn id_matches(actual: &str, expected: &str) -> bool {
    actual
        .trim_start_matches("0x")
        .eq_ignore_ascii_case(expected.trim_start_matches("0x"))
}

fn recommendations(report: &HardwareReport, findings: &[DeviceFinding]) -> Vec<String> {
    let mut recommendations = Vec::new();
    if report.os_family != OsFamily::Linux || findings.is_empty() {
        recommendations.push(
            "Run the probe from the installed Andromeda Linux image before making a support claim; the source-OS inventory is migration evidence only.".into(),
        );
    }
    if findings
        .iter()
        .any(|finding| finding.support == DeviceSupport::MissingDriver)
    {
        recommendations.push(
            "Resolve every unbound support-relevant device with a redistributable signed driver and firmware, then attach boot and functional evidence to an exact-machine HCM.".into(),
        );
    }
    if findings.iter().any(|finding| {
        finding.category == DeviceCategory::Graphics
            && finding
                .vendor_id
                .as_deref()
                .is_some_and(|vendor| id_matches(vendor, "10de"))
    }) {
        recommendations.push(
            "NVIDIA systems require a separate HCM: Nouveau/NVK remains Community until proven, while the proprietary stack needs licensed distribution, kernel-module signing and Secure Boot tests.".into(),
        );
    }
    if findings.iter().any(|finding| {
        finding.category == DeviceCategory::Network
            && finding
                .vendor_id
                .as_deref()
                .is_some_and(|vendor| id_matches(vendor, "14e4"))
            && finding.support == DeviceSupport::MissingDriver
    }) {
        recommendations.push(
            "This Broadcom network device has no bound driver; use a model-specific, legally redistributable extension instead of weakening the base image.".into(),
        );
    }
    recommendations
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::{BootInfo, CpuInfo, HardwareIdentity, MemoryInfo, OsFamily};

    use super::*;

    fn report(devices: Vec<DeviceInfo>) -> HardwareReport {
        HardwareReport {
            schema_version: 1,
            collected_at: Utc::now(),
            os_family: OsFamily::Linux,
            identity: HardwareIdentity {
                manufacturer: None,
                model: None,
                board: None,
                firmware_version: None,
            },
            cpu: CpuInfo {
                architecture: "x86_64".into(),
                model: None,
                logical_cores: 4,
            },
            memory: MemoryInfo { bytes: None },
            boot: BootInfo {
                uefi: Some(true),
                secure_boot: Some(false),
                tpm2: false,
                virtualization: true,
            },
            devices,
            warnings: Vec::new(),
        }
    }

    fn pci(class: &str, vendor: &str, driver: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            bus: "pci".into(),
            address: Some("0000:01:00.0".into()),
            vendor_id: Some(vendor.into()),
            product_id: Some("0001".into()),
            subsystem_vendor_id: None,
            subsystem_product_id: None,
            revision: None,
            class: Some(class.into()),
            driver: driver.map(str::to_owned),
            name: None,
            modalias: None,
        }
    }

    #[test]
    fn unbound_storage_blocks_support() {
        let diagnosis = diagnose_report(&report(vec![pci("0x010802", "0x8086", None)]));
        assert_eq!(diagnosis.readiness, HardwareReadiness::Blocked);
        assert_eq!(diagnosis.boot_critical_missing, 1);
    }

    #[test]
    fn bound_common_devices_are_ready() {
        let diagnosis = diagnose_report(&report(vec![
            pci("0x020000", "0x8086", Some("e1000e")),
            pci("0x030000", "0x1002", Some("amdgpu")),
        ]));
        assert_eq!(diagnosis.readiness, HardwareReadiness::Ready);
        assert_eq!(diagnosis.ready_devices, 2);
    }

    #[test]
    fn nouveau_requires_model_specific_review() {
        let diagnosis = diagnose_report(&report(vec![pci("0x030000", "0x10de", Some("nouveau"))]));
        assert_eq!(diagnosis.readiness, HardwareReadiness::NeedsReview);
        assert_eq!(diagnosis.limited_devices, 1);
        assert!(
            diagnosis
                .recommendations
                .iter()
                .any(|item| item.contains("NVIDIA"))
        );
    }

    #[test]
    fn empty_inventory_never_claims_ready() {
        let diagnosis = diagnose_report(&report(Vec::new()));
        assert_eq!(diagnosis.readiness, HardwareReadiness::NeedsReview);
    }
}
