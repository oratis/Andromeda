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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
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
