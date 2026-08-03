mod taskd;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use andromeda_core::{
    ActionId, ActionKind, ActionOutcome, ActionPlan, ActionSpec, Capability, CapabilityId,
    CapabilityResource, Evidence, FileAccess, Intent, IsolationLevel, OutcomeStatus,
    RecoverySemantics, RiskLevel, TaskId, TaskState,
};
use andromeda_hardware::{
    ArtifactVerifier, DirectoryArtifactVerifier, HcmManifest, ManifestSigningKey, SupportTier,
    TrustedKeyring, diagnose_report, evaluate_manifest_verified, evaluate_manifest_with_verifier,
    probe_host,
};
use andromeda_policy::PolicyEngine;
use andromeda_runtime::{
    CapabilityAdmission, CreateTaskRequest, EvaluationRequest, FileTaskStore, RecordOutcomeRequest,
    StateTransitionRequest, TaskService,
};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "andromeda", about = "Andromeda developer control plane")]
struct Cli {
    /// Read and drive the tasks of a running andromeda-taskd over its HTTP
    /// API, for example `http://127.0.0.1:7777`.
    ///
    /// This is the mode that matches an installed system: the daemon owns
    /// `/var/lib/andromeda-taskd/state` under a systemd `DynamicUser`, so its
    /// records are not merely elsewhere, they are unreadable to anyone else.
    /// Only loopback endpoints are accepted, because the request carries the
    /// local bearer token.
    #[arg(
        long,
        value_name = "URL",
        env = "ANDROMEDA_TASKD_URL",
        global = true,
        conflicts_with = "state_dir"
    )]
    connect: Option<String>,
    /// Open a task store directly, in process, with no daemon involved.
    ///
    /// Useful for development and for inspecting a store by hand. It is a
    /// *different* set of tasks from the daemon's unless the path happens to be
    /// the daemon's own state directory and this process can read it.
    #[arg(long, value_name = "PATH", env = "ANDROMEDA_STATE_DIR", global = true)]
    state_dir: Option<PathBuf>,
    /// File holding andromeda-taskd's local bearer token; only meaningful with
    /// `--connect`.
    ///
    /// Defaults to the first of `/run/andromeda-taskd/token` (what the shipped
    /// unit uses) and `.andromeda/taskd-token` (what a hand-started daemon
    /// uses) that exists. There is deliberately no flag or variable that takes
    /// the token *value*: `argv` is world-readable through `/proc`.
    #[arg(
        long,
        value_name = "PATH",
        env = "ANDROMEDA_AUTH_TOKEN_FILE",
        global = true
    )]
    auth_token_file: Option<PathBuf>,
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
    ///
    /// Connected mode lists what `GET /v1/tasks` returns: **summaries** with
    /// counts and no event bodies. Local mode prints whole records, because it
    /// reads them out of the store directly. The banner on stderr says which
    /// one you got.
    List,
    /// Show one task record.
    Show {
        task_id: String,
        /// How many of the most recent events to ask taskd for. Connected mode
        /// only: a local store read is unbounded because it is a library call,
        /// not an API response.
        ///
        /// Without it the daemon returns its own default window and the CLI
        /// reports how much history was left behind.
        ///
        /// Not a clap `requires = "connect"`: `--connect` is a global argument,
        /// and clap only satisfies `requires` when the required argument
        /// appears at the *same* level, so that would reject the natural
        /// `andromeda --connect <URL> task show <ID> --events <N>`. The local
        /// path refuses it instead.
        #[arg(long, value_name = "N")]
        events: Option<usize>,
    },
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
    /// Exit code 1 (an `Err` from `main`) means the check was refused
    /// outright — `--require-tier supported|certified` without
    /// `--trusted-keys` and without `--allow-unverified` — or that an input
    /// failed (unreadable manifest, unknown schema version, probe error).
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
        /// with no `--trusted-keys`. Contradicts `--trusted-keys` (the check
        /// is verified then) and means nothing without `--require-tier`, so
        /// both combinations are parse errors.
        #[arg(long, conflicts_with = "trusted_keys", requires = "require_tier")]
        allow_unverified: bool,
    },
    /// Derive the public half of a signing seed, for publishing into a keyring.
    ///
    /// Signing is deterministic from a 32-byte seed rather than an RNG, so an
    /// offline signer is reproducible. Generating and protecting that seed is a
    /// deployment concern this tool deliberately does not perform: it will not
    /// invent key material, only read a seed you already manage.
    Keygen {
        /// File holding the 32-byte seed, either 64 hex characters or 32 raw
        /// bytes. Keep it offline; anyone holding it can sign manifests.
        #[arg(long)]
        seed_file: PathBuf,
        /// Key id to print alongside the verifying key.
        #[arg(long, default_value = "andromeda-hcm-root")]
        key_id: String,
    },
    /// Sign an HCM manifest, emitting the manifest with its `signature` set.
    ///
    /// Canonicalization strips any existing signature, so re-signing an
    /// already-signed manifest is safe.
    Sign {
        manifest: PathBuf,
        /// File holding the 32-byte seed (64 hex characters or 32 raw bytes).
        #[arg(long)]
        seed_file: PathBuf,
        /// Key id recorded in the signature; must match the keyring entry that
        /// verifiers will use.
        #[arg(long, default_value = "andromeda-hcm-root")]
        key_id: String,
        /// Write the signed manifest here instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

/// Reads a 32-byte signing seed from `path`, accepting either 64 hex
/// characters (optionally whitespace-terminated) or 32 raw bytes.
///
/// Seeds are read, never generated: inventing key material in a developer CLI
/// would produce keys nobody can account for.
fn load_seed(path: &Path) -> Result<[u8; 32], String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read seed {}: {error}", path.display()))?;
    if let Ok(text) = std::str::from_utf8(&bytes) {
        let trimmed = text.trim();
        if trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut seed = [0u8; 32];
            for (index, slot) in seed.iter_mut().enumerate() {
                *slot = u8::from_str_radix(&trimmed[index * 2..index * 2 + 2], 16)
                    .map_err(|error| format!("invalid hex in seed: {error}"))?;
            }
            return Ok(seed);
        }
    }
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        format!(
            "seed {} must be 64 hex characters or exactly 32 raw bytes, found {} bytes",
            path.display(),
            bytes.len()
        )
    })
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

