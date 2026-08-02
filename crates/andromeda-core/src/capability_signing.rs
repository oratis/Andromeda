//! Detached ed25519 authenticity for [`Capability`] grants.
//!
//! Security review finding #3 records that a capability is *self-asserted*: the
//! caller hands `taskd` a plan and the capabilities for it in one request, and
//! the only subject binding — `issued_to == plan.task_id` — is satisfied by a
//! `task_id` the same caller chose. This module supplies the missing piece: a
//! way for an issuer that is **not** the caller to vouch for a grant, and a way
//! for `taskd` to check that claim without holding any private key.
//!
//! ## What this achieves today, and what it does not
//!
//! There is no capability issuer, no executor, and no host broker in this
//! repository. A signature therefore proves exactly one thing right now: *the
//! holder of key `key_id` produced these grant bytes, and nobody has altered
//! them since*. It does **not** by itself close finding #3, because whoever can
//! run the signing helper is the issuer. The gap closes only when a trusted
//! host component owns the key material and the requesting process cannot reach
//! it — see `docs/andromeda-threat-model.md` §4.2.
//!
//! What it does buy today is real: with a keyring configured, `taskd` will
//! refuse a capability that no configured key vouched for, and any tampering
//! with an issued grant (widening a file root, deleting an expiry) invalidates
//! it. That turns "capabilities are unforgeable" from a documentation claim
//! into a checkable one, and it is the mechanism an issuer will plug into.
//!
//! ## Shape
//!
//! Mirrors `andromeda_hardware::signing`, deliberately: one canonicalization
//! function that both signing and verification go through, a keyring keyed by
//! `key_id`, a status enum whose only accepting variant is `Verified`, hex
//! encoding, `verify_strict`, and seed-derived keys so tests never touch an
//! RNG. The two schemes share [`crate::encoding`] so their canonical JSON and
//! hex can never drift apart.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::capability::{Capability, CapabilitySignature};
use crate::encoding::{canonical_json, hex};

/// Domain separator prefixed to every capability message.
///
/// A signature over a bare JSON object is a signature over *any* context that
/// object could be lifted into. Binding the message to this scheme means a
/// capability signature can never be replayed as a signature over some other
/// Andromeda structure, even if a future type serializes to the same JSON.
/// (HCM manifest signatures predate this module and are not prefixed; the two
/// message shapes are disjoint, and changing manifest bytes would invalidate
/// signatures already issued against them.)
const DOMAIN: &[u8] = b"andromeda-capability-v1\n";

/// Errors from loading key material or canonicalizing a capability.
///
/// Distinct from a verification *verdict* (see [`CapabilitySignatureStatus`]):
/// these are malformed inputs a caller can fix, not "this grant is untrusted".
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("invalid hex encoding: {0}")]
    Hex(String),
    #[error("ed25519 verifying key must be 32 bytes, got {0}")]
    KeyLength(usize),
    #[error("invalid ed25519 verifying key: {0}")]
    VerifyingKey(String),
    #[error("could not canonicalize capability: {0}")]
    Canonicalize(String),
    #[error("a keyring that trusts no keys cannot authenticate anything")]
    EmptyKeyring,
}

/// The verdict of checking a capability's signature against a keyring.
///
/// Only [`Verified`](CapabilitySignatureStatus::Verified) is an accept; every
/// other variant is a reason the capability fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySignatureStatus {
    /// The capability carried a signature by `key_id`, that key is in the
    /// keyring, and the ed25519 signature verified over the canonical bytes.
    Verified { key_id: String },
    /// The capability carried no `signature` field.
    Unsigned,
    /// The `signature.key_id` is not present in the keyring.
    UnknownKey { key_id: String },
    /// The signature bytes were not decodable / not 64 bytes, or the capability
    /// could not be canonicalized.
    Malformed { reason: String },
    /// The key resolved but the ed25519 signature did not verify — the grant
    /// was altered after issuance, or signed by a different key.
    Invalid { key_id: String, reason: String },
}

