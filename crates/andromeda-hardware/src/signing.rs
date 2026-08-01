//! Detached ed25519 authenticity for HCM manifests.
//!
//! The matcher (`crate::matcher`) only ever checked a manifest's *internal
//! consistency and freshness*; a forged manifest could therefore claim any
//! [`crate::SupportTier`]. This module closes that gap (security review
//! finding #1) with a real detached signature:
//!
//! 1. A manifest is canonicalized — serialized through the typed model, the
//!    `signature` field removed, and every object key sorted — so the signed
//!    bytes do not depend on JSON whitespace, key order, or whether optional
//!    fields were written as `null` or omitted. See [`canonical_signing_bytes`].
//! 2. Those bytes are signed with an ed25519 signing key held offline by the
//!    manifest publisher, producing a [`crate::ManifestSignature`].
//! 3. At evaluation time a [`TrustedKeyring`] maps `key_id -> verifying key`.
//!    When the caller supplies a keyring, the matcher requires a signature that
//!    resolves to a trusted key and verifies; anything else is fail-closed.
//!
//! Key generation, distribution, and the signing of *production* manifests are
//! deployment/ops concerns and live outside this crate. What lives here is the
//! verification path plus a deterministic signing helper ([`ManifestSigningKey`])
//! used by tests and by any future offline signing tool.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::model::{HcmManifest, ManifestSignature};

/// Errors from loading key material or canonicalizing a manifest.
///
/// Deliberately distinct from a verification *verdict* (see
/// [`ManifestSignatureStatus`]): these are malformed inputs a caller can fix,
/// not "this manifest is untrusted".
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("invalid hex encoding: {0}")]
    Hex(String),
    #[error("ed25519 verifying key must be 32 bytes, got {0}")]
    KeyLength(usize),
    #[error("invalid ed25519 verifying key: {0}")]
    VerifyingKey(String),
    #[error("could not canonicalize manifest: {0}")]
    Canonicalize(String),
}

/// The verdict of checking a manifest's signature against a keyring.
///
/// Only [`Verified`](ManifestSignatureStatus::Verified) is an accept; every
/// other variant is a reason the manifest fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestSignatureStatus {
    /// The manifest carried a signature by `key_id`, that key is in the
    /// keyring, and the ed25519 signature verified over the canonical bytes.
    Verified { key_id: String },
    /// The manifest carried no `signature` field.
    Unsigned,
    /// The `signature.key_id` is not present in the keyring.
    UnknownKey { key_id: String },
    /// The signature bytes were not decodable / not 64 bytes, or the manifest
    /// could not be canonicalized.
    Malformed { reason: String },
    /// The key resolved but the ed25519 signature did not verify (tampering, or
    /// a signature produced by a different key/message).
    Invalid { key_id: String, reason: String },
}

/// A set of trusted ed25519 verifying keys, indexed by `key_id`.
///
/// This is the trust anchor: a manifest (and its pinned artifacts) is only
/// authentic if it resolves to a key in here. Construct one from the operator's
/// published keys and pass it to `crate::evaluate_manifest_verified`. An empty
/// keyring trusts nothing — every signed manifest fails closed with
/// [`ManifestSignatureStatus::UnknownKey`].
#[derive(Debug, Clone, Default)]
pub struct TrustedKeyring {
    keys: BTreeMap<String, VerifyingKey>,
}

impl TrustedKeyring {
    /// An empty keyring that trusts no keys.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a verifying key given as 64 lowercase/uppercase hex characters
    /// (32 raw bytes). A later insert with the same `key_id` replaces the
    /// earlier key.
    ///
    /// # Errors
    /// Returns [`SignatureError`] if the hex is malformed, is not 32 bytes, or
    /// is not a valid ed25519 point.
    pub fn insert_hex(
        &mut self,
        key_id: impl Into<String>,
        verifying_key_hex: &str,
    ) -> Result<(), SignatureError> {
        let bytes = hex_decode(verifying_key_hex)?;
        let array: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| SignatureError::KeyLength(bytes.len()))?;
        let key = VerifyingKey::from_bytes(&array)
            .map_err(|error| SignatureError::VerifyingKey(error.to_string()))?;
        self.keys.insert(key_id.into(), key);
        Ok(())
    }

    /// Builds a keyring from `(key_id, verifying_key_hex)` pairs — for example a
    /// parsed `{ "key-id": "<hex>" }` trusted-keys file.
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

