//! Nebula server — single composition-root binary. One process, one entry
//! point: `--transport` selects the ingress (api/webhook/realtime/all).
mod compose;
mod transport;

use clap::Parser;
use transport::{ApiTransport, RealtimeTransport, Transport, WebhookIngressTransport};

#[derive(Parser)]
#[command(name = "nebula-server", about = "Nebula workflow engine server")]
struct Cli {
    /// Ingress transport to run in this process.
    #[arg(long, value_enum, env = "NEBULA_TRANSPORT", default_value = "all")]
    transport: Transport,
}

#[tokio::main]
async fn main() -> Result<(), compose::ServerRunError> {
    nebula_api::init_api_telemetry();
    let cli = Cli::parse();
    match cli.transport {
        Transport::Api | Transport::All => compose::run_transport(ApiTransport).await,
        Transport::Webhook => compose::run_transport(WebhookIngressTransport).await,
        Transport::Realtime => compose::run_transport(RealtimeTransport).await,
    }
}
