use chrono::{DateTime, Utc};

use crate::{
    CapabilityRequirement, CompatibilityEvaluation, DeviceInfo, EvidenceResult, HardwareReport,
    HardwareSelector, HcmManifest, SupportTier,
};

/// Evaluates a manifest against a report using the current wall clock.
#[must_use]
pub fn evaluate_manifest(
    report: &HardwareReport,
    manifest: &HcmManifest,
) -> CompatibilityEvaluation {
    evaluate_manifest_at(report, manifest, Utc::now())
}

/// Evaluates a manifest against a report at an explicit instant.
///
/// Injecting `now` keeps expiry logic deterministic and testable.
#[must_use]
pub fn evaluate_manifest_at(
    report: &HardwareReport,
    manifest: &HcmManifest,
    now: DateTime<Utc>,
) -> CompatibilityEvaluation {
    let selector_matched = !manifest.selectors.is_empty()
        && manifest
            .selectors
            .iter()
            .any(|selector| selector_matches(report, selector));
    let mut evidence = Vec::new();
    let mut missing = Vec::new();

    if manifest.schema_version == HcmManifest::CURRENT_SCHEMA_VERSION {
        evidence.push(format!(
            "HCM schema version {} is current",
            manifest.schema_version
        ));
    } else {
        missing.push(format!(
            "HCM schema version {} is unsupported; expected {}",
            manifest.schema_version,
            HcmManifest::CURRENT_SCHEMA_VERSION
        ));
    }

    if selector_matched {
        evidence.push("hardware identity matched a manifest selector".into());
    } else {
        missing.push("hardware identity did not match any manifest selector".into());
    }

    evidence.push(format!(
        "boot provider '{}' is declared by the manifest; the installer preflight, not this matcher, enforces it against the target platform",
        json_name(&manifest.boot_provider)
    ));

    for requirement in &manifest.requirements {
        evaluate_requirement(report, requirement, &mut evidence, &mut missing);
    }
    evaluate_manifest_evidence(manifest, now, &mut evidence, &mut missing);

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
        boot_provider: manifest.boot_provider,
        evidence,
        missing,
    }
}

fn json_name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
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
            check_optional_bool("TPM 2", report.boot.tpm2, *tpm2, evidence, missing);
            check_optional_bool(
                "virtualization",
                report.boot.virtualization,
                *virtualization,
                evidence,
                missing,
            );
        }
        requirement @ CapabilityRequirement::Device { .. } => {
            evaluate_device(report, requirement, evidence, missing);
        }
    }
}

fn evaluate_device(
    report: &HardwareReport,
    requirement: &CapabilityRequirement,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    let CapabilityRequirement::Device {
        bus,
        vendor_id,
        product_id,
        subsystem_vendor_id,
        subsystem_product_id,
        revision,
        driver_required,
    } = requirement
    else {
        return;
    };
    let vendor_id = vendor_id.as_deref();
    let product_id = product_id.as_deref();
    let subsystem_vendor_id = subsystem_vendor_id.as_deref();
    let subsystem_product_id = subsystem_product_id.as_deref();
    let revision = revision.as_deref();
    let id_matches_requirement = |device: &&DeviceInfo| {
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
            && subsystem_vendor_id.is_none_or(|expected| {
                device
                    .subsystem_vendor_id
                    .as_ref()
                    .is_some_and(|actual| id_matches(actual, expected))
            })
            && subsystem_product_id.is_none_or(|expected| {
                device
                    .subsystem_product_id
                    .as_ref()
                    .is_some_and(|actual| id_matches(actual, expected))
            })
            && revision.is_none_or(|expected| {
                device
                    .revision
                    .as_ref()
                    .is_some_and(|actual| id_matches(actual, expected))
            })
    };
    // The requirement is satisfied by ANY matching device that also satisfies
    // the driver condition. An unbound twin earlier in the inventory (dual
    // NIC/GPU, or several USB functions of one device) must not mask a bound
    // one, so the driver condition is part of the search, not a post-check.
    let mut any_id_match = false;
    let satisfied = report
        .devices
        .iter()
        .filter(id_matches_requirement)
        .inspect(|_| any_id_match = true)
        .any(|device| !*driver_required || device.driver.is_some());
    if satisfied {
        evidence.push(format!(
            "{} device {}:{} matched",
            bus,
            vendor_id.unwrap_or("*"),
            product_id.unwrap_or("*")
        ));
    } else if any_id_match {
        missing.push(format!(
            "{bus} device matched but no bound driver was detected"
        ));
    } else {
        missing.push(format!(
            "{} device {}:{} was not detected",
            bus,
            vendor_id.unwrap_or("*"),
            product_id.unwrap_or("*")
        ));
    }
}

