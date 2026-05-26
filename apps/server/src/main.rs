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
    // The telemetry guard owns the OTel `SdkTracerProvider` when OTLP shipping is enabled
    // (`OTEL_EXPORTER_OTLP_ENDPOINT` set). It is moved into `run_transport` so the metrics
    // pipeline can be attached against the shared `MetricsRegistry` once `AppState` is
    // built, and so the whole guard drops only when the transport returns — flushing both
    // span and metric batches via `provider.shutdown()` (see ADR-0050 binary init contract).
    let telemetry_guard =
        nebula_api::init_api_telemetry().map_err(compose::ServerRunError::Telemetry)?;
    let cli = Cli::parse();
    match cli.transport {
        Transport::Api | Transport::All => {
            compose::run_transport(ApiTransport, telemetry_guard).await
        },
        Transport::Webhook => {
            compose::run_transport(WebhookIngressTransport, telemetry_guard).await
        },
        Transport::Realtime => compose::run_transport(RealtimeTransport, telemetry_guard).await,
    }
}
