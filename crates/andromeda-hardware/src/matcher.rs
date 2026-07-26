use crate::{
    CapabilityRequirement, CompatibilityEvaluation, HardwareReport, HardwareSelector, HcmManifest,
    SupportTier,
};

#[must_use]
pub fn evaluate_manifest(
    report: &HardwareReport,
    manifest: &HcmManifest,
) -> CompatibilityEvaluation {
    let selector_matched = !manifest.selectors.is_empty()
        && manifest
            .selectors
            .iter()
            .any(|selector| selector_matches(report, selector));
    let mut evidence = Vec::new();
    let mut missing = Vec::new();

    if selector_matched {
        evidence.push("hardware identity matched a manifest selector".into());
    } else {
        missing.push("hardware identity did not match any manifest selector".into());
    }

    for requirement in &manifest.requirements {
        evaluate_requirement(report, requirement, &mut evidence, &mut missing);
    }

    let requirements_met = missing.is_empty();
    CompatibilityEvaluation {
        manifest_id: manifest.id.clone(),
        selector_matched,
        requirements_met,
        declared_tier: manifest.tier,
        effective_tier: if selector_matched && requirements_met {
            manifest.tier
        } else {
            SupportTier::Blocked
        },
        evidence,
        missing,
    }
}

fn evaluate_requirement(
    report: &HardwareReport,
    requirement: &CapabilityRequirement,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    match requirement {
        CapabilityRequirement::MemoryBytes { minimum } => match report.memory.bytes {
            Some(bytes) if bytes >= *minimum => {
                evidence.push(format!("memory {bytes} >= required {minimum}"));
            }
            Some(bytes) => missing.push(format!("memory {bytes} < required {minimum}")),
            None => missing.push("memory size could not be verified".into()),
        },
        CapabilityRequirement::Boot {
            uefi,
            secure_boot,
            tpm2,
            virtualization,
        } => {
            check_optional_bool("UEFI", report.boot.uefi, *uefi, evidence, missing);
            check_optional_bool(
                "Secure Boot",
                report.boot.secure_boot,
                *secure_boot,
                evidence,
                missing,
            );
            check_bool("TPM 2", report.boot.tpm2, *tpm2, evidence, missing);
            check_bool(
                "virtualization",
                report.boot.virtualization,
                *virtualization,
                evidence,
                missing,
            );
        }
        CapabilityRequirement::Device {
            bus,
            vendor_id,
            product_id,
            driver_required,
        } => evaluate_device(
            report,
            bus,
            vendor_id.as_deref(),
            product_id.as_deref(),
            *driver_required,
            evidence,
            missing,
        ),
    }
}

fn evaluate_device(
    report: &HardwareReport,
    bus: &str,
    vendor_id: Option<&str>,
    product_id: Option<&str>,
    driver_required: bool,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    let matched = report.devices.iter().find(|device| {
        device.bus.eq_ignore_ascii_case(bus)
            && vendor_id.is_none_or(|expected| {
                device
                    .vendor_id
                    .as_ref()
                    .is_some_and(|actual| id_matches(actual, expected))
            })
            && product_id.is_none_or(|expected| {
                device
                    .product_id
                    .as_ref()
                    .is_some_and(|actual| id_matches(actual, expected))
            })
    });
    match matched {
        Some(device) if !driver_required || device.driver.is_some() => evidence.push(format!(
            "{} device {}:{} matched",
            bus,
            vendor_id.unwrap_or("*"),
            product_id.unwrap_or("*")
        )),
        Some(_) => missing.push(format!(
            "{bus} device matched but no bound driver was detected"
        )),
        None => missing.push(format!(
            "{} device {}:{} was not detected",
            bus,
            vendor_id.unwrap_or("*"),
            product_id.unwrap_or("*")
        )),
    }
}

fn selector_matches(report: &HardwareReport, selector: &HardwareSelector) -> bool {
    selector
        .os_family
        .is_none_or(|expected| expected == report.os_family)
        && (selector.architectures.is_empty()
            || selector
                .architectures
                .iter()
                .any(|expected| expected.eq_ignore_ascii_case(&report.cpu.architecture)))
        && selector
            .manufacturer_contains
            .as_ref()
            .is_none_or(|needle| {
                report
                    .identity
                    .manufacturer
                    .as_ref()
                    .is_some_and(|value| contains_case_insensitive(value, needle))
            })
        && selector.model_prefix.as_ref().is_none_or(|prefix| {
            report
                .identity
                .model
                .as_ref()
                .is_some_and(|value| starts_with_case_insensitive(value, prefix))
        })
}

