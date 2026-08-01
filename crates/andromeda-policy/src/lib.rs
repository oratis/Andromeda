//! Deterministic authorization for Andromeda actions.
//!
//! Model output is treated as untrusted input. The policy engine validates
//! risk floors, isolation, capability expiry, resource scope, and explicit
//! deny rules before an executor receives an action.

use std::path::Path;

use andromeda_core::{
    ActionKind, ActionSpec, Capability, CapabilityResource, FileAccess, IsolationLevel, RiskLevel,
    normalized_absolute,
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
    /// Optional requesting subject. When set, every capability required by
    /// the action must have been issued to this subject
    /// ([`Capability::issued_to`]); a mismatch denies the action. When
    /// `None`, no subject binding is enforced.
    pub subject: Option<String>,
}

impl<'a> EvaluationContext<'a> {
    /// Builds a context stamped with the wall-clock time. Prefer
    /// [`EvaluationContext::at`] where determinism or replayability matters.
    #[must_use]
    pub fn current(
        isolation: IsolationLevel,
        capabilities: &'a [Capability],
        external_side_effect_confirmed: bool,
    ) -> Self {
        Self::at(
            Utc::now(),
            isolation,
            capabilities,
            external_side_effect_confirmed,
        )
    }

    /// Builds a context with an explicit evaluation time, keeping the engine
    /// fully deterministic for a given input.
    #[must_use]
    pub const fn at(
        now: DateTime<Utc>,
        isolation: IsolationLevel,
        capabilities: &'a [Capability],
        external_side_effect_confirmed: bool,
    ) -> Self {
        Self {
            now,
            isolation,
            capabilities,
            external_side_effect_confirmed,
            subject: None,
        }
    }

    /// Binds the context to a requesting subject; see
    /// [`EvaluationContext::subject`].
    #[must_use]
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
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

    /// Evaluates one action against the host policy and granted capabilities.
    ///
    /// All failing checks are accumulated into
    /// [`PolicyDecision::reasons`] for auditability; the decision effect is
    /// `Deny` as soon as any check fails, `Ask` when only the final L3
    /// confirmation is missing, and `Allow` otherwise.
    #[must_use]
    pub fn evaluate(&self, action: &ActionSpec, context: &EvaluationContext<'_>) -> PolicyDecision {
        let mut reasons = Vec::new();

        if !action.has_valid_risk() {
            reasons.push(format!(
                "declared risk {:?} is below the {:?} floor",
                action.risk,
                action.kind.minimum_risk()
            ));
        }

        if !context.isolation.satisfies(action.risk.minimum_isolation()) {
            reasons.push(format!(
                "{:?} isolation is required, but {:?} is active",
                action.risk.minimum_isolation(),
                context.isolation
            ));
        }

        self.push_denied_target_reasons(action, &mut reasons);

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

        if capabilities.len() == action.required_capabilities.len() {
            if capabilities
                .iter()
                .any(|capability| !capability.is_active_at(context.now))
            {
                reasons
                    .push("a required capability is not active (expired or not yet issued)".into());
            }

            if let Some(subject) = &context.subject {
                if capabilities
                    .iter()
                    .any(|capability| &capability.issued_to != subject)
                {
                    reasons.push(format!(
                        "a required capability was not issued to subject {subject}"
                    ));
                }
            }

            if !Self::capabilities_cover_action(action, &capabilities) {
                reasons.push("granted capabilities do not cover the action target".into());
            }
        } else {
            reasons.push("one or more required capabilities were not granted".into());
        }

        if !reasons.is_empty() {
            return PolicyDecision {
                effect: DecisionEffect::Deny,
                reasons,
            };
        }

        if action.risk == RiskLevel::L3ExternalSideEffect
            && self.policy.require_confirmation_for_external_side_effects
            && !context.external_side_effect_confirmed
        {
            return PolicyDecision::ask("external side effect requires final confirmation");
        }

        PolicyDecision::allow("risk, isolation, capability, and deny-rule checks passed")
    }

    fn push_denied_target_reasons(&self, action: &ActionSpec, reasons: &mut Vec<String>) {
        if is_file_action(&action.kind) {
            if let Some(reason) = self.denied_path_reason(&action.target) {
                reasons.push(reason);
            }
            if action.kind == ActionKind::MoveFile {
                match action.arguments.get(ActionSpec::MOVE_DESTINATION_ARGUMENT) {
                    Some(destination) => {
                        if let Some(reason) = self.denied_path_reason(destination) {
                            reasons.push(reason);
                        }
                    }
                    None => reasons.push(format!(
                        "move_file action is missing the {:?} argument",
                        ActionSpec::MOVE_DESTINATION_ARGUMENT
                    )),
                }
            }
        }

        if action.kind == ActionKind::NetworkRequest {
            let (target_host, _) = split_host_port(&action.target);
            if self.policy.denied_network_hosts.iter().any(|denied| {
                let (denied_host, _) = split_host_port(denied);
                host_is_or_is_subdomain_of(&target_host, &denied_host)
            }) {
                reasons.push(format!("network host {} is denied", action.target));
            }
        }
    }

