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
#[serde(deny_unknown_fields)]
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
///
/// The only entry states are `AwaitingApproval` and `Ready`: task creation
/// runs the policy engine over the whole plan and picks one of them, and no
/// edge leads back into either from outside. There is deliberately no `Draft`
/// state — one existed, produced by nothing and reachable by no edge, and a
/// state a security-relevant machine can never be in is contract noise that
/// still has to be handled by every client that reads `state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
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
            (Self::AwaitingApproval, Self::Ready | Self::Cancelled)
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

    /// Every state, at its own index. Kept in one place so the matrix test
    /// below cannot quietly stop covering part of the machine.
    const ALL_STATES: [TaskState; 10] = [
        TaskState::AwaitingApproval,
        TaskState::Ready,
        TaskState::Running,
        TaskState::Verifying,
        TaskState::Succeeded,
        TaskState::Failed,
        TaskState::Cancelling,
        TaskState::Cancelled,
        TaskState::Compensating,
        TaskState::Compensated,
    ];

    /// Proves `ALL_STATES` lists every state exactly once: the `match` is
    /// exhaustive, so a new variant forces a new arm (and a longer array),
    /// and each element must sit at the index its own variant names.
    #[test]
    fn the_state_list_covers_the_whole_machine() {
        for (position, state) in ALL_STATES.into_iter().enumerate() {
            let index = match state {
                TaskState::AwaitingApproval => 0,
                TaskState::Ready => 1,
                TaskState::Running => 2,
                TaskState::Verifying => 3,
                TaskState::Succeeded => 4,
                TaskState::Failed => 5,
                TaskState::Cancelling => 6,
                TaskState::Cancelled => 7,
                TaskState::Compensating => 8,
                TaskState::Compensated => 9,
            };
            assert_eq!(
                position, index,
                "ALL_STATES must list {state:?} exactly once, in order"
            );
        }
    }

    /// The whole transition relation, pinned edge by edge over every ordered
    /// pair of states. An edge added, removed, or widened anywhere in
    /// [`TaskState::transition`] shows up here as a named failure.
    #[test]
    fn the_transition_matrix_is_pinned() {
        const ALLOWED: [(TaskState, TaskState); 15] = [
            // Creation lands in one of the two entry states; approval (or a
            // late grant) is the only way forward from AwaitingApproval.
            (TaskState::AwaitingApproval, TaskState::Ready),
            (TaskState::AwaitingApproval, TaskState::Cancelled),
            (TaskState::Ready, TaskState::Running),
            (TaskState::Ready, TaskState::Cancelled),
            (TaskState::Running, TaskState::Verifying),
            (TaskState::Running, TaskState::Failed),
            (TaskState::Running, TaskState::Cancelling),
            (TaskState::Verifying, TaskState::Succeeded),
            (TaskState::Verifying, TaskState::Failed),
            (TaskState::Verifying, TaskState::Cancelling),
            // The one outgoing edge of a terminal state: recovery reopens a
            // failure when the plan asks for compensation.
            (TaskState::Failed, TaskState::Compensating),
            (TaskState::Cancelling, TaskState::Cancelled),
            (TaskState::Cancelling, TaskState::Compensating),
            (TaskState::Compensating, TaskState::Compensated),
            (TaskState::Compensating, TaskState::Failed),
        ];

        for from in ALL_STATES {
            for to in ALL_STATES {
                let expected = ALLOWED.contains(&(from, to));
                assert_eq!(
                    from.transition(to).is_ok(),
                    expected,
                    "{from:?} -> {to:?} must be {}",
                    if expected { "allowed" } else { "rejected" }
                );
            }
        }
    }

    #[test]
    fn successful_lifecycle_is_valid() {
        let state = TaskState::AwaitingApproval
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
