//! One entry point: spawn N node workers (tokio::spawn) + run HTTP API server.
//!
//! ```bash
//! cargo run -p nebula-api --example unified_server
//! ```
//!
//! Then:
//! - `GET http://127.0.0.1:5678/health` → OK
//! - `GET http://127.0.0.1:5678/api/v1/status` → JSON with workers

use nebula_api::{ApiServerConfig, WorkerStatus, run};
use std::net::SocketAddr;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

const WORKER_COUNT: usize = 4;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nebula_api=info".parse()?))
        .init();

    let bind: SocketAddr = "127.0.0.1:5678".parse()?;
    let api_config = ApiServerConfig { bind_addr: bind };

    // Одна точка входа: сперва spawn воркеров (в продакшене они бы тянули из TaskQueue и вызывали engine)
    for i in 0..WORKER_COUNT {
        let worker_id = format!("wrk-{}", i + 1);
        tokio::spawn(async move {
            loop {
                // В реальности: queue.dequeue() -> engine.execute() -> queue.ack()
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        tracing::info!(%worker_id, "worker spawned");
    }

    // Снимок для /api/v1/status (позже можно заменить на живой Arc<WorkerState>)
    let workers = (0..WORKER_COUNT)
        .map(|i| WorkerStatus {
            id: format!("wrk-{}", i + 1),
            status: if i < 3 { "active" } else { "idle" }.to_string(),
            queue_len: [2, 1, 0, 0][i],
        })
        .collect::<Vec<_>>();

    // HTTP API сервер — блокирует до shutdown
    run(api_config, workers).await?;
    Ok(())
}