fn check_optional_bool(
    name: &str,
    actual: Option<bool>,
    expected: Option<bool>,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    let Some(expected) = expected else {
        return;
    };
    match actual {
        Some(actual) if actual == expected => evidence.push(format!("{name} = {actual}")),
        Some(actual) => missing.push(format!("{name} = {actual}, expected {expected}")),
        None => missing.push(format!("{name} state could not be verified")),
    }
}

fn check_bool(
    name: &str,
    actual: bool,
    expected: Option<bool>,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    if let Some(expected) = expected {
        if actual == expected {
            evidence.push(format!("{name} = {actual}"));
        } else {
            missing.push(format!("{name} = {actual}, expected {expected}"));
        }
    }
}

fn id_matches(actual: &str, expected: &str) -> bool {
    actual
        .trim_start_matches("0x")
        .eq_ignore_ascii_case(expected.trim_start_matches("0x"))
}

fn contains_case_insensitive(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(&needle.to_lowercase())
}

fn starts_with_case_insensitive(value: &str, prefix: &str) -> bool {
    value.to_lowercase().starts_with(&prefix.to_lowercase())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::{
        BootInfo, CpuInfo, HardwareIdentity, HardwareReport, HardwareSelector, MemoryInfo,
        OsFamily, SupportTier,
    };

    use super::*;

    fn report() -> HardwareReport {
        HardwareReport {
            schema_version: 1,
            collected_at: Utc::now(),
            os_family: OsFamily::Linux,
            identity: HardwareIdentity {
                manufacturer: Some("Andromeda Labs".into()),
                model: Some("Reference PC 1".into()),
                board: None,
                firmware_version: None,
            },
            cpu: CpuInfo {
                architecture: "x86_64".into(),
                model: None,
                logical_cores: 8,
            },
            memory: MemoryInfo {
                bytes: Some(16 * 1024 * 1024 * 1024),
            },
            boot: BootInfo {
                uefi: Some(true),
                secure_boot: Some(true),
                tpm2: true,
                virtualization: true,
            },
            devices: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn manifest() -> HcmManifest {
        HcmManifest {
            schema_version: 1,
            id: "reference-pc".into(),
            name: "Reference PC".into(),
            tier: SupportTier::Reference,
            selectors: vec![HardwareSelector {
                os_family: Some(OsFamily::Linux),
                architectures: vec!["x86_64".into()],
                manufacturer_contains: Some("andromeda".into()),
                model_prefix: Some("reference pc".into()),
            }],
            requirements: vec![
                CapabilityRequirement::MemoryBytes {
                    minimum: 8 * 1024 * 1024 * 1024,
                },
                CapabilityRequirement::Boot {
                    uefi: Some(true),
                    secure_boot: Some(true),
                    tpm2: Some(true),
                    virtualization: Some(true),
                },
            ],
            kernel_channels: vec!["stable".into()],
            notes: Vec::new(),
        }
    }

    #[test]
    fn matching_hardware_retains_declared_tier() {
        let evaluation = evaluate_manifest(&report(), &manifest());
        assert_eq!(evaluation.effective_tier, SupportTier::Reference);
        assert!(evaluation.requirements_met);
    }

    #[test]
    fn missing_required_capability_blocks_support() {
        let mut report = report();
        report.boot.virtualization = false;
        let evaluation = evaluate_manifest(&report, &manifest());
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("virtualization"))
        );
    }

    #[test]
    fn empty_selector_list_never_matches_everything() {
        let mut manifest = manifest();
        manifest.selectors.clear();
        assert!(!evaluate_manifest(&report(), &manifest).selector_matched);
    }

    #[test]
    fn repository_example_manifest_matches_the_runtime_schema() {
        let manifest: HcmManifest = serde_json::from_str(include_str!(
            "../../../examples/hcm/developer-x86_64-pc.json"
        ))
        .expect("example manifest");
        assert_eq!(manifest.schema_version, HcmManifest::CURRENT_SCHEMA_VERSION);
        assert!(!manifest.selectors.is_empty());
    }
}
