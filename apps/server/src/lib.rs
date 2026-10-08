//! First-party Nebula server composition root.
//!
//! The ordinary deployment binary stays environment-driven. The optional
//! `runtime-repair-red` feature adds an app-owned evidence profile; it is not a
//! deployment surface and is not re-exported by `nebula-sdk`.

#![forbid(unsafe_code)]

mod compose;
mod credential_adapters;
mod credential_composition;
mod credential_runtime;
mod deployment_database;
mod email;
mod execution_binding_resolver;
mod execution_runtime;
mod execution_store_backends;
mod oauth_egress;
mod owner_enrollment;
mod storage_diagnostics;
mod tenant_bootstrap;
mod tenant_directory;
mod transport;
mod webhook_credential_resolver;

#[cfg(feature = "runtime-repair-red")]
pub mod runtime_repair_red;

use clap::Parser;
use transport::{ApiTransport, RealtimeTransport, Transport, WebhookIngressTransport};

/// Failure from serving or an operator setup command.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ServerRunError(RunFailure);

#[derive(Debug, thiserror::Error)]
enum RunFailure {
    #[error(transparent)]
    Serving(#[from] compose::ServerRunError),
    #[error(transparent)]
    Setup(#[from] owner_enrollment::SetupError),
}

impl From<compose::ServerRunError> for ServerRunError {
    fn from(error: compose::ServerRunError) -> Self {
        Self(RunFailure::Serving(error))
    }
}

#[derive(clap::Subcommand)]
enum Command {
    /// Operator-only initial account and organization enrollment.
    Setup {
        #[command(subcommand)]
        command: owner_enrollment::SetupCommand,
    },
}

#[derive(Parser)]
#[command(
    name = "nebula-server",
    version,
    about = "Nebula workflow engine server"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Ingress transport to run in this process.
    #[arg(long, value_enum, env = "NEBULA_TRANSPORT", default_value = "all")]
    transport: Transport,
    /// Run execution in this server or delegate to separate PostgreSQL workers.
    #[arg(
        long,
        value_enum,
        env = "NEBULA_EXECUTION",
        default_value = "in-process"
    )]
    execution: execution_runtime::ExecutionTopology,
}

/// Run the selected operator command or the environment-driven server process.
///
/// This is the same entry path used by the `nebula-server` binary. It remains
/// separate from the evidence-only runtime-repair profile, which never reads
/// process-global configuration or installs signal handlers.
/// Operator setup initializes only deployment storage and never starts serving.
///
/// # Errors
///
/// Returns a typed startup or serving error.
pub async fn run_from_env() -> Result<(), ServerRunError> {
    let cli = Cli::parse();
    if let Some(Command::Setup { command }) = cli.command {
        return owner_enrollment::run(command)
            .await
            .map_err(|error| ServerRunError(RunFailure::Setup(error)));
    }
    let telemetry_guard = nebula_api::init_api_telemetry()
        .map_err(compose::ServerRunError::Telemetry)
        .map_err(ServerRunError::from)?;
    match cli.transport {
        Transport::Api | Transport::All => {
            compose::run_transport(ApiTransport, telemetry_guard, cli.execution)
                .await
                .map_err(ServerRunError::from)
        },
        Transport::Webhook => {
            compose::run_transport(WebhookIngressTransport, telemetry_guard, cli.execution)
                .await
                .map_err(ServerRunError::from)
        },
        Transport::Realtime => {
            compose::run_transport(RealtimeTransport, telemetry_guard, cli.execution)
                .await
                .map_err(ServerRunError::from)
        },
    }
}
