use chrono::{DateTime, Utc};

use crate::model::id_matches;
use crate::signing::{ManifestSignatureStatus, TrustedKeyring, verify_manifest_signature};
use crate::{
    ArtifactPin, CapabilityRequirement, CompatibilityEvaluation, DeviceInfo, EvidenceResult,
    HardwareReport, HardwareSelector, HcmManifest, ManifestAuthenticity, SupportTier,
};

/// Verdict for a single pinned artifact, produced by an [`ArtifactVerifier`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactVerdict {
    /// The artifact resolved and hashed to the pinned `sha256`, and — when the
    /// verifier enforces a trusted-key set — the pin named a trusted
    /// `signing_key_id`.
    Trusted,
    /// The artifact resolved but failed verification (hash mismatch, or an
    /// untrusted/absent signing key). Fail-closed: it drives `effective_tier`
    /// to `Blocked`.
    Rejected(String),
    /// The artifact could not be resolved from the configured store. When a
    /// verifier is supplied this is fail-closed: every declared pin must be
    /// resolvable, so it drives `effective_tier` to `Blocked`.
    Unresolved(String),
}

/// Resolves and verifies the real bytes behind an [`ArtifactPin`].
///
/// The matcher trusts declared `sha256` values only when **no** verifier is
/// supplied — the historical, advisory behavior, kept so existing callers do
/// not change. Supplying a verifier makes the trust boundary explicit and
/// fail-closed: every pinned artifact must resolve, hash to its pin, and —
/// when a trusted-key set is configured — name a trusted `signing_key_id`, or
/// the manifest is driven to `Blocked`.
///
/// The actual signature cryptography is a documented TODO: today a verifier
/// checks `signing_key_id` membership in its trusted-key set, not a real
/// detached signature. See [`crate::DirectoryArtifactVerifier`].
pub trait ArtifactVerifier {
    /// Resolves `artifact` and reports whether its bytes and signing key match
    /// the pin.
    fn verify(&self, artifact: &ArtifactPin) -> ArtifactVerdict;
}

/// Evaluates a manifest against a report using the current wall clock and
/// **no** artifact verification (declared hashes are trusted as-is).
#[must_use]
pub fn evaluate_manifest(
    report: &HardwareReport,
    manifest: &HcmManifest,
) -> CompatibilityEvaluation {
    evaluate_manifest_at(report, manifest, Utc::now())
}

/// Evaluates a manifest against a report at an explicit instant, with **no**
/// artifact verification. Injecting `now` keeps expiry logic deterministic.
#[must_use]
pub fn evaluate_manifest_at(
    report: &HardwareReport,
    manifest: &HcmManifest,
    now: DateTime<Utc>,
) -> CompatibilityEvaluation {
    evaluate_manifest_at_with_verifier(report, manifest, now, None)
}

/// Evaluates a manifest using the current wall clock and an optional artifact
/// verifier. See [`evaluate_manifest_at_with_verifier`].
#[must_use]
pub fn evaluate_manifest_with_verifier(
    report: &HardwareReport,
    manifest: &HcmManifest,
    verifier: Option<&dyn ArtifactVerifier>,
) -> CompatibilityEvaluation {
    evaluate_manifest_at_with_verifier(report, manifest, Utc::now(), verifier)
}

/// Evaluates a manifest against a report at an explicit instant, optionally
/// verifying pinned artifacts.
///
/// When `verifier` is `None` the declared artifact hashes are trusted (the
/// historical behavior); the trust boundary is explicit because callers opt
/// into verification by supplying an [`ArtifactVerifier`]. When a verifier is
/// supplied, every pinned artifact must resolve and match, or `effective_tier`
/// is driven to `Blocked`.
///
/// This entry point performs **no** authenticity check on the manifest itself;
/// for that, supply a [`TrustedKeyring`] via [`evaluate_manifest_verified`].
#[must_use]
pub fn evaluate_manifest_at_with_verifier(
    report: &HardwareReport,
    manifest: &HcmManifest,
    now: DateTime<Utc>,
    verifier: Option<&dyn ArtifactVerifier>,
) -> CompatibilityEvaluation {
    evaluate(report, manifest, now, None, verifier)
}

