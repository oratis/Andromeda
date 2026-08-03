//! The HTTP wire contract.
//!
//! Handlers serialize the types in this module and **never** the runtime's
//! internal structs. Before this layer existed every handler called
//! `serde_json::to_value(record)` on [`TaskRecord`] / [`TaskEvent`] /
//! [`EvaluationReport`], which made the persisted, in-process representation
//! the public API by accident: renaming an internal field, adding one, or
//! reordering an enum variant silently changed what clients receive
//! (`docs/reviews/architecture-review.md` §3).
//!
//! The views below are deliberately *dumb*: they hold borrowed data, own no
//! logic, and exist to be the one place a wire-format change has to be typed
//! out. Because the mapping functions name every field explicitly, an internal
//! rename becomes a compile error here rather than a silent API break, and
//! `tests::task_json_shape_is_locked` pins the resulting document so an
//! *accidental* change fails a test instead of shipping.
//!
//! Scope note: the plan, capabilities, and outcomes are re-exposed as the
//! `andromeda-core` contract types they already are — those carry their own
//! versioning (`ActionPlan::schema_version`) and are the shared vocabulary of
//! the whole system, so mirroring them here would duplicate a contract rather
//! than insulate one. The golden test covers them anyway: it locks the whole
//! document, nested core fields included.

use std::collections::BTreeMap;
use std::path::Path;

use andromeda_core::{
    ActionId, ActionOutcome, ActionPlan, Capability, CapabilityId, IsolationLevel, OutcomeStatus,
    TaskId, TaskState,
};
use andromeda_policy::{DecisionEffect, PolicyDecision};
use andromeda_runtime::{
    EvaluationReport, ListWarning, TaskEvent, TaskEventKind, TaskListing, TaskRecord,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Events a single-task read returns when the caller does not ask for more.
///
/// `TaskRecord.events` is append-only and unbounded — every `evaluate`,
/// `transition`, `grant`, and `outcome` adds one, and an `Evaluated` event
/// embeds a decision per action — so a long-lived task's full history is
/// unbounded response size. Fifty is chosen to cover the whole history of an
/// ordinary task (create, grant, a handful of evaluations, the lifecycle
/// edges, one outcome per action) so the common case is not truncated at all,
/// while capping the pathological one. Callers that need more ask for it
/// explicitly and learn the total from `event_count`.
pub(crate) const DEFAULT_EVENT_LIMIT: usize = 50;

/// Hard ceiling on `?events=`, whatever the caller asks for.
///
/// A ceiling, not a suggestion: it is applied by clamping rather than by
/// rejecting, and `event_count` still reports the true total, so a caller can
/// always tell that more history exists. It bounds the response even when the
/// caller is the one being unreasonable.
pub(crate) const MAX_EVENT_LIMIT: usize = 1_000;

/// One task as the API presents it.
///
/// `events` holds only the most recent [`TaskView::bounded`] slice of the
/// history; `event_count` is always the true total, so a truncated read is
/// visible rather than silent.
#[derive(Debug, Serialize)]
pub(crate) struct TaskView<'a> {
    plan: &'a ActionPlan,
    state: TaskState,
    revision: u64,
    capabilities: &'a [Capability],
    events: Vec<EventView<'a>>,
    event_count: usize,
    outcomes: &'a [ActionOutcome],
}

impl<'a> TaskView<'a> {
    /// Builds a view carrying at most `limit` of the *most recent* events, in
    /// the same chronological order as the full history.
    ///
    /// `limit` is clamped to [`MAX_EVENT_LIMIT`] here, at the single point
    /// where the wire response is built, so no caller path can widen it.
    pub(crate) fn bounded(record: &'a TaskRecord, limit: usize) -> Self {
        let limit = limit.min(MAX_EVENT_LIMIT);
        let skipped = record.events.len().saturating_sub(limit);
        Self {
            plan: &record.plan,
            state: record.state,
            revision: record.revision,
            capabilities: &record.capabilities,
            events: record.events[skipped..]
                .iter()
                .map(EventView::from)
                .collect(),
            event_count: record.events.len(),
            outcomes: &record.outcomes,
        }
    }
}

