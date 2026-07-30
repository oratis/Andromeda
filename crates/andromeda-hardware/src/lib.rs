//! Hardware discovery and Hardware Compatibility Manifest evaluation.
//!
//! The probe deliberately omits serial numbers and other stable device
//! identifiers. A report describes compatibility-relevant capabilities, not a
//! user-tracking identity.

mod diagnosis;
mod matcher;
mod model;
mod probe;

pub use diagnosis::{
    DeviceCategory, DeviceFinding, DeviceSupport, HardwareDiagnosis, HardwareReadiness,
    diagnose_report,
};
pub use matcher::evaluate_manifest;
pub use model::{
    ArtifactKind, ArtifactPin, BootInfo, BootProvider, CapabilityEvidence, CapabilityRequirement,
    CompatibilityEvaluation, CpuInfo, DeviceInfo, EvidenceResult, HardwareIdentity, HardwareReport,
    HardwareSelector, HcmManifest, MemoryInfo, OsFamily, SupportTier,
};
pub use probe::{ProbeError, probe_host};