/// The `hardware check` refusal gate, the command's one new security control:
/// decides whether the requested tier gate must be refused because it would
/// rest on an unauthenticated manifest.
///
/// Returns the refusal message when `--require-tier` names a tier that
/// [`requires_authenticity`] while no keyring was supplied and the caller did
/// not acknowledge the risk with `--allow-unverified`; `None` means the check
/// may proceed. Pure so every arm is directly testable.
fn refuse_unverified_gate(
    required: Option<SupportTier>,
    has_keyring: bool,
    allow_unverified: bool,
) -> Option<String> {
    let tier = required?;
    if has_keyring || allow_unverified || !requires_authenticity(tier) {
        return None;
    }
    // Total, wildcard-free mapping: adding a `SupportTier` variant without
    // naming it here fails to compile instead of silently printing nothing.
    let tier_name = match tier {
        SupportTier::Blocked => "blocked",
        SupportTier::Community => "community",
        SupportTier::Reference => "reference",
        SupportTier::Supported => "supported",
        SupportTier::Certified => "certified",
    };
    Some(format!(
        "--require-tier {tier_name} gates a real support promise, so it requires \
         --trusted-keys to authenticate the manifest. Without a keyring the manifest \
         asserts its own tier and any file can claim it. Pass --trusted-keys <file>, \
         or --allow-unverified to acknowledge an advisory-only check."
    ))
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

/// Which set of tasks a `task` subcommand acts on, decided before anything is
/// opened or connected.
///
/// There is no third option and no default. Architecture review #5 records what
/// a default cost: the CLI opened `.andromeda/state` under the working
/// directory while the installed daemon kept its records in
/// `/var/lib/andromeda-taskd/state` under a `DynamicUser` at mode 0700, so
/// `andromeda task list` on a real machine printed an empty list that looked
/// exactly like "there are no tasks". Requiring the choice makes that
/// particular silence unreachable; naming the resolved target on every command
/// makes any remaining confusion visible.
#[derive(Debug, PartialEq, Eq)]
enum TaskTargetChoice {
    Local(PathBuf),
    Connected {
        url: String,
        token_file: Option<PathBuf>,
    },
}

/// Resolves the mode from the two flags, without touching the filesystem or the
/// network, so every arm is directly testable.
fn resolve_task_target(
    connect: Option<String>,
    state_dir: Option<PathBuf>,
    auth_token_file: Option<PathBuf>,
) -> Result<TaskTargetChoice, String> {
    match (connect, state_dir) {
        (Some(url), _) => Ok(TaskTargetChoice::Connected {
            url,
            token_file: auth_token_file,
        }),
        (None, Some(state_dir)) => {
            // Silently ignoring an input the caller went out of their way to
            // supply is how the two-store confusion started; say so instead.
            if let Some(path) = auth_token_file {
                return Err(format!(
                    "--auth-token-file {} only applies to --connect: a local store is opened \
                     directly and authenticates nobody. Drop it (or unset \
                     ANDROMEDA_AUTH_TOKEN_FILE), or pass --connect <URL> to talk to the daemon",
                    path.display()
                ));
            }
            Ok(TaskTargetChoice::Local(state_dir))
        }
        (None, None) => Err(format!(
            "choose which tasks to act on: --connect <URL> (ANDROMEDA_TASKD_URL) drives a \
             running andromeda-taskd, and --state-dir <PATH> (ANDROMEDA_STATE_DIR) opens a task \
             store in this process.\n\nThere is no default. An installed system keeps its tasks \
             in /var/lib/andromeda-taskd/state, owned by the daemon's DynamicUser and readable \
             by nobody else, so a defaulted local store answered `task list` with an empty list \
             that meant \"you are looking at the wrong store\", not \"there are no tasks\".\n\n\
             On an installed system:   andromeda --connect http://127.0.0.1:{DEFAULT_TASKD_PORT} \
             task list   (as root, for the token)\nFor local development:     andromeda \
             --state-dir .andromeda/state task list"
        )),
    }
}

/// Port used in the guidance above; `andromeda-taskd`'s own `--listen` default.
const DEFAULT_TASKD_PORT: u16 = 7777;

/// What the CLI acts on once the choice has been carried out.
enum TaskTarget {
    Local {
        service: TaskService,
        state_dir: PathBuf,
    },
    Connected(taskd::Client),
}

impl TaskTarget {
    fn open(choice: TaskTargetChoice) -> Result<Self, Box<dyn std::error::Error>> {
        match choice {
            TaskTargetChoice::Local(state_dir) => {
                let store = FileTaskStore::open(&state_dir)?;
                // The store is reported by its absolute path, because the whole
                // failure being fixed here is a relative path resolving
                // somewhere the reader did not expect.
                let state_dir = std::fs::canonicalize(&state_dir).unwrap_or(state_dir);
                Ok(Self::Local {
                    // A local store has no daemon, no network, and no caller to
                    // authenticate: whoever runs this can already write the
                    // state directory. Requiring issuer signatures here would
                    // protect nothing the filesystem does not already decide,
                    // so the permissive posture is named rather than pretending
                    // to a guarantee it cannot make.
                    service: TaskService::new(
                        store,
                        PolicyEngine::default(),
                        CapabilityAdmission::unsigned_for_development(),
                    ),
                    state_dir,
                })
            }
            TaskTargetChoice::Connected { url, token_file } => {
                let endpoint = taskd::Endpoint::parse(&url)?;
                Ok(Self::Connected(taskd::Client::connect(
                    endpoint,
                    token_file.as_deref(),
                )?))
            }
        }
    }
}

/// The one line printed to stderr before every `task` subcommand's output.
///
/// stderr, not stdout, so `andromeda task list | jq` still sees only JSON. An
/// empty result is now always accompanied by the identity of the thing that was
/// empty.
fn target_banner(target: &TaskTarget) -> String {
    match target {
        TaskTarget::Local { state_dir, .. } => format!(
            "reading the local task store at {} (in process; this is NOT andromeda-taskd's \
             store — pass --connect <URL> for the daemon's tasks)",
            state_dir.display()
        ),
        TaskTarget::Connected(client) => format!(
            "reading andromeda-taskd at {} (token {})",
            client.endpoint().url(),
            client.token_path().display()
        ),
    }
}

/// Runs the CLI, reporting a failure as a *readable* message.
///
/// `fn main() -> Result<_, _>` prints the error's `Debug`, so a message built
/// from a `String` arrives wrapped in quotes with its newlines escaped. The
/// errors here — above all the one that explains the two task stores — exist to
/// be read, so `main` prints `Display` and walks the `source()` chain, the same
/// way `andromeda-taskd`'s entry point does.
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Task { command } => {
            let choice = resolve_task_target(cli.connect, cli.state_dir, cli.auth_token_file)?;
            let target = TaskTarget::open(choice)?;
            eprintln!("andromeda task: {}", target_banner(&target));
            match &target {
                TaskTarget::Local { service, .. } => handle_task(service, command)?,
                TaskTarget::Connected(client) => handle_task_connected(client, command)?,
            }
        }
        Command::Hardware { command } => handle_hardware(command)?,
    }
    Ok(())
}