impl<'a> From<&'a TaskRecord> for TaskView<'a> {
    /// The default read: the most recent [`DEFAULT_EVENT_LIMIT`] events.
    fn from(record: &'a TaskRecord) -> Self {
        Self::bounded(record, DEFAULT_EVENT_LIMIT)
    }
}

/// One task as `GET /v1/tasks` presents it: enough to identify and triage a
/// task, with no event bodies at all.
///
/// The listing used to return complete [`TaskRecord`]s, so every task's entire
/// event history — decision sets included — was multiplied by the number of
/// tasks in a single response. Counts answer the triage questions ("how much
/// history is there, is anything recorded yet?") in a fixed number of bytes;
/// a caller that wants the bodies reads the one task it cares about.
#[derive(Debug, Serialize)]
pub(crate) struct TaskSummaryView<'a> {
    task_id: TaskId,
    state: TaskState,
    revision: u64,
    /// The captured intent, so a human can tell tasks apart without a second
    /// request per row.
    intent_summary: &'a str,
    requested_by: &'a str,
    /// When the task was created, and when it last changed. Both come from the
    /// event history (first and last entry), and are absent only for a record
    /// with no events at all — which the service does not produce, since
    /// `create` writes a `created` event.
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    action_count: usize,
    capability_count: usize,
    event_count: usize,
    outcome_count: usize,
}

impl<'a> From<&'a TaskRecord> for TaskSummaryView<'a> {
    fn from(record: &'a TaskRecord) -> Self {
        Self {
            task_id: record.plan.task_id,
            state: record.state,
            revision: record.revision,
            intent_summary: &record.plan.intent.summary,
            requested_by: &record.plan.intent.requested_by,
            created_at: record.events.first().map(|event| event.occurred_at),
            updated_at: record.events.last().map(|event| event.occurred_at),
            action_count: record.plan.actions.len(),
            capability_count: record.capabilities.len(),
            event_count: record.events.len(),
            outcome_count: record.outcomes.len(),
        }
    }
}

/// One entry of the task event history.
#[derive(Debug, Serialize)]
pub(crate) struct EventView<'a> {
    id: Uuid,
    occurred_at: DateTime<Utc>,
    actor: &'a str,
    kind: EventKindView<'a>,
}

impl<'a> From<&'a TaskEvent> for EventView<'a> {
    fn from(event: &'a TaskEvent) -> Self {
        Self {
            id: event.id,
            occurred_at: event.occurred_at,
            actor: &event.actor,
            kind: EventKindView::from(&event.kind),
        }
    }
}

/// The tagged payload of one event.
///
/// Mirrors [`TaskEventKind`] on purpose rather than re-serializing it: this is
/// the enum a client matches on, so its variant names and payload fields are
/// the contract, and the exhaustive `match` below makes adding an internal
/// variant a compile error instead of a new, undocumented wire tag.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum EventKindView<'a> {
    Created,
    StateChanged {
        from: TaskState,
        to: TaskState,
        external_side_effect_confirmed: Option<bool>,
    },
    Granted {
        capabilities: &'a [CapabilityId],
        plan_fully_granted: bool,
    },
    Evaluated {
        effective_isolation: &'a BTreeMap<ActionId, IsolationLevel>,
        external_side_effect_confirmed: bool,
        decisions: BTreeMap<ActionId, DecisionView<'a>>,
    },
    OutcomeRecorded {
        action_id: ActionId,
        status: OutcomeStatus,
        evidence_count: usize,
    },
}

