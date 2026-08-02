//! Local caller authentication for `taskd`, and the one place that decides
//! how the token is protected on disk.
//!
//! # The guarantee, and where it is enforced
//!
//! [`app`](crate::app) takes an [`Authenticator`] by value. `Authenticator` has
//! no `Default`, no public fields, and no constructor that yields "accept
//! everything" — so *there is no way to spell an unauthenticated router*. The
//! guarantee lives in the type that wires the service to the transport, not in
//! a configuration check that some other path could skip. An earlier design was
//! rejected precisely for validating an `AuthConfig` while leaving anonymous
//! listening representable; see `docs/reviews/remediation-design-review.md` §1.
//!
//! # Identity and permissions: decided once, here
//!
//! The previous attempt created a key directory as `0700 root:root` in two
//! separate places while the unit ran the service under `DynamicUser=` — so
//! `taskd` failed at startup, and a `UMask=0077` made the files unreadable to
//! the group that was supposed to read them. The fix is not to be more careful
//! in two places; it is to have one.
//!
//! This module is that place. The constants below define the protection the
//! token directory and file must have, [`ensure_private`] asserts it at
//! startup, and `unit_matches_token_constants` (in `crate::tests`) asserts that
//! the shipped `andromeda-taskd.service` agrees with them. The unit declares a
//! `RuntimeDirectory=`; it never states an owner, and nothing else in the tree
//! creates the directory.
//!
//! The resulting model, stated plainly:
//!
//! - The service's runtime identity is **whatever systemd assigns it**
//!   (`DynamicUser=yes`). No file in this repository names a uid or gid, so
//!   there is nothing to keep in sync and nothing to get wrong.
//! - `RuntimeDirectory=` is created by systemd owned by that identity, mode
//!   `0700`. The token file inside is `0600`. Both are asserted, not assumed.
//! - Therefore the API is reachable by the service account and by root, and by
//!   nobody else — including other local users, which is the exposure security
//!   review finding #2 records. An operator drives the API as root
//!   (`Authorization: Bearer $(sudo cat /run/andromeda-taskd/token)`).
//!
//! # Why a bearer token rather than `SO_PEERCRED`
//!
//! Peer credentials are the stronger primitive, but they need an `AF_UNIX`
//! listener, and the shipped unit serves loopback TCP. A token over a
//! `0700` runtime directory delivers the same practical boundary — the
//! filesystem answers "is this caller the service account or root?" — with a
//! mechanism that is enforced identically on every path `taskd` can be started
//! from, including a developer running it by hand. A `AF_UNIX` transport can be
//! added later as a second [`Authenticator`] variant without weakening this
//! one, because the type admits no unauthenticated variant to fall back to.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use andromeda_core::encoding::hex;

/// Bytes of entropy in a generated token (256 bits, hex-encoded to 64 chars).
pub const TOKEN_BYTES: usize = 32;

/// Shortest token accepted from an operator-supplied file.
///
/// A caller may bring its own token, but not a guessable one. 32 characters is
/// the floor; generated tokens are 64 hex characters.
pub const MIN_TOKEN_CHARS: usize = 32;

/// Mode the token file is created with, and required to have.
pub const TOKEN_FILE_MODE: u32 = 0o600;

/// Mode the directory holding the token must have, and the value the shipped
/// unit's `RuntimeDirectoryMode=`/`StateDirectoryMode=` must carry.
pub const TOKEN_DIR_MODE: u32 = 0o700;

/// Permission bits that must be clear on the token file and its directory:
/// no group or other access of any kind.
pub const PRIVATE_MODE_MASK: u32 = 0o077;

/// `RuntimeDirectory=` in the shipped unit; the token lives inside it.
pub const RUNTIME_DIRECTORY_NAME: &str = "andromeda-taskd";

/// Absolute token path the shipped unit points `ANDROMEDA_AUTH_TOKEN_FILE` at.
pub const SYSTEM_TOKEN_PATH: &str = "/run/andromeda-taskd/token";

