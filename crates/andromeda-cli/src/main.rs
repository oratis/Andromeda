use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use andromeda_core::{
    ActionId, ActionKind, ActionOutcome, ActionPlan, ActionSpec, Capability, CapabilityId,
    CapabilityResource, Evidence, FileAccess, Intent, IsolationLevel, OutcomeStatus,
    RecoverySemantics, RiskLevel, TaskId, TaskState,
};
use andromeda_hardware::{
    ArtifactVerifier, DirectoryArtifactVerifier, HcmManifest, SupportTier, TrustedKeyring,
    diagnose_report, evaluate_manifest_verified, evaluate_manifest_with_verifier, probe_host,
};
use andromeda_policy::PolicyEngine;
use andromeda_runtime::{
    CreateTaskRequest, EvaluationRequest, FileTaskStore, RecordOutcomeRequest,
    StateTransitionRequest, TaskService,
};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "andromeda", about = "Andromeda developer control plane")]
struct Cli {
    #[arg(
        long,
        env = "ANDROMEDA_STATE_DIR",
        default_value = ".andromeda/state",
        global = true
    )]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create, inspect, and evaluate durable tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Probe the current machine and evaluate Hardware Compatibility Manifests.
    Hardware {
        #[command(subcommand)]
        command: HardwareCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Create a read-only directory inspection plan and explicitly grant its scope.
    CreateInspection {
        path: PathBuf,
        #[arg(long, default_value = "local-user")]
        requested_by: String,
    },
    /// List all durable task records.
    List,
    /// Show one task record.
    Show { task_id: String },
    /// Evaluate policy without executing any action.
    Evaluate {
        task_id: String,
        /// Isolation level to evaluate against. When omitted, each action is
        /// evaluated at its own declared-risk minimum isolation (the same
        /// per-action model used at task creation). When set, this overrides
        /// the isolation for *every* action, which is mainly useful for
        /// probing a whole plan under one level.
        #[arg(long, value_enum)]
        isolation: Option<CliIsolation>,
        #[arg(long)]
        confirm_external: bool,
    },
    /// Record one action's execution outcome, the evidence a task needs to
    /// reach `succeeded`.
    RecordOutcome {
        task_id: String,
        #[arg(long)]
        action_id: String,
        #[arg(long, value_enum, default_value = "succeeded")]
        status: CliOutcomeStatus,
        /// Human-readable evidence summary. Repeat for multiple items; a
        /// `Verifying -> Succeeded` transition requires at least one.
        #[arg(long = "evidence")]
        evidence: Vec<String>,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, default_value = "local-user")]
        actor: String,
    },
    /// Apply a checked state transition.
    Transition {
        task_id: String,
        #[arg(long, value_enum)]
        to: CliTaskState,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, default_value = "local-user")]
        actor: String,
        /// Assert that the final human confirmation for L3 external side
        /// effects was obtained. Required for `Ready -> Running` on any plan
        /// containing external side effects; without it the transition is
        /// rejected rather than silently treated as confirmed.
        #[arg(long)]
        confirm_external: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliOutcomeStatus {
    Succeeded,
    Failed,
    Skipped,
    RolledBack,
    Compensated,
}

impl From<CliOutcomeStatus> for OutcomeStatus {
    fn from(value: CliOutcomeStatus) -> Self {
        match value {
            CliOutcomeStatus::Succeeded => Self::Succeeded,
            CliOutcomeStatus::Failed => Self::Failed,
            CliOutcomeStatus::Skipped => Self::Skipped,
            CliOutcomeStatus::RolledBack => Self::RolledBack,
            CliOutcomeStatus::Compensated => Self::Compensated,
        }
    }
}

