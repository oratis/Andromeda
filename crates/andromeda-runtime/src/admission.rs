//! Whether a capability is allowed into a task at all.
//!
//! `andromeda-core` can tell you whether a capability carries a signature that
//! a trusted issuer produced. This module decides what the control plane does
//! with that answer, and it makes the decision *explicit*: [`CapabilityAdmission`]
//! has no `Default`, so every construction of a `TaskService` has to name the
//! policy it is running under. There is no way to end up unsigned by omission.

use andromeda_core::{
    Capability, CapabilityId, CapabilityKeyring, SignatureError, verify_capability_signature,
};

/// The policy a [`TaskService`](crate::TaskService) applies to incoming
/// capabilities.
///
/// # Why this is configuration rather than a hard requirement
///
/// Nothing in this repository *issues* capabilities yet. Rejecting unsigned
/// grants unconditionally would therefore reject every request any current
/// client can make, and orphan every task record already on disk — a change
/// that fails closed so hard it simply removes the feature. Instead the
/// mechanism ships now and the enforcement is a deployment decision, so an
/// operator who has an issuer can turn it on today and the shipped image can
/// state honestly which mode it runs in.
///
/// Both variants are spelled out at every call site, which is the point: the
/// weak one is named [`CapabilityAdmission::unsigned_for_development`] and
/// cannot be selected by forgetting an argument.
#[derive(Debug, Clone)]
pub enum CapabilityAdmission {
    /// Accept capabilities with or without a signature.
    ///
    /// This is the v0 development posture and it is **not** a security
    /// boundary: a caller mints its own grants, exactly as described in
    /// `docs/reviews/security-review.md` finding #3. Any signature that *is*
    /// present is left untouched and unverified — a verified-looking record
    /// under this mode means nothing.
    UnsignedAllowed,
    /// Require every capability to carry a signature that verifies against a
    /// key in the keyring. Unsigned, unknown-key, malformed, and tampered
    /// grants are all rejected.
    RequireSigned(Box<CapabilityKeyring>),
}

impl CapabilityAdmission {
    /// The development posture: unsigned capabilities are accepted.
    ///
    /// Named for what it costs, not for what it does, so that reading a call
    /// site tells you the deployment is unprotected.
    #[must_use]
    pub const fn unsigned_for_development() -> Self {
        Self::UnsignedAllowed
    }

    /// Require issuer signatures against `keyring`.
    ///
    /// # Errors
    /// Returns [`SignatureError::EmptyKeyring`] when `keyring` holds no keys.
    /// An empty keyring would reject every request while presenting as a
    /// hardened configuration; refusing to build it turns a typo in a
    /// trusted-keys file into a startup failure with a reason.
    pub fn require_signed(keyring: CapabilityKeyring) -> Result<Self, SignatureError> {
        Ok(Self::RequireSigned(Box::new(keyring.require_non_empty()?)))
    }

    /// Whether signatures are enforced. Reported on `/healthz` so the posture
    /// of a running daemon is observable from outside it.
    #[must_use]
    pub const fn requires_signatures(&self) -> bool {
        matches!(self, Self::RequireSigned(_))
    }

    /// A stable name for logs, `/healthz`, and documentation.
    #[must_use]
    pub const fn mode_name(&self) -> &'static str {
        match self {
            Self::UnsignedAllowed => "unsigned_allowed",
            Self::RequireSigned(_) => "require_signed",
        }
    }

    /// Checks every capability in `capabilities`, returning the first rejection.
    ///
    /// # Length bound
    ///
    /// Ed25519 verification is deliberately expensive, so running it over a
    /// caller-supplied vector of unbounded length is a local denial of service
    /// (the flaw recorded as `remediation-design-review.md` §1 item 6, bounded
    /// by PR #24). This function is therefore **only** called after the caller
    /// has enforced `MAX_TASK_CAPABILITIES`; it re-states that requirement in
    /// [`AdmissionError::Unbounded`] rather than trusting it, so a future call
    /// site that forgets gets an error instead of an unbounded verification
    /// loop.
    ///
    /// # Errors
    /// Returns [`AdmissionError`] for the first capability that is not
    /// admissible under this policy, or if `capabilities` exceeds `limit`.
    pub fn admit(&self, capabilities: &[Capability], limit: usize) -> Result<(), AdmissionError> {
        if capabilities.len() > limit {
            return Err(AdmissionError::Unbounded {
                capabilities: capabilities.len(),
                limit,
            });
        }
        let Self::RequireSigned(keyring) = self else {
            return Ok(());
        };
        for capability in capabilities {
            let status = verify_capability_signature(capability, keyring);
            if let Some(reason) = status.rejection_reason() {
                return Err(AdmissionError::Rejected {
                    capability: capability.id,
                    reason,
                });
            }
        }
        Ok(())
    }
}

