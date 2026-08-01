use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use andromeda_core::{
    ActionId, ActionKind, ActionPlan, ActionSpec, Capability, CapabilityId, CapabilityResource,
    FileAccess, Intent, IsolationLevel, RecoverySemantics, RiskLevel, TaskId, TaskState,
};
use andromeda_hardware::{
    HcmManifest, SupportTier, diagnose_report, evaluate_manifest, probe_host,
};
use andromeda_policy::PolicyEngine;
use andromeda_runtime::{CreateTaskRequest, FileTaskStore, StateTransitionRequest, TaskService};
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
        /// Isolation level to evaluate against. Defaults to `sandbox`
        /// because the inspection plans this CLI creates carry sandboxed
        /// risk; evaluating with `none` always denies them.
        #[arg(long, value_enum, default_value = "sandbox")]
        isolation: CliIsolation,
        #[arg(long)]
        confirm_external: bool,
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
    },
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
    Check {
        manifest: PathBuf,
        /// Fail (exit code 3) unless the effective tier is at least this
        /// tier on the ladder blocked < community < reference < supported <
        /// certified.
        #[arg(long, value_enum)]
        require_tier: Option<CliSupportTier>,
    },
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
        } => {
            let manifest = load_manifest(&manifest)?;
            let evaluation = evaluate_manifest(&report, &manifest);
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
            print_json(&service.evaluate(
                TaskId::from_str(&task_id)?,
                isolation.into(),
                confirm_external,
            )?)?;
        }
        TaskCommand::Transition {
            task_id,
            to,
            expected_revision,
            actor,
        } => {
            print_json(&service.transition(
                TaskId::from_str(&task_id)?,
                StateTransitionRequest {
                    to: to.into(),
                    actor,
                    expected_revision,
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