#[derive(Debug, Subcommand)]
enum HardwareCommand {
    /// Print a privacy-conscious hardware report.
    Probe,
    /// Diagnose driver binding and support-relevant device readiness.
    Diagnose,
    /// Probe this host and evaluate one HCM JSON document.
    ///
    /// Exit codes: 0 when the effective tier is usable, 2 when it is
    /// `blocked`, 3 when it is below the tier given via `--require-tier`.
    ///
    /// Authenticity is off by default and must be opted into with
    /// `--trusted-keys`. Without it the manifest's own claims — including its
    /// declared tier — are taken on faith, so the result proves internal
    /// consistency and freshness only and is not a trust gate. Asking for
    /// `--require-tier supported` or `certified` without a keyring is refused
    /// outright unless `--allow-unverified` is passed.
    Check {
        manifest: PathBuf,
        /// Fail (exit code 3) unless the effective tier is at least this
        /// tier on the ladder blocked < community < reference < supported <
        /// certified.
        #[arg(long, value_enum)]
        require_tier: Option<CliSupportTier>,
        /// JSON file mapping trusted `key_id` to an ed25519 verifying key in
        /// hex, e.g. `{"andromeda-hcm-root-2026": "<64 hex chars>"}`. Enables
        /// fail-closed authenticity: the manifest must carry a detached
        /// signature that resolves to one of these keys and verifies over its
        /// canonical bytes, or the effective tier is driven to `blocked`.
        #[arg(long)]
        trusted_keys: Option<PathBuf>,
        /// Directory holding the pinned artifacts. When given, each pin is
        /// resolved to `<root>/<name>` and its SHA-256 is recomputed and
        /// compared; a mismatched or missing artifact blocks the tier.
        #[arg(long)]
        artifact_root: Option<PathBuf>,
        /// Trusted `signing_key_id` for an *artifact pin*; repeatable. When any
        /// is given, a pin naming no key or an untrusted key is rejected.
        /// Distinct from `--trusted-keys`, which authenticates the manifest
        /// itself. Requires `--artifact-root`.
        #[arg(long = "artifact-signing-key", requires = "artifact_root")]
        artifact_signing_keys: Vec<String>,
        /// Acknowledge that an unverified evaluation is being used to gate a
        /// high tier. Required to combine `--require-tier supported|certified`
        /// with no `--trusted-keys`.
        #[arg(long)]
        allow_unverified: bool,
    },
}

/// Tiers whose evaluation must not be gated on an unauthenticated manifest.
///
/// `blocked` and `community` carry no support promise, and `reference` means
/// virtual-only evidence, so a self-asserted manifest claiming them misleads
/// nobody. `supported` and `certified` are real promises about real hardware:
/// letting a manifest assert one for itself is the whole forgery problem, so
/// gating on them requires either a keyring or an explicit acknowledgement.
fn requires_authenticity(tier: SupportTier) -> bool {
    matches!(tier, SupportTier::Supported | SupportTier::Certified)
}

/// Loads a `{ "key_id": "<hex>" }` keyring file.
fn load_trusted_keys(path: &Path) -> Result<TrustedKeyring, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read trusted keys {}: {error}", path.display()))?;
    let entries: BTreeMap<String, String> = serde_json::from_str(&text).map_err(|error| {
        format!(
            "trusted keys {} must be a JSON object of key_id -> hex verifying key: {error}",
            path.display()
        )
    })?;
    if entries.is_empty() {
        return Err(format!(
            "trusted keys {} is empty; an empty keyring trusts nothing and would block every \
             manifest",
            path.display()
        ));
    }
    TrustedKeyring::from_hex_entries(entries)
        .map_err(|error| format!("invalid key in {}: {error}", path.display()))
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSupportTier {
    Blocked,
    Community,
    Reference,
    Supported,
    Certified,
}

