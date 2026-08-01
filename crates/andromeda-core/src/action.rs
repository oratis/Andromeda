use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{CapabilityId, Intent, IsolationLevel, TaskId};

/// Stable identifier for one action inside a task plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(Uuid);

impl ActionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ActionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for ActionId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Deterministic operation classes understood by the host runtime.
///
/// The meaning of [`ActionSpec::target`] depends on the kind:
///
/// - File kinds (`Inspect`, `ReadFile`, `WriteFile`, `CreateDirectory`,
///   `MoveFile`, `DeleteFile`, `ParseUntrustedContent`): an absolute path.
/// - `MoveFile`: `target` is the **source** path; the destination path is
///   stored in `ActionSpec::arguments` under
///   [`ActionSpec::MOVE_DESTINATION_ARGUMENT`]. Policy checks write coverage
///   and deny rules on both endpoints.
/// - `NetworkRequest`: `host` or `host:port` (a bare hostname or IPv4
///   address, optionally followed by a decimal port; bracketless IPv6
///   literals are not supported by this textual form).
/// - `SystemChange`: the system setting key.
/// - `ExternalCall`: `service:operation`, split at the **first** `':'`;
///   service names must not contain `':'`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Reason,
    Inspect,
    ReadFile,
    WriteFile,
    CreateDirectory,
    MoveFile,
    DeleteFile,
    ParseUntrustedContent,
    NetworkRequest,
    SystemChange,
    ExternalCall,
}

/// Risk levels defined by the Andromeda product threat model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// No tool use or external data.
    L0Reasoning,
    /// Normal task sandbox with scoped capabilities.
    L1Sandboxed,
    /// Strong isolation for unknown content or executable inputs.
    L2StrongIsolation,
    /// A real external side effect performed through a host broker.
    L3ExternalSideEffect,
}

impl RiskLevel {
    #[must_use]
    pub const fn minimum_isolation(self) -> IsolationLevel {
        match self {
            Self::L0Reasoning => IsolationLevel::None,
            Self::L1Sandboxed => IsolationLevel::Sandbox,
            Self::L2StrongIsolation => IsolationLevel::MicroVm,
            Self::L3ExternalSideEffect => IsolationLevel::Brokered,
        }
    }
}

impl ActionKind {
    /// Lowest risk level an action of this kind may declare.
    #[must_use]
    pub const fn minimum_risk(&self) -> RiskLevel {
        match self {
            Self::Reason => RiskLevel::L0Reasoning,
            Self::Inspect
            | Self::ReadFile
            | Self::WriteFile
            | Self::CreateDirectory
            | Self::MoveFile
            | Self::DeleteFile => RiskLevel::L1Sandboxed,
            Self::ParseUntrustedContent => RiskLevel::L2StrongIsolation,
            Self::NetworkRequest | Self::SystemChange | Self::ExternalCall => {
                RiskLevel::L3ExternalSideEffect
            }
        }
    }
}

/// How the runtime can recover after an action has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySemantics {
    Rollback,
    Compensate,
    RotateSecret,
    None,
}

/// One typed action proposed as part of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSpec {
    pub id: ActionId,
    pub name: String,
    pub kind: ActionKind,
    pub target: String,
    #[serde(default)]
    pub arguments: BTreeMap<String, String>,
    #[serde(default)]
    pub depends_on: Vec<ActionId>,
    #[serde(default)]
    pub required_capabilities: Vec<CapabilityId>,
    pub risk: RiskLevel,
    pub recovery: RecoverySemantics,
}

impl ActionSpec {
    /// Key in [`ActionSpec::arguments`] holding the destination path of a
    /// [`ActionKind::MoveFile`] action (`target` holds the source path).
    pub const MOVE_DESTINATION_ARGUMENT: &'static str = "destination";

    #[must_use]
    pub fn has_valid_risk(&self) -> bool {
        self.risk >= self.kind.minimum_risk()
    }
}

/// A versioned, serializable plan. Plans contain no credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub schema_version: u32,
    pub task_id: TaskId,
    pub intent: Intent,
    pub actions: Vec<ActionSpec>,
}

