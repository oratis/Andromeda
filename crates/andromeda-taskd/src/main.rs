use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use andromeda_core::CapabilityKeyring;
use andromeda_policy::PolicyEngine;
use andromeda_runtime::{CapabilityAdmission, FileTaskStore, TaskService};
use andromeda_taskd::Authenticator;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(about = "Andromeda durable task control plane")]
struct Args {
    #[arg(long, env = "ANDROMEDA_LISTEN", default_value = "127.0.0.1:7777")]
    listen: SocketAddr,
    #[arg(long, env = "ANDROMEDA_STATE_DIR", default_value = ".andromeda/state")]
    state_dir: PathBuf,
    /// File holding the local API bearer token.
    ///
    /// Generated on first start if absent, with mode 0600 in a directory that
    /// must already grant nothing to group or other. There is no flag to serve
    /// without it: the API router cannot be built without an
    /// [`Authenticator`], so a missing or unusable token file is a startup
    /// failure, never a downgrade to anonymous access.
    ///
    /// The shipped unit points this at `/run/andromeda-taskd/token`, inside
    /// systemd's `RuntimeDirectory=`.
    #[arg(
        long,
        env = "ANDROMEDA_AUTH_TOKEN_FILE",
        default_value = ".andromeda/taskd-token"
    )]
    auth_token_file: PathBuf,
    /// JSON file mapping capability issuer key ids to ed25519 verifying keys
    /// in hex: `{"issuer-2026": "<64 hex chars>"}`.
    ///
    /// When supplied, every capability offered at task creation or grant must
    /// carry a signature that verifies against one of these keys. When absent,
    /// unsigned capabilities are accepted — the v0 posture, which the daemon
    /// warns about at startup and reports on `/healthz`, because no component
    /// in this repository issues capabilities yet.
    #[arg(long, env = "ANDROMEDA_CAPABILITY_KEYRING")]
    capability_keyring: Option<PathBuf>,
    /// Permit binding to a non-loopback address. The API has no
    /// authentication, so this exposes every task, plan, and capability to
    /// that network; only meaningful inside an already-isolated network
    /// namespace.
    ///
    /// The boolish value parser accepts the documented
    /// `ANDROMEDA_ALLOW_NON_LOOPBACK=1` (as well as `true`/`yes`/`on`);
    /// clap's default bool parser would reject everything but the literals
    /// `true`/`false` and abort startup with a usage error.
    #[arg(
        long,
        env = "ANDROMEDA_ALLOW_NON_LOOPBACK",
        default_value_t = false,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    allow_non_loopback: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    andromeda_taskd::ensure_loopback_bind(args.listen, args.allow_non_loopback)?;
    if !args.listen.ip().to_canonical().is_loopback() {
        warn!(
            listen = %args.listen,
            "binding beyond loopback with ANDROMEDA_ALLOW_NON_LOOPBACK; the task API is \
             reachable from this network and is protected only by the local bearer token"
        );
    }
    // Built before the store and before the listener: if the token cannot be
    // established, the daemon must not reach a state where it could serve.
    let authenticator = Authenticator::from_token_file(&args.auth_token_file)?;
    let admission = load_admission(args.capability_keyring.as_deref())?;
    let store = FileTaskStore::open(&args.state_dir)?;
    let service = TaskService::new(store, PolicyEngine::default(), admission);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    info!(
        listen = %args.listen,
        state_dir = %args.state_dir.display(),
        auth_token_file = %args.auth_token_file.display(),
        capability_admission = service.admission().mode_name(),
        "task service ready; every request requires Authorization: Bearer <token from \
         auth_token_file>"
    );
    axum::serve(listener, andromeda_taskd::app(service, authenticator))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Resolves the capability admission policy from an optional keyring file.
///
/// Absent file means the v0 posture: capabilities are accepted unsigned. That
/// is warned about loudly rather than silently defaulted, because it is the
/// posture in which a caller still mints its own grants (security review #3).
fn load_admission(
    keyring_path: Option<&std::path::Path>,
) -> Result<CapabilityAdmission, Box<dyn std::error::Error>> {
    let Some(path) = keyring_path else {
        warn!(
            "no capability keyring configured: capabilities are accepted UNSIGNED, so a caller \
             still issues its own grants. Pass --capability-keyring once a trusted issuer exists."
        );
        return Ok(CapabilityAdmission::unsigned_for_development());
    };
    let contents = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "could not read capability keyring {}: {error}",
            path.display()
        )
    })?;
    let entries: BTreeMap<String, String> = serde_json::from_str(&contents).map_err(|error| {
        format!(
            "capability keyring {} must be a JSON object of {{\"key_id\": \"<64 hex chars>\"}}: \
             {error}",
            path.display()
        )
    })?;
    let keyring = CapabilityKeyring::from_hex_entries(entries)?;
    let key_ids: Vec<String> = keyring.key_ids().map(str::to_owned).collect();
    // `require_signed` rejects an empty keyring, so a typo that parses to `{}`
    // is a startup failure rather than a daemon that refuses every request
    // while looking hardened.
    let admission = CapabilityAdmission::require_signed(keyring)?;
    info!(
        keyring = %path.display(),
        trusted_key_ids = ?key_ids,
        "capability signatures are REQUIRED; unsigned or untrusted grants are refused"
    );
    Ok(admission)
}

/// Completes when a shutdown signal (Ctrl+C, or SIGTERM on Unix) arrives.
///
/// A failure to install a signal handler must not shut the daemon down, so
/// each failed installation is logged and replaced by a future that never
/// resolves; the daemon then runs until it is killed externally.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to install Ctrl+C handler; running until killed");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "failed to install SIGTERM handler; running until killed");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sentinel that marks the re-executed child process of the env test.
    const CHILD_SENTINEL: &str = "ANDROMEDA_TASKD_BOOLISH_ENV_CHILD";

    /// Regression guard: `ANDROMEDA_ALLOW_NON_LOOPBACK=1` (the value the
    /// startup error message documents) must parse as `true` instead of
    /// aborting with a clap usage error.
    ///
    /// clap reads environment variables from the real process environment,
    /// and `std::env::set_var` is unsafe in edition 2024 (this workspace
    /// forbids unsafe code), so the test re-executes itself as a child
    /// process with the variable set and performs the assertion there. A
    /// single test owns the variable, so there are no env races.
    #[test]
    fn allow_non_loopback_env_accepts_boolish_values() {
        if std::env::var_os(CHILD_SENTINEL).is_some() {
            // Child: ANDROMEDA_ALLOW_NON_LOOPBACK=1 is present.
            let args = Args::try_parse_from(["andromeda-taskd"])
                .expect("ANDROMEDA_ALLOW_NON_LOOPBACK=1 must parse, not abort startup");
            assert!(args.allow_non_loopback, "env value 1 must mean true");
            return;
        }

        // Parent: the variable is absent, so the flag must default to false.
        let args = Args::try_parse_from(["andromeda-taskd"]).expect("defaults must parse");
        assert!(!args.allow_non_loopback, "flag must default to false");

        let status = std::process::Command::new(std::env::current_exe().expect("test binary path"))
            .args([
                "tests::allow_non_loopback_env_accepts_boolish_values",
                "--exact",
            ])
            .env(CHILD_SENTINEL, "1")
            .env("ANDROMEDA_ALLOW_NON_LOOPBACK", "1")
            .status()
            .expect("re-run test binary");
        assert!(status.success(), "child assertion failed: {status}");
    }
}
