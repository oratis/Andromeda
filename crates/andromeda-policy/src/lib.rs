//! Deterministic authorization for Andromeda actions.
//!
//! Model output is treated as untrusted input. The policy engine validates
//! risk floors, isolation, capability expiry, resource scope, and explicit
//! deny rules before an executor receives an action.

use std::path::Path;

use andromeda_core::{
    ActionKind, ActionSpec, Capability, CapabilityResource, FileAccess, IsolationLevel, RiskLevel,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEffect {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub effect: DecisionEffect,
    pub reasons: Vec<String>,
}

impl PolicyDecision {
    fn allow(reason: impl Into<String>) -> Self {
        Self {
            effect: DecisionEffect::Allow,
            reasons: vec![reason.into()],
        }
    }

    fn ask(reason: impl Into<String>) -> Self {
        Self {
            effect: DecisionEffect::Ask,
            reasons: vec![reason.into()],
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Self {
            effect: DecisionEffect::Deny,
            reasons: vec![reason.into()],
        }
    }
}

/// Host-enforced policy that the model cannot modify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySet {
    #[serde(default)]
    pub denied_network_hosts: Vec<String>,
    #[serde(default)]
    pub denied_path_roots: Vec<String>,
    pub require_confirmation_for_external_side_effects: bool,
}

impl Default for PolicySet {
    fn default() -> Self {
        Self {
            denied_network_hosts: Vec::new(),
            denied_path_roots: vec![
                "/boot".into(),
                "/etc".into(),
                "/usr".into(),
                "/System".into(),
                "C:\\Windows".into(),
            ],
            require_confirmation_for_external_side_effects: true,
        }
    }
}

pub struct EvaluationContext<'a> {
    pub now: DateTime<Utc>,
    pub isolation: IsolationLevel,
    pub capabilities: &'a [Capability],
    pub external_side_effect_confirmed: bool,
}