fn evaluate_manifest_evidence(
    manifest: &HcmManifest,
    now: DateTime<Utc>,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    if manifest
        .expires_at
        .is_some_and(|expiration| expiration <= now)
    {
        missing.push("HCM support declaration has expired".into());
    }

    for item in &manifest.evidence {
        if item.result == EvidenceResult::Failed {
            missing.push(format!(
                "capability evidence '{}' records a failure",
                item.capability
            ));
        } else if item.expires_at <= now {
            missing.push(format!(
                "capability evidence '{}' has expired",
                item.capability
            ));
        } else {
            evidence.push(format!(
                "capability evidence '{}' is passed and current",
                item.capability
            ));
        }
    }

    // Explicit tier list instead of an `Ord` comparison: inserting a new
    // variant into `SupportTier` must not silently widen or narrow this gate.
    if matches!(
        manifest.tier,
        SupportTier::Supported | SupportTier::Certified | SupportTier::Reference
    ) {
        if manifest.artifacts.is_empty() {
            missing
                .push("Supported or higher HCM requires pinned driver/firmware artifacts".into());
        }
        if manifest.evidence.is_empty() {
            missing.push("Supported or higher HCM requires current capability evidence".into());
        }
        match manifest.expires_at {
            Some(expiration) if expiration > now => {
                evidence.push(format!(
                    "HCM support declaration is current until {expiration}"
                ));
            }
            Some(_) => {}
            None => missing.push("Supported or higher HCM requires an expiration date".into()),
        }
    }
}

/// A selector must carry at least one non-empty discriminating field.
///
/// Without this rule a selector whose fields are all null/empty would match
/// every machine and grant the manifest tier to arbitrary hardware, defeating
/// the fail-closed design (the JSON schema enforces the same constraint via
/// `anyOf`).
fn selector_is_discriminating(selector: &HardwareSelector) -> bool {
    selector.os_family.is_some()
        || selector
            .architectures
            .iter()
            .any(|architecture| !architecture.trim().is_empty())
        || selector
            .manufacturer_contains
            .as_deref()
            .is_some_and(|needle| !needle.trim().is_empty())
        || selector
            .model_prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.trim().is_empty())
}

