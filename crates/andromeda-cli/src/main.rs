use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use andromeda_core::{
    ActionId, ActionKind, ActionPlan, ActionSpec, Capability, CapabilityId, CapabilityResource,
    FileAccess, Intent, IsolationLevel, RecoverySemantics, RiskLevel, TaskId, TaskState,
};
use andromeda_hardware::{HcmManifest, evaluate_manifest, probe_host};
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
        #[arg(long, value_enum, default_value = "none")]
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
    /// Probe this host and evaluate one HCM JSON document.
    Check { manifest: PathBuf },
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
        HardwareCommand::Check { manifest } => {
            let manifest: HcmManifest = serde_json::from_reader(std::fs::File::open(manifest)?)?;
            print_json(&evaluate_manifest(&report, &manifest))?;
        }
    }
    Ok(())
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