impl From<CliSupportTier> for SupportTier {
    fn from(value: CliSupportTier) -> Self {
        match value {
            CliSupportTier::Blocked => Self::Blocked,
            CliSupportTier::Community => Self::Community,
            CliSupportTier::Reference => Self::Reference,
            CliSupportTier::Supported => Self::Supported,
            CliSupportTier::Certified => Self::Certified,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliIsolation {
    None,
    Sandbox,
    MicroVm,
    Brokered,
}

impl From<CliIsolation> for IsolationLevel {
    fn from(value: CliIsolation) -> Self {
        match value {
            CliIsolation::None => Self::None,
            CliIsolation::Sandbox => Self::Sandbox,
            CliIsolation::MicroVm => Self::MicroVm,
            CliIsolation::Brokered => Self::Brokered,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliTaskState {
    AwaitingApproval,
    Ready,
    Running,
    Verifying,
    Succeeded,
    Failed,
    Cancelling,
    Cancelled,
    Compensating,
    Compensated,
}

impl From<CliTaskState> for TaskState {
    fn from(value: CliTaskState) -> Self {
        match value {
            CliTaskState::AwaitingApproval => Self::AwaitingApproval,
            CliTaskState::Ready => Self::Ready,
            CliTaskState::Running => Self::Running,
            CliTaskState::Verifying => Self::Verifying,
            CliTaskState::Succeeded => Self::Succeeded,
            CliTaskState::Failed => Self::Failed,
            CliTaskState::Cancelling => Self::Cancelling,
            CliTaskState::Cancelled => Self::Cancelled,
            CliTaskState::Compensating => Self::Compensating,
            CliTaskState::Compensated => Self::Compensated,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Task { command } => {
            let service =
                TaskService::new(FileTaskStore::open(cli.state_dir)?, PolicyEngine::default());
            handle_task(&service, command)?;
        }
        Command::Hardware { command } => handle_hardware(command)?,
    }
    Ok(())
}

fn handle_hardware(command: HardwareCommand) -> Result<(), Box<dyn std::error::Error>> {
    let report = probe_host()?;
    match command {
        HardwareCommand::Probe => print_json(&report)?,
        HardwareCommand::Diagnose => print_json(&diagnose_report(&report))?,
        HardwareCommand::Check {
            manifest,
            require_tier,
            trusted_keys,
            artifact_root,
            artifact_signing_keys,
            allow_unverified,
        } => {
            let required = require_tier.map(SupportTier::from);
            // Refuse the dangerous combination outright rather than printing a
            // warning nobody reads: gating a real support promise on a manifest
            // that was never authenticated is the forgery problem itself.
            if trusted_keys.is_none()
                && !allow_unverified
                && required.is_some_and(requires_authenticity)
            {
                return Err(format!(
                    "--require-tier {} gates a real support promise, so it requires \
                     --trusted-keys to authenticate the manifest. Without a keyring the manifest \
                     asserts its own tier and any file can claim it. Pass --trusted-keys <file>, \
                     or --allow-unverified to acknowledge an advisory-only check.",
                    required.map_or("", |tier| match tier {
                        SupportTier::Supported => "supported",
                        SupportTier::Certified => "certified",
                        _ => "",
                    })
                )
                .into());
            }

            let manifest = load_manifest(&manifest)?;
            let verifier = artifact_root.map(|root| {
                if artifact_signing_keys.is_empty() {
                    DirectoryArtifactVerifier::new(root)
                } else {
                    DirectoryArtifactVerifier::with_trusted_keys(root, artifact_signing_keys)
                }
            });
            let verifier_ref = verifier.as_ref().map(|v| v as &dyn ArtifactVerifier);

            let evaluation = if let Some(path) = trusted_keys.as_deref() {
                let keyring = load_trusted_keys(path)?;
                evaluate_manifest_verified(&report, &manifest, &keyring, verifier_ref)
            } else {
                eprintln!(
                    "warning: --trusted-keys was not given, so the manifest's signature was not \
                     checked and its declared tier is self-asserted; this result attests \
                     consistency and freshness only and is not a trust gate"
                );
                if verifier_ref.is_none() {
                    eprintln!(
                        "warning: --artifact-root was not given either, so pinned artifact \
                         digests were not verified"
                    );
                }
                evaluate_manifest_with_verifier(&report, &manifest, verifier_ref)
            };
            print_json(&evaluation)?;
            if evaluation.effective_tier == SupportTier::Blocked {
                std::process::exit(2);
            }
            if let Some(required) = require_tier {
                if evaluation.effective_tier < SupportTier::from(required) {
                    std::process::exit(3);
                }
            }
        }
    }
    Ok(())
}

fn load_manifest(path: &Path) -> Result<HcmManifest, Box<dyn std::error::Error>> {
    let value: serde_json::Value = serde_json::from_reader(std::fs::File::open(path)?)?;
    manifest_from_value(value).map_err(Into::into)
}

/// Rejects unknown manifest schema versions before evaluation instead of
/// silently accepting whatever deserializes.
fn manifest_from_value(value: serde_json::Value) -> Result<HcmManifest, String> {
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) if version == u64::from(HcmManifest::CURRENT_SCHEMA_VERSION) => {
            serde_json::from_value(value).map_err(|error| format!("invalid HCM manifest: {error}"))
        }
        Some(version) => Err(format!(
            "unsupported HCM schema_version {version}; this build evaluates version {}",
            HcmManifest::CURRENT_SCHEMA_VERSION
        )),
        None => Err("HCM manifest is missing a numeric schema_version field".into()),
    }
}

fn handle_task(
    service: &TaskService,
    command: TaskCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        TaskCommand::CreateInspection { path, requested_by } => {
            let request = create_inspection_request(&path, &requested_by)?;
            print_json(&service.create(request)?)?;
        }
        TaskCommand::List => print_json(&service.list()?)?,
        TaskCommand::Show { task_id } => {
            print_json(&service.get(TaskId::from_str(&task_id)?)?)?;
        }
        TaskCommand::Evaluate {
            task_id,
            isolation,
            confirm_external,
        } => {
            let request = EvaluationRequest {
                isolation: isolation.map(Into::into),
                overrides: BTreeMap::new(),
                external_side_effect_confirmed: confirm_external,
                subject: None,
            };
            print_json(&service.evaluate(TaskId::from_str(&task_id)?, &request)?)?;
        }
        TaskCommand::RecordOutcome {
            task_id,
            action_id,
            status,
            evidence,
            expected_revision,
            actor,
        } => {
            let now = Utc::now();
            let outcome = ActionOutcome {
                action_id: ActionId::from_str(&action_id)?,
                status: status.into(),
                started_at: now,
                finished_at: now,
                evidence: evidence
                    .into_iter()
                    .map(|summary| Evidence {
                        kind: "operator-assertion".into(),
                        summary,
                        attributes: BTreeMap::new(),
                    })
                    .collect(),
                error: None,
            };
            print_json(&service.record_outcome(
                TaskId::from_str(&task_id)?,
                RecordOutcomeRequest {
                    outcome,
                    actor,
                    expected_revision,
                },
            )?)?;
        }
        TaskCommand::Transition {
            task_id,
            to,
            expected_revision,
            actor,
            confirm_external,
        } => {
            print_json(&service.transition(
                TaskId::from_str(&task_id)?,
                StateTransitionRequest {
                    to: to.into(),
                    actor,
                    expected_revision,
                    external_side_effect_confirmed: confirm_external,
                },
            )?)?;
        }
    }
    Ok(())
}

fn create_inspection_request(
    path: &Path,
    requested_by: &str,
) -> Result<CreateTaskRequest, std::io::Error> {
    let path = path.canonicalize()?;
    let task_id = TaskId::new();
    let capability = Capability {
        id: CapabilityId::new(),
        resource: CapabilityResource::Files {
            root: path.clone(),
            access: FileAccess::Read,
        },
        issued_to: task_id.to_string(),
        issued_at: Utc::now(),
        expires_at: None,
        single_use: false,
    };
    let plan = ActionPlan {
        schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
        task_id,
        intent: Intent::new(format!("Inspect {}", path.display()), requested_by),
        actions: vec![ActionSpec {
            id: ActionId::new(),
            name: "Inspect directory metadata".into(),
            kind: ActionKind::Inspect,
            target: path.display().to_string(),
            arguments: BTreeMap::new(),
            depends_on: Vec::new(),
            required_capabilities: vec![capability.id],
            risk: RiskLevel::L1Sandboxed,
            recovery: RecoverySemantics::None,
        }],
    };
    Ok(CreateTaskRequest {
        plan,
        capabilities: vec![capability],
        actor: requested_by.into(),
    })
}

fn print_json(value: &impl serde::Serialize) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn manifest_value(schema_version: &serde_json::Value) -> serde_json::Value {
        json!({
            "schema_version": schema_version,
            "id": "test",
            "name": "Test",
            "tier": "community",
            "boot_provider": "pc_uefi_shim",
            "selectors": [{"os_family": "linux"}]
        })
    }

    #[test]
    fn current_schema_version_is_accepted() {
        let manifest =
            manifest_from_value(manifest_value(&json!(HcmManifest::CURRENT_SCHEMA_VERSION)))
                .expect("current version parses");
        assert_eq!(manifest.id, "test");
    }

    #[test]
    fn only_real_support_promises_demand_authenticity() {
        // blocked/community/reference carry no promise about real hardware, so
        // a self-asserted manifest claiming them misleads nobody.
        assert!(!requires_authenticity(SupportTier::Blocked));
        assert!(!requires_authenticity(SupportTier::Community));
        assert!(!requires_authenticity(SupportTier::Reference));
        // supported/certified are the forgery target.
        assert!(requires_authenticity(SupportTier::Supported));
        assert!(requires_authenticity(SupportTier::Certified));
    }

    /// Writes `contents` to a uniquely named temp file and returns its path.
    fn temp_keys(tag: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "andromeda-cli-keys-{tag}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, contents).expect("write keys");
        path
    }

    #[test]
    fn an_empty_keyring_is_rejected_rather_than_silently_blocking_everything() {
        let path = temp_keys("empty", "{}");
        let error = load_trusted_keys(&path).expect_err("empty keyring must be rejected");
        let _ = std::fs::remove_file(&path);
        assert!(error.contains("empty"), "unhelpful error: {error}");
    }

    #[test]
    fn a_malformed_key_is_rejected_with_the_file_named() {
        let path = temp_keys("bad", r#"{"root": "not-hex"}"#);
        let error = load_trusted_keys(&path).expect_err("malformed key must be rejected");
        let _ = std::fs::remove_file(&path);
        assert!(
            error.contains("invalid key in") && error.contains("andromeda-cli-keys-bad"),
            "error should name the offending file: {error}"
        );
    }

    #[test]
    fn a_valid_keyring_loads() {
        // Derive a real verifying key so the test exercises actual ed25519
        // decoding rather than an arbitrary hex string.
        let key = andromeda_hardware::ManifestSigningKey::from_seed(&[7u8; 32]);
        let path = temp_keys(
            "ok",
            &format!(r#"{{"root": "{}"}}"#, key.verifying_key_hex()),
        );
        let keyring = load_trusted_keys(&path).expect("valid keyring loads");
        let _ = std::fs::remove_file(&path);
        assert!(keyring.contains("root"));
        assert_eq!(keyring.len(), 1);
    }

    #[test]
    fn missing_keyring_file_names_the_path() {
        let error = load_trusted_keys(Path::new("/nonexistent/andromeda-keys.json"))
            .expect_err("missing file must be rejected");
        assert!(error.contains("cannot read trusted keys"), "{error}");
    }

    #[test]
    fn unknown_schema_version_is_rejected_with_a_clear_error() {
        let error = manifest_from_value(manifest_value(&json!(999))).expect_err("must reject");
        assert!(
            error.contains("unsupported HCM schema_version 999"),
            "{error}"
        );
        assert!(
            error.contains(&HcmManifest::CURRENT_SCHEMA_VERSION.to_string()),
            "{error}"
        );
    }

    #[test]
    fn missing_schema_version_is_rejected() {
        let error =
            manifest_from_value(json!({"id": "test"})).expect_err("must reject missing version");
        assert!(error.contains("schema_version"), "{error}");
    }
}