fn selector_matches(report: &HardwareReport, selector: &HardwareSelector) -> bool {
    selector_is_discriminating(selector)
        && selector
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

/// Strips at most one leading `0x`/`0X` prefix (case-insensitive).
///
/// `trim_start_matches` would strip repeated prefixes, silently equating
/// `0x0x10de` with `10de`.
fn strip_hex_prefix(value: &str) -> &str {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
}

fn id_matches(actual: &str, expected: &str) -> bool {
    strip_hex_prefix(actual).eq_ignore_ascii_case(strip_hex_prefix(expected))
}

fn contains_case_insensitive(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(&needle.to_lowercase())
}

fn starts_with_case_insensitive(value: &str, prefix: &str) -> bool {
    value.to_lowercase().starts_with(&prefix.to_lowercase())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use crate::{
        BootInfo, CpuInfo, DeviceInfo, HardwareIdentity, HardwareReport, HardwareSelector,
        MemoryInfo, OsFamily, SupportTier,
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
                tpm2: Some(true),
                virtualization: Some(true),
            },
            devices: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn device(driver: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            bus: "pci".into(),
            address: Some("0000:01:00.0".into()),
            vendor_id: Some("0x8086".into()),
            product_id: Some("0x1521".into()),
            subsystem_vendor_id: Some("0x1043".into()),
            subsystem_product_id: Some("0x85f0".into()),
            revision: Some("0x03".into()),
            class: Some("0x020000".into()),
            driver: driver.map(str::to_owned),
            name: None,
            modalias: None,
        }
    }

    fn manifest() -> HcmManifest {
        HcmManifest {
            schema_version: HcmManifest::CURRENT_SCHEMA_VERSION,
            id: "reference-pc".into(),
            name: "Reference PC".into(),
            tier: SupportTier::Reference,
            boot_provider: crate::BootProvider::PcUefiShim,
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
            artifacts: vec![crate::ArtifactPin {
                kind: crate::ArtifactKind::Kernel,
                name: "kernel".into(),
                version: "test".into(),
                sha256: "0".repeat(64),
                source: "test".into(),
                signing_key_id: Some("test-key".into()),
            }],
            evidence: vec![crate::CapabilityEvidence {
                capability: "boot".into(),
                result: crate::EvidenceResult::Passed,
                evidence_uri: "https://example.invalid/evidence".into(),
                collected_at: Utc::now(),
                expires_at: Utc::now() + Duration::days(1),
            }],
            expires_at: Some(Utc::now() + Duration::days(1)),
            notes: Vec::new(),
        }
    }

    fn device_requirement(driver_required: bool) -> CapabilityRequirement {
        CapabilityRequirement::Device {
            bus: "pci".into(),
            vendor_id: Some("8086".into()),
            product_id: Some("1521".into()),
            subsystem_vendor_id: None,
            subsystem_product_id: None,
            revision: None,
            driver_required,
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
        report.boot.virtualization = Some(false);
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
    fn unverified_boot_capability_blocks_but_names_the_uncertainty() {
        let mut report = report();
        report.boot.tpm2 = None;
        let evaluation = evaluate_manifest(&report, &manifest());
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("TPM 2 state could not be verified"))
        );
    }

    #[test]
    fn empty_selector_list_never_matches_everything() {
        let mut manifest = manifest();
        manifest.selectors.clear();
        assert!(!evaluate_manifest(&report(), &manifest).selector_matched);
    }

    /// Regression: a selector whose fields are all null/empty must never
    /// match, otherwise it would grant an arbitrary tier to every machine.
    #[test]
    fn all_null_selector_never_matches() {
        let manifest: HcmManifest = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "id": "wildcard",
                "name": "Wildcard",
                "tier": "certified",
                "boot_provider": "pc_uefi_shim",
                "selectors": [
                    {
                        "os_family": null,
                        "architectures": [],
                        "manufacturer_contains": null,
                        "model_prefix": null
                    }
                ],
                "requirements": [],
                "kernel_channels": [],
                "artifacts": [],
                "evidence": [],
                "expires_at": null,
                "notes": []
            }"#,
        )
        .expect("all-null selector manifest");
        let evaluation = evaluate_manifest(&report(), &manifest);
        assert!(!evaluation.selector_matched);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
    }

    #[test]
    fn whitespace_only_selector_fields_are_not_discriminating() {
        let mut manifest = manifest();
        manifest.selectors = vec![HardwareSelector {
            os_family: None,
            architectures: vec![" ".into()],
            manufacturer_contains: Some(String::new()),
            model_prefix: Some("  ".into()),
        }];
        assert!(!evaluate_manifest(&report(), &manifest).selector_matched);
    }

    /// A second identical device with a bound driver must satisfy the
    /// requirement even when the first match has no driver (dual NIC/GPU).
    #[test]
    fn bound_twin_device_satisfies_driver_requirement() {
        let mut report = report();
        report.devices = vec![device(None), device(Some("igb"))];
        let mut manifest = manifest();
        manifest.requirements = vec![device_requirement(true)];
        let evaluation = evaluate_manifest(&report, &manifest);
        assert!(evaluation.requirements_met, "{:?}", evaluation.missing);
        assert_eq!(evaluation.effective_tier, SupportTier::Reference);
    }

    #[test]
    fn unbound_only_device_reports_missing_driver() {
        let mut report = report();
        report.devices = vec![device(None)];
        let mut manifest = manifest();
        manifest.requirements = vec![device_requirement(true)];
        let evaluation = evaluate_manifest(&report, &manifest);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("no bound driver"))
        );
    }

    #[test]
    fn subsystem_and_revision_ids_participate_in_matching() {
        let mut report = report();
        report.devices = vec![device(Some("igb"))];
        let mut manifest = manifest();
        manifest.requirements = vec![CapabilityRequirement::Device {
            bus: "pci".into(),
            vendor_id: Some("8086".into()),
            product_id: Some("1521".into()),
            subsystem_vendor_id: Some("1043".into()),
            subsystem_product_id: Some("85F0".into()),
            revision: Some("03".into()),
            driver_required: true,
        }];
        assert!(evaluate_manifest(&report, &manifest).requirements_met);

        let CapabilityRequirement::Device { revision, .. } = &mut manifest.requirements[0] else {
            unreachable!();
        };
        *revision = Some("04".into());
        let evaluation = evaluate_manifest(&report, &manifest);
        assert!(!evaluation.requirements_met);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("was not detected"))
        );
    }

    #[test]
    fn expired_capability_evidence_blocks_support() {
        let manifest = manifest();
        let now = Utc::now() + Duration::days(2);
        let evaluation = evaluate_manifest_at(&report(), &manifest, now);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("capability evidence 'boot' has expired"))
        );
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("HCM support declaration has expired"))
        );
    }

    #[test]
    fn failed_capability_evidence_blocks_support() {
        let mut manifest = manifest();
        manifest.evidence[0].result = crate::EvidenceResult::Failed;
        let evaluation = evaluate_manifest(&report(), &manifest);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("records a failure"))
        );
    }

    #[test]
    fn id_comparison_strips_one_hex_prefix_case_insensitively() {
        assert!(id_matches("0X10DE", "10de"));
        assert!(id_matches("0x10de", "0X10DE"));
        assert!(id_matches("10DE", "10de"));
        assert!(
            !id_matches("0x0x10de", "10de"),
            "repeated prefixes must not be stripped"
        );
    }

    #[test]
    fn boot_provider_is_surfaced_in_the_evaluation() {
        let evaluation = evaluate_manifest(&report(), &manifest());
        assert_eq!(evaluation.boot_provider, crate::BootProvider::PcUefiShim);
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("boot provider 'pc_uefi_shim'")
                    && item.contains("installer preflight"))
        );
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

    /// Keeps the JSON schema and the serde model on the same convention:
    /// fields the schema leaves optional must deserialize with defaults, and
    /// a fully populated document must round-trip.
    #[test]
    fn schema_minimal_and_schema_full_documents_deserialize() {
        let minimal: HcmManifest = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "id": "minimal",
                "name": "Minimal",
                "tier": "community",
                "boot_provider": "pc_uefi_shim",
                "selectors": [{"os_family": "linux"}]
            }"#,
        )
        .expect("schema-minimal manifest");
        assert!(minimal.requirements.is_empty());
        assert!(minimal.artifacts.is_empty());
        assert!(minimal.evidence.is_empty());
        assert!(minimal.expires_at.is_none());
        assert_eq!(minimal.selectors[0].os_family, Some(OsFamily::Linux));
        assert!(minimal.selectors[0].architectures.is_empty());

        let full: HcmManifest = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "id": "full",
                "name": "Full",
                "tier": "supported",
                "boot_provider": "pc_uefi_shim",
                "selectors": [
                    {
                        "os_family": "linux",
                        "architectures": ["x86_64"],
                        "manufacturer_contains": "vendor",
                        "model_prefix": "model"
                    }
                ],
                "requirements": [
                    {"type": "memory_bytes", "minimum": 1},
                    {
                        "type": "boot",
                        "uefi": true,
                        "secure_boot": null,
                        "tpm2": null,
                        "virtualization": null
                    },
                    {
                        "type": "device",
                        "bus": "pci",
                        "vendor_id": "8086",
                        "product_id": "1521",
                        "subsystem_vendor_id": "1043",
                        "subsystem_product_id": "85f0",
                        "revision": "03",
                        "driver_required": true
                    }
                ],
                "kernel_channels": ["stable"],
                "artifacts": [
                    {
                        "kind": "kernel",
                        "name": "kernel",
                        "version": "1",
                        "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                        "source": "registry",
                        "signing_key_id": null
                    }
                ],
                "evidence": [
                    {
                        "capability": "boot",
                        "result": "passed",
                        "evidence_uri": "https://example.invalid/e",
                        "collected_at": "2026-01-01T00:00:00Z",
                        "expires_at": "2027-01-01T00:00:00Z"
                    }
                ],
                "expires_at": "2027-01-01T00:00:00Z",
                "notes": ["note"]
            }"#,
        )
        .expect("schema-full manifest");
        assert_eq!(full.requirements.len(), 3);
        assert_eq!(full.artifacts.len(), 1);
        assert_eq!(full.evidence.len(), 1);
    }

    #[test]
    fn supported_manifest_without_pins_or_evidence_is_blocked() {
        let mut manifest = manifest();
        manifest.tier = SupportTier::Supported;
        manifest.artifacts.clear();
        manifest.evidence.clear();
        manifest.expires_at = None;
        let evaluation = evaluate_manifest(&report(), &manifest);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("pinned"))
        );
    }

    #[test]
    fn reference_tier_requires_pins_and_evidence_like_supported() {
        let mut manifest = manifest();
        manifest.tier = SupportTier::Reference;
        manifest.artifacts.clear();
        manifest.evidence.clear();
        let evaluation = evaluate_manifest(&report(), &manifest);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
    }

    #[test]
    fn support_tier_ladder_orders_reference_below_supported() {
        assert!(SupportTier::Blocked < SupportTier::Community);
        assert!(SupportTier::Community < SupportTier::Reference);
        assert!(SupportTier::Reference < SupportTier::Supported);
        assert!(SupportTier::Supported < SupportTier::Certified);
    }
}