/// A deterministic ed25519 signing key for producing manifest signatures.
///
/// Built from a 32-byte seed (never from an RNG), so signing is reproducible in
/// tests and in an offline signing tool. Production key management — how the
/// seed is generated, stored (HSM/offline root), and rotated — is a deployment
/// concern this type does not dictate.
pub struct ManifestSigningKey {
    inner: SigningKey,
}

impl ManifestSigningKey {
    /// Builds a signing key from a fixed 32-byte seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            inner: SigningKey::from_bytes(seed),
        }
    }

    /// The matching verifying key as 64 lowercase hex characters — the value to
    /// publish and load into a [`TrustedKeyring`].
    #[must_use]
    pub fn verifying_key_hex(&self) -> String {
        hex_encode(self.inner.verifying_key().as_bytes())
    }

    /// Signs `manifest`'s canonical bytes and returns a [`ManifestSignature`]
    /// tagged with `key_id`. Any existing `signature` on the manifest is
    /// ignored (canonicalization strips it), so this is safe to call on an
    /// already-signed manifest.
    ///
    /// # Errors
    /// Returns [`SignatureError::Canonicalize`] if the manifest cannot be
    /// serialized to canonical bytes.
    pub fn sign_manifest(
        &self,
        manifest: &HcmManifest,
        key_id: impl Into<String>,
    ) -> Result<ManifestSignature, SignatureError> {
        let message = canonical_signing_bytes(manifest)?;
        let signature = self.inner.sign(&message);
        Ok(ManifestSignature {
            key_id: key_id.into(),
            sig: hex_encode(&signature.to_bytes()),
        })
    }
}

/// Verifies a manifest's detached signature against `keyring`.
///
/// Pure and side-effect free; the matcher calls this and turns any non-verified
/// status into a `Blocked` reason. See [`ManifestSignatureStatus`].
#[must_use]
pub fn verify_manifest_signature(
    manifest: &HcmManifest,
    keyring: &TrustedKeyring,
) -> ManifestSignatureStatus {
    let Some(signature) = manifest.signature.as_ref() else {
        return ManifestSignatureStatus::Unsigned;
    };
    let Some(verifying_key) = keyring.get(&signature.key_id) else {
        return ManifestSignatureStatus::UnknownKey {
            key_id: signature.key_id.clone(),
        };
    };
    let signature_bytes = match hex_decode(&signature.sig) {
        Ok(bytes) => bytes,
        Err(error) => {
            return ManifestSignatureStatus::Malformed {
                reason: format!("signature hex: {error}"),
            };
        }
    };
    let signature_array: [u8; 64] = match signature_bytes.as_slice().try_into() {
        Ok(array) => array,
        Err(_) => {
            return ManifestSignatureStatus::Malformed {
                reason: format!(
                    "ed25519 signature must be 64 bytes, got {}",
                    signature_bytes.len()
                ),
            };
        }
    };
    let message = match canonical_signing_bytes(manifest) {
        Ok(message) => message,
        Err(error) => {
            return ManifestSignatureStatus::Malformed {
                reason: error.to_string(),
            };
        }
    };
    // `verify_strict` rejects non-canonical signatures and small-order keys,
    // closing signature-malleability gaps that plain `verify` would accept.
    match verifying_key.verify_strict(&message, &Signature::from_bytes(&signature_array)) {
        Ok(()) => ManifestSignatureStatus::Verified {
            key_id: signature.key_id.clone(),
        },
        Err(error) => ManifestSignatureStatus::Invalid {
            key_id: signature.key_id.clone(),
            reason: error.to_string(),
        },
    }
}

