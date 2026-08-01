use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Stable identifier for a user-visible Andromeda task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Uuid);

impl TaskId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// The immutable user intent captured before planning begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub summary: String,
    pub requested_by: String,
    pub created_at: DateTime<Utc>,
}

impl Intent {
    #[must_use]
    pub fn new(summary: impl Into<String>, requested_by: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            requested_by: requested_by.into(),
            created_at: Utc::now(),
        }
    }
}

/// Durable task lifecycle. Every transition is checked by deterministic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Draft,
    AwaitingApproval,
    Ready,
    Running,
    Verifying,
    Succeeded,
    Failed,
    Cancelling,
    Cancelled,
    Compensating,
    Compensated,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid task transition from {from:?} to {to:?}")]
pub struct TaskTransitionError {
    pub from: TaskState,
    pub to: TaskState,
}

impl TaskState {
    /// Moves a task to a valid next state.
    ///
    /// `Failed` is a terminal state (see [`TaskState::is_terminal`]), but it
    /// keeps one explicit outgoing edge: `Failed -> Compensating` "reopens"
    /// the terminal failure when the plan's recovery semantics call for
    /// compensation. Tasks without recovery semantics simply rest in
    /// `Failed`.
    ///
    /// # Errors
    ///
    /// Returns [`TaskTransitionError`] when the requested edge is not part of
    /// the durable task lifecycle.
    pub fn transition(self, to: Self) -> Result<Self, TaskTransitionError> {
        let allowed = matches!(
            (self, to),
            (
                Self::Draft,
                Self::AwaitingApproval | Self::Ready | Self::Cancelled
            ) | (Self::AwaitingApproval, Self::Ready | Self::Cancelled)
                | (Self::Ready, Self::Running | Self::Cancelled)
                | (
                    Self::Running,
                    Self::Verifying | Self::Failed | Self::Cancelling
                )
                | (
                    Self::Verifying,
                    Self::Succeeded | Self::Failed | Self::Cancelling
                )
                | (Self::Failed, Self::Compensating)
                | (Self::Cancelling, Self::Cancelled | Self::Compensating)
                | (Self::Compensating, Self::Compensated | Self::Failed)
        );

        if allowed {
            Ok(to)
        } else {
            Err(TaskTransitionError { from: self, to })
        }
    }

    /// Whether the state holds no scheduled work.
    ///
    /// `Failed` counts as terminal so that tasks with
    /// `RecoverySemantics::None` are retained/cleaned up like any other
    /// finished task instead of leaking as forever-pending work. It is the
    /// only terminal state with an outgoing edge: `Failed -> Compensating`
    /// deliberately reopens it when recovery runs.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Compensated
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_lifecycle_is_valid() {
        let state = TaskState::Draft
            .transition(TaskState::Ready)
            .and_then(|state| state.transition(TaskState::Running))
            .and_then(|state| state.transition(TaskState::Verifying))
            .and_then(|state| state.transition(TaskState::Succeeded))
            .expect("valid lifecycle");
        assert!(state.is_terminal());
    }

    #[test]
    fn cannot_skip_verification() {
        assert_eq!(
            TaskState::Running.transition(TaskState::Succeeded),
            Err(TaskTransitionError {
                from: TaskState::Running,
                to: TaskState::Succeeded,
            })
        );
    }

    #[test]
    fn verification_can_be_cancelled() {
        let state = TaskState::Verifying
            .transition(TaskState::Cancelling)
            .and_then(|state| state.transition(TaskState::Cancelled))
            .expect("cancellation from verification");
        assert!(state.is_terminal());
    }

    #[test]
    fn failed_is_terminal_for_tasks_without_recovery() {
        assert!(TaskState::Failed.is_terminal());
    }

    #[test]
    fn failed_can_still_be_reopened_by_compensation() {
        let state = TaskState::Failed
            .transition(TaskState::Compensating)
            .expect("compensation edge");
        assert!(!state.is_terminal());
        assert!(
            state
                .transition(TaskState::Compensated)
                .expect("compensated")
                .is_terminal()
        );
    }
}
