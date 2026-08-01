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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CapabilityResource {
    Files { root: PathBuf, access: FileAccess },
    Network { host: String, port: Option<u16> },
    SystemSetting { key: String },
    ExternalService { service: String, operation: String },
}

/// A scoped permission proposed by a plan and granted by host policy or a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    pub resource: CapabilityResource,
    pub issued_to: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub single_use: bool,
}

impl Capability {
    #[must_use]
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_none_or(|expires_at| now < expires_at)
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

fn normalized_absolute(path: &Path) -> Option<PathBuf> {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    None,
    Sandbox,
    MicroVm,
    Brokered,
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
        let capability = file_capability("/work/project", FileAccess::Read);
        assert!(capability.permits_file(Path::new("/work/project/src/lib.rs"), FileAccess::Read));
        assert!(!capability.permits_file(Path::new("/work/project-evil/key"), FileAccess::Read));
    }

    #[test]
    fn file_scope_normalizes_parent_components() {
        let capability = file_capability("/work/project", FileAccess::Read);
        assert!(!capability.permits_file(
            Path::new("/work/project/../secrets/token"),
            FileAccess::Read
        ));
    }

    #[test]
    fn write_grant_does_not_imply_read() {
        let capability = file_capability("/work/project", FileAccess::Write);
        assert!(!capability.permits_file(Path::new("/work/project/file"), FileAccess::Read));
    }
}