impl<'a> From<&'a TaskEventKind> for EventKindView<'a> {
    fn from(kind: &'a TaskEventKind) -> Self {
        match kind {
            TaskEventKind::Created => Self::Created,
            TaskEventKind::StateChanged {
                from,
                to,
                external_side_effect_confirmed,
            } => Self::StateChanged {
                from: *from,
                to: *to,
                external_side_effect_confirmed: *external_side_effect_confirmed,
            },
            TaskEventKind::Granted {
                capabilities,
                plan_fully_granted,
            } => Self::Granted {
                capabilities,
                plan_fully_granted: *plan_fully_granted,
            },
            TaskEventKind::Evaluated {
                effective_isolation,
                external_side_effect_confirmed,
                decisions,
            } => Self::Evaluated {
                effective_isolation,
                external_side_effect_confirmed: *external_side_effect_confirmed,
                decisions: decision_views(decisions),
            },
            TaskEventKind::OutcomeRecorded {
                action_id,
                status,
                evidence_count,
            } => Self::OutcomeRecorded {
                action_id: *action_id,
                status: *status,
                evidence_count: *evidence_count,
            },
        }
    }
}

/// One policy decision.
#[derive(Debug, Serialize)]
pub(crate) struct DecisionView<'a> {
    effect: DecisionEffect,
    reasons: &'a [String],
}

impl<'a> From<&'a PolicyDecision> for DecisionView<'a> {
    fn from(decision: &'a PolicyDecision) -> Self {
        Self {
            effect: decision.effect,
            reasons: &decision.reasons,
        }
    }
}

fn decision_views(
    decisions: &BTreeMap<ActionId, PolicyDecision>,
) -> BTreeMap<ActionId, DecisionView<'_>> {
    decisions
        .iter()
        .map(|(action, decision)| (*action, DecisionView::from(decision)))
        .collect()
}

/// The result of one `POST /v1/tasks/{id}/evaluate`.
#[derive(Debug, Serialize)]
pub(crate) struct EvaluationReportView<'a> {
    task_id: TaskId,
    revision: u64,
    decisions: BTreeMap<ActionId, DecisionView<'a>>,
}

impl<'a> From<&'a EvaluationReport> for EvaluationReportView<'a> {
    fn from(report: &'a EvaluationReport) -> Self {
        Self {
            task_id: report.task_id,
            revision: report.revision,
            decisions: decision_views(&report.decisions),
        }
    }
}

/// The body of `GET /v1/tasks`: summaries, never full records.
#[derive(Debug, Serialize)]
pub(crate) struct TaskListingView<'a> {
    tasks: Vec<TaskSummaryView<'a>>,
    warnings: Vec<ListWarningView<'a>>,
}

impl<'a> From<&'a TaskListing> for TaskListingView<'a> {
    fn from(listing: &'a TaskListing) -> Self {
        Self {
            tasks: listing.records.iter().map(TaskSummaryView::from).collect(),
            warnings: listing.warnings.iter().map(ListWarningView::from).collect(),
        }
    }
}

/// A record file that could not be read, reported instead of failing the whole
/// listing.
#[derive(Debug, Serialize)]
pub(crate) struct ListWarningView<'a> {
    path: &'a Path,
    reason: &'a str,
}

