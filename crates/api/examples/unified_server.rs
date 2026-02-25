//! One entry point: spawn N node workers (tokio::spawn) + run HTTP server (API + webhook) on one port.
//!
//! ```bash
//! cargo run -p nebula-api --example unified_server
//! ```
//!
//! Then:
//! - `GET http://127.0.0.1:5678/health` → OK
//! - `GET http://127.0.0.1:5678/api/v1/status` → JSON with workers + webhook
//! - `POST http://127.0.0.1:5678/webhooks/...` → webhook endpoints (when registered)

use nebula_api::{ApiServerConfig, WorkerStatus, run};
use nebula_webhook::WebhookServerConfig;
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

    let webhook_config = WebhookServerConfig {
        bind_addr: bind, // ignored in embedded mode
        base_url: format!("http://127.0.0.1:{}", bind.port()),
        path_prefix: "/webhooks".to_string(),
        enable_compression: true,
        enable_cors: true,
        body_limit: 10 * 1024 * 1024,
    };

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

    // HTTP сервер (API + webhook) — один порт, блокирует до shutdown
    run(api_config, webhook_config, workers).await?;
    Ok(())
}