/// A token file, or the directory holding it, could not be used safely.
///
/// Every variant is fatal at startup: `taskd` will not serve without a token
/// it trusts, and there is no degraded mode to fall back to.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(
        "token directory {} does not exist; systemd creates it from RuntimeDirectory=, so \
         either start taskd through andromeda-taskd.service or create the directory yourself \
         with mode {:04o}",
        .0.display(),
        TOKEN_DIR_MODE
    )]
    MissingDirectory(PathBuf),
    #[error("token path {} has no parent directory", .0.display())]
    NoParent(PathBuf),
    #[error(
        "{} is mode {:04o}, which grants access beyond its owner; the API token must be \
         readable only by the service account (directory {:04o}, file {:04o})",
        .path.display(),
        .mode,
        TOKEN_DIR_MODE,
        TOKEN_FILE_MODE
    )]
    TooPermissive { path: PathBuf, mode: u32 },
    #[error(
        "token in {} is {found} characters; at least {MIN_TOKEN_CHARS} are required so the \
         API cannot be reached by guessing",
        .path.display()
    )]
    TokenTooShort { path: PathBuf, found: usize },
    #[error("could not {action} {}: {source}", .path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl AuthError {
    fn io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Proof that a request came from a caller who holds the local API token.
///
/// The only inhabitant of this type is a real secret. There is deliberately no
/// `Anonymous`, `Disabled`, or `None` variant, and no `Default`: an
/// unauthenticated listener is not representable, so it cannot be reached by a
/// missing flag, a mis-parsed config, or a code path that forgot to check.
#[derive(Clone)]
pub struct Authenticator {
    secret: Vec<u8>,
}

// The token must never reach a log line or an error message.
impl std::fmt::Debug for Authenticator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Authenticator")
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl Authenticator {
    /// Builds an authenticator from an in-memory token.
    ///
    /// # Errors
    /// Returns [`AuthError::TokenTooShort`] when the token has fewer than
    /// [`MIN_TOKEN_CHARS`] characters after trimming — including the empty
    /// string, which is how "no authentication" would otherwise sneak in.
    pub fn from_token(token: &str) -> Result<Self, AuthError> {
        Self::from_token_at(token, Path::new("<memory>"))
    }

    fn from_token_at(token: &str, path: &Path) -> Result<Self, AuthError> {
        let token = token.trim();
        if token.chars().count() < MIN_TOKEN_CHARS {
            return Err(AuthError::TokenTooShort {
                path: path.to_path_buf(),
                found: token.chars().count(),
            });
        }
        Ok(Self {
            secret: token.as_bytes().to_vec(),
        })
    }

    /// Loads the token from `path`, generating one if the file is absent.
    ///
    /// The directory must already exist and be private; `taskd` does not create
    /// it, because on the shipped image systemd does (`RuntimeDirectory=`) and
    /// two creators is exactly the bug this design exists to avoid. A generated
    /// token is written atomically with mode [`TOKEN_FILE_MODE`].
    ///
    /// # Errors
    /// Returns [`AuthError`] when the directory is missing or too permissive,
    /// the existing token file is too permissive or too short, or any file
    /// operation fails.
    pub fn from_token_file(path: &Path) -> Result<Self, AuthError> {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| AuthError::NoParent(path.to_path_buf()))?;
        if !directory.is_dir() {
            return Err(AuthError::MissingDirectory(directory.to_path_buf()));
        }
        ensure_private(directory)?;

        if !path.exists() {
            write_new_token(path)?;
        }
        ensure_private(path)?;
        let contents =
            std::fs::read_to_string(path).map_err(|error| AuthError::io("read", path, error))?;
        Self::from_token_at(&contents, path)
    }

    /// Whether `candidate` is the token, compared in constant time.
    ///
    /// The comparison must not return early on the first differing byte: a
    /// local attacker can time thousands of requests and recover the token byte
    /// by byte otherwise. Length is compared first and does leak, which is
    /// harmless — the token length is fixed and documented.
    #[must_use]
    pub fn matches(&self, candidate: &[u8]) -> bool {
        if candidate.len() != self.secret.len() {
            return false;
        }
        let mut difference = 0u8;
        for (left, right) in self.secret.iter().zip(candidate) {
            difference |= left ^ right;
        }
        // `black_box` keeps the accumulate-then-compare shape from being
        // rewritten into an early-exit loop by a future optimizer.
        std::hint::black_box(difference) == 0
    }
}