/// Structural problems detected by [`ActionPlan::validate`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlanValidationError {
    #[error("duplicate action id {0}")]
    DuplicateActionId(ActionId),
    #[error("action {action} depends on unknown action {dependency}")]
    UnknownDependency {
        action: ActionId,
        dependency: ActionId,
    },
    #[error("dependency cycle involving action {0}")]
    DependencyCycle(ActionId),
}

impl ActionPlan {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    #[must_use]
    pub fn new(intent: Intent, actions: Vec<ActionSpec>) -> Self {
        Self::for_task(TaskId::new(), intent, actions)
    }

    /// Builds a plan for an already existing task instead of minting a fresh
    /// [`TaskId`].
    #[must_use]
    pub fn for_task(task_id: TaskId, intent: Intent, actions: Vec<ActionSpec>) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            task_id,
            intent,
            actions,
        }
    }

    /// Validates plan structure: action ids must be unique, every
    /// `depends_on` reference must resolve, and the dependency graph must be
    /// acyclic. Cycle detection uses an iterative Kahn topological sort, so
    /// adversarially deep plans cannot overflow the stack.
    ///
    /// # Errors
    ///
    /// Returns the first [`PlanValidationError`] found: a duplicate id, a
    /// dangling dependency reference, or a dependency cycle.
    pub fn validate(&self) -> Result<(), PlanValidationError> {
        let mut ids = BTreeSet::new();
        for action in &self.actions {
            if !ids.insert(action.id) {
                return Err(PlanValidationError::DuplicateActionId(action.id));
            }
        }

        for action in &self.actions {
            for dependency in &action.depends_on {
                if !ids.contains(dependency) {
                    return Err(PlanValidationError::UnknownDependency {
                        action: action.id,
                        dependency: *dependency,
                    });
                }
            }
        }

        // Kahn's algorithm: repeatedly release actions whose dependencies are
        // all resolved. Anything left unresolved sits on a cycle.
        let mut blocking_dependencies: BTreeMap<ActionId, usize> = self
            .actions
            .iter()
            .map(|action| (action.id, action.depends_on.len()))
            .collect();
        let mut dependents: BTreeMap<ActionId, Vec<ActionId>> = BTreeMap::new();
        for action in &self.actions {
            for dependency in &action.depends_on {
                dependents.entry(*dependency).or_default().push(action.id);
            }
        }

        let mut ready: VecDeque<ActionId> = blocking_dependencies
            .iter()
            .filter(|(_, blocking)| **blocking == 0)
            .map(|(id, _)| *id)
            .collect();
        while let Some(id) = ready.pop_front() {
            for dependent in dependents.get(&id).into_iter().flatten() {
                if let Some(blocking) = blocking_dependencies.get_mut(dependent) {
                    *blocking -= 1;
                    if *blocking == 0 {
                        ready.push_back(*dependent);
                    }
                }
            }
        }

        // Any action still blocked after the sort sits on a cycle.
        if let Some(cycle_member) = blocking_dependencies
            .iter()
            .find_map(|(id, blocking)| (*blocking > 0).then_some(*id))
        {
            return Err(PlanValidationError::DependencyCycle(cycle_member));
        }

        Ok(())
    }
}

/// Evidence produced by deterministic executors and verifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Succeeded,
    Failed,
    Skipped,
    RolledBack,
    Compensated,
}

