use nebula_api::server::{ApiTransport, run_transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    nebula_api::init_api_telemetry();
    run_transport(ApiTransport).await?;
    Ok(())
}