/// Signing operations describe a manifest, not this machine, and are meant to
/// run on an offline signer that may not even be the target platform. Probing
/// there would be pointless and could fail for unrelated reasons, so these are
/// handled before any probe happens.
fn handle_signing(command: HardwareCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        HardwareCommand::Keygen { seed_file, key_id } => {
            let key = ManifestSigningKey::from_seed(&load_seed(&seed_file)?);
            print_json(&serde_json::json!({
                "key_id": key_id,
                "verifying_key_hex": key.verifying_key_hex(),
                "keyring_entry": { key_id.clone(): key.verifying_key_hex() },
            }))?;
        }
        HardwareCommand::Sign {
            manifest,
            seed_file,
            key_id,
            output,
        } => {
            let key = ManifestSigningKey::from_seed(&load_seed(&seed_file)?);
            let mut manifest = load_manifest(&manifest)?;
            manifest.signature = Some(key.sign_manifest(&manifest, key_id)?);
            let json = serde_json::to_string_pretty(&manifest)?;
            match output {
                Some(path) => {
                    std::fs::write(&path, format!("{json}\n"))?;
                    eprintln!("signed manifest written to {}", path.display());
                }
                None => println!("{json}"),
            }
        }
        _ => unreachable!("handle_signing only accepts Keygen and Sign"),
    }
    Ok(())
}

