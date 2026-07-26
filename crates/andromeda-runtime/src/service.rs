use std::collections::{BTreeMap, BTreeSet};

use andromeda_core::{
    ActionId, ActionPlan, Capability, CapabilityId, IsolationLevel, TaskId, TaskState,
};
use andromeda_policy::{EvaluationContext, PolicyDecision, PolicyEngine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{FileTaskStore, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub plan: ActionPlan,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransitionRequest {
    pub to: TaskState,
    pub actor: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor: String,
    pub kind: TaskEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEventKind {
    Created,
    StateChanged { from: TaskState, to: TaskState },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub plan: ActionPlan,
    pub state: TaskState,
    pub revision: u64,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub events: Vec<TaskEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationReport {
    pub task_id: TaskId,
    pub revision: u64,
    pub decisions: BTreeMap<ActionId, PolicyDecision>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("unsupported plan schema version {0}")]
    SchemaVersion(u32),
    #[error("duplicate action id {0}")]
    DuplicateAction(ActionId),
    #[error("action {action} depends on missing action {dependency}")]
    MissingDependency {
        action: ActionId,
        dependency: ActionId,
    },
    #[error("action dependency graph contains a cycle")]
    DependencyCycle,
    #[error("action {0} declares risk below its operation floor")]
    InvalidRisk(ActionId),
    #[error("capability {0} is issued to a different task")]
    WrongCapabilitySubject(CapabilityId),
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Transition(#[from] andromeda_core::TaskTransitionError),
}

#[derive(Debug, Clone)]
pub struct TaskService {
    store: FileTaskStore,
    policy: PolicyEngine,
}

impl TaskService {
    #[must_use]
    pub const fn new(store: FileTaskStore, policy: PolicyEngine) -> Self {
        Self { store, policy }
    }

    /// Validates and durably creates a task.
    ///
    /// # Errors
    ///
    /// Returns validation errors for malformed/untrusted plans and store errors
    /// for persistence failures.
    pub fn create(&self, request: CreateTaskRequest) -> Result<TaskRecord, ServiceError> {
        validate_plan(&request.plan, &request.capabilities)?;
        let granted = request
            .plan
            .actions
            .iter()
            .flat_map(|action| &action.required_capabilities)
            .all(|required| {
                request
                    .capabilities
                    .iter()
                    .any(|capability| capability.id == *required)
            });
        let state = if granted {
            TaskState::Ready
        } else {
            TaskState::AwaitingApproval
        };
        let record = TaskRecord {
            plan: request.plan,
            state,
            revision: 0,
            capabilities: request.capabilities,
            events: vec![TaskEvent {
                id: Uuid::new_v4(),
                occurred_at: Utc::now(),
                actor: request.actor,
                kind: TaskEventKind::Created,
            }],
        };
        self.store.create(&record)?;
        Ok(record)
    }

    /// Loads one task.
    ///
    /// # Errors
    ///
    /// Returns a store error when the task is absent or unreadable.
    pub fn get(&self, task_id: TaskId) -> Result<TaskRecord, ServiceError> {
        self.store.get(task_id).map_err(ServiceError::from)
    }

    /// Lists persisted tasks.
    ///
    /// # Errors
    ///
    /// Returns a store error for unreadable or malformed state.
    pub fn list(&self) -> Result<Vec<TaskRecord>, ServiceError> {
        self.store.list().map_err(ServiceError::from)
    }

    /// Evaluates every action without executing it.
    ///
    /// # Errors
    ///
    /// Returns a store error when the task is absent or unreadable.
    pub fn evaluate(
        &self,
        task_id: TaskId,
        isolation: IsolationLevel,
        external_side_effect_confirmed: bool,
    ) -> Result<EvaluationReport, ServiceError> {
        let record = self.store.get(task_id)?;
        let context = EvaluationContext::current(
            isolation,
            &record.capabilities,
            external_side_effect_confirmed,
        );
        let decisions = record
            .plan
            .actions
            .iter()
            .map(|action| (action.id, self.policy.evaluate(action, &context)))
            .collect();
        Ok(EvaluationReport {
            task_id,
            revision: record.revision,
            decisions,
        })
    }

    /// Applies one checked, optimistic-concurrency state transition.
    ///
    /// # Errors
    ///
    /// Returns an invalid transition, revision conflict, or persistence error.
    pub fn transition(
        &self,
        task_id: TaskId,
        request: StateTransitionRequest,
    ) -> Result<TaskRecord, ServiceError> {
        let mut record = self.store.get(task_id)?;
        if record.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict {
                task_id,
                expected: request.expected_revision,
                actual: record.revision,
            }
            .into());
        }
        let from = record.state;
        record.state = from.transition(request.to)?;
        record.revision += 1;
        record.events.push(TaskEvent {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor: request.actor,
            kind: TaskEventKind::StateChanged {
                from,
                to: request.to,
            },
        });
        self.store.save(&record, request.expected_revision)?;
        Ok(record)
    }
}

fn validate_plan(plan: &ActionPlan, capabilities: &[Capability]) -> Result<(), ValidationError> {
    if plan.schema_version != ActionPlan::CURRENT_SCHEMA_VERSION {
        return Err(ValidationError::SchemaVersion(plan.schema_version));
    }

    let ids = plan
        .actions
        .iter()
        .map(|action| action.id)
        .collect::<BTreeSet<_>>();
    if ids.len() != plan.actions.len() {
        let mut seen = BTreeSet::new();
        let duplicate = plan
            .actions
            .iter()
            .map(|action| action.id)
            .find(|id| !seen.insert(*id))
            .expect("duplicate exists");
        return Err(ValidationError::DuplicateAction(duplicate));
    }

    for action in &plan.actions {
        if !action.has_valid_risk() {
            return Err(ValidationError::InvalidRisk(action.id));
        }
        for dependency in &action.depends_on {
            if !ids.contains(dependency) {
                return Err(ValidationError::MissingDependency {
                    action: action.id,
                    dependency: *dependency,
                });
            }
        }
    }

    if has_cycle(plan) {
        return Err(ValidationError::DependencyCycle);
    }

    let expected_subject = plan.task_id.to_string();
    if let Some(capability) = capabilities
        .iter()
        .find(|capability| capability.issued_to != expected_subject)
    {
        return Err(ValidationError::WrongCapabilitySubject(capability.id));
    }

    Ok(())
}

fn has_cycle(plan: &ActionPlan) -> bool {
    fn visit(
        id: ActionId,
        dependencies: &BTreeMap<ActionId, &[ActionId]>,
        visiting: &mut BTreeSet<ActionId>,
        visited: &mut BTreeSet<ActionId>,
    ) -> bool {
        if visited.contains(&id) {
            return false;
        }
        if !visiting.insert(id) {
            return true;
        }
        if dependencies.get(&id).is_some_and(|items| {
            items
                .iter()
                .any(|item| visit(*item, dependencies, visiting, visited))
        }) {
            return true;
        }
        visiting.remove(&id);
        visited.insert(id);
        false
    }

    let dependencies = plan
        .actions
        .iter()
        .map(|action| (action.id, action.depends_on.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    plan.actions
        .iter()
        .any(|action| visit(action.id, &dependencies, &mut visiting, &mut visited))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use andromeda_core::{
        ActionKind, ActionSpec, CapabilityResource, FileAccess, Intent, RecoverySemantics,
        RiskLevel,
    };
    use andromeda_policy::{DecisionEffect, PolicySet};
    use tempfile::TempDir;

    use super::*;

    fn inspection_request(path: &str) -> CreateTaskRequest {
        let task_id = TaskId::new();
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Files {
                root: PathBuf::from(path),
                access: FileAccess::Read,
            },
            issued_to: task_id.to_string(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        };
        let plan = ActionPlan {
            schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
            task_id,
            intent: Intent::new("Inspect", "test"),
            actions: vec![ActionSpec {
                id: ActionId::new(),
                name: "Inspect directory".into(),
                kind: andromeda_core::ActionKind::Inspect,
                target: path.into(),
                arguments: BTreeMap::new(),
                depends_on: Vec::new(),
                required_capabilities: vec![capability.id],
                risk: RiskLevel::L1Sandboxed,
                recovery: RecoverySemantics::None,
            }],
        };
        CreateTaskRequest {
            plan,
            capabilities: vec![capability],
            actor: "test".into(),
        }
    }

    fn service(temp: &TempDir) -> TaskService {
        TaskService::new(
            FileTaskStore::open(temp.path()).expect("store"),
            PolicyEngine::new(PolicySet::default()),
        )
    }

    #[test]
    fn task_is_durable_and_policy_evaluable() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service
            .create(inspection_request("/workspace"))
            .expect("create");
        assert_eq!(created.state, TaskState::Ready);

        let reloaded = service.get(created.plan.task_id).expect("reload");
        assert_eq!(reloaded, created);

        let report = service
            .evaluate(created.plan.task_id, IsolationLevel::Sandbox, false)
            .expect("evaluate");
        assert!(
            report
                .decisions
                .values()
                .all(|decision| decision.effect == DecisionEffect::Allow)
        );
    }

    #[test]
    fn transition_uses_optimistic_concurrency() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service
            .create(inspection_request("/workspace"))
            .expect("create");
        let running = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "runner".into(),
                    expected_revision: 0,
                },
            )
            .expect("transition");
        assert_eq!(running.revision, 1);

        let error = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "stale-runner".into(),
                    expected_revision: 0,
                },
            )
            .expect_err("stale transition");
        assert!(matches!(
            error,
            ServiceError::Store(StoreError::RevisionConflict { .. })
        ));
    }

    #[test]
    fn rejects_dependency_cycles() {
        let mut request = inspection_request("/workspace");
        let second = ActionId::new();
        request.plan.actions[0].depends_on = vec![second];
        request.plan.actions.push(ActionSpec {
            id: second,
            name: "cycle".into(),
            kind: ActionKind::Reason,
            target: String::new(),
            arguments: BTreeMap::new(),
            depends_on: vec![request.plan.actions[0].id],
            required_capabilities: Vec::new(),
            risk: RiskLevel::L0Reasoning,
            recovery: RecoverySemantics::None,
        });
        assert_eq!(
            validate_plan(&request.plan, &request.capabilities),
            Err(ValidationError::DependencyCycle)
        );
    }
}