impl CapabilitySignatureStatus {
    /// Whether this verdict admits the capability. Exactly one variant does.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    /// A short reason suitable for an error message or audit event. Returns
    /// `None` for [`Verified`](CapabilitySignatureStatus::Verified).
    #[must_use]
    pub fn rejection_reason(&self) -> Option<String> {
        match self {
            Self::Verified { .. } => None,
            Self::Unsigned => Some("capability carries no issuer signature".to_owned()),
            Self::UnknownKey { key_id } => {
                Some(format!("signing key '{key_id}' is not in the keyring"))
            }
            Self::Malformed { reason } => Some(format!("malformed signature: {reason}")),
            Self::Invalid { key_id, reason } => {
                Some(format!("signature by '{key_id}' did not verify: {reason}"))
            }
        }
    }
}

/// A set of trusted ed25519 verifying keys, indexed by `key_id`.
///
/// This is the trust anchor for capability issuance: a grant is authentic only
/// if it resolves to a key in here. An empty keyring trusts nothing, which is
/// why [`CapabilityKeyring::require_non_empty`] exists — a configuration path
/// that accidentally produced an empty keyring would otherwise reject every
/// request while looking like it had enabled a security feature.
#[derive(Debug, Clone, Default)]
pub struct CapabilityKeyring {
    keys: BTreeMap<String, VerifyingKey>,
}

impl CapabilityKeyring {
    /// An empty keyring that trusts no keys.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a verifying key given as 64 hex characters (32 raw bytes). A later
    /// insert with the same `key_id` replaces the earlier key.
    ///
    /// # Errors
    /// Returns [`SignatureError`] if the hex is malformed, is not 32 bytes, or
    /// is not a valid ed25519 point.
    pub fn insert_hex(
        &mut self,
        key_id: impl Into<String>,
        verifying_key_hex: &str,
    ) -> Result<(), SignatureError> {
        let bytes = hex::decode(verifying_key_hex)
            .map_err(|error| SignatureError::Hex(error.to_string()))?;
        let array: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| SignatureError::KeyLength(bytes.len()))?;
        let key = VerifyingKey::from_bytes(&array)
            .map_err(|error| SignatureError::VerifyingKey(error.to_string()))?;
        self.keys.insert(key_id.into(), key);
        Ok(())
    }

    /// Builds a keyring from `(key_id, verifying_key_hex)` pairs — for example
    /// a parsed `{ "key-id": "<hex>" }` trusted-keys file.
    ///
    /// # Errors
    /// Returns the first [`SignatureError`] encountered while decoding a key.
    pub fn from_hex_entries<I>(entries: I) -> Result<Self, SignatureError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut keyring = Self::new();
        for (key_id, key_hex) in entries {
            keyring.insert_hex(key_id, &key_hex)?;
        }
        Ok(keyring)
    }

    /// Returns the keyring only if it holds at least one key.
    ///
    /// # Errors
    /// Returns [`SignatureError::EmptyKeyring`] when the keyring is empty.
    pub fn require_non_empty(self) -> Result<Self, SignatureError> {
        if self.is_empty() {
            Err(SignatureError::EmptyKeyring)
        } else {
            Ok(self)
        }
    }

    /// Whether the keyring holds no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The number of trusted keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether `key_id` names a trusted key.
    #[must_use]
    pub fn contains(&self, key_id: &str) -> bool {
        self.keys.contains_key(key_id)
    }

    /// The trusted `key_id`s, sorted.
    pub fn key_ids(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(String::as_str)
    }

    fn get(&self, key_id: &str) -> Option<&VerifyingKey> {
        self.keys.get(key_id)
    }
}

/// A deterministic ed25519 signing key for issuing capability signatures.
///
/// Built from a 32-byte seed, never from an RNG, so signing is reproducible in
/// tests and in an offline issuing tool. How a production seed is generated,
/// stored, and rotated is a deployment concern this type does not dictate —
/// and, importantly, `taskd` never constructs one: it holds verifying keys
/// only, so compromising the daemon does not yield the power to issue grants.
pub struct CapabilitySigningKey {
    inner: SigningKey,
}

