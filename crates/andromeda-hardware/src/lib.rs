//! Hardware discovery and Hardware Compatibility Manifest evaluation.
//!
//! The probe deliberately omits serial numbers and other stable device
//! identifiers. A report describes compatibility-relevant capabilities, not a
//! user-tracking identity.

mod matcher;
mod model;
mod probe;

pub use matcher::evaluate_manifest;
pub use model::{
    BootInfo, CapabilityRequirement, CompatibilityEvaluation, CpuInfo, DeviceInfo,
    HardwareIdentity, HardwareReport, HardwareSelector, HcmManifest, MemoryInfo, OsFamily,
    SupportTier,
};
pub use probe::{ProbeError, probe_host};
