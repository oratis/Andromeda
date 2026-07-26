//! Stable, model-independent contracts for the Andromeda task runtime.
//!
//! The model may propose an [`ActionPlan`], but only deterministic host code
//! may grant capabilities, transition task state, execute actions, or record
//! evidence.

mod action;
mod capability;
mod task;

pub use action::{
    ActionId, ActionKind, ActionOutcome, ActionPlan, ActionSpec, Evidence, OutcomeStatus,
    RecoverySemantics, RiskLevel,
};
pub use capability::{Capability, CapabilityId, CapabilityResource, FileAccess, IsolationLevel};
pub use task::{Intent, TaskId, TaskState, TaskTransitionError};
