use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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

/// Deterministic operation classes understood by the host runtime.
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

impl ActionPlan {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    #[must_use]
    pub fn new(intent: Intent, actions: Vec<ActionSpec>) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            task_id: TaskId::new(),
            intent,
            actions,
        }
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