/// Evaluates a manifest against a report using the current wall clock while
/// enforcing manifest authenticity against `keyring`. See
/// [`evaluate_manifest_at_verified`].
#[must_use]
pub fn evaluate_manifest_verified(
    report: &HardwareReport,
    manifest: &HcmManifest,
    keyring: &TrustedKeyring,
    verifier: Option<&dyn ArtifactVerifier>,
) -> CompatibilityEvaluation {
    evaluate(report, manifest, Utc::now(), Some(keyring), verifier)
}

/// Evaluates a manifest against a report at an explicit instant, **fail-closed
/// on authenticity**: the manifest must carry a detached ed25519 signature that
/// resolves to a trusted key in `keyring` and verifies over the manifest's
/// canonical bytes, or `effective_tier` is driven to `Blocked`.
///
/// This is the gate that closes security review finding #1. With a keyring:
///
/// - an unsigned manifest, an unknown `key_id`, a malformed signature, or a
///   signature that does not verify all fail closed;
/// - additionally, every pinned [`ArtifactPin`] must name a `signing_key_id`
///   that is itself in the keyring — the manifest signature authenticates the
///   pinned digests, and the keyring is the single set of trusted signing
///   identities for both the manifest and its artifacts.
///
/// The optional `verifier` is orthogonal: it confirms that the *local* artifact
/// bytes hash to the (now authenticated) pinned `sha256`.
///
/// Passing the keyring-less entry points (e.g. [`evaluate_manifest`]) keeps the
/// historical advisory behavior: no signature is required or checked.
#[must_use]
pub fn evaluate_manifest_at_verified(
    report: &HardwareReport,
    manifest: &HcmManifest,
    now: DateTime<Utc>,
    keyring: &TrustedKeyring,
    verifier: Option<&dyn ArtifactVerifier>,
) -> CompatibilityEvaluation {
    evaluate(report, manifest, now, Some(keyring), verifier)
}

/// The shared evaluation core. `keyring` is `Some` only on the authenticity-
/// enforcing entry points; when `None`, no signature is required or checked and
/// behavior is identical to the historical matcher.
#[must_use]
fn evaluate(
    report: &HardwareReport,
    manifest: &HcmManifest,
    now: DateTime<Utc>,
    keyring: Option<&TrustedKeyring>,
    verifier: Option<&dyn ArtifactVerifier>,
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
    let manifest_authenticity = match keyring {
        Some(keyring) => evaluate_signature(manifest, keyring, &mut evidence, &mut missing),
        None => ManifestAuthenticity::NotChecked,
    };
    evaluate_artifacts(manifest, keyring, verifier, &mut evidence, &mut missing);

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
        manifest_authenticity,
        evidence,
        missing,
    }
}

/// Turns the manifest signature verdict into evidence or a fail-closed reason,
/// and returns the same verdict as structured [`ManifestAuthenticity`] for the
/// evaluation output.
///
/// Only ever called when a [`TrustedKeyring`] is supplied. Any status other
/// than `Verified` lands in `missing`, which drives `effective_tier` to
/// `Blocked` — an unsigned, unknown-key, malformed, or invalid signature can
/// never retain a declared tier.
fn evaluate_signature(
    manifest: &HcmManifest,
    keyring: &TrustedKeyring,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) -> ManifestAuthenticity {
    // The structured `reason` deliberately equals the `missing` string, so the
    // two views of one verdict can never drift apart.
    let fail = |missing: &mut Vec<String>, reason: String| {
        missing.push(reason.clone());
        ManifestAuthenticity::Failed { reason }
    };
    match verify_manifest_signature(manifest, keyring) {
        ManifestSignatureStatus::Verified { key_id } => {
            evidence.push(format!(
                "manifest signature verified against trusted key '{key_id}'"
            ));
            ManifestAuthenticity::Verified { key_id }
        }
        ManifestSignatureStatus::Unsigned => fail(
            missing,
            "manifest is unsigned but a trusted keyring is configured; authenticity cannot be established"
                .to_owned(),
        ),
        ManifestSignatureStatus::UnknownKey { key_id } => fail(
            missing,
            format!(
                "manifest signature names key '{key_id}', which is not in the trusted keyring"
            ),
        ),
        ManifestSignatureStatus::Malformed { reason } => fail(
            missing,
            format!("manifest signature is malformed: {reason}"),
        ),
        ManifestSignatureStatus::Invalid { key_id, reason } => fail(
            missing,
            format!(
                "manifest signature failed ed25519 verification for key '{key_id}': {reason}"
            ),
        ),
    }
}

