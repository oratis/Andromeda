use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for a capability grant request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityId(Uuid);

impl CapabilityId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CapabilityId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for CapabilityId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAccess {
    Read,
    Write,
    ReadWrite,
}

impl FileAccess {
    #[must_use]
    pub const fn permits(self, requested: Self) -> bool {
        matches!(
            (self, requested),
            (Self::ReadWrite, _) | (Self::Read, Self::Read) | (Self::Write, Self::Write)
        )
    }
}

/// Resource scope. Secret values are intentionally never stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityResource {
    /// Grants file access under `root` (after lexical normalization).
    Files { root: PathBuf, access: FileAccess },
    /// Grants network access to exactly `host` (no subdomains).
    ///
    /// The host is compared case-insensitively after normalization
    /// (lowercased, trailing dot stripped). When `port` is `Some`, the
    /// capability only covers action targets that carry an explicit matching
    /// `:port` suffix; when `None`, any port is covered.
    Network { host: String, port: Option<u16> },
    /// Grants a change to one named system setting.
    SystemSetting { key: String },
    /// Grants one operation on one external service.
    ///
    /// `service` must not contain `':'` — action targets use the
    /// `service:operation` format and are split at the first `':'`, so a
    /// colon inside the service name would make matching ambiguous. The
    /// policy engine refuses to match such capabilities.
    ExternalService { service: String, operation: String },
}

/// A scoped permission proposed by a plan and granted by host policy or a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub id: CapabilityId,
    pub resource: CapabilityResource,
    pub issued_to: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Reserved; not yet enforced (no executor exists). Single-use
    /// consumption — invalidating the capability after its first successful
    /// execution — will belong to the runtime execution layer once it exists;
    /// there is no such layer today, so nothing consumes or invalidates this
    /// flag. The policy engine treats a `single_use` capability like any other
    /// grant.
    pub single_use: bool,
}

impl Capability {
    /// Whether the capability is valid at `now`: it must already have been
    /// issued (`now >= issued_at`) and must not have expired.
    #[must_use]
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        now >= self.issued_at && self.expires_at.is_none_or(|expires_at| now < expires_at)
    }

    #[must_use]
    pub fn permits_file(&self, path: &Path, access: FileAccess) -> bool {
        let CapabilityResource::Files {
            root,
            access: granted,
        } = &self.resource
        else {
            return false;
        };

        granted.permits(access)
            && normalized_absolute(path).is_some_and(|candidate| {
                normalized_absolute(root).is_some_and(|root| candidate.starts_with(root))
            })
    }
}

/// Lexically normalizes an absolute path, resolving `.` and `..` components.
///
/// Returns `None` when the path is relative or when a `..` component would
/// climb past the root. Callers making security decisions must treat `None`
/// as a denial.
///
/// This is a purely lexical operation: symlinks are not resolved, so
/// `/workspace/link/../x` may normalize differently from what the kernel
/// would resolve. Executors must re-validate with `realpath`/`openat`
/// semantics before touching the filesystem.
#[must_use]
pub fn normalized_absolute(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Some(normalized)
}

/// Isolation supplied by the executor.
///
/// The variants do not form a single linear strength hierarchy: `Brokered`
/// means external side effects are proxied through the host broker, which is
/// a *channel* property, not a memory/execution isolation guarantee. Policy
/// decisions must therefore use [`IsolationLevel::satisfies`] rather than the
/// derived comparison operators. `Ord`/`PartialOrd` are kept only for stable
/// sorting and carry no security meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    None,
    Sandbox,
    MicroVm,
    Brokered,
}

