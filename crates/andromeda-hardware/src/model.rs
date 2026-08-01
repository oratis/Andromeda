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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub bus: String,
    pub address: Option<String>,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    #[serde(default)]
    pub subsystem_vendor_id: Option<String>,
    #[serde(default)]
    pub subsystem_product_id: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    pub class: Option<String>,
    pub driver: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
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
pub struct HardwareSelector {
    pub os_family: Option<OsFamily>,
    #[serde(default)]
    pub architectures: Vec<String>,
    pub manufacturer_contains: Option<String>,
    pub model_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    pub capability: String,
    pub result: EvidenceResult,
    pub evidence_uri: String,
    pub collected_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl HcmManifest {
    pub const CURRENT_SCHEMA_VERSION: u32 = 2;
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
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub missing: Vec<String>,
}