    /// Deny-rule check for one file path. The path is lexically normalized
    /// first so `..` traversal (`/tmp/../etc/passwd`) cannot dodge a denied
    /// root; targets that cannot be normalized (relative paths, `..` past the
    /// root) are denied outright.
    fn denied_path_reason(&self, target: &str) -> Option<String> {
        let Some(normalized) = normalized_absolute(Path::new(target)) else {
            return Some(format!(
                "target {target} is not a normalizable absolute path"
            ));
        };

        let denied = self.policy.denied_path_roots.iter().any(|root| {
            let root_path = Path::new(root);
            normalized_absolute(root_path).map_or_else(
                || path_starts_with(&normalized, root_path),
                |normalized_root| path_starts_with(&normalized, &normalized_root),
            )
        });

        denied.then(|| format!("target {target} is inside a denied path"))
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
            ActionKind::WriteFile | ActionKind::CreateDirectory | ActionKind::DeleteFile => {
                capabilities.iter().any(|capability| {
                    capability.permits_file(Path::new(&action.target), FileAccess::Write)
                })
            }
            // A move touches two endpoints: `target` is the source and
            // `arguments["destination"]` is the destination. Both need write
            // coverage; a missing destination argument is never covered.
            ActionKind::MoveFile => {
                let Some(destination) = action.arguments.get(ActionSpec::MOVE_DESTINATION_ARGUMENT)
                else {
                    return false;
                };
                let covers = |path: &str| {
                    capabilities.iter().any(|capability| {
                        capability.permits_file(Path::new(path), FileAccess::Write)
                    })
                };
                covers(&action.target) && covers(destination)
            }
            // Target format: `host` or `host:port`. The capability host must
            // match exactly (case-insensitive, trailing dot ignored) — grants
            // never extend to subdomains. A port-restricted capability only
            // covers targets carrying that explicit port.
            ActionKind::NetworkRequest => {
                let (target_host, target_port) = split_host_port(&action.target);
                capabilities.iter().any(|capability| {
                    let CapabilityResource::Network { host, port } = &capability.resource else {
                        return false;
                    };
                    let (granted_host, granted_inline_port) = split_host_port(host);
                    let port_covered = match ((*port).or(granted_inline_port), target_port) {
                        (None, _) => true,
                        (Some(granted), Some(requested)) => granted == requested,
                        (Some(_), None) => false,
                    };
                    granted_host == target_host && port_covered
                })
            }
            ActionKind::SystemChange => capabilities.iter().any(|capability| {
                matches!(
                    &capability.resource,
                    CapabilityResource::SystemSetting { key } if key == &action.target
                )
            }),
            // Target format: `service:operation`, split at the FIRST ':'.
            // Capabilities whose service name contains ':' are ambiguous by
            // construction and never match.
            ActionKind::ExternalCall => {
                let Some((target_service, target_operation)) = action.target.split_once(':') else {
                    return false;
                };
                capabilities.iter().any(|capability| {
                    let CapabilityResource::ExternalService { service, operation } =
                        &capability.resource
                    else {
                        return false;
                    };
                    !service.contains(':')
                        && service == target_service
                        && operation == target_operation
                })
            }
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

/// Splits a network target of the form `host` or `host:port` and normalizes
/// the host: ASCII-lowercased, one trailing dot stripped, surrounding
/// whitespace trimmed. A suffix after the last `':'` is treated as a port
/// only when it parses as a `u16`; bracketless IPv6 literals are not
/// supported by this textual form.
fn split_host_port(target: &str) -> (String, Option<u16>) {
    let trimmed = target.trim();
    let (host, port) = match trimmed.rsplit_once(':') {
        Some((host, port_text)) => match port_text.parse::<u16>() {
            Ok(port) => (host, Some(port)),
            Err(_) => (trimmed, None),
        },
        None => (trimmed, None),
    };
    let host = host.strip_suffix('.').unwrap_or(host);
    (host.to_ascii_lowercase(), port)
}

/// Whether `host` equals `ancestor` or is a subdomain of it. Both sides must
/// already be normalized (see [`split_host_port`]). Used for the deny list
/// only: denying `evil.com` also denies `sub.evil.com`.
fn host_is_or_is_subdomain_of(host: &str, ancestor: &str) -> bool {
    if ancestor.is_empty() {
        return false;
    }
    host == ancestor
        || host
            .strip_suffix(ancestor)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Component-wise prefix check used for denied-root matching.
///
/// On Windows the comparison is ASCII case-insensitive, matching the default
/// case-insensitive filesystem semantics so `C:\WINDOWS` cannot dodge a
/// `C:\Windows` deny root.
#[cfg(not(windows))]
fn path_starts_with(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

#[cfg(windows)]
fn path_starts_with(candidate: &Path, root: &Path) -> bool {
    let mut candidate_components = candidate.components();
    root.components().all(|root_component| {
        candidate_components.next().is_some_and(|component| {
            component
                .as_os_str()
                .eq_ignore_ascii_case(root_component.as_os_str())
        })
    })
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
        let capability = file_capability(workspace_root(), FileAccess::Read);
        let action = action(
            ActionKind::ReadFile,
            workspace_file(),
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
        let capability = file_capability(system_root(), FileAccess::ReadWrite);
        let action = action(
            ActionKind::WriteFile,
            denied_system_file(),
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
        let capability = file_capability(downloads_root(), FileAccess::Read);
        let action = action(
            ActionKind::ParseUntrustedContent,
            unknown_download(),
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

    fn network_capability(host: &str, port: Option<u16>) -> Capability {
        Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Network {
                host: host.into(),
                port,
            },
            issued_to: "task".into(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        }
    }

    fn engine() -> PolicyEngine {
        PolicyEngine::default()
    }

    fn sandbox_context(capabilities: &[Capability]) -> EvaluationContext<'_> {
        EvaluationContext::current(IsolationLevel::Sandbox, capabilities, false)
    }

    #[test]
    fn deny_rules_cannot_be_dodged_by_path_traversal() {
        // Regression: a `/`-rooted ReadWrite capability plus a raw
        // `/tmp/../etc/passwd` target used to slip past the `/etc` deny root.
        let capability = file_capability(system_root(), FileAccess::ReadWrite);
        let action = action(
            ActionKind::WriteFile,
            traversal_into_denied_root(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn relative_file_targets_are_denied() {
        let capability = file_capability(system_root(), FileAccess::ReadWrite);
        let action = action(
            ActionKind::WriteFile,
            "relative/path.txt",
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn brokered_context_does_not_satisfy_micro_vm_requirement() {
        let capability = file_capability(downloads_root(), FileAccess::Read);
        let action = action(
            ActionKind::ParseUntrustedContent,
            unknown_download(),
            RiskLevel::L2StrongIsolation,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, false),
        );
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn expired_capability_is_denied() {
        let mut capability = file_capability(workspace_root(), FileAccess::Read);
        capability.issued_at = Utc::now() - chrono::Duration::hours(2);
        capability.expires_at = Some(Utc::now() - chrono::Duration::hours(1));
        let action = action(
            ActionKind::ReadFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn future_issued_capability_is_denied() {
        let mut capability = file_capability(workspace_root(), FileAccess::Read);
        capability.issued_at = Utc::now() + chrono::Duration::hours(1);
        let action = action(
            ActionKind::ReadFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn missing_capability_is_denied() {
        let capability = file_capability(workspace_root(), FileAccess::Read);
        let action = action(
            ActionKind::ReadFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        // The granted set does not contain the required capability.
        let decision = engine().evaluate(&action, &sandbox_context(&[]));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn non_reason_action_with_no_capabilities_is_denied() {
        let capability = file_capability(workspace_root(), FileAccess::Read);
        let mut action = action(
            ActionKind::ReadFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        action.required_capabilities.clear();
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn subject_mismatch_is_denied() {
        let capability = file_capability(workspace_root(), FileAccess::Read);
        let action = action(
            ActionKind::ReadFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let matching = EvaluationContext::current(IsolationLevel::Sandbox, &capabilities, false)
            .with_subject("task");
        assert_eq!(
            engine().evaluate(&action, &matching).effect,
            DecisionEffect::Allow
        );
        let mismatched = EvaluationContext::current(IsolationLevel::Sandbox, &capabilities, false)
            .with_subject("someone-else");
        assert_eq!(
            engine().evaluate(&action, &mismatched).effect,
            DecisionEffect::Deny
        );
    }

    #[test]
    fn network_deny_rule_covers_subdomains_ports_and_case() {
        let engine = PolicyEngine::new(PolicySet {
            denied_network_hosts: vec!["evil.com".into()],
            ..PolicySet::default()
        });
        for target in [
            "evil.com",
            "EVIL.com",
            "evil.com:443",
            "evil.com.",
            "sub.evil.com",
            "sub.evil.com:8443",
        ] {
            let capability = network_capability(target, None);
            let action = action(
                ActionKind::NetworkRequest,
                target,
                RiskLevel::L3ExternalSideEffect,
                &capability,
            );
            let capabilities = [capability];
            let decision = engine.evaluate(
                &action,
                &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
            );
            assert_eq!(decision.effect, DecisionEffect::Deny, "target {target}");
        }
    }

    #[test]
    fn network_deny_rule_does_not_hit_lookalike_hosts() {
        let engine = PolicyEngine::new(PolicySet {
            denied_network_hosts: vec!["evil.com".into()],
            ..PolicySet::default()
        });
        let capability = network_capability("notevil.com", None);
        let action = action(
            ActionKind::NetworkRequest,
            "notevil.com",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine.evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
        );
        assert_eq!(decision.effect, DecisionEffect::Allow);
    }

    #[test]
    fn network_capability_matches_normalized_host() {
        let capability = network_capability("API.Example.com.", None);
        let action = action(
            ActionKind::NetworkRequest,
            "api.example.com:443",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
        );
        assert_eq!(decision.effect, DecisionEffect::Allow);
    }

    #[test]
    fn network_capability_does_not_extend_to_subdomains() {
        let capability = network_capability("example.com", None);
        let action = action(
            ActionKind::NetworkRequest,
            "api.example.com",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
        );
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn port_restricted_capability_requires_matching_explicit_port() {
        let capability = network_capability("api.example.com", Some(443));
        let capabilities = [capability.clone()];
        let context = EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true);

        let matching = action(
            ActionKind::NetworkRequest,
            "api.example.com:443",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        assert_eq!(
            engine().evaluate(&matching, &context).effect,
            DecisionEffect::Allow
        );

        let wrong_port = action(
            ActionKind::NetworkRequest,
            "api.example.com:8443",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        assert_eq!(
            engine().evaluate(&wrong_port, &context).effect,
            DecisionEffect::Deny
        );

        let no_port = action(
            ActionKind::NetworkRequest,
            "api.example.com",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        assert_eq!(
            engine().evaluate(&no_port, &context).effect,
            DecisionEffect::Deny
        );
    }

    #[test]
    fn unrestricted_network_capability_covers_any_port() {
        let capability = network_capability("api.example.com", None);
        let action = action(
            ActionKind::NetworkRequest,
            "api.example.com:8443",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
        );
        assert_eq!(decision.effect, DecisionEffect::Allow);
    }

    #[test]
    fn external_call_matches_by_splitting_at_the_first_colon() {
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::ExternalService {
                service: "mail".into(),
                operation: "send:bulk".into(),
            },
            issued_to: "task".into(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        };
        let action = action(
            ActionKind::ExternalCall,
            "mail:send:bulk",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
        );
        assert_eq!(decision.effect, DecisionEffect::Allow);
    }

    #[test]
    fn external_service_with_colon_in_service_name_never_matches() {
        // {service: "mail:send", operation: "bulk"} would collide with
        // {service: "mail", operation: "send:bulk"} under naive
        // concatenation; such capabilities are rejected at match time.
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::ExternalService {
                service: "mail:send".into(),
                operation: "bulk".into(),
            },
            issued_to: "task".into(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
        };
        let action = action(
            ActionKind::ExternalCall,
            "mail:send:bulk",
            RiskLevel::L3ExternalSideEffect,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(
            &action,
            &EvaluationContext::current(IsolationLevel::Brokered, &capabilities, true),
        );
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn move_file_requires_write_coverage_on_both_endpoints() {
        let capability = file_capability(workspace_root(), FileAccess::ReadWrite);
        let mut covered = action(
            ActionKind::MoveFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        covered.arguments.insert(
            ActionSpec::MOVE_DESTINATION_ARGUMENT.into(),
            workspace_destination().into(),
        );
        let capabilities = [capability.clone()];
        assert_eq!(
            engine()
                .evaluate(&covered, &sandbox_context(&capabilities))
                .effect,
            DecisionEffect::Allow
        );

        let mut outside = covered.clone();
        outside.arguments.insert(
            ActionSpec::MOVE_DESTINATION_ARGUMENT.into(),
            outside_destination().into(),
        );
        assert_eq!(
            engine()
                .evaluate(&outside, &sandbox_context(&capabilities))
                .effect,
            DecisionEffect::Deny
        );
    }

    #[test]
    fn move_file_without_destination_argument_is_denied() {
        let capability = file_capability(workspace_root(), FileAccess::ReadWrite);
        let action = action(
            ActionKind::MoveFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn move_file_destination_is_checked_against_deny_roots() {
        let capability = file_capability(system_root(), FileAccess::ReadWrite);
        let mut action = action(
            ActionKind::MoveFile,
            workspace_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        action.arguments.insert(
            ActionSpec::MOVE_DESTINATION_ARGUMENT.into(),
            denied_system_file().into(),
        );
        let capabilities = [capability];
        let decision = engine().evaluate(&action, &sandbox_context(&capabilities));
        assert_eq!(decision.effect, DecisionEffect::Deny);
    }

    #[test]
    fn evaluate_accumulates_every_failure_reason() {
        let mut capability = file_capability(workspace_root(), FileAccess::Read);
        capability.issued_at = Utc::now() - chrono::Duration::hours(2);
        capability.expires_at = Some(Utc::now() - chrono::Duration::hours(1));
        let action = action(
            ActionKind::WriteFile,
            denied_system_file(),
            RiskLevel::L1Sandboxed,
            &capability,
        );
        let capabilities = [capability];
        let context = EvaluationContext::current(IsolationLevel::None, &capabilities, false);
        let decision = engine().evaluate(&action, &context);
        assert_eq!(decision.effect, DecisionEffect::Deny);
        // Isolation, deny rule, capability activity, and coverage all failed.
        assert!(
            decision.reasons.len() >= 4,
            "expected accumulated reasons, got {:?}",
            decision.reasons
        );
    }

    #[test]
    fn default_policy_denies_system_roots() {
        let policy = PolicySet::default();
        assert!(policy.denied_path_roots.iter().any(|root| root == "/etc"));
        assert!(
            policy
                .denied_path_roots
                .iter()
                .any(|root| root == r"C:\Windows")
        );
        assert!(policy.require_confirmation_for_external_side_effects);
    }

    #[cfg(not(target_os = "windows"))]
    const fn workspace_root() -> &'static str {
        "/workspace"
    }

    #[cfg(target_os = "windows")]
    const fn workspace_root() -> &'static str {
        r"C:\workspace"
    }

    #[cfg(not(target_os = "windows"))]
    const fn workspace_file() -> &'static str {
        "/workspace/README.md"
    }

    #[cfg(target_os = "windows")]
    const fn workspace_file() -> &'static str {
        r"C:\workspace\README.md"
    }

    #[cfg(not(target_os = "windows"))]
    const fn system_root() -> &'static str {
        "/"
    }

    #[cfg(target_os = "windows")]
    const fn system_root() -> &'static str {
        r"C:\"
    }

    #[cfg(not(target_os = "windows"))]
    const fn denied_system_file() -> &'static str {
        "/etc/passwd"
    }

    #[cfg(target_os = "windows")]
    const fn denied_system_file() -> &'static str {
        r"C:\Windows\System32\config\SAM"
    }

    #[cfg(not(target_os = "windows"))]
    const fn downloads_root() -> &'static str {
        "/downloads"
    }

    #[cfg(target_os = "windows")]
    const fn downloads_root() -> &'static str {
        r"C:\downloads"
    }

    #[cfg(not(target_os = "windows"))]
    const fn unknown_download() -> &'static str {
        "/downloads/unknown.zip"
    }

    #[cfg(target_os = "windows")]
    const fn unknown_download() -> &'static str {
        r"C:\downloads\unknown.zip"
    }

    #[cfg(not(target_os = "windows"))]
    const fn traversal_into_denied_root() -> &'static str {
        "/tmp/../etc/passwd"
    }

    #[cfg(target_os = "windows")]
    const fn traversal_into_denied_root() -> &'static str {
        r"C:\Temp\..\Windows\System32\config\SAM"
    }

    #[cfg(not(target_os = "windows"))]
    const fn workspace_destination() -> &'static str {
        "/workspace/archive/README.md"
    }

    #[cfg(target_os = "windows")]
    const fn workspace_destination() -> &'static str {
        r"C:\workspace\archive\README.md"
    }

    #[cfg(not(target_os = "windows"))]
    const fn outside_destination() -> &'static str {
        "/elsewhere/README.md"
    }

    #[cfg(target_os = "windows")]
    const fn outside_destination() -> &'static str {
        r"C:\elsewhere\README.md"
    }
}