/// Serializes `manifest` to the canonical byte string that is signed and
/// verified.
///
/// Canonicalization rules (stable, and the single source of truth both signing
/// and verification go through):
///
/// - Serialize the *typed* manifest, not the raw file, so omitted vs explicit
///   `null` and source formatting never change the bytes.
/// - Remove the `signature` field (a signature cannot cover itself).
/// - Emit compact JSON with **every object's keys sorted** by Unicode scalar
///   value; arrays keep their order (arrays are semantically ordered).
/// - Scalars use `serde_json`'s own encoding (correct string escaping;
///   integers and booleans are already deterministic — the manifest has no
///   floats).
///
/// # Errors
/// Returns [`SignatureError::Canonicalize`] if the manifest cannot be turned
/// into a JSON value.
pub fn canonical_signing_bytes(manifest: &HcmManifest) -> Result<Vec<u8>, SignatureError> {
    let mut value = serde_json::to_value(manifest)
        .map_err(|error| SignatureError::Canonicalize(error.to_string()))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("signature");
    }
    let mut canonical = String::new();
    write_canonical(&value, &mut canonical);
    Ok(canonical.into_bytes())
}

/// Recursively writes `value` as canonical JSON with sorted object keys.
fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_json_string(key, out);
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        // Scalars: `Value`'s Display is compact JSON with correct escaping.
        scalar => {
            let _ = write!(out, "{scalar}");
        }
    }
}

/// Writes `text` as a JSON string literal (quoted and escaped).
fn write_json_string(text: &str, out: &mut String) {
    let _ = write!(out, "{}", serde_json::Value::String(text.to_owned()));
}

