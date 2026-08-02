//! Durable orchestration for Andromeda tasks.
//!
//! This crate persists plans, grants, state changes, and policy decisions. It
//! deliberately does not execute model-proposed tools. Executors are added
//! behind separately attested isolation and broker interfaces.

mod service;
mod store;

pub use service::{
    CreateTaskRequest, EvaluationReport, EvaluationRequest, GrantCapabilitiesRequest,
    MAX_PLAN_ACTIONS, MAX_TASK_CAPABILITIES, RecordOutcomeRequest, ServiceError,
    StateTransitionRequest, TaskEvent, TaskEventKind, TaskRecord, TaskService,
    TransitionGuardError, ValidationError,
};
pub use store::{FileTaskStore, ListWarning, StoreError, TaskListing};
