use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Linux,
    Macos,
    Windows,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareIdentity {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub board: Option<String>,
    pub firmware_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub architecture: String,
    pub model: Option<String>,
    pub logical_cores: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub bytes: Option<u64>,
}

/// Boot-security capabilities of the probed host.
///
/// Every field is tri-state: `None` means "could not be verified" (for
/// example a probe running without the privileges the platform needs), which
/// is deliberately distinct from `Some(false)` ("verified absent").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootInfo {
    pub uefi: Option<bool>,
    pub secure_boot: Option<bool>,
    pub tpm2: Option<bool>,
    pub virtualization: Option<bool>,
}

/// A probed device.
///
/// Every optional field relies on serde's built-in behavior for missing
/// `Option` fields (absent input deserializes to `None`); no field carries a
/// redundant `#[serde(default)]`. `DeviceInfo` deliberately does *not* use
/// `deny_unknown_fields`: reports come from the local probe and must stay
/// forward-compatible when a newer probe adds fields, unlike the untrusted
/// manifest structs below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub bus: String,
    pub address: Option<String>,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_product_id: Option<String>,
    pub revision: Option<String>,
    pub class: Option<String>,
    pub driver: Option<String>,
    pub name: Option<String>,
    pub modalias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareReport {
    pub schema_version: u32,
    pub collected_at: DateTime<Utc>,
    pub os_family: OsFamily,
    pub identity: HardwareIdentity,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub boot: BootInfo,
    #[serde(default)]
    pub devices: Vec<DeviceInfo>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl HardwareReport {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

/// Support tiers, declared in ascending order of assurance.
///
/// The derived `Ord` follows this ladder: `Blocked < Community < Reference <
/// Supported < Certified`. `Reference` sits below `Supported` because it is
/// backed by virtual (L0–L2) evidence only, while `Supported`/`Certified`
/// require physical-machine certification. Keep the declaration order, the
/// JSON schema enum, and the ladder documented in
/// `docs/development/hardware-certification-test-plan.md` in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportTier {
    Blocked,
    Community,
    Reference,
    Supported,
    Certified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareSelector {
    pub os_family: Option<OsFamily>,
    #[serde(default)]
    pub architectures: Vec<String>,
    pub manufacturer_contains: Option<String>,
    pub model_prefix: Option<String>,
}

// `deny_unknown_fields` on the internally tagged `CapabilityRequirement`
// applies to each variant's own fields; the `type` tag is consumed by the
// enum deserializer, so a mistyped requirement field is rejected rather than
// silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityRequirement {
    MemoryBytes {
        minimum: u64,
    },
    Boot {
        uefi: Option<bool>,
        secure_boot: Option<bool>,
        tpm2: Option<bool>,
        virtualization: Option<bool>,
    },
    Device {
        bus: String,
        vendor_id: Option<String>,
        product_id: Option<String>,
        #[serde(default)]
        subsystem_vendor_id: Option<String>,
        #[serde(default)]
        subsystem_product_id: Option<String>,
        #[serde(default)]
        revision: Option<String>,
        driver_required: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootProvider {
    PcUefiShim,
    IntelMacEfi,
    T2Experimental,
    AppleSiliconAsahi,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Kernel,
    Driver,
    Firmware,
    HardwareEnablementImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPin {
    pub kind: ArtifactKind,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub source: String,
    pub signing_key_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceResult {
    Passed,
    /// The capability works with a limitation that must be publicly disclosed.
    ///
    /// The certification plan's suites emit this constantly (a codec that
    /// decodes but not encodes, suspend that works on AC but not battery), and
    /// until now it had to be flattened into `Passed` — which overstates — or
    /// `Failed` — which understates and blocks a machine that is genuinely
    /// usable. It satisfies evidence below `Certified` and blocks `Certified`,
    /// which is the tier that promises no known degradation.
    Degraded,
    Failed,
    /// The suite did not produce a verdict: not run, inconclusive, or the
    /// evidence could not be retrieved.
    ///
    /// Always blocking. "Not measured" is the state most easily confused with
    /// "measured fine", and the certification plan is explicit that `unknown`
    /// cannot promote, so it fails closed at every tier rather than being
    /// silently treated as absence.
    Unknown,
}

impl EvidenceResult {
    /// Whether this result blocks the given declared tier.
    ///
    /// Deliberately a `match` over explicit tiers rather than an `Ord`
    /// comparison: adding a `SupportTier` variant must not silently widen or
    /// narrow the gate. Mirrors the reasoning already used for the artifact
    /// gate in the matcher.
    #[must_use]
    pub const fn blocks(self, tier: SupportTier) -> bool {
        match self {
            Self::Passed => false,
            Self::Degraded => match tier {
                SupportTier::Certified => true,
                SupportTier::Blocked
                | SupportTier::Community
                | SupportTier::Reference
                | SupportTier::Supported => false,
            },
            Self::Failed | Self::Unknown => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEvidence {
    pub capability: String,
    pub result: EvidenceResult,
    pub evidence_uri: String,
    pub collected_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Detached ed25519 signature that authenticates a whole [`HcmManifest`].
///
/// `sig` is the lowercase-hex ed25519 signature (64 bytes / 128 hex chars)
/// over the manifest's *canonical* bytes — the manifest serialized through this
/// model, with the `signature` field removed and all object keys sorted (see
/// `crate::canonical_signing_bytes`). `key_id` names the public key expected to
/// verify it; the verifier resolves that id in a [`crate::TrustedKeyring`].
///
/// A signature is only an authenticity *claim*. It gates nothing on its own:
/// the matcher enforces it only when a keyring is supplied, and an unknown
/// `key_id` or a bad `sig` is then fail-closed to `Blocked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSignature {
    pub key_id: String,
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HcmManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub tier: SupportTier,
    pub boot_provider: BootProvider,
    #[serde(default)]
    pub selectors: Vec<HardwareSelector>,
    #[serde(default)]
    pub requirements: Vec<CapabilityRequirement>,
    #[serde(default)]
    pub kernel_channels: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactPin>,
    #[serde(default)]
    pub evidence: Vec<CapabilityEvidence>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Detached authenticity signature over the rest of the manifest. Absent on
    /// legacy/unsigned manifests; the canonicalization that produces the signed
    /// bytes deliberately excludes this field so it can be added after signing.
    pub signature: Option<ManifestSignature>,
}

impl HcmManifest {
    pub const CURRENT_SCHEMA_VERSION: u32 = 2;
}

/// Authenticity verdict for the evaluated manifest, surfaced as a structured
/// field on [`CompatibilityEvaluation`] so consumers can gate on it instead of
/// substring-matching `evidence`/`missing` strings.
///
/// Internally tagged like [`CapabilityRequirement`], so it serializes as
/// `{"type": "not_checked"}`, `{"type": "verified", "key_id": "..."}`, or
/// `{"type": "failed", "reason": "..."}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestAuthenticity {
    /// No trusted keyring was supplied: the advisory path. The manifest's
    /// signature (if any) was neither required nor checked, and its declared
    /// tier is self-asserted.
    #[default]
    NotChecked,
    /// A keyring was supplied and the manifest's detached ed25519 signature
    /// verified against the trusted key named `key_id`.
    Verified { key_id: String },
    /// A keyring was supplied but authenticity could not be established
    /// (unsigned, unknown key, malformed signature, or failed verification).
    /// Fail-closed: the evaluation's `effective_tier` is `Blocked`.
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityEvaluation {
    pub manifest_id: String,
    pub selector_matched: bool,
    pub requirements_met: bool,
    pub declared_tier: SupportTier,
    pub effective_tier: SupportTier,
    /// The boot provider declared by the manifest. The matcher surfaces it
    /// for transparency; enforcement against the target platform happens in
    /// the installer preflight, not here.
    pub boot_provider: BootProvider,
    /// Whether the manifest itself was authenticated, and how. Defaults to
    /// [`ManifestAuthenticity::NotChecked`] when deserializing evaluations
    /// recorded before this field existed.
    #[serde(default)]
    pub manifest_authenticity: ManifestAuthenticity,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub missing: Vec<String>,
}

/// Strips at most one leading `0x`/`0X` prefix (case-insensitive).
///
/// `trim_start_matches` would strip repeated prefixes, silently equating
/// `0x0x10de` with `10de`. This is the single source of truth shared by the
/// matcher and the diagnosis classifier.
#[must_use]
pub(crate) fn strip_hex_prefix(value: &str) -> &str {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
}

/// Case-insensitive hardware-ID comparison that ignores a single `0x`/`0X`
/// prefix on either side. Repeated prefixes (`0x0x10de`) are *not* collapsed.
#[must_use]
pub(crate) fn id_matches(actual: &str, expected: &str) -> bool {
    strip_hex_prefix(actual).eq_ignore_ascii_case(strip_hex_prefix(expected))
}

#[cfg(test)]
mod tests {
    use super::{id_matches, strip_hex_prefix};

    #[test]
    fn strips_at_most_one_hex_prefix_case_insensitively() {
        assert_eq!(strip_hex_prefix("0x10de"), "10de");
        assert_eq!(strip_hex_prefix("0X10DE"), "10DE");
        assert_eq!(strip_hex_prefix("10de"), "10de");
        // Only the first prefix is removed; the inner `0x` survives.
        assert_eq!(strip_hex_prefix("0x0x10de"), "0x10de");
    }

    #[test]
    fn id_matches_ignores_one_prefix_and_case_but_not_repeats() {
        assert!(id_matches("0X10DE", "10de"));
        assert!(id_matches("0x10de", "0X10DE"));
        assert!(id_matches("10DE", "10de"));
        assert!(
            !id_matches("0x0x10de", "10de"),
            "repeated prefixes must not be stripped"
        );
    }
}
