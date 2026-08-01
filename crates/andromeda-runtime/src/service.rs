use std::collections::BTreeMap;

use andromeda_core::{
    ActionId, ActionPlan, Capability, CapabilityId, IsolationLevel, PlanValidationError, TaskId,
    TaskState,
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
#[serde(deny_unknown_fields)]
pub struct CreateTaskRequest {
    pub plan: ActionPlan,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateTransitionRequest {
    pub to: TaskState,
    pub actor: String,
    pub expected_revision: u64,
}

/// Additional capabilities granted to an existing task (see
/// [`TaskService::grant_capabilities`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantCapabilitiesRequest {
    pub capabilities: Vec<Capability>,
    pub actor: String,
    /// The revision the caller believes is current; the grant is rejected
    /// with a [`StoreError::RevisionConflict`] on a mismatch.
    pub expected_revision: u64,
}

/// A policy evaluation request. Isolation is resolved *per action* so that a
/// mixed plan (for example an L2 microVM parse feeding an L3 brokered network
/// call) can be fully allowed — no single task-level isolation could satisfy
/// the non-linear [`IsolationLevel::satisfies`] matrix for both at once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationRequest {
    /// Convenience override applied to *every* action. When present it wins
    /// over `overrides` and the per-action minimums; intended for testing a
    /// whole plan under one isolation level.
    #[serde(default)]
    pub isolation: Option<IsolationLevel>,
    /// Per-action isolation overrides. Any action absent from the map is
    /// evaluated at its declared risk's minimum isolation, matching the
    /// per-action model used when a task is created.
    #[serde(default)]
    pub overrides: BTreeMap<ActionId, IsolationLevel>,
    #[serde(default)]
    pub external_side_effect_confirmed: bool,
    /// Optional requesting subject. When set, each capability required by an
    /// action must have been issued to this subject or the action is denied.
    #[serde(default)]
    pub subject: Option<String>,
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
    /// Capabilities were granted to the task after creation. `capabilities`
    /// lists the newly added grants and `plan_fully_granted` records whether,
    /// as a result, an `AwaitingApproval` task now satisfies every action's
    /// requirements (the caller may then transition it to `Ready`).
    Granted {
        capabilities: Vec<CapabilityId>,
        plan_fully_granted: bool,
    },
    /// A policy evaluation was performed; the full decision set is recorded
    /// so the audit trail preserves every authorization outcome. Isolation is
    /// resolved per action, so `effective_isolation` records the level each
    /// action was evaluated under.
    Evaluated {
        effective_isolation: BTreeMap<ActionId, IsolationLevel>,
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
    #[error("capability {0} is not active (expired or not yet issued)")]
    InactiveCapability(CapabilityId),
}

impl From<PlanValidationError> for ValidationError {
    fn from(error: PlanValidationError) -> Self {
        match error {
            PlanValidationError::DuplicateActionId(id) => Self::DuplicateAction(id),
            PlanValidationError::UnknownDependency { action, dependency } => {
                Self::MissingDependency { action, dependency }
            }
            PlanValidationError::DependencyCycle(_) => Self::DependencyCycle,
        }
    }
}

/// A state transition was structurally valid but rejected by a policy gate.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionGuardError {
    #[error(
        "transition to Ready requires a fully granted plan (missing or insufficient capabilities)"
    )]
    PlanNotFullyGranted,
    #[error("transition to Running blocked: {blocked} action(s) not allowed by policy: {}", .reasons.join(" | "))]
    PolicyBlocked {
        blocked: usize,
        reasons: Vec<String>,
    },
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Transition(#[from] andromeda_core::TaskTransitionError),
    #[error(transparent)]
    Guard(#[from] TransitionGuardError),
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
    /// Each action is evaluated under its *own* isolation level, resolved as:
    /// the request's whole-plan `isolation` override if set, else the
    /// per-action `overrides` entry, else the action's declared-risk minimum
    /// isolation. This mirrors the per-action model used at creation time and
    /// lets a mixed plan (L2 microVM + L3 brokered) be fully allowed, which a
    /// single task-level isolation level can never achieve.
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
        request: &EvaluationRequest,
    ) -> Result<EvaluationReport, ServiceError> {
        let mut record = self.store.get(task_id)?;
        let now = Utc::now();
        let mut effective_isolation = BTreeMap::new();
        let mut decisions = BTreeMap::new();
        for action in &record.plan.actions {
            let isolation = request
                .isolation
                .or_else(|| request.overrides.get(&action.id).copied())
                .unwrap_or_else(|| action.risk.minimum_isolation());
            let mut context = EvaluationContext::at(
                now,
                isolation,
                &record.capabilities,
                request.external_side_effect_confirmed,
            );
            if let Some(subject) = request.subject.as_deref() {
                context = context.with_subject(subject);
            }
            let decision = self.policy.evaluate(action, &context);
            effective_isolation.insert(action.id, isolation);
            decisions.insert(action.id, decision);
        }
        let expected = record.revision;
        record.revision += 1;
        record.events.push(TaskEvent {
            id: Uuid::new_v4(),
            occurred_at: now,
            actor: EVALUATION_ACTOR.into(),
            kind: TaskEventKind::Evaluated {
                effective_isolation,
                external_side_effect_confirmed: request.external_side_effect_confirmed,
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

    /// Grants additional capabilities to an existing task under optimistic
    /// concurrency.
    ///
    /// Each new capability must be issued to this task and active now; the
    /// grant is appended to the record, a [`TaskEventKind::Granted`] event is
    /// recorded (noting whether the plan is now fully granted), and the
    /// revision is bumped. This deliberately does **not** transition the task:
    /// a caller that observes `plan_fully_granted` on the returned record may
    /// drive the (policy-gated) `AwaitingApproval -> Ready` transition itself.
    ///
    /// # Errors
    ///
    /// Returns a validation error when a capability is issued to a different
    /// task or is not active, a revision conflict on a stale
    /// `expected_revision`, or a store error on persistence failure.
    pub fn grant_capabilities(
        &self,
        task_id: TaskId,
        request: GrantCapabilitiesRequest,
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
        let now = Utc::now();
        let expected_subject = record.plan.task_id.to_string();
        for capability in &request.capabilities {
            if capability.issued_to != expected_subject {
                return Err(ValidationError::WrongCapabilitySubject(capability.id).into());
            }
            if !capability.is_active_at(now) {
                return Err(ValidationError::InactiveCapability(capability.id).into());
            }
        }
        let granted_ids: Vec<CapabilityId> =
            request.capabilities.iter().map(|cap| cap.id).collect();
        record.capabilities.extend(request.capabilities);
        let plan_fully_granted = record.state == TaskState::AwaitingApproval
            && self.plan_fully_granted(&record.plan, &record.capabilities);
        let expected = record.revision;
        record.revision += 1;
        record.events.push(TaskEvent {
            id: Uuid::new_v4(),
            occurred_at: now,
            actor: request.actor,
            kind: TaskEventKind::Granted {
                capabilities: granted_ids,
                plan_fully_granted,
            },
        });
        self.store.save(&record, expected)?;
        Ok(record)
    }

    /// Applies one checked, optimistic-concurrency state transition.
    ///
    /// Two edges are additionally policy-gated so that `Ready`/`Running` are
    /// not reachable merely by asserting them:
    ///
    /// - `AwaitingApproval -> Ready` requires the plan to be fully granted.
    /// - `Ready -> Running` re-runs policy for every action at its per-action
    ///   minimum isolation and is rejected if any action is `Deny` or `Ask`.
    ///
    /// # Errors
    ///
    /// Returns an invalid transition, a policy-gate rejection
    /// ([`TransitionGuardError`]), a revision conflict, or a persistence
    /// error.
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
        let to = from.transition(request.to)?;
        self.guard_transition(from, to, &record)?;
        record.state = to;
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

    /// Policy gate for authorization-sensitive transitions. Structural edge
    /// validity is already checked by [`TaskState::transition`]; this adds the
    /// authorization checks that make `Ready`/`Running` meaningful.
    fn guard_transition(
        &self,
        from: TaskState,
        to: TaskState,
        record: &TaskRecord,
    ) -> Result<(), TransitionGuardError> {
        match (from, to) {
            (TaskState::AwaitingApproval, TaskState::Ready) => {
                if !self.plan_fully_granted(&record.plan, &record.capabilities) {
                    return Err(TransitionGuardError::PlanNotFullyGranted);
                }
            }
            (TaskState::Ready, TaskState::Running) => {
                let now = Utc::now();
                let reasons: Vec<String> = record
                    .plan
                    .actions
                    .iter()
                    .filter_map(|action| {
                        let context = EvaluationContext::at(
                            now,
                            action.risk.minimum_isolation(),
                            &record.capabilities,
                            true,
                        );
                        let decision = self.policy.evaluate(action, &context);
                        (decision.effect != DecisionEffect::Allow).then(|| {
                            format!(
                                "action {} -> {:?}: {}",
                                action.id,
                                decision.effect,
                                decision.reasons.join("; ")
                            )
                        })
                    })
                    .collect();
                if !reasons.is_empty() {
                    return Err(TransitionGuardError::PolicyBlocked {
                        blocked: reasons.len(),
                        reasons,
                    });
                }
            }
            _ => {}
        }
        Ok(())
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

    // Structural checks (duplicate ids, dangling dependencies, cycles) are
    // owned by the core contract's single implementation (iterative Kahn
    // topological sort), so runtime and core cannot drift apart.
    plan.validate().map_err(ValidationError::from)?;

    for action in &plan.actions {
        if !action.has_valid_risk() {
            return Err(ValidationError::InvalidRisk(action.id));
        }
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
            .evaluate(
                created.plan.task_id,
                &EvaluationRequest {
                    isolation: Some(IsolationLevel::Sandbox),
                    ..EvaluationRequest::default()
                },
            )
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
        // Compaction reclaims the superseded revision, leaving only the latest.
        assert_eq!(revision_files, 1);

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
            .evaluate(
                created.plan.task_id,
                &EvaluationRequest {
                    isolation: Some(IsolationLevel::Sandbox),
                    ..EvaluationRequest::default()
                },
            )
            .expect("evaluate");
        assert_eq!(report.revision, 1);

        let reloaded = service.get(created.plan.task_id).expect("reload");
        assert_eq!(reloaded.revision, 1);
        let event = reloaded.events.last().expect("evaluation event");
        match &event.kind {
            TaskEventKind::Evaluated {
                effective_isolation,
                external_side_effect_confirmed,
                decisions,
            } => {
                assert!(!effective_isolation.is_empty());
                assert!(
                    effective_isolation
                        .values()
                        .all(|isolation| *isolation == IsolationLevel::Sandbox)
                );
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

    fn mixed_isolation_request() -> CreateTaskRequest {
        let task_id = TaskId::new();
        let read_cap = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Files {
                root: PathBuf::from(workspace_path()),
                access: FileAccess::Read,
            },
            issued_to: task_id.to_string(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        };
        let network_cap = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Network {
                host: "api.example.com".into(),
                port: None,
            },
            issued_to: task_id.to_string(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        };
        let parse = ActionSpec {
            id: ActionId::new(),
            name: "parse untrusted download".into(),
            kind: ActionKind::ParseUntrustedContent,
            target: download_file().into(),
            arguments: BTreeMap::new(),
            depends_on: Vec::new(),
            required_capabilities: vec![read_cap.id],
            risk: RiskLevel::L2StrongIsolation,
            recovery: RecoverySemantics::None,
        };
        let network = ActionSpec {
            id: ActionId::new(),
            name: "send result through broker".into(),
            kind: ActionKind::NetworkRequest,
            target: "api.example.com".into(),
            arguments: BTreeMap::new(),
            depends_on: vec![parse.id],
            required_capabilities: vec![network_cap.id],
            risk: RiskLevel::L3ExternalSideEffect,
            recovery: RecoverySemantics::None,
        };
        let plan = ActionPlan {
            schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
            task_id,
            intent: Intent::new("Parse then send", "test"),
            actions: vec![parse, network],
        };
        CreateTaskRequest {
            plan,
            capabilities: vec![read_cap, network_cap],
            actor: "test".into(),
        }
    }

    #[test]
    fn mixed_isolation_plan_is_fully_allowed_per_action() {
        // A plan mixing an L2 microVM parse (satisfied only by MicroVm) and an
        // L3 brokered network call (satisfied only by Brokered) can never be
        // all-Allow under a single task-level isolation, because no isolation
        // level satisfies both. Per-action isolation fixes this.
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service.create(mixed_isolation_request()).expect("create");
        assert_eq!(created.state, TaskState::Ready);

        let report = service
            .evaluate(
                created.plan.task_id,
                &EvaluationRequest {
                    external_side_effect_confirmed: true,
                    ..EvaluationRequest::default()
                },
            )
            .expect("evaluate");
        assert_eq!(report.decisions.len(), 2);
        assert!(
            report
                .decisions
                .values()
                .all(|decision| decision.effect == DecisionEffect::Allow),
            "expected every action Allow under per-action isolation, got {:?}",
            report.decisions
        );
    }

    #[test]
    fn granting_capabilities_unblocks_awaiting_approval() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        let needed = request.capabilities[0].clone();
        request.capabilities.clear();
        let created = service.create(request).expect("create");
        assert_eq!(created.state, TaskState::AwaitingApproval);

        let granted = service
            .grant_capabilities(
                created.plan.task_id,
                GrantCapabilitiesRequest {
                    capabilities: vec![needed],
                    actor: "approver".into(),
                    expected_revision: 0,
                },
            )
            .expect("grant");
        assert_eq!(granted.revision, 1);
        match &granted.events.last().expect("granted event").kind {
            TaskEventKind::Granted {
                plan_fully_granted, ..
            } => assert!(plan_fully_granted),
            other => panic!("expected granted event, found {other:?}"),
        }

        let ready = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Ready,
                    actor: "approver".into(),
                    expected_revision: 1,
                },
            )
            .expect("ready");
        assert_eq!(ready.state, TaskState::Ready);
    }

    #[test]
    fn ungranted_awaiting_approval_cannot_reach_ready() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        request.capabilities.clear();
        let created = service.create(request).expect("create");
        assert_eq!(created.state, TaskState::AwaitingApproval);

        let error = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Ready,
                    actor: "sneaky".into(),
                    expected_revision: 0,
                },
            )
            .expect_err("ungranted approval must not reach Ready");
        assert!(matches!(
            error,
            ServiceError::Guard(TransitionGuardError::PlanNotFullyGranted)
        ));
    }

    #[test]
    fn ready_to_running_is_rejected_when_an_action_is_denied() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        // Seed a record already in Ready whose action has no covering grant,
        // so the Ready -> Running policy re-check must block it.
        let task_id = TaskId::new();
        let plan = ActionPlan {
            schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
            task_id,
            intent: Intent::new("ungranted ready", "test"),
            actions: vec![ActionSpec {
                id: ActionId::new(),
                name: "read".into(),
                kind: ActionKind::ReadFile,
                target: workspace_path().into(),
                arguments: BTreeMap::new(),
                depends_on: Vec::new(),
                required_capabilities: vec![CapabilityId::new()],
                risk: RiskLevel::L1Sandboxed,
                recovery: RecoverySemantics::None,
            }],
        };
        let record = TaskRecord {
            plan,
            state: TaskState::Ready,
            revision: 0,
            capabilities: Vec::new(),
            events: Vec::new(),
        };
        service.store.create(&record).expect("seed ready record");

        let error = service
            .transition(
                task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "runner".into(),
                    expected_revision: 0,
                },
            )
            .expect_err("denied action must block Running");
        assert!(matches!(
            error,
            ServiceError::Guard(TransitionGuardError::PolicyBlocked { .. })
        ));
    }

    #[cfg(not(target_os = "windows"))]
    const fn download_file() -> &'static str {
        "/workspace/unknown.zip"
    }

    #[cfg(target_os = "windows")]
    const fn download_file() -> &'static str {
        r"C:\workspace\unknown.zip"
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
