//! Hardware discovery and Hardware Compatibility Manifest evaluation.
//!
//! The probe deliberately omits serial numbers and other stable device
//! identifiers. A report describes compatibility-relevant capabilities, not a
//! user-tracking identity.

mod diagnosis;
mod matcher;
mod model;
mod probe;
mod signing;
mod verify;

pub use diagnosis::{
    DeviceCategory, DeviceFinding, DeviceSupport, HardwareDiagnosis, HardwareReadiness,
    diagnose_report,
};
pub use matcher::{
    ArtifactVerdict, ArtifactVerifier, evaluate_manifest, evaluate_manifest_at,
    evaluate_manifest_at_verified, evaluate_manifest_at_with_verifier, evaluate_manifest_verified,
    evaluate_manifest_with_verifier,
};
pub use model::{
    ArtifactKind, ArtifactPin, BootInfo, BootProvider, CapabilityEvidence, CapabilityRequirement,
    CompatibilityEvaluation, CpuInfo, DeviceInfo, EvidenceResult, HardwareIdentity, HardwareReport,
    HardwareSelector, HcmManifest, ManifestSignature, MemoryInfo, OsFamily, SupportTier,
};
pub use probe::{ProbeError, probe_host};
pub use signing::{
    ManifestSignatureStatus, ManifestSigningKey, SignatureError, TrustedKeyring,
    canonical_signing_bytes, verify_manifest_signature,
};
pub use verify::DirectoryArtifactVerifier;