/// A capability was refused before it could be attached to a task.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdmissionError {
    #[error("capability {capability} was not admitted: {reason}")]
    Rejected {
        capability: CapabilityId,
        reason: String,
    },
    /// The caller handed in more capabilities than the bound allows. Reaching
    /// this means a call site skipped its own length check; it is a programming
    /// error surfaced as a refusal rather than as unbounded work.
    #[error(
        "refusing to verify {capabilities} capabilities, which exceeds the limit of {limit}; \
         the length bound must be enforced before any signature check"
    )]
    Unbounded { capabilities: usize, limit: usize },
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use andromeda_core::{
        CapabilityResource, CapabilitySignature, CapabilitySigningKey, FileAccess,
    };
    use chrono::{TimeZone as _, Utc};

    use super::*;

    /// Fixed seed, mirroring `andromeda-hardware`: no test may depend on an RNG.
    const SEED: [u8; 32] = [7u8; 32];
    const OTHER_SEED: [u8; 32] = [8u8; 32];
    const KEY_ID: &str = "issuer-2026";

    fn signing_key() -> CapabilitySigningKey {
        CapabilitySigningKey::from_seed(&SEED)
    }

    fn keyring() -> CapabilityKeyring {
        let mut keyring = CapabilityKeyring::new();
        keyring
            .insert_hex(KEY_ID, &signing_key().verifying_key_hex())
            .expect("valid key hex");
        keyring
    }

    fn admission() -> CapabilityAdmission {
        CapabilityAdmission::require_signed(keyring()).expect("non-empty keyring")
    }

    fn capability() -> Capability {
        Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Files {
                root: PathBuf::from("/work/project"),
                access: FileAccess::Read,
            },
            issued_to: "task".to_owned(),
            issued_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            expires_at: None,
            single_use: false,
            signature: None,
        }
    }

    fn signed_capability() -> Capability {
        let mut capability = capability();
        signing_key()
            .sign_in_place(&mut capability, KEY_ID)
            .expect("sign");
        capability
    }

    #[test]
    fn a_valid_signed_capability_is_admitted() {
        assert_eq!(admission().admit(&[signed_capability()], 10), Ok(()));
    }

    #[test]
    fn a_tampered_capability_is_rejected() {
        let mut capability = signed_capability();
        capability.resource = CapabilityResource::Files {
            root: PathBuf::from("/"),
            access: FileAccess::ReadWrite,
        };
        let error = admission().admit(&[capability], 10).unwrap_err();
        assert!(
            matches!(&error, AdmissionError::Rejected { reason, .. } if reason.contains("did not verify")),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        let mut capability = capability();
        CapabilitySigningKey::from_seed(&OTHER_SEED)
            .sign_in_place(&mut capability, "rogue")
            .expect("sign");
        let error = admission().admit(&[capability], 10).unwrap_err();
        assert!(
            matches!(&error, AdmissionError::Rejected { reason, .. } if reason.contains("not in the keyring")),
            "{error}"
        );
    }

    #[test]
    fn an_unsigned_capability_is_rejected_when_signatures_are_required() {
        let error = admission().admit(&[capability()], 10).unwrap_err();
        assert!(
            matches!(&error, AdmissionError::Rejected { reason, .. } if reason.contains("no issuer signature")),
            "{error}"
        );
    }

    #[test]
    fn a_malformed_signature_is_rejected() {
        let mut capability = capability();
        capability.signature = Some(CapabilitySignature {
            key_id: KEY_ID.to_owned(),
            sig: "not-hex".to_owned(),
        });
        let error = admission().admit(&[capability], 10).unwrap_err();
        assert!(
            matches!(&error, AdmissionError::Rejected { reason, .. } if reason.contains("malformed")),
            "{error}"
        );
    }

    /// Constraint: the length bound must come *before* any cryptography. The
    /// list here is both over the limit and full of grants that would each fail
    /// verification, so the error identifies which check ran first.
    #[test]
    fn the_length_bound_is_enforced_before_any_verification() {
        let limit = 4;
        let capabilities: Vec<Capability> = std::iter::repeat_n(capability(), limit + 1).collect();
        assert_eq!(
            admission().admit(&capabilities, limit),
            Err(AdmissionError::Unbounded {
                capabilities: limit + 1,
                limit,
            })
        );
        // Exactly at the limit, the (failing) verification is what reports.
        let at_limit: Vec<Capability> = std::iter::repeat_n(capability(), limit).collect();
        assert!(matches!(
            admission().admit(&at_limit, limit),
            Err(AdmissionError::Rejected { .. })
        ));
    }

    /// The bound applies even in the permissive mode, so a call site cannot use
    /// "signatures are off today" to smuggle in an unbounded vector.
    #[test]
    fn the_length_bound_applies_in_unsigned_mode_too() {
        let admission = CapabilityAdmission::unsigned_for_development();
        assert!(matches!(
            admission.admit(&[capability(), capability()], 1),
            Err(AdmissionError::Unbounded { .. })
        ));
        assert_eq!(admission.admit(&[capability()], 1), Ok(()));
    }

    #[test]
    fn an_empty_keyring_cannot_be_configured() {
        assert!(CapabilityAdmission::require_signed(CapabilityKeyring::new()).is_err());
    }

    #[test]
    fn modes_report_themselves() {
        assert!(admission().requires_signatures());
        assert_eq!(admission().mode_name(), "require_signed");
        let development = CapabilityAdmission::unsigned_for_development();
        assert!(!development.requires_signatures());
        assert_eq!(development.mode_name(), "unsigned_allowed");
    }
}
