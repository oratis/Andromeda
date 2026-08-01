use std::collections::{BTreeMap, BTreeSet};

use andromeda_core::{
    ActionId, ActionPlan, Capability, CapabilityId, IsolationLevel, TaskId, TaskState,
};
use andromeda_policy::{DecisionEffect, EvaluationContext, PolicyDecision, PolicyEngine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::store::TaskListing;
use crate::{FileTaskStore, StoreError};

/// Upper bound on the number of actions accepted in a single plan.
///
/// The limit caps validation and policy-evaluation work per request and
/// bounds the size of persisted task records. Plans above the limit are
/// rejected with [`ValidationError::TooManyActions`]. 10 000 actions is far
/// beyond any realistic plan while keeping worst-case validation cheap.
pub const MAX_PLAN_ACTIONS: usize = 10_000;

/// Actor recorded on events that the policy engine itself appends.
const EVALUATION_ACTOR: &str = "policy-engine";

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
    StateChanged {
        from: TaskState,
        to: TaskState,
    },
    /// A policy evaluation was performed; the full decision set is recorded
    /// so the audit trail preserves every authorization outcome.
    Evaluated {
        isolation: IsolationLevel,
        external_side_effect_confirmed: bool,
        decisions: BTreeMap<ActionId, PolicyDecision>,
    },
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
    /// Revision of the task record after the evaluation event was appended.
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
    #[error("plan contains {actions} actions, which exceeds the limit of {limit}")]
    TooManyActions { actions: usize, limit: usize },
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
        let state = if self.plan_fully_granted(&request.plan, &request.capabilities) {
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

    /// Lists persisted tasks, silently skipping unreadable records.
    ///
    /// Use [`TaskService::list_detailed`] to also observe which record files
    /// were skipped and why.
    ///
    /// # Errors
    ///
    /// Returns a store error when the state directory itself is unreadable.
    pub fn list(&self) -> Result<Vec<TaskRecord>, ServiceError> {
        Ok(self.store.list()?.records)
    }

    /// Lists persisted tasks together with warnings for skipped records.
    ///
    /// # Errors
    ///
    /// Returns a store error when the state directory itself is unreadable.
    pub fn list_detailed(&self) -> Result<TaskListing, ServiceError> {
        self.store.list().map_err(ServiceError::from)
    }

    /// Evaluates every action without executing it and appends the outcome
    /// to the task's durable event history.
    ///
    /// The persisted [`TaskEventKind::Evaluated`] event bumps the record
    /// revision, so callers should use the revision from the returned report
    /// for subsequent optimistic-concurrency operations.
    ///
    /// # Errors
    ///
    /// Returns a store error when the task is absent, unreadable, or when a
    /// concurrent write prevents persisting the evaluation event.
    pub fn evaluate(
        &self,
        task_id: TaskId,
        isolation: IsolationLevel,
        external_side_effect_confirmed: bool,
    ) -> Result<EvaluationReport, ServiceError> {
        let mut record = self.store.get(task_id)?;
        let now = Utc::now();
        let decisions: BTreeMap<ActionId, PolicyDecision> = {
            let context = EvaluationContext::at(
                now,
                isolation,
                &record.capabilities,
                external_side_effect_confirmed,
            );
            record
                .plan
                .actions
                .iter()
                .map(|action| (action.id, self.policy.evaluate(action, &context)))
                .collect()
        };
        let expected = record.revision;
        record.revision += 1;
        record.events.push(TaskEvent {
            id: Uuid::new_v4(),
            occurred_at: now,
            actor: EVALUATION_ACTOR.into(),
            kind: TaskEventKind::Evaluated {
                isolation,
                external_side_effect_confirmed,
                decisions: decisions.clone(),
            },
        });
        self.store.save(&record, expected)?;
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

    /// Returns whether every action in the plan would be allowed by the
    /// deterministic policy engine under the most permissive execution
    /// assumptions: each action is checked with exactly the isolation its
    /// declared risk requires, and external side effects are treated as
    /// confirmed.
    ///
    /// This verifies that each action's required capabilities exist among
    /// the provided grants, are unexpired at creation time, actually cover
    /// the action target (file scope, network host, system setting, or
    /// external service), and that no deny rule matches. It deliberately
    /// does not check the real executor isolation or final side-effect
    /// confirmation; both are re-evaluated with real values at evaluation
    /// time.
    fn plan_fully_granted(&self, plan: &ActionPlan, capabilities: &[Capability]) -> bool {
        let now = Utc::now();
        plan.actions.iter().all(|action| {
            let context =
                EvaluationContext::at(now, action.risk.minimum_isolation(), capabilities, true);
            self.policy.evaluate(action, &context).effect == DecisionEffect::Allow
        })
    }
}

fn validate_plan(plan: &ActionPlan, capabilities: &[Capability]) -> Result<(), ValidationError> {
    if plan.schema_version != ActionPlan::CURRENT_SCHEMA_VERSION {
        return Err(ValidationError::SchemaVersion(plan.schema_version));
    }

    if plan.actions.len() > MAX_PLAN_ACTIONS {
        return Err(ValidationError::TooManyActions {
            actions: plan.actions.len(),
            limit: MAX_PLAN_ACTIONS,
        });
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

/// Detects dependency cycles with an iterative three-color depth-first
/// search. An explicit stack keeps arbitrarily long dependency chains from
/// overflowing the thread stack, which the previous recursive version could
/// do for plans near [`MAX_PLAN_ACTIONS`].
fn has_cycle(plan: &ActionPlan) -> bool {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }

    let dependencies = plan
        .actions
        .iter()
        .map(|action| (action.id, action.depends_on.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut marks = BTreeMap::<ActionId, Mark>::new();
    for action in &plan.actions {
        if marks.contains_key(&action.id) {
            continue;
        }
        marks.insert(action.id, Mark::Visiting);
        let mut stack = vec![(action.id, 0_usize)];
        while let Some((id, cursor)) = stack.last().copied() {
            let edges = dependencies.get(&id).copied().unwrap_or_default();
            if let Some(&dependency) = edges.get(cursor) {
                stack.last_mut().expect("stack is non-empty").1 += 1;
                match marks.get(&dependency) {
                    Some(Mark::Visiting) => return true,
                    Some(Mark::Done) => {}
                    None => {
                        marks.insert(dependency, Mark::Visiting);
                        stack.push((dependency, 0));
                    }
                }
            } else {
                marks.insert(id, Mark::Done);
                stack.pop();
            }
        }
    }
    false
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
            .create(inspection_request(workspace_path()))
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
            .create(inspection_request(workspace_path()))
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
        let revision_files = std::fs::read_dir(temp.path())
            .expect("state directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
            })
            .count();
        assert_eq!(revision_files, 2);

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
    fn create_without_grants_awaits_approval() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        request.capabilities.clear();
        let created = service.create(request).expect("create");
        assert_eq!(created.state, TaskState::AwaitingApproval);
    }

    #[test]
    fn create_with_expired_capability_awaits_approval() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        request.capabilities[0].expires_at = Some(Utc::now() - chrono::TimeDelta::minutes(5));
        let created = service.create(request).expect("create");
        assert_eq!(created.state, TaskState::AwaitingApproval);
    }

    #[test]
    fn create_with_out_of_scope_capability_awaits_approval() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        request.plan.actions[0].target = outside_path().into();
        let created = service.create(request).expect("create");
        assert_eq!(created.state, TaskState::AwaitingApproval);
    }

    #[test]
    fn evaluate_appends_audit_event() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service
            .create(inspection_request(workspace_path()))
            .expect("create");

        let report = service
            .evaluate(created.plan.task_id, IsolationLevel::Sandbox, false)
            .expect("evaluate");
        assert_eq!(report.revision, 1);

        let reloaded = service.get(created.plan.task_id).expect("reload");
        assert_eq!(reloaded.revision, 1);
        let event = reloaded.events.last().expect("evaluation event");
        match &event.kind {
            TaskEventKind::Evaluated {
                isolation,
                external_side_effect_confirmed,
                decisions,
            } => {
                assert_eq!(*isolation, IsolationLevel::Sandbox);
                assert!(!external_side_effect_confirmed);
                assert_eq!(decisions, &report.decisions);
            }
            other => panic!("expected evaluation event, found {other:?}"),
        }
    }

    #[test]
    fn oversized_plan_is_rejected() {
        let mut request = inspection_request(workspace_path());
        request.plan.actions = (0..=MAX_PLAN_ACTIONS).map(|_| reason_action()).collect();
        assert_eq!(
            validate_plan(&request.plan, &[]),
            Err(ValidationError::TooManyActions {
                actions: MAX_PLAN_ACTIONS + 1,
                limit: MAX_PLAN_ACTIONS,
            })
        );
    }

    #[test]
    fn maximum_length_dependency_chain_does_not_overflow_the_stack() {
        let mut request = inspection_request(workspace_path());
        let mut actions: Vec<ActionSpec> = Vec::with_capacity(MAX_PLAN_ACTIONS);
        for index in 0..MAX_PLAN_ACTIONS {
            let mut action = reason_action();
            if index > 0 {
                action.depends_on = vec![actions[index - 1].id];
            }
            actions.push(action);
        }
        request.plan.actions = actions;
        assert_eq!(validate_plan(&request.plan, &[]), Ok(()));
    }

    fn reason_action() -> ActionSpec {
        ActionSpec {
            id: ActionId::new(),
            name: "reason".into(),
            kind: ActionKind::Reason,
            target: String::new(),
            arguments: BTreeMap::new(),
            depends_on: Vec::new(),
            required_capabilities: Vec::new(),
            risk: RiskLevel::L0Reasoning,
            recovery: RecoverySemantics::None,
        }
    }

    #[test]
    fn rejects_dependency_cycles() {
        let mut request = inspection_request(workspace_path());
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

    #[cfg(not(target_os = "windows"))]
    const fn workspace_path() -> &'static str {
        "/workspace"
    }

    #[cfg(target_os = "windows")]
    const fn workspace_path() -> &'static str {
        r"C:\workspace"
    }

    #[cfg(not(target_os = "windows"))]
    const fn outside_path() -> &'static str {
        "/elsewhere/report.txt"
    }

    #[cfg(target_os = "windows")]
    const fn outside_path() -> &'static str {
        r"C:\elsewhere\report.txt"
    }
}