impl CapabilitySigningKey {
    /// Builds a signing key from a fixed 32-byte seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            inner: SigningKey::from_bytes(seed),
        }
    }

    /// The matching verifying key as 64 lowercase hex characters — the value to
    /// publish and load into a [`CapabilityKeyring`].
    #[must_use]
    pub fn verifying_key_hex(&self) -> String {
        hex::encode(self.inner.verifying_key().as_bytes())
    }

    /// Signs `capability`'s canonical bytes and returns a
    /// [`CapabilitySignature`] tagged with `key_id`. Any signature already on
    /// the capability is ignored (canonicalization strips it), so this is safe
    /// to call on an already-signed grant.
    ///
    /// # Errors
    /// Returns [`SignatureError::Canonicalize`] if the capability cannot be
    /// serialized to canonical bytes.
    pub fn sign(
        &self,
        capability: &Capability,
        key_id: impl Into<String>,
    ) -> Result<CapabilitySignature, SignatureError> {
        let message = canonical_signing_bytes(capability)?;
        let signature = self.inner.sign(&message);
        Ok(CapabilitySignature {
            key_id: key_id.into(),
            sig: hex::encode(&signature.to_bytes()),
        })
    }

    /// Convenience for tests and issuing tools: signs `capability` and stores
    /// the resulting signature on it.
    ///
    /// # Errors
    /// Returns [`SignatureError::Canonicalize`] if the capability cannot be
    /// serialized to canonical bytes.
    pub fn sign_in_place(
        &self,
        capability: &mut Capability,
        key_id: impl Into<String>,
    ) -> Result<(), SignatureError> {
        capability.signature = Some(self.sign(capability, key_id)?);
        Ok(())
    }
}

/// Verifies a capability's detached signature against `keyring`.
///
/// Pure and side-effect free. Callers turn any non-verified status into a
/// refusal; see `andromeda_runtime::CapabilityAdmission`.
#[must_use]
pub fn verify_capability_signature(
    capability: &Capability,
    keyring: &CapabilityKeyring,
) -> CapabilitySignatureStatus {
    let Some(signature) = capability.signature.as_ref() else {
        return CapabilitySignatureStatus::Unsigned;
    };
    let Some(verifying_key) = keyring.get(&signature.key_id) else {
        return CapabilitySignatureStatus::UnknownKey {
            key_id: signature.key_id.clone(),
        };
    };
    let signature_bytes = match hex::decode(&signature.sig) {
        Ok(bytes) => bytes,
        Err(error) => {
            return CapabilitySignatureStatus::Malformed {
                reason: format!("signature hex: {error}"),
            };
        }
    };
    let signature_array: [u8; 64] = match signature_bytes.as_slice().try_into() {
        Ok(array) => array,
        Err(_) => {
            return CapabilitySignatureStatus::Malformed {
                reason: format!(
                    "ed25519 signature must be 64 bytes, got {}",
                    signature_bytes.len()
                ),
            };
        }
    };
    let message = match canonical_signing_bytes(capability) {
        Ok(message) => message,
        Err(error) => {
            return CapabilitySignatureStatus::Malformed {
                reason: error.to_string(),
            };
        }
    };
    // `verify_strict` rejects non-canonical signatures and small-order keys,
    // closing signature-malleability gaps that plain `verify` would accept.
    match verifying_key.verify_strict(&message, &Signature::from_bytes(&signature_array)) {
        Ok(()) => CapabilitySignatureStatus::Verified {
            key_id: signature.key_id.clone(),
        },
        Err(error) => CapabilitySignatureStatus::Invalid {
            key_id: signature.key_id.clone(),
            reason: error.to_string(),
        },
    }
}