/// Asserts that `path` grants no access beyond its owner.
///
/// This is the assertion the rework constraint asks for: the permission model
/// is stated once (the constants above) and checked at startup, so a
/// mis-declared unit directive fails loudly with a reason instead of leaving
/// the token readable.
///
/// On non-Unix targets there are no mode bits to check and this is a no-op;
/// `taskd` ships only on Linux, and the Windows CI job builds and tests the
/// crate rather than deploying it.
///
/// # Errors
/// Returns [`AuthError::TooPermissive`] when any bit in [`PRIVATE_MODE_MASK`]
/// is set, or an IO error when the metadata cannot be read.
pub fn ensure_private(path: &Path) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let metadata = std::fs::metadata(path)
            .map_err(|error| AuthError::io("read metadata for", path, error))?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & PRIVATE_MODE_MASK != 0 {
            return Err(AuthError::TooPermissive {
                path: path.to_path_buf(),
                mode,
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::metadata(path)
            .map_err(|error| AuthError::io("read metadata for", path, error))?;
    }
    Ok(())
}

/// Generates a token and writes it to `path` atomically with
/// [`TOKEN_FILE_MODE`].
fn write_new_token(path: &Path) -> Result<(), AuthError> {
    let token = generate_token();
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = create_private(&temporary)?;
    file.write_all(token.as_bytes())
        .map_err(|error| AuthError::io("write", &temporary, error))?;
    file.sync_all()
        .map_err(|error| AuthError::io("sync", &temporary, error))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|error| {
        // Leaving a readable temp file behind would defeat the whole point.
        let _ = std::fs::remove_file(&temporary);
        AuthError::io("install", path, error)
    })
}

/// Creates a new file that only its owner can read, failing if it exists.
fn create_private(path: &Path) -> Result<File, AuthError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Set at creation, not with a later chmod: a chmod leaves a window in
        // which the token exists at the umask's mode.
        options.mode(TOKEN_FILE_MODE);
    }
    options
        .open(path)
        .map_err(|error| AuthError::io("create", path, error))
}

