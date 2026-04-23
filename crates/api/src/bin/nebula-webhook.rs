use nebula_api::server::{WebhookIngressTransport, run_transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    run_transport(WebhookIngressTransport).await?;
    Ok(())
}