/// Serializes `capability` to the canonical byte string that is signed and
/// verified: [`DOMAIN`] followed by canonical JSON of the typed capability with
/// the `signature` field removed (a signature cannot cover itself).
///
/// Serializing the *typed* model rather than any bytes the caller supplied is
/// what makes the encoding stable: whether an optional field arrived omitted or
/// as an explicit `null`, and in which order the fields were written, cannot
/// change the message. See [`crate::encoding::canonical_json`] for the rules.
///
/// # Errors
/// Returns [`SignatureError::Canonicalize`] if the capability cannot be turned
/// into a JSON value.
pub fn canonical_signing_bytes(capability: &Capability) -> Result<Vec<u8>, SignatureError> {
    let mut value = serde_json::to_value(capability)
        .map_err(|error| SignatureError::Canonicalize(error.to_string()))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("signature");
    }
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(canonical_json::to_string(&value).as_bytes());
    Ok(message)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeZone as _, Utc};

    use super::*;
    use crate::capability::{CapabilityResource, FileAccess};

    /// A fixed, non-random seed: signing must be reproducible, and no test may
    /// depend on an RNG. Mirrors `andromeda-hardware`'s approach. Any 32 bytes
    /// work.
    const SEED: [u8; 32] = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        0x00, 0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
        0xe1, 0xf0,
    ];

    fn signing_key() -> CapabilitySigningKey {
        CapabilitySigningKey::from_seed(&SEED)
    }

    fn keyring_with(key_id: &str, key: &CapabilitySigningKey) -> CapabilityKeyring {
        let mut keyring = CapabilityKeyring::new();
        keyring
            .insert_hex(key_id.to_owned(), &key.verifying_key_hex())
            .expect("valid verifying key hex");
        keyring
    }

    /// A fixed capability: no `Utc::now()`, so canonical bytes are stable.
    fn capability() -> Capability {
        Capability {
            id: "6d1f0a4e-7c3b-4b0e-9f2a-1c5d8e3b7a90"
                .parse()
                .expect("fixed capability id"),
            resource: CapabilityResource::Files {
                root: PathBuf::from("/work/project"),
                access: FileAccess::Read,
            },
            issued_to: "task-1".to_owned(),
            issued_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            expires_at: Some(Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap()),
            single_use: false,
            signature: None,
        }
    }

    #[test]
    fn seed_is_deterministic() {
        assert_eq!(
            signing_key().verifying_key_hex(),
            signing_key().verifying_key_hex()
        );
        assert_eq!(signing_key().verifying_key_hex().len(), 64);
    }

    #[test]
    fn signed_capability_verifies_against_its_key() {
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "issuer-2026").unwrap();
        let signature = capability.signature.clone().expect("signature");
        assert_eq!(signature.sig.len(), 128);
        assert!(signature.sig.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &key)),
            CapabilitySignatureStatus::Verified {
                key_id: "issuer-2026".to_owned()
            }
        );
    }

    #[test]
    fn canonicalization_survives_a_json_round_trip() {
        // A grant signed as a model must still verify after it has been
        // persisted and re-parsed, or every restart would invalidate the store.
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "issuer-2026").unwrap();
        let json = serde_json::to_string_pretty(&capability).unwrap();
        let reparsed: Capability = serde_json::from_str(&json).unwrap();
        assert!(
            verify_capability_signature(&reparsed, &keyring_with("issuer-2026", &key))
                .is_verified()
        );
    }

    /// The adversarial case this whole module exists for: an attacker takes a
    /// legitimately issued narrow grant and widens it.
    #[test]
    fn widening_a_grant_after_signing_is_detected() {
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "issuer-2026").unwrap();
        capability.resource = CapabilityResource::Files {
            root: PathBuf::from("/"),
            access: FileAccess::ReadWrite,
        };
        assert!(matches!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &key)),
            CapabilitySignatureStatus::Invalid { .. }
        ));
    }

    /// Dropping the expiry turns a scoped grant into a permanent one; the
    /// threat model calls this out specifically (§4.2).
    #[test]
    fn removing_the_expiry_after_signing_is_detected() {
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "issuer-2026").unwrap();
        capability.expires_at = None;
        assert!(matches!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &key)),
            CapabilitySignatureStatus::Invalid { .. }
        ));
    }

    /// Re-pointing a grant at another task must not survive either.
    #[test]
    fn changing_the_subject_after_signing_is_detected() {
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "issuer-2026").unwrap();
        capability.issued_to = "task-2".to_owned();
        assert!(matches!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &key)),
            CapabilitySignatureStatus::Invalid { .. }
        ));
    }

    #[test]
    fn unsigned_capability_reports_unsigned() {
        assert_eq!(
            verify_capability_signature(
                &capability(),
                &keyring_with("issuer-2026", &signing_key())
            ),
            CapabilitySignatureStatus::Unsigned
        );
    }

    #[test]
    fn unknown_key_id_is_reported() {
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "rogue-issuer").unwrap();
        assert_eq!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &key)),
            CapabilitySignatureStatus::UnknownKey {
                key_id: "rogue-issuer".to_owned()
            }
        );
    }

    #[test]
    fn a_different_key_under_a_trusted_id_fails_verification() {
        let signer = CapabilitySigningKey::from_seed(&[9u8; 32]);
        let trusted = signing_key();
        let mut capability = capability();
        signer
            .sign_in_place(&mut capability, "issuer-2026")
            .unwrap();
        assert!(matches!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &trusted)),
            CapabilitySignatureStatus::Invalid { .. }
        ));
    }

    #[test]
    fn malformed_signature_hex_is_reported() {
        let mut capability = capability();
        capability.signature = Some(CapabilitySignature {
            key_id: "issuer-2026".to_owned(),
            sig: "not-hex".to_owned(),
        });
        assert!(matches!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &signing_key())),
            CapabilitySignatureStatus::Malformed { .. }
        ));
    }

    #[test]
    fn short_signature_is_malformed_not_invalid() {
        let mut capability = capability();
        capability.signature = Some(CapabilitySignature {
            key_id: "issuer-2026".to_owned(),
            sig: hex::encode(&[0u8; 32]),
        });
        assert!(matches!(
            verify_capability_signature(&capability, &keyring_with("issuer-2026", &signing_key())),
            CapabilitySignatureStatus::Malformed { .. }
        ));
    }

    #[test]
    fn empty_keyring_trusts_nothing() {
        let key = signing_key();
        let mut capability = capability();
        key.sign_in_place(&mut capability, "issuer-2026").unwrap();
        assert!(matches!(
            verify_capability_signature(&capability, &CapabilityKeyring::new()),
            CapabilitySignatureStatus::UnknownKey { .. }
        ));
        assert!(CapabilityKeyring::new().require_non_empty().is_err());
    }

    #[test]
    fn canonical_bytes_ignore_the_signature_field() {
        let key = signing_key();
        let unsigned = capability();
        let mut signed = capability();
        key.sign_in_place(&mut signed, "issuer-2026").unwrap();
        assert_eq!(
            canonical_signing_bytes(&unsigned).unwrap(),
            canonical_signing_bytes(&signed).unwrap()
        );
    }

    /// Domain separation is part of the message, not decoration: the bytes must
    /// actually start with the tag, so a signature over a bare capability JSON
    /// produced elsewhere cannot be replayed here.
    #[test]
    fn canonical_bytes_are_domain_separated() {
        let bytes = canonical_signing_bytes(&capability()).unwrap();
        assert!(bytes.starts_with(DOMAIN));
        assert_eq!(bytes[DOMAIN.len()], b'{');
    }

    #[test]
    fn signing_is_reproducible_from_the_seed() {
        // Two independently constructed keys from the same seed must produce
        // identical signature bytes; this is what makes fixed-seed tests
        // meaningful rather than merely non-random.
        let first = signing_key().sign(&capability(), "issuer-2026").unwrap();
        let second = signing_key().sign(&capability(), "issuer-2026").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn rejection_reasons_are_reported_for_every_failure() {
        assert!(
            CapabilitySignatureStatus::Verified {
                key_id: "k".to_owned()
            }
            .rejection_reason()
            .is_none()
        );
        for status in [
            CapabilitySignatureStatus::Unsigned,
            CapabilitySignatureStatus::UnknownKey {
                key_id: "k".to_owned(),
            },
            CapabilitySignatureStatus::Malformed {
                reason: "r".to_owned(),
            },
            CapabilitySignatureStatus::Invalid {
                key_id: "k".to_owned(),
                reason: "r".to_owned(),
            },
        ] {
            assert!(!status.is_verified());
            assert!(status.rejection_reason().is_some(), "{status:?}");
        }
    }
}