/// Lowercase-hex encodes `bytes`.
fn hex_encode(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String is infallible.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Decodes an even-length hex string, ignoring surrounding whitespace.
fn hex_decode(input: &str) -> Result<Vec<u8>, SignatureError> {
    let input = input.trim();
    if input.len() % 2 != 0 {
        return Err(SignatureError::Hex("odd number of hex digits".to_owned()));
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_value(bytes[index])?;
        let low = hex_value(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Ok(out)
}

fn hex_value(character: u8) -> Result<u8, SignatureError> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        other => Err(SignatureError::Hex(format!(
            "invalid hex digit '{}'",
            char::from(other)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed, non-random seed: signing must be reproducible, and the CI
    /// environment forbids RNG in some contexts. Any 32 bytes work.
    const SEED: [u8; 32] = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        0x00, 0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
        0xe1, 0xf0,
    ];

    fn signing_key() -> ManifestSigningKey {
        ManifestSigningKey::from_seed(&SEED)
    }

    fn keyring_with(key_id: &str, key: &ManifestSigningKey) -> TrustedKeyring {
        let mut keyring = TrustedKeyring::new();
        keyring
            .insert_hex(key_id.to_owned(), &key.verifying_key_hex())
            .expect("valid verifying key hex");
        keyring
    }

    fn manifest() -> HcmManifest {
        serde_json::from_str(
            r#"{
                "schema_version": 2,
                "id": "signed-example",
                "name": "Signed Example",
                "tier": "supported",
                "boot_provider": "pc_uefi_shim",
                "selectors": [{"os_family": "linux", "architectures": ["x86_64"]}],
                "requirements": [{"type": "memory_bytes", "minimum": 1}],
                "kernel_channels": ["stable"],
                "artifacts": [],
                "evidence": [],
                "expires_at": "2099-01-01T00:00:00Z",
                "notes": []
            }"#,
        )
        .expect("manifest fixture")
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
    fn signed_manifest_verifies_against_its_key() {
        let key = signing_key();
        let mut manifest = manifest();
        let signature = key.sign_manifest(&manifest, "prod-2026").unwrap();
        // The emitted `sig` must satisfy the JSON schema's 128-hex-char pattern
        // for `$defs/signature.sig`, so real output stays schema-valid.
        assert_eq!(signature.sig.len(), 128);
        assert!(signature.sig.bytes().all(|b| b.is_ascii_hexdigit()));
        manifest.signature = Some(signature);
        let keyring = keyring_with("prod-2026", &key);
        assert_eq!(
            verify_manifest_signature(&manifest, &keyring),
            ManifestSignatureStatus::Verified {
                key_id: "prod-2026".to_owned()
            }
        );
    }

    #[test]
    fn canonicalization_survives_a_json_round_trip() {
        // A signature made over the model must still verify after the manifest
        // is serialized and re-parsed (proving key order / whitespace / omitted
        // optionals do not change the signed bytes).
        let key = signing_key();
        let mut manifest = manifest();
        manifest.signature = Some(key.sign_manifest(&manifest, "prod-2026").unwrap());
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let reparsed: HcmManifest = serde_json::from_str(&json).unwrap();
        let keyring = keyring_with("prod-2026", &key);
        assert!(matches!(
            verify_manifest_signature(&reparsed, &keyring),
            ManifestSignatureStatus::Verified { .. }
        ));
    }

    #[test]
    fn tampering_after_signing_is_detected() {
        let key = signing_key();
        let mut manifest = manifest();
        manifest.signature = Some(key.sign_manifest(&manifest, "prod-2026").unwrap());
        manifest.tier = crate::SupportTier::Certified; // flip a field after signing
        let keyring = keyring_with("prod-2026", &key);
        assert!(matches!(
            verify_manifest_signature(&manifest, &keyring),
            ManifestSignatureStatus::Invalid { .. }
        ));
    }

    #[test]
    fn unsigned_manifest_reports_unsigned() {
        let keyring = keyring_with("prod-2026", &signing_key());
        assert_eq!(
            verify_manifest_signature(&manifest(), &keyring),
            ManifestSignatureStatus::Unsigned
        );
    }

    #[test]
    fn unknown_key_id_is_reported() {
        let key = signing_key();
        let mut manifest = manifest();
        manifest.signature = Some(key.sign_manifest(&manifest, "unknown").unwrap());
        let keyring = keyring_with("prod-2026", &key);
        assert_eq!(
            verify_manifest_signature(&manifest, &keyring),
            ManifestSignatureStatus::UnknownKey {
                key_id: "unknown".to_owned()
            }
        );
    }

    #[test]
    fn wrong_key_in_keyring_fails_verification() {
        // key_id matches, but the keyring holds a different key for that id.
        let signer = signing_key();
        let other = ManifestSigningKey::from_seed(&[9u8; 32]);
        let mut manifest = manifest();
        manifest.signature = Some(signer.sign_manifest(&manifest, "prod-2026").unwrap());
        let keyring = keyring_with("prod-2026", &other);
        assert!(matches!(
            verify_manifest_signature(&manifest, &keyring),
            ManifestSignatureStatus::Invalid { .. }
        ));
    }

    #[test]
    fn malformed_signature_hex_is_reported() {
        let mut manifest = manifest();
        manifest.signature = Some(ManifestSignature {
            key_id: "prod-2026".to_owned(),
            sig: "not-hex".to_owned(),
        });
        let keyring = keyring_with("prod-2026", &signing_key());
        assert!(matches!(
            verify_manifest_signature(&manifest, &keyring),
            ManifestSignatureStatus::Malformed { .. }
        ));
    }

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        let bytes = [0x00u8, 0x0f, 0xa5, 0xff];
        assert_eq!(hex_encode(&bytes), "000fa5ff");
        assert_eq!(hex_decode("000fA5ff").unwrap(), bytes);
        assert!(hex_decode("abc").is_err()); // odd length
        assert!(hex_decode("zz").is_err()); // non-hex digit
    }

    #[test]
    fn empty_keyring_trusts_nothing() {
        let key = signing_key();
        let mut manifest = manifest();
        manifest.signature = Some(key.sign_manifest(&manifest, "prod-2026").unwrap());
        assert!(matches!(
            verify_manifest_signature(&manifest, &TrustedKeyring::new()),
            ManifestSignatureStatus::UnknownKey { .. }
        ));
    }

    #[test]
    fn canonical_bytes_ignore_signature_field() {
        let key = signing_key();
        let unsigned = manifest();
        let mut signed = manifest();
        signed.signature = Some(key.sign_manifest(&unsigned, "prod-2026").unwrap());
        assert_eq!(
            canonical_signing_bytes(&unsigned).unwrap(),
            canonical_signing_bytes(&signed).unwrap()
        );
    }
}