impl IsolationLevel {
    /// Whether this (provided) isolation level meets a `required` level.
    ///
    /// Semantic matrix:
    ///
    /// - `None` is satisfied by every level.
    /// - `Sandbox` is satisfied by `Sandbox` and `MicroVm`.
    /// - `MicroVm` is satisfied only by `MicroVm`. `Brokered` does **not**
    ///   satisfy `MicroVm` or `Sandbox`: a brokered channel provides host
    ///   mediation for side effects, not strong memory/execution isolation.
    /// - `Brokered` is satisfied only by `Brokered`: neither a sandbox nor a
    ///   microVM provides the host-mediated side-effect channel.
    #[must_use]
    pub const fn satisfies(self, required: Self) -> bool {
        matches!(
            (self, required),
            (_, Self::None)
                | (Self::Sandbox | Self::MicroVm, Self::Sandbox)
                | (Self::MicroVm, Self::MicroVm)
                | (Self::Brokered, Self::Brokered)
        )
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn file_scope_rejects_sibling_prefixes() {
        let (root, child, sibling, _, _) = test_paths();
        let capability = file_capability(root, FileAccess::Read);
        assert!(capability.permits_file(Path::new(child), FileAccess::Read));
        assert!(!capability.permits_file(Path::new(sibling), FileAccess::Read));
    }

    #[test]
    fn file_scope_normalizes_parent_components() {
        let (root, _, _, traversal, _) = test_paths();
        let capability = file_capability(root, FileAccess::Read);
        assert!(!capability.permits_file(Path::new(traversal), FileAccess::Read));
    }

    #[test]
    fn write_grant_does_not_imply_read() {
        let (root, _, _, _, file) = test_paths();
        let capability = file_capability(root, FileAccess::Write);
        assert!(!capability.permits_file(Path::new(file), FileAccess::Read));
    }

    #[test]
    fn future_issued_capability_is_not_active() {
        let (root, _, _, _, _) = test_paths();
        let mut capability = file_capability(root, FileAccess::Read);
        let now = Utc::now();
        capability.issued_at = now + chrono::Duration::hours(1);
        assert!(!capability.is_active_at(now));
        assert!(capability.is_active_at(now + chrono::Duration::hours(2)));
    }

    #[test]
    fn expired_capability_is_not_active() {
        let (root, _, _, _, _) = test_paths();
        let mut capability = file_capability(root, FileAccess::Read);
        let now = Utc::now();
        capability.issued_at = now - chrono::Duration::hours(2);
        capability.expires_at = Some(now - chrono::Duration::hours(1));
        assert!(!capability.is_active_at(now));
    }

    #[test]
    fn brokered_does_not_satisfy_strong_isolation() {
        assert!(!IsolationLevel::Brokered.satisfies(IsolationLevel::MicroVm));
        assert!(!IsolationLevel::Brokered.satisfies(IsolationLevel::Sandbox));
        assert!(IsolationLevel::Brokered.satisfies(IsolationLevel::Brokered));
        assert!(IsolationLevel::Brokered.satisfies(IsolationLevel::None));
    }

    #[test]
    fn satisfies_matrix_is_exact() {
        use IsolationLevel::{Brokered, MicroVm, None, Sandbox};
        let expectations = [
            (None, None, true),
            (None, Sandbox, false),
            (None, MicroVm, false),
            (None, Brokered, false),
            (Sandbox, None, true),
            (Sandbox, Sandbox, true),
            (Sandbox, MicroVm, false),
            (Sandbox, Brokered, false),
            (MicroVm, None, true),
            (MicroVm, Sandbox, true),
            (MicroVm, MicroVm, true),
            (MicroVm, Brokered, false),
            (Brokered, None, true),
            (Brokered, Sandbox, false),
            (Brokered, MicroVm, false),
            (Brokered, Brokered, true),
        ];
        for (provided, required, expected) in expectations {
            assert_eq!(
                provided.satisfies(required),
                expected,
                "{provided:?}.satisfies({required:?})"
            );
        }
    }

    /// Locks the declaration order so that inserting a variant (which would
    /// silently change the derived `Ord` used for sorting) fails loudly.
    #[test]
    fn isolation_level_declaration_order_is_locked() {
        assert_eq!(IsolationLevel::None as u8, 0);
        assert_eq!(IsolationLevel::Sandbox as u8, 1);
        assert_eq!(IsolationLevel::MicroVm as u8, 2);
        assert_eq!(IsolationLevel::Brokered as u8, 3);
    }

    #[test]
    fn capability_round_trips_as_json() {
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Network {
                host: "api.example.com".into(),
                port: Some(443),
            },
            issued_to: "task".into(),
            issued_at: Utc::now(),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
            single_use: true,
        };
        let encoded = serde_json::to_string(&capability).expect("serialize capability");
        let decoded: Capability = serde_json::from_str(&encoded).expect("deserialize capability");
        assert_eq!(decoded, capability);
    }

    #[test]
    fn capability_rejects_unknown_fields() {
        // A camelCase typo of `expires_at` must now be rejected outright
        // instead of being silently dropped (which would leave `expires_at`
        // as `None`, i.e. a never-expiring grant — a fail-open widening).
        let capability = file_capability("/work/project", FileAccess::Read);
        let mut value = serde_json::to_value(&capability).expect("serialize capability");
        value.as_object_mut().expect("object").insert(
            "expiresAt".into(),
            serde_json::json!("2099-01-01T00:00:00Z"),
        );
        let decoded: Result<Capability, _> = serde_json::from_value(value);
        assert!(
            decoded.is_err(),
            "unknown field expiresAt must be rejected, got {decoded:?}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    const fn test_paths() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "/work/project",
            "/work/project/src/lib.rs",
            "/work/project-evil/key",
            "/work/project/../secrets/token",
            "/work/project/file",
        )
    }

    #[cfg(target_os = "windows")]
    const fn test_paths() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            r"C:\work\project",
            r"C:\work\project\src\lib.rs",
            r"C:\work\project-evil\key",
            r"C:\work\project\..\secrets\token",
            r"C:\work\project\file",
        )
    }
}