/// Append-only audit record for one attempted action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub action_id: ActionId,
    pub status: OutcomeStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_kind_sets_a_risk_floor() {
        assert_eq!(
            ActionKind::ParseUntrustedContent.minimum_risk(),
            RiskLevel::L2StrongIsolation
        );
        assert_eq!(
            ActionKind::ExternalCall.minimum_risk(),
            RiskLevel::L3ExternalSideEffect
        );
    }

    /// Locks the declaration order so that inserting a variant (which would
    /// silently change the derived `Ord` used by `has_valid_risk`) fails
    /// loudly.
    #[test]
    fn risk_level_declaration_order_is_locked() {
        assert_eq!(RiskLevel::L0Reasoning as u8, 0);
        assert_eq!(RiskLevel::L1Sandboxed as u8, 1);
        assert_eq!(RiskLevel::L2StrongIsolation as u8, 2);
        assert_eq!(RiskLevel::L3ExternalSideEffect as u8, 3);
        assert!(RiskLevel::L0Reasoning < RiskLevel::L1Sandboxed);
        assert!(RiskLevel::L1Sandboxed < RiskLevel::L2StrongIsolation);
        assert!(RiskLevel::L2StrongIsolation < RiskLevel::L3ExternalSideEffect);
    }

    fn reason_action(id: ActionId, depends_on: Vec<ActionId>) -> ActionSpec {
        ActionSpec {
            id,
            name: "step".into(),
            kind: ActionKind::Reason,
            target: String::new(),
            arguments: BTreeMap::new(),
            depends_on,
            required_capabilities: Vec::new(),
            risk: RiskLevel::L0Reasoning,
            recovery: RecoverySemantics::None,
        }
    }

    #[test]
    fn for_task_preserves_the_given_task_id() {
        let task_id = crate::TaskId::new();
        let plan = ActionPlan::for_task(task_id, Intent::new("noop", "test"), Vec::new());
        assert_eq!(plan.task_id, task_id);
        assert_eq!(plan.schema_version, ActionPlan::CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn validate_accepts_a_linear_plan() {
        let first = ActionId::new();
        let second = ActionId::new();
        let plan = ActionPlan::new(
            Intent::new("ok", "test"),
            vec![
                reason_action(first, Vec::new()),
                reason_action(second, vec![first]),
            ],
        );
        assert_eq!(plan.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_duplicate_action_ids() {
        let id = ActionId::new();
        let plan = ActionPlan::new(
            Intent::new("dup", "test"),
            vec![reason_action(id, Vec::new()), reason_action(id, Vec::new())],
        );
        assert_eq!(
            plan.validate(),
            Err(PlanValidationError::DuplicateActionId(id))
        );
    }

    #[test]
    fn validate_rejects_dangling_dependencies() {
        let id = ActionId::new();
        let missing = ActionId::new();
        let plan = ActionPlan::new(
            Intent::new("dangling", "test"),
            vec![reason_action(id, vec![missing])],
        );
        assert_eq!(
            plan.validate(),
            Err(PlanValidationError::UnknownDependency {
                action: id,
                dependency: missing,
            })
        );
    }

    #[test]
    fn validate_rejects_dependency_cycles() {
        let first = ActionId::new();
        let second = ActionId::new();
        let plan = ActionPlan::new(
            Intent::new("cycle", "test"),
            vec![
                reason_action(first, vec![second]),
                reason_action(second, vec![first]),
            ],
        );
        assert!(matches!(
            plan.validate(),
            Err(PlanValidationError::DependencyCycle(_))
        ));
    }

    #[test]
    fn validate_rejects_self_dependency() {
        let id = ActionId::new();
        let plan = ActionPlan::new(
            Intent::new("self-cycle", "test"),
            vec![reason_action(id, vec![id])],
        );
        assert_eq!(
            plan.validate(),
            Err(PlanValidationError::DependencyCycle(id))
        );
    }

    #[test]
    fn validate_handles_deep_chains_iteratively() {
        let ids: Vec<ActionId> = (0..10_000).map(|_| ActionId::new()).collect();
        let actions = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let depends_on = if index == 0 {
                    Vec::new()
                } else {
                    vec![ids[index - 1]]
                };
                reason_action(*id, depends_on)
            })
            .collect();
        let plan = ActionPlan::new(Intent::new("deep", "test"), actions);
        assert_eq!(plan.validate(), Ok(()));
    }

    #[test]
    fn plans_round_trip_as_json() {
        let plan = ActionPlan::new(
            Intent::new("Inspect a workspace", "local-user"),
            vec![ActionSpec {
                id: ActionId::new(),
                name: "List files".into(),
                kind: ActionKind::Inspect,
                target: "/workspace".into(),
                arguments: BTreeMap::new(),
                depends_on: Vec::new(),
                required_capabilities: Vec::new(),
                risk: RiskLevel::L1Sandboxed,
                recovery: RecoverySemantics::None,
            }],
        );

        let encoded = serde_json::to_string(&plan).expect("serialize plan");
        let decoded: ActionPlan = serde_json::from_str(&encoded).expect("deserialize plan");
        assert_eq!(decoded, plan);
    }
}
