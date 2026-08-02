use std::collections::{BTreeMap, BTreeSet};

use andromeda_core::{
    ActionId, ActionOutcome, ActionPlan, Capability, CapabilityId, IsolationLevel, OutcomeStatus,
    PlanValidationError, TaskId, TaskState,
};
use andromeda_policy::{DecisionEffect, EvaluationContext, PolicyDecision, PolicyEngine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::admission::{AdmissionError, CapabilityAdmission};
use crate::store::TaskListing;
use crate::{FileTaskStore, StoreError};

/// Upper bound on the number of actions accepted in a single plan.
///
/// The limit caps validation and policy-evaluation work per request and
/// bounds the size of persisted task records. Plans above the limit are
/// rejected with [`ValidationError::TooManyActions`]. 10 000 actions is far
/// beyond any realistic plan while keeping worst-case validation cheap.
pub const MAX_PLAN_ACTIONS: usize = 10_000;

/// Upper bound on the capabilities a single task may accumulate.
///
/// Grants were previously unbounded while actions were capped, so a caller
/// could attach an arbitrarily long capability list and have every policy
/// evaluation scan all of it — and every write persist and fsync all of it.
/// The bound covers the task's *total* after a grant, not one request, so
/// repeated `grant_capabilities` calls cannot walk past it.
///
/// The limit is deliberately far above any real plan: a capability exists to
/// scope one action's resource, so a plan at the action ceiling needs at most
/// a comparable number of grants — which is why this ceiling is *defined as*
/// [`MAX_PLAN_ACTIONS`] rather than as an independent literal.
///
/// The two bounds compound: policy evaluation runs once per action, so the
/// worst case grows with the product of per-action required-capability ids
/// and held capabilities. The policy engine resolves required ids through a
/// hash map of the held capabilities built once per evaluation, so that
/// product costs O(1) lookups plus one O(held) map build per action — it no
/// longer multiplies into per-id linear scans of the full capability list.
pub const MAX_TASK_CAPABILITIES: usize = MAX_PLAN_ACTIONS;

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
    /// Whether the caller has obtained the final human confirmation that an
    /// L3 external side effect requires.
    ///
    /// Defaults to `false`, so the `Ready -> Running` gate rejects a plan
    /// containing unconfirmed external side effects unless the caller states
    /// otherwise. On that edge — the only one whose gate consumes the value —
    /// it is recorded on the resulting [`TaskEventKind::StateChanged`] event
    /// so the audit trail shows who asserted the confirmation; other edges
    /// record no confirmation.
    ///
    /// v0 boundary: the confirmation is *asserted* by the caller, not attested
    /// by a trusted broker. Until a host confirmation broker exists this gate
    /// proves that a confirmation step was taken, not who took it.
    #[serde(default)]
    pub external_side_effect_confirmed: bool,
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

/// One recorded execution outcome for a single action (see
/// [`TaskService::record_outcome`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordOutcomeRequest {
    pub outcome: ActionOutcome,
    pub actor: String,
    /// The revision the caller believes is current; the record is rejected
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
    /// A state transition was applied. `external_side_effect_confirmed` is
    /// `Some` only on the `Ready -> Running` edge — the one transition whose
    /// gate consumes the confirmation — and records the value the caller
    /// supplied there, so the audit trail preserves whether the L3 commit
    /// point was confirmed and by which actor. Every other edge records
    /// `None` rather than stamping a meaningless `false`/`true` on events
    /// where no confirmation was consulted. Old records that persisted a
    /// plain bool deserialize as `Some(bool)`; records without the field
    /// deserialize as `None`.
    StateChanged {
        from: TaskState,
        to: TaskState,
        #[serde(default)]
        external_side_effect_confirmed: Option<bool>,
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
    /// An execution outcome was recorded for one action. The outcome itself
    /// lives in [`TaskRecord::outcomes`]; the event marks when and by whom it
    /// was appended.
    OutcomeRecorded {
        action_id: ActionId,
        status: OutcomeStatus,
        evidence_count: usize,
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
    /// Execution outcomes recorded so far, at most one per action. The
    /// `Verifying -> Succeeded` gate requires every planned action to have a
    /// successful, evidenced outcome here, so a task cannot be declared
    /// successful merely by asserting the state.
    #[serde(default)]
    pub outcomes: Vec<ActionOutcome>,
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
    #[error("task would hold {capabilities} capabilities, which exceeds the limit of {limit}")]
    TooManyCapabilities { capabilities: usize, limit: usize },
    #[error("action {0} declares risk below its operation floor")]
    InvalidRisk(ActionId),
    #[error("capability {0} is issued to a different task")]
    WrongCapabilitySubject(CapabilityId),
    #[error("capability {0} is not active (expired or not yet issued)")]
    InactiveCapability(CapabilityId),
    #[error("outcome references action {0}, which is not part of the plan")]
    UnknownOutcomeAction(ActionId),
    #[error("action {0} already has a recorded outcome")]
    DuplicateOutcome(ActionId),
    #[error("outcomes may only be recorded while Running or Verifying, not in {0:?}")]
    OutcomeNotAllowedInState(TaskState),
    #[error(
        "outcome for action {action} finished at {finished_at}, before it started at {started_at}"
    )]
    OutcomeFinishedBeforeStarted {
        action: ActionId,
        started_at: DateTime<Utc>,
        finished_at: DateTime<Utc>,
    },
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
    /// The plan contains L3 external side effects and the transition did not
    /// carry the final confirmation. This is the commit point promised by the
    /// product security boundary; it is deliberately **not** implied.
    #[error(
        "transition to Running requires explicit external side-effect confirmation for action(s): {}",
        .actions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    ExternalConfirmationRequired { actions: Vec<ActionId> },
    /// `Verifying -> Succeeded` was attempted while some planned action had no
    /// recorded outcome at all.
    #[error(
        "transition to Succeeded requires a recorded outcome for every action; missing: {}",
        .actions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    MissingOutcomes { actions: Vec<ActionId> },
    /// An action's recorded outcome is not compatible with overall success.
    #[error(
        "transition to Succeeded blocked by non-successful outcome(s) for action(s): {}",
        .actions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    UnsuccessfulOutcomes { actions: Vec<ActionId> },
    /// An action's outcome carried no evidence, so success would be
    /// self-asserted rather than demonstrated.
    #[error(
        "transition to Succeeded requires evidence on every outcome; action(s) without evidence: {}",
        .actions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    MissingEvidence { actions: Vec<ActionId> },
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
    /// A capability was refused by the configured [`CapabilityAdmission`].
    #[error(transparent)]
    Admission(#[from] AdmissionError),
}

#[derive(Debug, Clone)]
pub struct TaskService {
    store: FileTaskStore,
    policy: PolicyEngine,
    admission: CapabilityAdmission,
}

impl TaskService {
    /// Builds a service.
    ///
    /// `admission` is a required argument on purpose: whether the control
    /// plane accepts self-minted capabilities is a security posture, and a
    /// posture that can be reached by leaving an argument off is one nobody
    /// chose. [`CapabilityAdmission`] has no `Default` for the same reason.
    #[must_use]
    pub const fn new(
        store: FileTaskStore,
        policy: PolicyEngine,
        admission: CapabilityAdmission,
    ) -> Self {
        Self {
            store,
            policy,
            admission,
        }
    }

    /// The capability admission policy in force, for `/healthz` and logs.
    #[must_use]
    pub const fn admission(&self) -> &CapabilityAdmission {
        &self.admission
    }

    /// Validates and durably creates a task.
    ///
    /// # Errors
    ///
    /// Returns validation errors for malformed/untrusted plans and store errors
    /// for persistence failures.
    pub fn create(&self, request: CreateTaskRequest) -> Result<TaskRecord, ServiceError> {
        // Order matters and is load-bearing: `validate_plan` enforces
        // MAX_TASK_CAPABILITIES, so by the time `admit` runs the vector it
        // verifies is already bounded. Signature verification must never be
        // reachable with an unbounded input.
        validate_plan(&request.plan, &request.capabilities)?;
        self.admission
            .admit(&request.capabilities, MAX_TASK_CAPABILITIES)?;
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
            outcomes: Vec::new(),
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
        // Bound the resulting total, not this request: otherwise repeated
        // grants of an allowed size would walk past the limit. Saturating so
        // the security check itself has no debug-build overflow panic path.
        let total = record
            .capabilities
            .len()
            .saturating_add(request.capabilities.len());
        if total > MAX_TASK_CAPABILITIES {
            return Err(ValidationError::TooManyCapabilities {
                capabilities: total,
                limit: MAX_TASK_CAPABILITIES,
            }
            .into());
        }
        // The bound above covers this request too: `total` includes
        // `request.capabilities.len()`, so a request over the limit is rejected
        // before any signature is verified. Same ordering rule as `create`.
        self.admission
            .admit(&request.capabilities, MAX_TASK_CAPABILITIES)?;
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

    /// Records one action's execution outcome under optimistic concurrency.
    ///
    /// Outcomes are the evidence substrate for the `Verifying -> Succeeded`
    /// gate: an action without a recorded, evidenced, successful outcome
    /// cannot contribute to a successful task. Outcomes are append-only —
    /// re-recording an action is rejected rather than overwriting history —
    /// and may only be recorded while the task is `Running` or `Verifying`.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the outcome references an unknown
    /// action, the action already has an outcome, the outcome claims to have
    /// finished before it started, or the task is in a state that does not
    /// accept outcomes; a revision conflict on a stale `expected_revision`;
    /// or a store error on persistence failure.
    pub fn record_outcome(
        &self,
        task_id: TaskId,
        request: RecordOutcomeRequest,
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
        if !matches!(record.state, TaskState::Running | TaskState::Verifying) {
            return Err(ValidationError::OutcomeNotAllowedInState(record.state).into());
        }
        let action_id = request.outcome.action_id;
        let planned: BTreeSet<ActionId> =
            record.plan.actions.iter().map(|action| action.id).collect();
        if !planned.contains(&action_id) {
            return Err(ValidationError::UnknownOutcomeAction(action_id).into());
        }
        let recorded: BTreeSet<ActionId> = record
            .outcomes
            .iter()
            .map(|outcome| outcome.action_id)
            .collect();
        if recorded.contains(&action_id) {
            return Err(ValidationError::DuplicateOutcome(action_id).into());
        }
        if request.outcome.finished_at < request.outcome.started_at {
            return Err(ValidationError::OutcomeFinishedBeforeStarted {
                action: action_id,
                started_at: request.outcome.started_at,
                finished_at: request.outcome.finished_at,
            }
            .into());
        }

        let status = request.outcome.status;
        let evidence_count = request.outcome.evidence.len();
        record.outcomes.push(request.outcome);
        let expected = record.revision;
        record.revision += 1;
        record.events.push(TaskEvent {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor: request.actor,
            kind: TaskEventKind::OutcomeRecorded {
                action_id,
                status,
                evidence_count,
            },
        });
        self.store.save(&record, expected)?;
        Ok(record)
    }

    /// Applies one checked, optimistic-concurrency state transition.
    ///
    /// Three edges are additionally gated so that `Ready`/`Running`/`Succeeded`
    /// are not reachable merely by asserting them:
    ///
    /// - `AwaitingApproval -> Ready` requires the plan to be fully granted.
    /// - `Ready -> Running` re-runs policy for every action at its per-action
    ///   minimum isolation, using the confirmation the caller supplied on the
    ///   request. Any `Deny` blocks the transition; an `Ask` (an unconfirmed
    ///   L3 external side effect) blocks it with
    ///   [`TransitionGuardError::ExternalConfirmationRequired`].
    /// - `Verifying -> Succeeded` requires every planned action to have a
    ///   recorded outcome that is successful and carries evidence.
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
        self.guard_transition(from, to, &record, request.external_side_effect_confirmed)?;
        record.state = to;
        record.revision += 1;
        // Only the `Ready -> Running` gate consumes the confirmation, so only
        // that edge stamps it on the audit event; every other edge records
        // `None` to keep the signal precise.
        let external_side_effect_confirmed = ((from, to) == (TaskState::Ready, TaskState::Running))
            .then_some(request.external_side_effect_confirmed);
        record.events.push(TaskEvent {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor: request.actor,
            kind: TaskEventKind::StateChanged {
                from,
                to: request.to,
                external_side_effect_confirmed,
            },
        });
        self.store.save(&record, request.expected_revision)?;
        Ok(record)
    }

    /// Policy gate for authorization-sensitive transitions. Structural edge
    /// validity is already checked by [`TaskState::transition`]; this adds the
    /// authorization and evidence checks that make `Ready`/`Running`/
    /// `Succeeded` meaningful.
    fn guard_transition(
        &self,
        from: TaskState,
        to: TaskState,
        record: &TaskRecord,
        external_side_effect_confirmed: bool,
    ) -> Result<(), TransitionGuardError> {
        match (from, to) {
            (TaskState::AwaitingApproval, TaskState::Ready) => {
                if !self.plan_fully_granted(&record.plan, &record.capabilities) {
                    return Err(TransitionGuardError::PlanNotFullyGranted);
                }
            }
            (TaskState::Ready, TaskState::Running) => {
                self.guard_ready_to_running(record, external_side_effect_confirmed)?;
            }
            (TaskState::Verifying, TaskState::Succeeded) => {
                Self::guard_verifying_to_succeeded(record)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Re-runs the deterministic policy engine over every action at the moment
    /// of execution, using the confirmation the caller actually supplied.
    ///
    /// `Ask` and `Deny` are reported separately: an `Ask` means the action is
    /// otherwise authorized but is an L3 external side effect awaiting its
    /// final confirmation, which is a different operator action than fixing a
    /// missing or expired grant.
    fn guard_ready_to_running(
        &self,
        record: &TaskRecord,
        external_side_effect_confirmed: bool,
    ) -> Result<(), TransitionGuardError> {
        let now = Utc::now();
        let mut denied = Vec::new();
        let mut awaiting_confirmation = Vec::new();
        for action in &record.plan.actions {
            let context = EvaluationContext::at(
                now,
                action.risk.minimum_isolation(),
                &record.capabilities,
                external_side_effect_confirmed,
            );
            let decision = self.policy.evaluate(action, &context);
            match decision.effect {
                DecisionEffect::Allow => {}
                DecisionEffect::Ask => awaiting_confirmation.push(action.id),
                DecisionEffect::Deny => denied.push(format!(
                    "action {} -> {:?}: {}",
                    action.id,
                    decision.effect,
                    decision.reasons.join("; ")
                )),
            }
        }
        if !denied.is_empty() {
            return Err(TransitionGuardError::PolicyBlocked {
                blocked: denied.len(),
                reasons: denied,
            });
        }
        if !awaiting_confirmation.is_empty() {
            return Err(TransitionGuardError::ExternalConfirmationRequired {
                actions: awaiting_confirmation,
            });
        }
        Ok(())
    }

    /// Requires demonstrated, not asserted, success: every planned action must
    /// have a recorded outcome, that outcome must be compatible with overall
    /// success, and it must carry at least one piece of evidence.
    fn guard_verifying_to_succeeded(record: &TaskRecord) -> Result<(), TransitionGuardError> {
        let recorded: BTreeMap<ActionId, &ActionOutcome> = record
            .outcomes
            .iter()
            .map(|outcome| (outcome.action_id, outcome))
            .collect();
        let mut missing = Vec::new();
        let mut unsuccessful = Vec::new();
        let mut without_evidence = Vec::new();
        for action in &record.plan.actions {
            let Some(outcome) = recorded.get(&action.id) else {
                missing.push(action.id);
                continue;
            };
            match outcome.status {
                OutcomeStatus::Succeeded | OutcomeStatus::Skipped => {
                    if outcome.evidence.is_empty() {
                        without_evidence.push(action.id);
                    }
                }
                OutcomeStatus::Failed | OutcomeStatus::RolledBack | OutcomeStatus::Compensated => {
                    unsuccessful.push(action.id);
                }
            }
        }
        if !missing.is_empty() {
            return Err(TransitionGuardError::MissingOutcomes { actions: missing });
        }
        if !unsuccessful.is_empty() {
            return Err(TransitionGuardError::UnsuccessfulOutcomes {
                actions: unsuccessful,
            });
        }
        if !without_evidence.is_empty() {
            return Err(TransitionGuardError::MissingEvidence {
                actions: without_evidence,
            });
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
    /// confirmation; both are re-evaluated with real values on the
    /// `Ready -> Running` edge (see [`TaskService::guard_ready_to_running`]),
    /// which is where an unconfirmed L3 side effect is actually blocked. This
    /// is why `Ready` means "authorized in principle", not "cleared to run".
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

    if capabilities.len() > MAX_TASK_CAPABILITIES {
        return Err(ValidationError::TooManyCapabilities {
            capabilities: capabilities.len(),
            limit: MAX_TASK_CAPABILITIES,
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
        ActionKind, ActionSpec, CapabilityResource, Evidence, FileAccess, Intent,
        RecoverySemantics, RiskLevel,
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
            signature: None,
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
        service_with(temp, CapabilityAdmission::unsigned_for_development())
    }

    fn service_with(temp: &TempDir, admission: CapabilityAdmission) -> TaskService {
        TaskService::new(
            FileTaskStore::open(temp.path()).expect("store"),
            PolicyEngine::new(PolicySet::default()),
            admission,
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
                    external_side_effect_confirmed: false,
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
                    external_side_effect_confirmed: false,
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
    fn plan_at_the_action_limit_is_accepted() {
        // Pins the boundary from the acceptance side: exactly
        // MAX_PLAN_ACTIONS is legal, so the rejection test above cannot be
        // satisfied by an off-by-one `>=` in the bound check.
        let mut request = inspection_request(workspace_path());
        request.plan.actions = (0..MAX_PLAN_ACTIONS).map(|_| reason_action()).collect();
        assert_eq!(validate_plan(&request.plan, &[]), Ok(()));
    }

    #[test]
    fn oversized_capability_list_is_rejected() {
        // Actions were capped but grants were not, so a caller could attach an
        // unbounded capability list that every evaluation scans and every
        // write persists.
        let request = inspection_request(workspace_path());
        let capability = request.capabilities[0].clone();
        let capabilities: Vec<Capability> =
            std::iter::repeat_n(capability, MAX_TASK_CAPABILITIES + 1).collect();
        assert_eq!(
            validate_plan(&request.plan, &capabilities),
            Err(ValidationError::TooManyCapabilities {
                capabilities: MAX_TASK_CAPABILITIES + 1,
                limit: MAX_TASK_CAPABILITIES,
            })
        );
    }

    #[test]
    fn capability_list_at_the_limit_is_accepted() {
        // Pins the boundary from the acceptance side: exactly
        // MAX_TASK_CAPABILITIES is legal, so the rejection test above cannot
        // be satisfied by an off-by-one `>=` in the bound check.
        let request = inspection_request(workspace_path());
        let capability = request.capabilities[0].clone();
        let capabilities: Vec<Capability> =
            std::iter::repeat_n(capability, MAX_TASK_CAPABILITIES).collect();
        assert_eq!(validate_plan(&request.plan, &capabilities), Ok(()));
    }

    /// Fixed seed: the runtime's admission tests must be reproducible, so no
    /// key material comes from an RNG.
    const ISSUER_SEED: [u8; 32] = [5u8; 32];
    const ISSUER_KEY_ID: &str = "issuer-2026";

    fn issuer() -> andromeda_core::CapabilitySigningKey {
        andromeda_core::CapabilitySigningKey::from_seed(&ISSUER_SEED)
    }

    fn signing_admission() -> CapabilityAdmission {
        let mut keyring = andromeda_core::CapabilityKeyring::new();
        keyring
            .insert_hex(ISSUER_KEY_ID, &issuer().verifying_key_hex())
            .expect("valid key");
        CapabilityAdmission::require_signed(keyring).expect("non-empty keyring")
    }

    #[test]
    fn create_accepts_a_capability_the_trusted_issuer_signed() {
        let temp = TempDir::new().expect("tempdir");
        let service = service_with(&temp, signing_admission());
        let mut request = inspection_request(workspace_path());
        issuer()
            .sign_in_place(&mut request.capabilities[0], ISSUER_KEY_ID)
            .expect("sign");
        let record = service.create(request).expect("signed capability admitted");
        assert_eq!(record.state, TaskState::Ready);
    }

    #[test]
    fn create_refuses_an_unsigned_capability_when_signatures_are_required() {
        let temp = TempDir::new().expect("tempdir");
        let service = service_with(&temp, signing_admission());
        let error = service
            .create(inspection_request(workspace_path()))
            .expect_err("unsigned capability must be refused");
        assert!(
            matches!(
                error,
                ServiceError::Admission(AdmissionError::Rejected { .. })
            ),
            "{error:?}"
        );
    }

    #[test]
    fn create_refuses_a_capability_tampered_with_after_signing() {
        let temp = TempDir::new().expect("tempdir");
        let service = service_with(&temp, signing_admission());
        let mut request = inspection_request(workspace_path());
        issuer()
            .sign_in_place(&mut request.capabilities[0], ISSUER_KEY_ID)
            .expect("sign");
        // Widen the grant to the filesystem root after the issuer vouched for
        // a single directory — the exact escalation the signature exists to
        // stop.
        request.capabilities[0].resource = CapabilityResource::Files {
            root: PathBuf::from(outside_path()),
            access: FileAccess::ReadWrite,
        };
        let error = service
            .create(request)
            .expect_err("tampering must be caught");
        assert!(
            matches!(
                error,
                ServiceError::Admission(AdmissionError::Rejected { .. })
            ),
            "{error:?}"
        );
    }

    #[test]
    fn grant_refuses_an_unsigned_capability_when_signatures_are_required() {
        let temp = TempDir::new().expect("tempdir");
        let service = service_with(&temp, signing_admission());
        let mut request = inspection_request(workspace_path());
        let unsigned = request.capabilities[0].clone();
        issuer()
            .sign_in_place(&mut request.capabilities[0], ISSUER_KEY_ID)
            .expect("sign");
        let created = service.create(request).expect("create");
        let error = service
            .grant_capabilities(
                created.plan.task_id,
                GrantCapabilitiesRequest {
                    capabilities: vec![unsigned],
                    actor: "caller".into(),
                    expected_revision: created.revision,
                },
            )
            .expect_err("the grant path must enforce the same rule as create");
        assert!(
            matches!(
                error,
                ServiceError::Admission(AdmissionError::Rejected { .. })
            ),
            "{error:?}"
        );
    }

    /// The bound must be enforced *before* any signature is verified, or an
    /// unauthenticated-shaped request could force unbounded ed25519 work. The
    /// capabilities here are both over the limit and individually inadmissible,
    /// so the error names whichever check ran first — and it must be the bound.
    #[test]
    fn the_capability_bound_is_enforced_before_signature_verification() {
        let temp = TempDir::new().expect("tempdir");
        let service = service_with(&temp, signing_admission());
        let mut request = inspection_request(workspace_path());
        let capability = request.capabilities[0].clone();
        request.capabilities = std::iter::repeat_n(capability, MAX_TASK_CAPABILITIES + 1).collect();
        let error = service.create(request).expect_err("must be rejected");
        assert!(
            matches!(
                error,
                ServiceError::Validation(ValidationError::TooManyCapabilities {
                    capabilities: c,
                    limit: MAX_TASK_CAPABILITIES,
                }) if c == MAX_TASK_CAPABILITIES + 1
            ),
            "the length bound must reject before verification, got {error:?}"
        );
    }

    #[test]
    fn repeated_grants_cannot_walk_past_the_capability_limit() {
        // The bound is on the resulting total, so a sequence of individually
        // legal grants still cannot exceed it.
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        let capability = request.capabilities[0].clone();
        request.capabilities.clear();
        let created = service.create(request).expect("create");

        // Seed the record exactly at the ceiling without going through the
        // (deliberately bounded) grant path.
        let mut seeded = service.get(created.plan.task_id).expect("reload");
        seeded.capabilities =
            std::iter::repeat_n(capability.clone(), MAX_TASK_CAPABILITIES).collect();
        let expected = seeded.revision;
        seeded.revision += 1;
        service.store.save(&seeded, expected).expect("seed");

        let error = service
            .grant_capabilities(
                created.plan.task_id,
                GrantCapabilitiesRequest {
                    capabilities: vec![capability],
                    actor: "greedy".into(),
                    expected_revision: seeded.revision,
                },
            )
            .expect_err("one more grant must exceed the ceiling");
        // Exact payload: the error must report the post-grant TOTAL, not the
        // size of the rejected request.
        match error {
            ServiceError::Validation(ValidationError::TooManyCapabilities {
                capabilities,
                limit,
            }) => {
                assert_eq!(capabilities, MAX_TASK_CAPABILITIES + 1);
                assert_eq!(limit, MAX_TASK_CAPABILITIES);
            }
            other => panic!("expected TooManyCapabilities, found {other:?}"),
        }
    }

    #[test]
    fn grant_landing_exactly_on_the_capability_limit_is_accepted() {
        // Pins the grant-path boundary from the acceptance side: a grant
        // whose post-grant total is exactly MAX_TASK_CAPABILITIES must
        // succeed, so the rejection test above cannot be satisfied by an
        // off-by-one `>=` in the grant gate.
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let mut request = inspection_request(workspace_path());
        let capability = request.capabilities[0].clone();
        request.capabilities.clear();
        let created = service.create(request).expect("create");

        // Seed the record one below the ceiling without going through the
        // grant path, then traverse the real grant bound check at the limit.
        let mut seeded = service.get(created.plan.task_id).expect("reload");
        seeded.capabilities =
            std::iter::repeat_n(capability.clone(), MAX_TASK_CAPABILITIES - 1).collect();
        let expected = seeded.revision;
        seeded.revision += 1;
        service.store.save(&seeded, expected).expect("seed");

        let granted = service
            .grant_capabilities(
                created.plan.task_id,
                GrantCapabilitiesRequest {
                    capabilities: vec![capability],
                    actor: "approver".into(),
                    expected_revision: seeded.revision,
                },
            )
            .expect("a grant landing exactly on the limit must succeed");
        assert_eq!(granted.capabilities.len(), MAX_TASK_CAPABILITIES);
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
            signature: None,
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
            signature: None,
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
                    external_side_effect_confirmed: false,
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
                    external_side_effect_confirmed: false,
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
            outcomes: Vec::new(),
        };
        service.store.create(&record).expect("seed ready record");

        let error = service
            .transition(
                task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "runner".into(),
                    expected_revision: 0,
                    external_side_effect_confirmed: false,
                },
            )
            .expect_err("denied action must block Running");
        assert!(matches!(
            error,
            ServiceError::Guard(TransitionGuardError::PolicyBlocked { .. })
        ));
    }

    // --- L3 external side-effect confirmation gate -------------------------

    #[test]
    fn l3_plan_cannot_start_without_explicit_confirmation() {
        // Regression guard: the Ready -> Running gate previously hardcoded
        // `external_side_effect_confirmed = true`, which made the L3 commit
        // point promised by the security boundary unreachable dead code.
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service.create(mixed_isolation_request()).expect("create");
        assert_eq!(created.state, TaskState::Ready);

        let error = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "runner".into(),
                    expected_revision: 0,
                    external_side_effect_confirmed: false,
                },
            )
            .expect_err("unconfirmed L3 side effect must not reach Running");
        match error {
            ServiceError::Guard(TransitionGuardError::ExternalConfirmationRequired { actions }) => {
                assert_eq!(actions.len(), 1, "only the L3 action awaits confirmation");
            }
            other => panic!("expected ExternalConfirmationRequired, found {other:?}"),
        }

        // The rejected transition must not have advanced the record.
        assert_eq!(
            service.get(created.plan.task_id).expect("reload").revision,
            0
        );
    }

    #[test]
    fn confirmed_l3_plan_starts_and_records_the_confirmation() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service.create(mixed_isolation_request()).expect("create");

        let running = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "operator".into(),
                    expected_revision: 0,
                    external_side_effect_confirmed: true,
                },
            )
            .expect("confirmed L3 transition");
        assert_eq!(running.state, TaskState::Running);
        // The confirmation is preserved for audit, attributed to the actor.
        let event = running.events.last().expect("state change event");
        assert_eq!(event.actor, "operator");
        match &event.kind {
            TaskEventKind::StateChanged {
                external_side_effect_confirmed,
                ..
            } => assert_eq!(*external_side_effect_confirmed, Some(true)),
            other => panic!("expected StateChanged, found {other:?}"),
        }

        // Edges whose gate does not consult the confirmation record `None`
        // instead of stamping a meaningless value on the audit trail.
        let verifying = service
            .transition(
                created.plan.task_id,
                StateTransitionRequest {
                    to: TaskState::Verifying,
                    actor: "runner".into(),
                    expected_revision: running.revision,
                    external_side_effect_confirmed: false,
                },
            )
            .expect("verifying");
        match &verifying.events.last().expect("state change event").kind {
            TaskEventKind::StateChanged {
                external_side_effect_confirmed,
                ..
            } => assert_eq!(*external_side_effect_confirmed, None),
            other => panic!("expected StateChanged, found {other:?}"),
        }
    }

    #[test]
    fn l1_only_plan_does_not_need_confirmation() {
        // The confirmation gate must fire only for real external side effects.
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
                    external_side_effect_confirmed: false,
                },
            )
            .expect("L1 plan needs no external confirmation");
        assert_eq!(running.state, TaskState::Running);
    }

    // --- Verifying -> Succeeded evidence gate ------------------------------

    /// Drives a freshly created L1 task to `Verifying`, returning the service,
    /// the task id, the single action id, and the current revision.
    fn task_in_verifying(temp: &TempDir) -> (TaskService, TaskId, ActionId, u64) {
        let service = service(temp);
        let created = service
            .create(inspection_request(workspace_path()))
            .expect("create");
        let task_id = created.plan.task_id;
        let action_id = created.plan.actions[0].id;
        let running = service
            .transition(
                task_id,
                StateTransitionRequest {
                    to: TaskState::Running,
                    actor: "runner".into(),
                    expected_revision: 0,
                    external_side_effect_confirmed: false,
                },
            )
            .expect("running");
        let verifying = service
            .transition(
                task_id,
                StateTransitionRequest {
                    to: TaskState::Verifying,
                    actor: "runner".into(),
                    expected_revision: running.revision,
                    external_side_effect_confirmed: false,
                },
            )
            .expect("verifying");
        (service, task_id, action_id, verifying.revision)
    }

    fn outcome(
        action_id: ActionId,
        status: OutcomeStatus,
        evidence: Vec<Evidence>,
    ) -> ActionOutcome {
        let now = Utc::now();
        ActionOutcome {
            action_id,
            status,
            started_at: now,
            finished_at: now,
            evidence,
            error: None,
        }
    }

    fn evidence() -> Vec<Evidence> {
        vec![Evidence {
            kind: "assertion".into(),
            summary: "directory listing matched the expected entries".into(),
            attributes: BTreeMap::new(),
        }]
    }

    fn succeed(
        service: &TaskService,
        task_id: TaskId,
        revision: u64,
    ) -> Result<TaskRecord, ServiceError> {
        service.transition(
            task_id,
            StateTransitionRequest {
                to: TaskState::Succeeded,
                actor: "verifier".into(),
                expected_revision: revision,
                external_side_effect_confirmed: false,
            },
        )
    }

    #[test]
    fn succeeded_requires_a_recorded_outcome_for_every_action() {
        // Regression guard: Verifying -> Succeeded was previously ungated, so
        // any caller could mark a task successful without executing anything.
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, _, revision) = task_in_verifying(&temp);
        let error = succeed(&service, task_id, revision).expect_err("no outcomes recorded");
        assert!(matches!(
            error,
            ServiceError::Guard(TransitionGuardError::MissingOutcomes { .. })
        ));
    }

    #[test]
    fn succeeded_requires_evidence_on_each_outcome() {
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, action_id, revision) = task_in_verifying(&temp);
        let recorded = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: outcome(action_id, OutcomeStatus::Succeeded, Vec::new()),
                    actor: "executor".into(),
                    expected_revision: revision,
                },
            )
            .expect("record outcome");
        let error =
            succeed(&service, task_id, recorded.revision).expect_err("outcome carries no evidence");
        assert!(matches!(
            error,
            ServiceError::Guard(TransitionGuardError::MissingEvidence { .. })
        ));
    }

    #[test]
    fn succeeded_is_blocked_by_a_failed_outcome() {
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, action_id, revision) = task_in_verifying(&temp);
        let recorded = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: outcome(action_id, OutcomeStatus::Failed, evidence()),
                    actor: "executor".into(),
                    expected_revision: revision,
                },
            )
            .expect("record outcome");
        let error = succeed(&service, task_id, recorded.revision).expect_err("failed outcome");
        assert!(matches!(
            error,
            ServiceError::Guard(TransitionGuardError::UnsuccessfulOutcomes { .. })
        ));
    }

    #[test]
    fn evidenced_successful_outcome_allows_success() {
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, action_id, revision) = task_in_verifying(&temp);
        let recorded = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: outcome(action_id, OutcomeStatus::Succeeded, evidence()),
                    actor: "executor".into(),
                    expected_revision: revision,
                },
            )
            .expect("record outcome");
        match &recorded.events.last().expect("outcome event").kind {
            TaskEventKind::OutcomeRecorded {
                status,
                evidence_count,
                ..
            } => {
                assert_eq!(*status, OutcomeStatus::Succeeded);
                assert_eq!(*evidence_count, 1);
            }
            other => panic!("expected OutcomeRecorded, found {other:?}"),
        }

        let succeeded = succeed(&service, task_id, recorded.revision).expect("success");
        assert_eq!(succeeded.state, TaskState::Succeeded);
        assert_eq!(succeeded.outcomes.len(), 1);
    }

    // --- record_outcome validation ----------------------------------------

    #[test]
    fn outcome_for_an_unplanned_action_is_rejected() {
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, _, revision) = task_in_verifying(&temp);
        let error = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: outcome(ActionId::new(), OutcomeStatus::Succeeded, evidence()),
                    actor: "executor".into(),
                    expected_revision: revision,
                },
            )
            .expect_err("action is not in the plan");
        assert!(matches!(
            error,
            ServiceError::Validation(ValidationError::UnknownOutcomeAction(_))
        ));
    }

    #[test]
    fn outcomes_are_append_only_per_action() {
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, action_id, revision) = task_in_verifying(&temp);
        let recorded = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: outcome(action_id, OutcomeStatus::Failed, evidence()),
                    actor: "executor".into(),
                    expected_revision: revision,
                },
            )
            .expect("first outcome");
        // A failed outcome must not be overwritable by a later "successful"
        // one; history is append-only.
        let error = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: outcome(action_id, OutcomeStatus::Succeeded, evidence()),
                    actor: "executor".into(),
                    expected_revision: recorded.revision,
                },
            )
            .expect_err("duplicate outcome");
        assert!(matches!(
            error,
            ServiceError::Validation(ValidationError::DuplicateOutcome(_))
        ));
    }

    #[test]
    fn outcome_that_finishes_before_it_starts_is_rejected() {
        let temp = TempDir::new().expect("tempdir");
        let (service, task_id, action_id, revision) = task_in_verifying(&temp);
        let mut backwards = outcome(action_id, OutcomeStatus::Succeeded, evidence());
        backwards.finished_at = backwards.started_at - chrono::TimeDelta::seconds(1);
        let error = service
            .record_outcome(
                task_id,
                RecordOutcomeRequest {
                    outcome: backwards,
                    actor: "executor".into(),
                    expected_revision: revision,
                },
            )
            .expect_err("finished_at must not precede started_at");
        assert!(matches!(
            error,
            ServiceError::Validation(ValidationError::OutcomeFinishedBeforeStarted { .. })
        ));
    }

    #[test]
    fn outcomes_cannot_be_recorded_before_execution_starts() {
        let temp = TempDir::new().expect("tempdir");
        let service = service(&temp);
        let created = service
            .create(inspection_request(workspace_path()))
            .expect("create");
        let error = service
            .record_outcome(
                created.plan.task_id,
                RecordOutcomeRequest {
                    outcome: outcome(
                        created.plan.actions[0].id,
                        OutcomeStatus::Succeeded,
                        evidence(),
                    ),
                    actor: "executor".into(),
                    expected_revision: 0,
                },
            )
            .expect_err("task is only Ready");
        assert!(matches!(
            error,
            ServiceError::Validation(ValidationError::OutcomeNotAllowedInState(TaskState::Ready))
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