/// Verifies pinned artifacts.
///
/// Two independent checks, both fail-closed into `missing`:
///
/// 1. **Identity** — when a `keyring` is supplied, every pin must name a
///    `signing_key_id` that is in the keyring. The verified manifest signature
///    authenticates the pinned digests; this ensures the manifest only vouches
///    for artifacts under keys the operator trusts.
/// 2. **Bytes** — when a `verifier` is supplied, each pin's local bytes must
///    resolve and hash to its `sha256`.
///
/// With neither a keyring nor a verifier the declared hashes are accepted on
/// trust (the historical advisory behavior); the trust boundary is explicit in
/// the API because callers opt into each check.
fn evaluate_artifacts(
    manifest: &HcmManifest,
    keyring: Option<&TrustedKeyring>,
    verifier: Option<&dyn ArtifactVerifier>,
    evidence: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    if let Some(keyring) = keyring {
        for artifact in &manifest.artifacts {
            match artifact.signing_key_id.as_deref() {
                Some(key_id) if keyring.contains(key_id) => evidence.push(format!(
                    "artifact '{}' {} names trusted signing key '{key_id}'",
                    artifact.name, artifact.version
                )),
                Some(key_id) => missing.push(format!(
                    "artifact '{}' {} names signing key '{key_id}', which is not in the trusted keyring",
                    artifact.name, artifact.version
                )),
                None => missing.push(format!(
                    "artifact '{}' {} is unsigned but a trusted keyring is configured",
                    artifact.name, artifact.version
                )),
            }
        }
    }
    let Some(verifier) = verifier else {
        if !manifest.artifacts.is_empty() {
            evidence.push(format!(
                "{} pinned artifact hash(es) trusted as declared; no artifact verifier was configured",
                manifest.artifacts.len()
            ));
        }
        return;
    };
    for artifact in &manifest.artifacts {
        match verifier.verify(artifact) {
            ArtifactVerdict::Trusted => evidence.push(format!(
                "artifact '{}' {} verified against its pinned sha256",
                artifact.name, artifact.version
            )),
            ArtifactVerdict::Rejected(reason) => missing.push(format!(
                "artifact '{}' {} failed verification: {reason}",
                artifact.name, artifact.version
            )),
            ArtifactVerdict::Unresolved(reason) => missing.push(format!(
                "artifact '{}' {} could not be resolved for verification: {reason}",
                artifact.name, artifact.version
            )),
        }
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
    //
    // Explicit two-step (collect id-matches, then check driver presence)
    // rather than an `inspect`-driven side effect: it does not depend on the
    // lazy short-circuit order of `any`, so a future iterator refactor cannot
    // silently break the "matched but no bound driver" branch.
    let id_matched: Vec<&DeviceInfo> = report
        .devices
        .iter()
        .filter(id_matches_requirement)
        .collect();
    let satisfied = id_matched
        .iter()
        .any(|device| !*driver_required || device.driver.is_some());
    if satisfied {
        evidence.push(format!(
            "{} device {}:{} matched",
            bus,
            vendor_id.unwrap_or("*"),
            product_id.unwrap_or("*")
        ));
    } else if !id_matched.is_empty() {
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
            signature: None,
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

    /// A verifier that returns the same verdict for every artifact, so the
    /// matcher's fail-closed wiring can be exercised without touching disk.
    struct FixedVerifier(ArtifactVerdict);

    impl ArtifactVerifier for FixedVerifier {
        fn verify(&self, _artifact: &ArtifactPin) -> ArtifactVerdict {
            self.0.clone()
        }
    }

    #[test]
    fn default_evaluation_trusts_declared_artifact_hashes() {
        // No verifier: even a bogus declared hash keeps the declared tier, and
        // the trust boundary is surfaced explicitly in the evidence.
        let mut manifest = manifest();
        manifest.artifacts[0].sha256 = "deadbeef".repeat(8);
        let evaluation = evaluate_manifest(&report(), &manifest);
        assert_eq!(evaluation.effective_tier, SupportTier::Reference);
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("trusted as declared"))
        );
    }

    #[test]
    fn verified_artifacts_retain_declared_tier() {
        let verifier = FixedVerifier(ArtifactVerdict::Trusted);
        let evaluation = evaluate_manifest_with_verifier(&report(), &manifest(), Some(&verifier));
        assert_eq!(evaluation.effective_tier, SupportTier::Reference);
        assert!(evaluation.requirements_met);
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("verified against its pinned sha256"))
        );
    }

    #[test]
    fn mismatched_artifact_hash_blocks_support() {
        let verifier = FixedVerifier(ArtifactVerdict::Rejected("sha256 mismatch".into()));
        let evaluation = evaluate_manifest_with_verifier(&report(), &manifest(), Some(&verifier));
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("failed verification"))
        );
    }

    #[test]
    fn unresolvable_required_artifact_blocks_support() {
        let verifier = FixedVerifier(ArtifactVerdict::Unresolved("kernel not found".into()));
        let evaluation = evaluate_manifest_with_verifier(&report(), &manifest(), Some(&verifier));
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("could not be resolved"))
        );
    }

    /// A fixed seed keeps manifest-signing tests reproducible without an RNG
    /// (which the CI environment forbids in some contexts). Any 32 bytes work.
    const TEST_SEED: [u8; 32] = [42u8; 32];

    /// Signs `manifest` under `key_id`, aligning every pin's `signing_key_id`
    /// to the same key first (the operator key that signs the manifest also
    /// vouches for its artifacts), and returns a keyring that trusts it.
    fn sign_with_key(manifest: &mut HcmManifest, key_id: &str) -> crate::TrustedKeyring {
        let key = crate::ManifestSigningKey::from_seed(&TEST_SEED);
        for artifact in &mut manifest.artifacts {
            artifact.signing_key_id = Some(key_id.to_owned());
        }
        manifest.signature = Some(key.sign_manifest(manifest, key_id).expect("sign manifest"));
        let mut keyring = crate::TrustedKeyring::new();
        keyring
            .insert_hex(key_id.to_owned(), &key.verifying_key_hex())
            .expect("insert verifying key");
        keyring
    }

    #[test]
    fn signed_manifest_with_trusted_key_retains_tier() {
        let mut manifest = manifest();
        let keyring = sign_with_key(&mut manifest, "prod-key");
        let evaluation =
            evaluate_manifest_at_verified(&report(), &manifest, Utc::now(), &keyring, None);
        assert_eq!(
            evaluation.effective_tier,
            SupportTier::Reference,
            "{:?}",
            evaluation.missing
        );
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("manifest signature verified against trusted key"))
        );
        assert_eq!(
            evaluation.manifest_authenticity,
            ManifestAuthenticity::Verified {
                key_id: "prod-key".into()
            }
        );
    }

    #[test]
    fn tampered_manifest_is_blocked() {
        let mut manifest = manifest();
        let keyring = sign_with_key(&mut manifest, "prod-key");
        // Flip a field after signing: the canonical bytes no longer match.
        manifest.name = "Forged PC".into();
        let evaluation =
            evaluate_manifest_at_verified(&report(), &manifest, Utc::now(), &keyring, None);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("failed ed25519 verification"))
        );
    }

    #[test]
    fn unknown_signing_key_is_blocked() {
        let mut manifest = manifest();
        // Signed by a key the operator's keyring has never heard of.
        let _ = sign_with_key(&mut manifest, "attacker-key");
        let mut keyring = crate::TrustedKeyring::new();
        keyring
            .insert_hex(
                "prod-key".to_owned(),
                &crate::ManifestSigningKey::from_seed(&[7u8; 32]).verifying_key_hex(),
            )
            .expect("insert verifying key");
        let evaluation =
            evaluate_manifest_at_verified(&report(), &manifest, Utc::now(), &keyring, None);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("not in the trusted keyring"))
        );
    }

    #[test]
    fn unsigned_manifest_with_keyring_is_blocked() {
        // Default fixture is unsigned; a configured keyring makes that fatal.
        let manifest = manifest();
        let mut keyring = crate::TrustedKeyring::new();
        keyring
            .insert_hex(
                "prod-key".to_owned(),
                &crate::ManifestSigningKey::from_seed(&TEST_SEED).verifying_key_hex(),
            )
            .expect("insert verifying key");
        let evaluation =
            evaluate_manifest_at_verified(&report(), &manifest, Utc::now(), &keyring, None);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("manifest is unsigned"))
        );
        assert!(matches!(
            &evaluation.manifest_authenticity,
            ManifestAuthenticity::Failed { reason } if reason.contains("unsigned")
        ));
    }

    #[test]
    fn no_keyring_keeps_advisory_behavior_for_unsigned_manifests() {
        // Back-compat: without a keyring an unsigned manifest still retains its
        // declared tier and grows no signature-related reasons.
        let manifest = manifest();
        let evaluation = evaluate_manifest(&report(), &manifest);
        assert_eq!(evaluation.effective_tier, SupportTier::Reference);
        assert!(
            !evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("signature") || reason.contains("unsigned"))
        );
        assert_eq!(
            evaluation.manifest_authenticity,
            ManifestAuthenticity::NotChecked
        );
    }

    /// Consumers gate on `manifest_authenticity` as a structured JSON field
    /// instead of substring-matching evidence strings, so its serialized shape
    /// is contract: internally tagged, `snake_case`.
    #[test]
    fn manifest_authenticity_serializes_as_a_structured_json_field() {
        // Verified.
        let mut signed = manifest();
        let keyring = sign_with_key(&mut signed, "prod-key");
        let evaluation =
            evaluate_manifest_at_verified(&report(), &signed, Utc::now(), &keyring, None);
        let json = serde_json::to_value(&evaluation).expect("serialize evaluation");
        assert_eq!(json["manifest_authenticity"]["type"], "verified");
        assert_eq!(json["manifest_authenticity"]["key_id"], "prod-key");

        // Failed: same keyring, but the manifest is unsigned.
        let evaluation =
            evaluate_manifest_at_verified(&report(), &manifest(), Utc::now(), &keyring, None);
        let json = serde_json::to_value(&evaluation).expect("serialize evaluation");
        assert_eq!(json["manifest_authenticity"]["type"], "failed");
        assert!(
            json["manifest_authenticity"]["reason"]
                .as_str()
                .expect("reason is a string")
                .contains("unsigned")
        );

        // Not checked: the advisory, keyring-less path.
        let evaluation = evaluate_manifest(&report(), &manifest());
        let json = serde_json::to_value(&evaluation).expect("serialize evaluation");
        assert_eq!(json["manifest_authenticity"]["type"], "not_checked");
        assert_eq!(
            json["manifest_authenticity"]
                .as_object()
                .expect("authenticity is an object")
                .len(),
            1,
            "not_checked carries no payload beyond its tag"
        );
    }

    #[test]
    fn artifact_key_outside_keyring_blocks_even_with_valid_manifest_signature() {
        // The manifest signature itself is valid, but a pin names a key the
        // operator does not trust: the artifact-identity gate fails closed.
        let mut manifest = manifest();
        let key = crate::ManifestSigningKey::from_seed(&TEST_SEED);
        manifest.artifacts[0].signing_key_id = Some("rogue-key".into());
        manifest.signature = Some(key.sign_manifest(&manifest, "prod-key").expect("sign"));
        let mut keyring = crate::TrustedKeyring::new();
        keyring
            .insert_hex("prod-key".to_owned(), &key.verifying_key_hex())
            .expect("insert verifying key");
        let evaluation =
            evaluate_manifest_at_verified(&report(), &manifest, Utc::now(), &keyring, None);
        assert_eq!(evaluation.effective_tier, SupportTier::Blocked);
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("manifest signature verified against trusted key")),
            "manifest signature should still verify: {:?}",
            evaluation.missing
        );
        assert!(
            evaluation
                .missing
                .iter()
                .any(|reason| reason.contains("rogue-key")
                    && reason.contains("not in the trusted keyring"))
        );
    }

    #[test]
    fn keyring_and_artifact_verifier_engage_together() {
        let mut manifest = manifest();
        let keyring = sign_with_key(&mut manifest, "prod-key");
        let verifier = FixedVerifier(ArtifactVerdict::Trusted);
        let evaluation = evaluate_manifest_at_verified(
            &report(),
            &manifest,
            Utc::now(),
            &keyring,
            Some(&verifier),
        );
        assert_eq!(
            evaluation.effective_tier,
            SupportTier::Reference,
            "{:?}",
            evaluation.missing
        );
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("verified against its pinned sha256"))
        );
        assert!(
            evaluation
                .evidence
                .iter()
                .any(|item| item.contains("manifest signature verified against trusted key"))
        );
    }

    /// Locks the JSON schema `tier` enum to the `SupportTier` declaration
    /// order in `model.rs`, so future reordering fails CI.
    #[test]
    fn schema_tier_enum_matches_model_declaration_order() {
        let declared = [
            SupportTier::Blocked,
            SupportTier::Community,
            SupportTier::Reference,
            SupportTier::Supported,
            SupportTier::Certified,
        ];
        // Exhaustive match: adding a `SupportTier` variant without listing it
        // here fails to compile, so this guard can never silently miss a tier.
        for tier in declared {
            match tier {
                SupportTier::Blocked
                | SupportTier::Community
                | SupportTier::Reference
                | SupportTier::Supported
                | SupportTier::Certified => {}
            }
        }
        let declared_names: Vec<String> = declared
            .iter()
            .map(|tier| {
                serde_json::to_value(tier)
                    .expect("serialize tier")
                    .as_str()
                    .expect("tier serializes to a string")
                    .to_owned()
            })
            .collect();

        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schemas/hardware-compatibility-manifest.schema.json"
        ))
        .expect("schema parses");
        let schema_names: Vec<String> = schema["properties"]["tier"]["enum"]
            .as_array()
            .expect("schema tier enum is an array")
            .iter()
            .map(|value| value.as_str().expect("enum entry is a string").to_owned())
            .collect();

        assert_eq!(
            schema_names, declared_names,
            "schema tier enum order must equal the SupportTier declaration order in model.rs"
        );
    }

    #[test]
    fn unknown_fields_in_untrusted_manifest_structs_are_rejected() {
        fn rejects_unknown(json: &str) -> bool {
            serde_json::from_str::<HcmManifest>(json)
                .err()
                .is_some_and(|error| error.to_string().contains("unknown field"))
        }
        // Top-level manifest.
        assert!(rejects_unknown(
            r#"{"schema_version":2,"id":"x","name":"X","tier":"community","boot_provider":"pc_uefi_shim","selectors":[{"os_family":"linux"}],"typo":1}"#
        ));
        // Selector.
        assert!(rejects_unknown(
            r#"{"schema_version":2,"id":"x","name":"X","tier":"community","boot_provider":"pc_uefi_shim","selectors":[{"os_family":"linux","typo":1}]}"#
        ));
        // Internally tagged requirement variant.
        assert!(rejects_unknown(
            r#"{"schema_version":2,"id":"x","name":"X","tier":"community","boot_provider":"pc_uefi_shim","selectors":[{"os_family":"linux"}],"requirements":[{"type":"memory_bytes","minimum":1,"typo":2}]}"#
        ));
        // Artifact pin.
        assert!(rejects_unknown(
            r#"{"schema_version":2,"id":"x","name":"X","tier":"community","boot_provider":"pc_uefi_shim","selectors":[{"os_family":"linux"}],"artifacts":[{"kind":"kernel","name":"k","version":"1","sha256":"0000000000000000000000000000000000000000000000000000000000000000","source":"s","typo":1}]}"#
        ));
        // A valid document without extras still parses: the guard is precise,
        // not rejecting everything.
        assert!(
            serde_json::from_str::<HcmManifest>(
                r#"{"schema_version":2,"id":"x","name":"X","tier":"community","boot_provider":"pc_uefi_shim","selectors":[{"os_family":"linux"}]}"#
            )
            .is_ok()
        );
    }
}
