//! Stable, model-independent contracts for the Andromeda task runtime.
//!
//! The model may propose an [`ActionPlan`], but only deterministic host code
//! may grant capabilities, transition task state, execute actions, or record
//! evidence.

mod action;
mod capability;
pub mod capability_signing;
pub mod encoding;
mod task;

pub use action::{
    ActionId, ActionKind, ActionOutcome, ActionPlan, ActionSpec, Evidence, OutcomeStatus,
    PlanValidationError, RecoverySemantics, RiskLevel,
};
pub use capability::{
    Capability, CapabilityId, CapabilityResource, CapabilitySignature, FileAccess, IsolationLevel,
    normalized_absolute,
};
pub use capability_signing::{
    CapabilityKeyring, CapabilitySignatureStatus, CapabilitySigningKey, SignatureError,
    verify_capability_signature,
};
pub use task::{Intent, TaskId, TaskState, TaskTransitionError};
