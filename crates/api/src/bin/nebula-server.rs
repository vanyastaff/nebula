use nebula_api::server::{ApiTransport, run_transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    run_transport(ApiTransport).await?;
    Ok(())
}