impl<'a> EvaluationContext<'a> {
    #[must_use]
    pub fn current(
        isolation: IsolationLevel,
        capabilities: &'a [Capability],
        external_side_effect_confirmed: bool,
    ) -> Self {
        Self {
            now: Utc::now(),
            isolation,
            capabilities,
            external_side_effect_confirmed,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    policy: PolicySet,
}

impl PolicyEngine {
    #[must_use]
    pub const fn new(policy: PolicySet) -> Self {
        Self { policy }
    }

    #[must_use]
    pub fn evaluate(&self, action: &ActionSpec, context: &EvaluationContext<'_>) -> PolicyDecision {
        if !action.has_valid_risk() {
            return PolicyDecision::deny(format!(
                "declared risk {:?} is below the {:?} floor",
                action.risk,
                action.kind.minimum_risk()
            ));
        }

        if context.isolation < action.risk.minimum_isolation() {
            return PolicyDecision::deny(format!(
                "{:?} isolation is required, but {:?} is active",
                action.risk.minimum_isolation(),
                context.isolation
            ));
        }

        if let Some(reason) = self.denied_target_reason(action) {
            return PolicyDecision::deny(reason);
        }

        let capabilities: Vec<_> = action
            .required_capabilities
            .iter()
            .filter_map(|required| {
                context
                    .capabilities
                    .iter()
                    .find(|candidate| candidate.id == *required)
            })
            .collect();

        if capabilities.len() != action.required_capabilities.len() {
            return PolicyDecision::deny("one or more required capabilities were not granted");
        }

        if capabilities
            .iter()
            .any(|capability| !capability.is_active_at(context.now))
        {
            return PolicyDecision::deny("a required capability has expired");
        }

        if !Self::capabilities_cover_action(action, &capabilities) {
            return PolicyDecision::deny("granted capabilities do not cover the action target");
        }

        if action.risk == RiskLevel::L3ExternalSideEffect
            && self.policy.require_confirmation_for_external_side_effects
            && !context.external_side_effect_confirmed
        {
            return PolicyDecision::ask("external side effect requires final confirmation");
        }

        PolicyDecision::allow("risk, isolation, capability, and deny-rule checks passed")
    }

    fn denied_target_reason(&self, action: &ActionSpec) -> Option<String> {
        if is_file_action(&action.kind) {
            let target = Path::new(&action.target);
            if self
                .policy
                .denied_path_roots
                .iter()
                .any(|root| target.starts_with(root))
            {
                return Some(format!("target {} is inside a denied path", action.target));
            }
        }

        if action.kind == ActionKind::NetworkRequest
            && self
                .policy
                .denied_network_hosts
                .iter()
                .any(|host| host.eq_ignore_ascii_case(&action.target))
        {
            return Some(format!("network host {} is denied", action.target));
        }

        None
    }

    fn capabilities_cover_action(action: &ActionSpec, capabilities: &[&Capability]) -> bool {
        if action.required_capabilities.is_empty() {
            return action.kind == ActionKind::Reason;
        }

        match action.kind {
            ActionKind::Inspect | ActionKind::ReadFile | ActionKind::ParseUntrustedContent => {
                capabilities.iter().any(|capability| {
                    capability.permits_file(Path::new(&action.target), FileAccess::Read)
                })
            }
            ActionKind::WriteFile
            | ActionKind::CreateDirectory
            | ActionKind::MoveFile
            | ActionKind::DeleteFile => capabilities.iter().any(|capability| {
                capability.permits_file(Path::new(&action.target), FileAccess::Write)
            }),
            ActionKind::NetworkRequest => capabilities.iter().any(|capability| {
                matches!(
                    &capability.resource,
                    CapabilityResource::Network { host, .. }
                    if host.eq_ignore_ascii_case(&action.target)
                )
            }),
            ActionKind::SystemChange => capabilities.iter().any(|capability| {
                matches!(
                    &capability.resource,
                    CapabilityResource::SystemSetting { key } if key == &action.target
                )
            }),
            ActionKind::ExternalCall => capabilities.iter().any(|capability| {
                matches!(
                    &capability.resource,
                    CapabilityResource::ExternalService { service, operation }
                    if format!("{service}:{operation}") == action.target
                )
            }),
            ActionKind::Reason => true,
        }
    }
}

const fn is_file_action(kind: &ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::Inspect
            | ActionKind::ReadFile
            | ActionKind::WriteFile
            | ActionKind::CreateDirectory
            | ActionKind::MoveFile
            | ActionKind::DeleteFile
            | ActionKind::ParseUntrustedContent
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use andromeda_core::{ActionId, CapabilityId, RecoverySemantics, RiskLevel};

    use super::*;

    fn file_capability(root: &str, access: FileAccess) -> Capability {
        Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Files {
                root: PathBuf::from(root),
                access,
            },
            issued_to: "task".into(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        }
    }

    fn action(
        kind: ActionKind,
        target: &str,
        risk: RiskLevel,
        capability: &Capability,
    ) -> ActionSpec {
        ActionSpec {
            id: ActionId::new(),
            name: "test".into(),
            kind,
            target: target.into(),
            arguments: BTreeMap::new(),
            depends_on: Vec::new(),
            required_capabilities: vec![capability.id],
            risk,
            recovery: RecoverySemantics::None,
        }
    }

    #[test]
    fn allows_scoped_read_inside_sandbox() {
        let capability = file_capability("/workspace", FileAccess::Read);
        let action = action(
            ActionKind::ReadFile,
            "/workspace/README.md",
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = PolicyEngine::default().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Sandbox, &capabilities, false),
        );
        assert_eq!(decision.effect, DecisionEffect::Allow);
    }

    #[test]
    fn deny_rules_override_a_matching_capability() {
        let capability = file_capability("/", FileAccess::ReadWrite);
        let action = action(
            ActionKind::WriteFile,
            "/etc/passwd",
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = PolicyEngine::default().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Sandbox, &capabilities, false),
        );
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn strong_isolation_cannot_be_downgraded_by_a_capability() {
        let capability = file_capability("/downloads", FileAccess::Read);
        let action = action(
            ActionKind::ParseUntrustedContent,
            "/downloads/unknown.zip",
            RiskLevel::L2StrongIsolation,
            &capability,
        );
        let capabilities = [capability];
        let decision = PolicyEngine::default().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Sandbox, &capabilities, false),
        );
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn external_side_effects_require_final_confirmation() {
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::ExternalService {
                service: "mail".into(),
                operation: "send".into(),
            },
            issued_to: "task".into(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: true,
        };
        let action = action(
            ActionKind::ExternalCall,
            "mail:send",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = PolicyEngine::default().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, false),
        );
        assert_eq!(decision.effect, DecisionEffect::Ask);
    }
}