impl<'a> From<&'a ListWarning> for ListWarningView<'a> {
    fn from(warning: &'a ListWarning) -> Self {
        Self {
            path: &warning.path,
            reason: &warning.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use andromeda_core::{
        ActionKind, ActionSpec, CapabilityResource, Evidence, FileAccess, Intent,
        RecoverySemantics, RiskLevel,
    };
    use serde_json::{Value, json};

    use super::*;

    const TASK_UUID: &str = "11111111-1111-4111-8111-111111111111";
    const ACTION_UUID: &str = "22222222-2222-4222-8222-222222222222";
    const CAPABILITY_UUID: &str = "33333333-3333-4333-8333-333333333333";
    const EVENT_UUID: &str = "44444444-4444-4444-8444-444444444444";
    const TIMESTAMP: &str = "2026-01-02T03:04:05Z";

    fn id<T: serde::de::DeserializeOwned>(uuid: &str) -> T {
        serde_json::from_value(json!(uuid)).expect("id")
    }

    fn timestamp() -> DateTime<Utc> {
        TIMESTAMP.parse().expect("timestamp")
    }

    /// A task exercising every wire construct at once: a plan, a capability, an
    /// outcome with evidence, and one event of every kind.
    fn representative_record() -> TaskRecord {
        let task_id: TaskId = id(TASK_UUID);
        let action_id: ActionId = id(ACTION_UUID);
        let capability_id: CapabilityId = id(CAPABILITY_UUID);
        let at = timestamp();

        let mut intent = Intent::new("Inspect the workspace", "operator");
        intent.created_at = at;

        TaskRecord {
            plan: ActionPlan {
                schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
                task_id,
                intent,
                actions: vec![ActionSpec {
                    id: action_id,
                    name: "Inspect directory".into(),
                    kind: ActionKind::Inspect,
                    target: "/workspace".into(),
                    arguments: BTreeMap::new(),
                    depends_on: Vec::new(),
                    required_capabilities: vec![capability_id],
                    risk: RiskLevel::L1Sandboxed,
                    recovery: RecoverySemantics::None,
                }],
            },
            state: TaskState::Verifying,
            revision: 4,
            capabilities: vec![Capability {
                id: capability_id,
                resource: CapabilityResource::Files {
                    root: "/workspace".into(),
                    access: FileAccess::Read,
                },
                issued_to: task_id.to_string(),
                issued_at: at,
                expires_at: None,
                single_use: false,
                signature: None,
            }],
            events: representative_events(action_id, capability_id),
            outcomes: vec![ActionOutcome {
                action_id,
                status: OutcomeStatus::Succeeded,
                started_at: at,
                finished_at: at,
                evidence: vec![Evidence {
                    kind: "assertion".into(),
                    summary: "listing matched".into(),
                    attributes: BTreeMap::new(),
                }],
                error: None,
            }],
        }
    }

    /// One event of every kind, so no variant of the wire enum is unlocked.
    fn representative_events(action_id: ActionId, capability_id: CapabilityId) -> Vec<TaskEvent> {
        let event_id: Uuid = id(EVENT_UUID);
        let at = timestamp();
        let event = |actor: &str, kind| TaskEvent {
            id: event_id,
            occurred_at: at,
            actor: actor.to_owned(),
            kind,
        };
        vec![
            event("operator", TaskEventKind::Created),
            event(
                "approver",
                TaskEventKind::Granted {
                    capabilities: vec![capability_id],
                    plan_fully_granted: true,
                },
            ),
            event(
                "policy-engine",
                TaskEventKind::Evaluated {
                    effective_isolation: [(action_id, IsolationLevel::Sandbox)]
                        .into_iter()
                        .collect(),
                    external_side_effect_confirmed: false,
                    decisions: [(
                        action_id,
                        PolicyDecision {
                            effect: DecisionEffect::Allow,
                            reasons: vec!["all checks passed".into()],
                        },
                    )]
                    .into_iter()
                    .collect(),
                },
            ),
            event(
                "runner",
                TaskEventKind::StateChanged {
                    from: TaskState::Ready,
                    to: TaskState::Running,
                    external_side_effect_confirmed: Some(true),
                },
            ),
            event(
                "executor",
                TaskEventKind::OutcomeRecorded {
                    action_id,
                    status: OutcomeStatus::Succeeded,
                    evidence_count: 1,
                },
            ),
        ]
    }

    /// The serialized form of [`representative_events`]: the tag and payload
    /// of every event kind a client can receive.
    fn golden_events_json() -> Value {
        json!([
            {
                "id": EVENT_UUID,
                "occurred_at": TIMESTAMP,
                "actor": "operator",
                "kind": {"type": "created"},
            },
            {
                "id": EVENT_UUID,
                "occurred_at": TIMESTAMP,
                "actor": "approver",
                "kind": {
                    "type": "granted",
                    "capabilities": [CAPABILITY_UUID],
                    "plan_fully_granted": true,
                },
            },
            {
                "id": EVENT_UUID,
                "occurred_at": TIMESTAMP,
                "actor": "policy-engine",
                "kind": {
                    "type": "evaluated",
                    "effective_isolation": {ACTION_UUID: "sandbox"},
                    "external_side_effect_confirmed": false,
                    "decisions": {
                        ACTION_UUID: {
                            "effect": "allow",
                            "reasons": ["all checks passed"],
                        },
                    },
                },
            },
            {
                "id": EVENT_UUID,
                "occurred_at": TIMESTAMP,
                "actor": "runner",
                "kind": {
                    "type": "state_changed",
                    "from": "ready",
                    "to": "running",
                    "external_side_effect_confirmed": true,
                },
            },
            {
                "id": EVENT_UUID,
                "occurred_at": TIMESTAMP,
                "actor": "executor",
                "kind": {
                    "type": "outcome_recorded",
                    "action_id": ACTION_UUID,
                    "status": "succeeded",
                    "evidence_count": 1,
                },
            },
        ])
    }

    /// The document `GET /v1/tasks/{id}` is expected to produce for
    /// [`representative_record`], written out in full.
    fn golden_task_json() -> Value {
        json!({
            "plan": {
                "schema_version": 1,
                "task_id": TASK_UUID,
                "intent": {
                    "summary": "Inspect the workspace",
                    "requested_by": "operator",
                    "created_at": TIMESTAMP,
                },
                "actions": [{
                    "id": ACTION_UUID,
                    "name": "Inspect directory",
                    "kind": "inspect",
                    "target": "/workspace",
                    "arguments": {},
                    "depends_on": [],
                    "required_capabilities": [CAPABILITY_UUID],
                    "risk": "l1_sandboxed",
                    "recovery": "none",
                }],
            },
            "state": "verifying",
            "revision": 4,
            "capabilities": [{
                "id": CAPABILITY_UUID,
                "resource": {
                    "type": "files",
                    "root": "/workspace",
                    "access": "read",
                },
                "issued_to": TASK_UUID,
                "issued_at": TIMESTAMP,
                "expires_at": Value::Null,
                "single_use": false,
            }],
            "events": golden_events_json(),
            "event_count": 5,
            "outcomes": [{
                "action_id": ACTION_UUID,
                "status": "succeeded",
                "started_at": TIMESTAMP,
                "finished_at": TIMESTAMP,
                "evidence": [{
                    "kind": "assertion",
                    "summary": "listing matched",
                    "attributes": {},
                }],
                "error": Value::Null,
            }],
        })
    }

    /// The wire format, written out by hand. Any change to a serialized field
    /// name, enum tag, or nesting — whether made here or by renaming an
    /// internal field in `andromeda-runtime`/`andromeda-core` — fails this
    /// test, which is the whole point: the API cannot drift silently.
    ///
    /// Updating this literal is a deliberate act, and one that must come with
    /// a documentation change in `docs/development/task-control-plane.md`.
    #[test]
    fn task_json_shape_is_locked() {
        let record = representative_record();
        let encoded = serde_json::to_value(TaskView::from(&record)).expect("serialize view");
        assert_eq!(encoded, golden_task_json());
    }

    /// The single-task view differs from the internal record in exactly one
    /// documented way: the bounded `events` slice and the `event_count` that
    /// makes the bound visible. Everything else is still identical, so an
    /// internal field added or renamed in `andromeda-runtime` cannot slip onto
    /// the wire unnoticed under cover of "the DTO layer changed something".
    #[test]
    fn the_view_differs_from_the_record_only_by_the_event_bound() {
        let record = representative_record();
        let mut view = serde_json::to_value(TaskView::from(&record)).expect("view");
        let internal = serde_json::to_value(&record).expect("record");
        assert_eq!(
            view.as_object_mut()
                .expect("object")
                .remove("event_count")
                .expect("event_count"),
            json!(record.events.len()),
        );
        assert_eq!(view, internal);
    }

    /// Truncation keeps the *most recent* events, in order, and reports the
    /// true total rather than the truncated length.
    #[test]
    fn a_bounded_read_keeps_the_newest_events() {
        let record = representative_record();
        let view = serde_json::to_value(TaskView::bounded(&record, 2)).expect("view");
        let events = view["events"].as_array().expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["actor"], "runner");
        assert_eq!(events[1]["actor"], "executor");
        assert_eq!(view["event_count"], json!(5));

        // A caller cannot widen the window past the ceiling.
        let view = serde_json::to_value(TaskView::bounded(&record, usize::MAX)).expect("view");
        assert_eq!(view["events"].as_array().expect("events").len(), 5);
    }

    /// The listing projection, written out in full: no event bodies, no plan,
    /// no capabilities — only what triage needs.
    #[test]
    fn task_summary_json_shape_is_locked() {
        let record = representative_record();
        let encoded =
            serde_json::to_value(TaskSummaryView::from(&record)).expect("serialize summary");
        assert_eq!(
            encoded,
            json!({
                "task_id": TASK_UUID,
                "state": "verifying",
                "revision": 4,
                "intent_summary": "Inspect the workspace",
                "requested_by": "operator",
                "created_at": TIMESTAMP,
                "updated_at": TIMESTAMP,
                "action_count": 1,
                "capability_count": 1,
                "event_count": 5,
                "outcome_count": 1,
            })
        );
    }

    /// Measures what the projection is for. `GET /v1/tasks` used to serialize
    /// whole `TaskRecord`s, so a task's entire history rode along in the
    /// listing — once per task. The summary is constant-size in the number of
    /// events, so the listing no longer grows as tasks are evaluated.
    #[test]
    fn the_listing_projection_is_constant_in_the_event_count() {
        let mut sizes = Vec::new();
        for events in [1_usize, 1_000] {
            let mut record = representative_record();
            let action_id: ActionId = id(ACTION_UUID);
            let capability_id: CapabilityId = id(CAPABILITY_UUID);
            record.events = std::iter::repeat_with(|| {
                representative_events(action_id, capability_id).into_iter()
            })
            .flatten()
            .take(events)
            .collect();

            // What the endpoint returned before this change: the whole record.
            let before = serde_json::to_vec(&record).expect("record").len();
            let after = serde_json::to_vec(&TaskSummaryView::from(&record))
                .expect("summary")
                .len();
            eprintln!("events={events:>5}  full record={before:>8} B  summary={after:>4} B");
            sizes.push((events, before, after));
        }

        let (_, small_before, small_after) = sizes[0];
        let (_, large_before, large_after) = sizes[1];
        // The summary does not grow with history (only `event_count`'s digits
        // change), while the full record grows without bound.
        assert!(
            large_after < small_after + 8,
            "summary grew with the event history: {small_after} -> {large_after}"
        );
        assert!(large_before > small_before * 100);
        assert!(
            large_before > large_after * 100,
            "expected a >100x reduction, got {large_before} -> {large_after}"
        );
    }

    /// Enum variants reach the wire as names, so the names are contract too.
    /// A rename in `andromeda-core` would otherwise change the API without
    /// touching a single line in this crate. Every variant must be listed.
    #[test]
    fn state_and_effect_names_are_pinned() {
        for (state, name) in [
            (TaskState::AwaitingApproval, "awaiting_approval"),
            (TaskState::Ready, "ready"),
            (TaskState::Running, "running"),
            (TaskState::Verifying, "verifying"),
            (TaskState::Succeeded, "succeeded"),
            (TaskState::Failed, "failed"),
            (TaskState::Cancelling, "cancelling"),
            (TaskState::Cancelled, "cancelled"),
            (TaskState::Compensating, "compensating"),
            (TaskState::Compensated, "compensated"),
        ] {
            assert_eq!(serde_json::to_value(state).expect("state"), json!(name));
        }
        for (effect, name) in [
            (DecisionEffect::Allow, "allow"),
            (DecisionEffect::Ask, "ask"),
            (DecisionEffect::Deny, "deny"),
        ] {
            assert_eq!(serde_json::to_value(effect).expect("effect"), json!(name));
        }

        // `draft` was a state nothing produced and no edge reached. It is gone
        // from the vocabulary, in both directions: a client will never be sent
        // it, and `{"to": "draft"}` no longer parses at all.
        assert!(serde_json::from_value::<TaskState>(json!("draft")).is_err());
    }
}
