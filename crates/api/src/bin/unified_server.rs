use nebula_api::{ApiServerConfig, WorkerStatus, run};
use nebula_webhook::WebhookServerConfig;
use std::env;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

fn parse_worker_count() -> usize {
    env::var("NEBULA_WORKER_COUNT")
        .ok()
        .and_then(|v| usize::from_str(&v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(4)
}

fn parse_bind_addr() -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
    let bind = env::var("NEBULA_API_BIND").unwrap_or_else(|_| "0.0.0.0:5678".to_string());
    Ok(bind.parse()?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nebula_api=info".parse()?))
        .init();

    let bind = parse_bind_addr()?;
    let worker_count = parse_worker_count();

    let api_config = ApiServerConfig { bind_addr: bind };

    let webhook_config = WebhookServerConfig {
        bind_addr: bind, // ignored in embedded mode
        base_url: format!("http://{}", bind),
        path_prefix: "/webhooks".to_string(),
        enable_compression: true,
        enable_cors: true,
        body_limit: 10 * 1024 * 1024,
    };

    for i in 0..worker_count {
        let worker_id = format!("wrk-{}", i + 1);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        tracing::info!(%worker_id, "worker spawned");
    }

    let workers = (0..worker_count)
        .map(|i| WorkerStatus {
            id: format!("wrk-{}", i + 1),
            status: if i < worker_count.saturating_sub(1) {
                "active"
            } else {
                "idle"
            }
            .to_string(),
            queue_len: if i == 0 { 1 } else { 0 },
        })
        .collect::<Vec<_>>();

    run(api_config, webhook_config, workers).await?;
    Ok(())
}
