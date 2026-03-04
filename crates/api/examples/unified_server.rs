//! One entry point: run HTTP API server.
//!
//! ```bash
//! cargo run -p nebula-api --example unified_server
//! ```
//!
//! Then:
//! - `GET http://127.0.0.1:5678/health` → OK
//! - `GET http://127.0.0.1:5678/api/v1/status` → JSON with API status

use nebula_api::{ApiServerConfig, run};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nebula_api=info".parse()?))
        .init();

    let bind: SocketAddr = "127.0.0.1:5678".parse()?;
    let api_config = ApiServerConfig { bind_addr: bind };

    // HTTP API сервер — блокирует до shutdown
    run(api_config).await?;
    Ok(())
}