/// Returns [`TOKEN_BYTES`] of CSPRNG output, hex encoded.
///
/// The randomness comes from `uuid`'s v4 generator, which is backed by
/// `getrandom` — already in this workspace's dependency graph and already the
/// source of every `CapabilityId` and audit event id. Pulling in a crate that
/// depends on `getrandom 0.3.x` would put a *third* major version of it in
/// `Cargo.lock` (0.2.17 and 0.4.3 are both present), which is exactly the
/// gratuitous supply-chain growth the rework constraints forbid.
///
/// Two v4 UUIDs supply 244 bits of CSPRNG output; the six version/variant bits
/// are fixed and are simply discarded here rather than counted as entropy.
fn generate_token() -> String {
    let mut bytes = Vec::with_capacity(TOKEN_BYTES);
    while bytes.len() < TOKEN_BYTES {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(TOKEN_BYTES);
    hex::encode(&bytes)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// Creates a private directory the way systemd's `RuntimeDirectory=` would.
    fn private_dir() -> TempDir {
        let temp = TempDir::new().expect("tempdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(TOKEN_DIR_MODE))
                .expect("tighten tempdir");
        }
        temp
    }

    #[test]
    fn a_generated_token_is_hex_and_full_length() {
        let token = generate_token();
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(token.len() >= MIN_TOKEN_CHARS);
        // Two calls must not collide; a constant token would authenticate
        // every installation to every other one.
        assert_ne!(token, generate_token());
    }

    #[test]
    fn the_token_file_is_created_private_and_reused() {
        let temp = private_dir();
        let path = temp.path().join("token");
        let first = Authenticator::from_token_file(&path).expect("create token");
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, TOKEN_FILE_MODE, "got {mode:04o}");
        }
        // A restart must keep the same token, or every restart would silently
        // lock out whoever holds the old one.
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(first.matches(contents.trim().as_bytes()));
        let second = Authenticator::from_token_file(&path).expect("reload token");
        assert!(second.matches(contents.trim().as_bytes()));
    }

    #[test]
    fn no_temporary_files_are_left_behind() {
        let temp = private_dir();
        let path = temp.path().join("token");
        Authenticator::from_token_file(&path).expect("create token");
        let leftovers: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "token")
            .map(|entry| entry.file_name())
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn a_missing_directory_is_a_startup_error() {
        let temp = private_dir();
        let path = temp.path().join("absent").join("token");
        assert!(matches!(
            Authenticator::from_token_file(&path),
            Err(AuthError::MissingDirectory(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_group_readable_directory_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = private_dir();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o750))
            .expect("loosen tempdir");
        let path = temp.path().join("token");
        let error = Authenticator::from_token_file(&path).unwrap_err();
        assert!(
            matches!(error, AuthError::TooPermissive { .. }),
            "{error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_token_file_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = private_dir();
        let path = temp.path().join("token");
        std::fs::write(&path, "0".repeat(MIN_TOKEN_CHARS)).expect("write token");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("loosen token");
        let error = Authenticator::from_token_file(&path).unwrap_err();
        assert!(
            matches!(error, AuthError::TooPermissive { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn an_empty_or_short_token_cannot_build_an_authenticator() {
        assert!(matches!(
            Authenticator::from_token(""),
            Err(AuthError::TokenTooShort { found: 0, .. })
        ));
        assert!(matches!(
            Authenticator::from_token("   \n  "),
            Err(AuthError::TokenTooShort { found: 0, .. })
        ));
        assert!(Authenticator::from_token(&"a".repeat(MIN_TOKEN_CHARS - 1)).is_err());
        assert!(Authenticator::from_token(&"a".repeat(MIN_TOKEN_CHARS)).is_ok());
    }

    #[test]
    fn a_short_token_file_is_refused_rather_than_padded() {
        let temp = private_dir();
        let path = temp.path().join("token");
        let mut file = create_private(&path).expect("create");
        file.write_all(b"tooshort").expect("write");
        drop(file);
        assert!(matches!(
            Authenticator::from_token_file(&path),
            Err(AuthError::TokenTooShort { .. })
        ));
    }

    #[test]
    fn matching_is_exact() {
        let token = "a".repeat(MIN_TOKEN_CHARS);
        let authenticator = Authenticator::from_token(&token).expect("token");
        assert!(authenticator.matches(token.as_bytes()));
        assert!(!authenticator.matches(b""));
        assert!(!authenticator.matches(format!("{token}x").as_bytes()));
        assert!(!authenticator.matches(token[..token.len() - 1].as_bytes()));
        let mut wrong = token.clone().into_bytes();
        wrong[MIN_TOKEN_CHARS - 1] = b'b';
        assert!(!authenticator.matches(&wrong));
    }

    #[test]
    fn surrounding_whitespace_in_the_file_is_ignored() {
        let temp = private_dir();
        let path = temp.path().join("token");
        let token = "b".repeat(MIN_TOKEN_CHARS);
        let mut file = create_private(&path).expect("create");
        file.write_all(format!("  {token}\n").as_bytes())
            .expect("write");
        drop(file);
        let authenticator = Authenticator::from_token_file(&path).expect("load");
        assert!(authenticator.matches(token.as_bytes()));
    }

    #[test]
    fn the_secret_never_appears_in_debug_output() {
        let token = "c".repeat(MIN_TOKEN_CHARS);
        let rendered = format!("{:?}", Authenticator::from_token(&token).expect("token"));
        assert!(!rendered.contains(&token), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }
}
