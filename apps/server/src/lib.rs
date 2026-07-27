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
mod email;
mod oauth_egress;
mod transport;
mod webhook_credential_resolver;

#[cfg(feature = "runtime-repair-red")]
pub mod runtime_repair_red;

use clap::Parser;
use transport::{ApiTransport, RealtimeTransport, Transport, WebhookIngressTransport};

/// Failure from the ordinary server composition or serving path.
#[derive(Debug)]
pub struct ServerRunError(compose::ServerRunError);

impl std::fmt::Display for ServerRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ServerRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[derive(Parser)]
#[command(name = "nebula-server", about = "Nebula workflow engine server")]
struct Cli {
    /// Ingress transport to run in this process.
    #[arg(long, value_enum, env = "NEBULA_TRANSPORT", default_value = "all")]
    transport: Transport,
}

/// Run the ordinary environment-driven server process.
///
/// This is the same entry path used by the `nebula-server` binary. It remains
/// separate from the evidence-only runtime-repair profile, which never reads
/// process-global configuration or installs signal handlers.
///
/// # Errors
///
/// Returns a typed startup or serving error.
pub async fn run_from_env() -> Result<(), ServerRunError> {
    let telemetry_guard = nebula_api::init_api_telemetry()
        .map_err(compose::ServerRunError::Telemetry)
        .map_err(ServerRunError)?;
    let cli = Cli::parse();
    match cli.transport {
        Transport::Api | Transport::All => compose::run_transport(ApiTransport, telemetry_guard)
            .await
            .map_err(ServerRunError),
        Transport::Webhook => compose::run_transport(WebhookIngressTransport, telemetry_guard)
            .await
            .map_err(ServerRunError),
        Transport::Realtime => compose::run_transport(RealtimeTransport, telemetry_guard)
            .await
            .map_err(ServerRunError),
    }
}
