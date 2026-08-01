//! Durable orchestration for Andromeda tasks.
//!
//! This crate persists plans, grants, state changes, and policy decisions. It
//! deliberately does not execute model-proposed tools. Executors are added
//! behind separately attested isolation and broker interfaces.

mod service;
mod store;

pub use service::{
    CreateTaskRequest, EvaluationReport, ServiceError, StateTransitionRequest, TaskEvent,
    TaskEventKind, TaskRecord, TaskService, ValidationError,
};
pub use store::{FileTaskStore, StoreError};
