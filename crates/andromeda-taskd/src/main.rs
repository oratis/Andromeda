use std::net::SocketAddr;
use std::path::PathBuf;

use andromeda_policy::PolicyEngine;
use andromeda_runtime::{FileTaskStore, TaskService};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(about = "Andromeda durable task control plane")]
struct Args {
    #[arg(long, env = "ANDROMEDA_LISTEN", default_value = "127.0.0.1:7777")]
    listen: SocketAddr,
    #[arg(long, env = "ANDROMEDA_STATE_DIR", default_value = ".andromeda/state")]
    state_dir: PathBuf,
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
        tracing::warn!(
            listen = %args.listen,
            "binding beyond loopback with ANDROMEDA_ALLOW_NON_LOOPBACK; the task API is \
             UNAUTHENTICATED and now reachable from this network"
        );
    }
    let store = FileTaskStore::open(&args.state_dir)?;
    let service = TaskService::new(store, PolicyEngine::default());
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    info!(listen = %args.listen, state_dir = %args.state_dir.display(), "task service ready");
    axum::serve(listener, andromeda_taskd::app(service))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
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