fn handle_hardware(command: HardwareCommand) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(
        command,
        HardwareCommand::Keygen { .. } | HardwareCommand::Sign { .. }
    ) {
        return handle_signing(command);
    }

    let report = probe_host()?;
    match command {
        HardwareCommand::Keygen { .. } | HardwareCommand::Sign { .. } => unreachable!(),
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
            if let Some(refusal) =
                refuse_unverified_gate(required, trusted_keys.is_some(), allow_unverified)
            {
                return Err(refusal.into());
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
            if let Some(required) = required {
                if evaluation.effective_tier < required {
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
        TaskCommand::Show { task_id, events } => {
            // Refused rather than ignored: an `--events 5` that quietly
            // returned every event would tell the caller the opposite of the
            // truth about how much history they are looking at.
            if events.is_some() {
                return Err(
                    "--events applies to --connect only: it asks taskd for a window of \
                            its bounded event history, and a local store read returns the whole \
                            record, with no window to set"
                        .into(),
                );
            }
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
            print_json(&service.record_outcome(
                TaskId::from_str(&task_id)?,
                RecordOutcomeRequest {
                    outcome: operator_outcome(&action_id, status, evidence)?,
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

/// Runs one `task` subcommand against a running daemon.
///
/// Every arm posts or gets the same request types the local path builds, so the
/// two modes cannot drift into two different contracts: `taskd` deserializes
/// exactly these structs.
fn handle_task_connected(
    client: &taskd::Client,
    command: TaskCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        TaskCommand::CreateInspection { path, requested_by } => {
            let request = create_inspection_request(&path, &requested_by)?;
            print_json(&client.post("/v1/tasks", &request)?)?;
        }
        TaskCommand::List => {
            let listing = client.get("/v1/tasks")?;
            print_json(&listing)?;
            eprintln!(
                "andromeda task: taskd lists summaries only — no plan and no events. Read one \
                 task with `task show <TASK_ID>` for its history."
            );
        }
        TaskCommand::Show { task_id, events } => {
            let task_id = TaskId::from_str(&task_id)?;
            let path = match events {
                Some(events) => format!("/v1/tasks/{task_id}?events={events}"),
                None => format!("/v1/tasks/{task_id}"),
            };
            let record = client.get(&path)?;
            print_json(&record)?;
            if let Some(note) = truncation_note(&record) {
                eprintln!("andromeda task: {note}");
            }
        }
        TaskCommand::Evaluate {
            task_id,
            isolation,
            confirm_external,
        } => {
            let task_id = TaskId::from_str(&task_id)?;
            let request = EvaluationRequest {
                isolation: isolation.map(Into::into),
                overrides: BTreeMap::new(),
                external_side_effect_confirmed: confirm_external,
                subject: None,
            };
            print_json(&client.post(&format!("/v1/tasks/{task_id}/evaluate"), &request)?)?;
        }
        TaskCommand::RecordOutcome {
            task_id,
            action_id,
            status,
            evidence,
            expected_revision,
            actor,
        } => {
            let task_id = TaskId::from_str(&task_id)?;
            let request = RecordOutcomeRequest {
                outcome: operator_outcome(&action_id, status, evidence)?,
                actor,
                expected_revision,
            };
            print_json(&client.post(&format!("/v1/tasks/{task_id}/outcomes"), &request)?)?;
        }
        TaskCommand::Transition {
            task_id,
            to,
            expected_revision,
            actor,
            confirm_external,
        } => {
            let task_id = TaskId::from_str(&task_id)?;
            let request = StateTransitionRequest {
                to: to.into(),
                actor,
                expected_revision,
                external_side_effect_confirmed: confirm_external,
            };
            print_json(&client.post(&format!("/v1/tasks/{task_id}/transition"), &request)?)?;
        }
    }
    Ok(())
}

/// Says how much event history a task read did *not* return.
///
/// `GET /v1/tasks/{id}` answers with a bounded window plus `event_count`, the
/// true total. Printing the record alone would let a reader take the window for
/// the whole history, which is the same class of mistake as taking an empty
/// local store for an empty system. Returns `None` when nothing was withheld.
fn truncation_note(record: &serde_json::Value) -> Option<String> {
    let total = record.get("event_count")?.as_u64()?;
    let shown = record.get("events")?.as_array()?.len() as u64;
    if shown >= total {
        return None;
    }
    let ceiling = taskd::MAX_EVENTS as u64;
    if total > ceiling {
        return Some(format!(
            "showing {shown} of {total} events; taskd bounds a task read at {ceiling}, so the \
             oldest {} are not reachable through the API at all",
            total - ceiling
        ));
    }
    Some(format!(
        "showing {shown} of {total} events; pass --events {total} for the rest (taskd's ceiling \
         is {ceiling})"
    ))
}

/// Builds the operator-asserted outcome both modes record.
fn operator_outcome(
    action_id: &str,
    status: CliOutcomeStatus,
    evidence: Vec<String>,
) -> Result<ActionOutcome, Box<dyn std::error::Error>> {
    let now = Utc::now();
    Ok(ActionOutcome {
        action_id: ActionId::from_str(action_id)?,
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
    })
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
        signature: None,
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

    /// The two mode variables clap reads from the real process environment. A
    /// developer who exports one of them would otherwise see the parse-level
    /// tests below fail for a reason that has nothing to do with the code.
    fn mode_env_is_clean() -> bool {
        ["ANDROMEDA_TASKD_URL", "ANDROMEDA_STATE_DIR"]
            .iter()
            .all(|name| std::env::var_os(name).is_none())
    }

    /// The finding this whole change exists for: with neither mode named, the
    /// CLI must refuse rather than quietly open a store that is not the
    /// daemon's.
    #[test]
    fn neither_mode_is_refused_with_both_options_spelled_out() {
        let error = resolve_task_target(None, None, None)
            .expect_err("an unnamed task store is what caused the two-universe failure");
        for expected in [
            "--connect",
            "--state-dir",
            "ANDROMEDA_TASKD_URL",
            "ANDROMEDA_STATE_DIR",
            "/var/lib/andromeda-taskd/state",
            "There is no default",
        ] {
            assert!(error.contains(expected), "{expected} missing from: {error}");
        }
    }

    #[test]
    fn each_mode_resolves_to_itself() {
        assert_eq!(
            resolve_task_target(None, Some(PathBuf::from("/tmp/store")), None),
            Ok(TaskTargetChoice::Local(PathBuf::from("/tmp/store")))
        );
        assert_eq!(
            resolve_task_target(Some("http://127.0.0.1:7777".to_owned()), None, None),
            Ok(TaskTargetChoice::Connected {
                url: "http://127.0.0.1:7777".to_owned(),
                token_file: None,
            })
        );
        assert_eq!(
            resolve_task_target(
                Some("http://127.0.0.1:7777".to_owned()),
                None,
                Some(PathBuf::from("/run/andromeda-taskd/token")),
            ),
            Ok(TaskTargetChoice::Connected {
                url: "http://127.0.0.1:7777".to_owned(),
                token_file: Some(PathBuf::from("/run/andromeda-taskd/token")),
            })
        );
    }

    /// A token file cannot authenticate a local store, so supplying one is a
    /// misunderstanding worth reporting rather than dropping.
    #[test]
    fn a_token_file_without_connect_is_refused_rather_than_ignored() {
        let error = resolve_task_target(
            None,
            Some(PathBuf::from("/tmp/store")),
            Some(PathBuf::from("/run/andromeda-taskd/token")),
        )
        .expect_err("a token means nothing to a local store");
        assert!(error.contains("--auth-token-file"), "{error}");
        assert!(error.contains("--connect"), "{error}");
    }

    /// Both modes at once is a contradiction clap catches before anything runs.
    #[test]
    fn the_two_modes_conflict_at_parse_time() {
        let error = Cli::try_parse_from([
            "andromeda",
            "--connect",
            "http://127.0.0.1:7777",
            "--state-dir",
            ".andromeda/state",
            "task",
            "list",
        ])
        .expect_err("naming both stores must not parse");
        assert!(error.to_string().contains("cannot be used with"), "{error}");
    }

    /// `--events` describes a bound the API applies; a local store read has no
    /// such bound, so the local path refuses it instead of quietly returning
    /// every event under a flag that asked for five.
    #[test]
    fn events_is_refused_by_the_local_path() {
        let temp = tempfile::TempDir::new().expect("state dir");
        let service = TaskService::new(
            FileTaskStore::open(temp.path()).expect("store"),
            PolicyEngine::default(),
            CapabilityAdmission::unsigned_for_development(),
        );
        let error = handle_task(
            &service,
            TaskCommand::Show {
                task_id: "11111111-1111-4111-8111-111111111111".to_owned(),
                events: Some(5),
            },
        )
        .expect_err("--events must not be silently dropped");
        let message = error.to_string();
        assert!(
            message.contains("--events applies to --connect only"),
            "{message}"
        );

        // The same command without it reaches the store, which is the real
        // proof that the refusal is about the flag and nothing else.
        let error = handle_task(
            &service,
            TaskCommand::Show {
                task_id: "11111111-1111-4111-8111-111111111111".to_owned(),
                events: None,
            },
        )
        .expect_err("the task does not exist");
        assert!(!error.to_string().contains("--events"), "{error}");
    }

    /// Both orderings of the global mode flags parse, so the documented
    /// `andromeda --connect <URL> task ...` form keeps working alongside
    /// `andromeda task ... --connect <URL>`.
    #[test]
    fn the_mode_flags_are_accepted_at_either_level() {
        if !mode_env_is_clean() {
            return;
        }
        for arguments in [
            vec![
                "andromeda",
                "--connect",
                "http://127.0.0.1:7777",
                "task",
                "show",
                "11111111-1111-4111-8111-111111111111",
                "--events",
                "100",
            ],
            vec![
                "andromeda",
                "task",
                "show",
                "11111111-1111-4111-8111-111111111111",
                "--events",
                "100",
                "--connect",
                "http://127.0.0.1:7777",
            ],
        ] {
            let cli = Cli::try_parse_from(&arguments)
                .unwrap_or_else(|error| panic!("{arguments:?}: {error}"));
            assert_eq!(cli.connect.as_deref(), Some("http://127.0.0.1:7777"));
        }
    }

    /// Whichever mode runs, the banner names the exact thing that was read, so
    /// an empty answer can never be mistaken for "there are no tasks".
    #[test]
    fn the_banner_names_the_store_or_the_endpoint() {
        let local = TaskTarget::open(TaskTargetChoice::Local(
            std::env::temp_dir().join(format!("andromeda-cli-banner-{}", std::process::id())),
        ))
        .expect("open local store");
        let banner = target_banner(&local);
        assert!(banner.contains("local task store"), "{banner}");
        assert!(banner.contains("andromeda-cli-banner"), "{banner}");
        assert!(
            banner.contains("NOT andromeda-taskd's store"),
            "the local banner must deny being the daemon's store: {banner}"
        );
        if let TaskTarget::Local { state_dir, .. } = &local {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }

    /// A bounded task read must announce what it did not return.
    #[test]
    fn a_bounded_read_reports_the_history_it_did_not_return() {
        let full = json!({"event_count": 3, "events": [1, 2, 3]});
        assert_eq!(truncation_note(&full), None, "nothing was withheld");

        let bounded = json!({"event_count": 137, "events": [1, 2]});
        let note = truncation_note(&bounded).expect("truncation must be reported");
        assert!(note.contains("showing 2 of 137 events"), "{note}");
        assert!(note.contains("--events 137"), "{note}");

        // Beyond the daemon's ceiling, telling the caller to ask for more would
        // be a lie: the API clamps and those events are unreachable.
        let beyond = json!({"event_count": 4000, "events": [1]});
        let note = truncation_note(&beyond).expect("truncation must be reported");
        assert!(note.contains("3000 are not reachable"), "{note}");
        assert!(!note.contains("--events 4000"), "{note}");

        // A local record carries no `event_count`, so there is nothing to say.
        assert_eq!(truncation_note(&json!({"events": [1, 2]})), None);
    }

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

    #[test]
    fn gate_refuses_certified_without_keyring_or_acknowledgement() {
        let refusal = refuse_unverified_gate(Some(SupportTier::Certified), false, false)
            .expect("certified without a keyring must be refused");
        assert!(refusal.contains("--require-tier certified"), "{refusal}");
        assert!(refusal.contains("--trusted-keys"), "{refusal}");
        assert!(refusal.contains("--allow-unverified"), "{refusal}");
    }

    #[test]
    fn gate_refuses_supported_without_keyring_or_acknowledgement() {
        let refusal = refuse_unverified_gate(Some(SupportTier::Supported), false, false)
            .expect("supported without a keyring must be refused");
        assert!(refusal.contains("--require-tier supported"), "{refusal}");
    }

    #[test]
    fn gate_allows_certified_with_a_keyring() {
        assert_eq!(
            refuse_unverified_gate(Some(SupportTier::Certified), true, false),
            None
        );
    }

    #[test]
    fn gate_allows_certified_with_explicit_acknowledgement() {
        assert_eq!(
            refuse_unverified_gate(Some(SupportTier::Certified), false, true),
            None
        );
    }

    #[test]
    fn gate_allows_community_without_a_keyring() {
        // community carries no real support promise; self-assertion is harmless.
        assert_eq!(
            refuse_unverified_gate(Some(SupportTier::Community), false, false),
            None
        );
    }

    #[test]
    fn gate_allows_when_no_tier_is_required() {
        assert_eq!(refuse_unverified_gate(None, false, false), None);
    }

    #[test]
    fn allow_unverified_is_constrained_at_parse_time() {
        // Contradiction: --allow-unverified alongside --trusted-keys.
        assert!(
            Cli::try_parse_from([
                "andromeda",
                "hardware",
                "check",
                "m.json",
                "--trusted-keys",
                "keys.json",
                "--allow-unverified",
            ])
            .is_err()
        );
        // Meaningless: --allow-unverified without --require-tier.
        assert!(
            Cli::try_parse_from([
                "andromeda",
                "hardware",
                "check",
                "m.json",
                "--allow-unverified",
            ])
            .is_err()
        );
        // The combination the flag exists for parses.
        assert!(
            Cli::try_parse_from([
                "andromeda",
                "hardware",
                "check",
                "m.json",
                "--require-tier",
                "certified",
                "--allow-unverified",
            ])
            .is_ok()
        );
    }

    /// RAII temp directory under the system temp dir; removed on drop so a
    /// failed assertion cannot leak files. Mirrors the guard in
    /// `andromeda-hardware/src/verify.rs`.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = format!(
                "andromeda-cli-keys-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        /// Writes `contents` to `name` inside the directory, returning the path.
        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("write file");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_empty_keyring_is_rejected_rather_than_silently_blocking_everything() {
        let dir = TempDir::new("empty");
        let path = dir.write("keys.json", "{}");
        let error = load_trusted_keys(&path).expect_err("empty keyring must be rejected");
        assert!(error.contains("empty"), "unhelpful error: {error}");
    }

    #[test]
    fn a_malformed_key_is_rejected_with_the_file_named() {
        let dir = TempDir::new("bad");
        let path = dir.write("keys.json", r#"{"root": "not-hex"}"#);
        let error = load_trusted_keys(&path).expect_err("malformed key must be rejected");
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
        let dir = TempDir::new("ok");
        let path = dir.write(
            "keys.json",
            &format!(r#"{{"root": "{}"}}"#, key.verifying_key_hex()),
        );
        let keyring = load_trusted_keys(&path).expect("valid keyring loads");
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
