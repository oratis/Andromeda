//! Concrete [`ArtifactVerifier`] implementations.
//!
//! The matcher itself is pure and does no I/O; resolving artifact bytes and
//! computing digests lives here so the trust boundary is a small, auditable
//! surface.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::matcher::{ArtifactVerdict, ArtifactVerifier};
use crate::model::ArtifactPin;

/// Verifies pinned artifacts against files in a base directory.
///
/// Each [`ArtifactPin`] is resolved to `<base_dir>/<name>`, its bytes are
/// hashed with SHA-256, and the lowercase hex digest is compared
/// case-insensitively to the pinned `sha256`. A file that is absent yields
/// [`ArtifactVerdict::Unresolved`]; a digest mismatch yields
/// [`ArtifactVerdict::Rejected`].
///
/// # Signing keys
///
/// When constructed with [`DirectoryArtifactVerifier::with_trusted_keys`], the
/// verifier also requires each pin to carry a `signing_key_id` present in the
/// trusted-key set; an unsigned pin or an untrusted key is rejected.
///
/// The signature check is intentionally a **documented stub**: it verifies
/// that the pin *names* a trusted key, not that a real detached signature over
/// the bytes is valid. Wiring actual signature cryptography (e.g. cosign /
/// an offline root key) is tracked as future work in
/// `docs/development/hardware-compatibility.md`.
#[derive(Debug, Clone)]
pub struct DirectoryArtifactVerifier {
    base_dir: PathBuf,
    trusted_key_ids: Option<BTreeSet<String>>,
}

impl DirectoryArtifactVerifier {
    /// Verifies artifact bytes against pinned digests, without enforcing any
    /// signing-key policy.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            trusted_key_ids: None,
        }
    }

    /// Verifies artifact bytes and additionally requires each pin to name a
    /// `signing_key_id` in `trusted_key_ids`.
    #[must_use]
    pub fn with_trusted_keys(
        base_dir: impl Into<PathBuf>,
        trusted_key_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            trusted_key_ids: Some(trusted_key_ids.into_iter().collect()),
        }
    }

    fn check_signing_key(&self, artifact: &ArtifactPin) -> Result<(), String> {
        let Some(trusted) = self.trusted_key_ids.as_ref() else {
            return Ok(());
        };
        // TODO: verify a real detached signature over the artifact bytes;
        // today only the declared key id is checked against the trusted set.
        match artifact.signing_key_id.as_deref() {
            Some(key_id) if trusted.contains(key_id) => Ok(()),
            Some(key_id) => Err(format!("signing key '{key_id}' is not trusted")),
            None => Err("artifact is unsigned but a trusted-key set is required".to_owned()),
        }
    }
}

impl ArtifactVerifier for DirectoryArtifactVerifier {
    fn verify(&self, artifact: &ArtifactPin) -> ArtifactVerdict {
        if let Err(reason) = self.check_signing_key(artifact) {
            return ArtifactVerdict::Rejected(reason);
        }
        let path = self.base_dir.join(&artifact.name);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ArtifactVerdict::Unresolved(format!("{} not found", path.display()));
            }
            Err(error) => {
                return ArtifactVerdict::Unresolved(format!(
                    "{} could not be read: {error}",
                    path.display()
                ));
            }
        };
        let actual = sha256_hex(&bytes);
        if actual.eq_ignore_ascii_case(artifact.sha256.trim()) {
            ArtifactVerdict::Trusted
        } else {
            ArtifactVerdict::Rejected(format!(
                "sha256 mismatch: pinned {}, computed {actual}",
                artifact.sha256
            ))
        }
    }
}

/// Computes the lowercase hex SHA-256 digest of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing to a String is infallible.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::model::ArtifactKind;

    use super::*;

    /// RAII temp directory under the system temp dir; removed on drop so the
    /// hardware crate stays dependency-free of `tempfile`.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = format!(
                "andromeda-hcm-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pin(name: &str, sha256: &str, signing_key_id: Option<&str>) -> ArtifactPin {
        ArtifactPin {
            kind: ArtifactKind::Kernel,
            name: name.to_owned(),
            version: "1".to_owned(),
            sha256: sha256.to_owned(),
            source: "test".to_owned(),
            signing_key_id: signing_key_id.map(str::to_owned),
        }
    }

    #[test]
    fn matching_bytes_are_trusted() {
        let dir = TempDir::new("match");
        let file = dir.path().join("kernel.img");
        fs::write(&file, b"hello andromeda").unwrap();
        let sha = sha256_hex(&fs::read(&file).unwrap());
        let verifier = DirectoryArtifactVerifier::new(dir.path());
        assert_eq!(
            verifier.verify(&pin("kernel.img", &sha, None)),
            ArtifactVerdict::Trusted
        );
        // Case-insensitive hex comparison.
        assert_eq!(
            verifier.verify(&pin("kernel.img", &sha.to_uppercase(), None)),
            ArtifactVerdict::Trusted
        );
    }

    #[test]
    fn tampered_bytes_are_rejected() {
        let dir = TempDir::new("tamper");
        let file = dir.path().join("kernel.img");
        fs::write(&file, b"actual bytes").unwrap();
        let verifier = DirectoryArtifactVerifier::new(dir.path());
        let verdict = verifier.verify(&pin("kernel.img", &"0".repeat(64), None));
        assert!(
            matches!(verdict, ArtifactVerdict::Rejected(reason) if reason.contains("mismatch"))
        );
    }

    #[test]
    fn missing_file_is_unresolved() {
        let dir = TempDir::new("missing");
        let verifier = DirectoryArtifactVerifier::new(dir.path());
        let verdict = verifier.verify(&pin("absent.img", &"0".repeat(64), None));
        assert!(
            matches!(verdict, ArtifactVerdict::Unresolved(reason) if reason.contains("not found"))
        );
    }

    #[test]
    fn untrusted_or_unsigned_key_is_rejected_before_reading_bytes() {
        let dir = TempDir::new("keys");
        let file = dir.path().join("kernel.img");
        fs::write(&file, b"bytes").unwrap();
        let sha = sha256_hex(&fs::read(&file).unwrap());
        let verifier =
            DirectoryArtifactVerifier::with_trusted_keys(dir.path(), ["trusted-key".to_owned()]);

        assert_eq!(
            verifier.verify(&pin("kernel.img", &sha, Some("trusted-key"))),
            ArtifactVerdict::Trusted
        );
        assert!(matches!(
            verifier.verify(&pin("kernel.img", &sha, Some("rogue-key"))),
            ArtifactVerdict::Rejected(reason) if reason.contains("not trusted")
        ));
        assert!(matches!(
            verifier.verify(&pin("kernel.img", &sha, None)),
            ArtifactVerdict::Rejected(reason) if reason.contains("unsigned")
        ));
    }
}
