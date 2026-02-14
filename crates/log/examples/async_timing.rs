use anyhow::Result;
use nebula_log::prelude::*;
use nebula_log::{async_timed, measure, with_context};

#[tokio::main]
async fn main() -> Result<()> {
    nebula_log::auto_init()?;

    // Build context, then scope it over our async work
    let ctx = with_context!(request_id = "req-123", user_id = "user-456");

    ctx.scope(async {
        // Time an async operation
        let result = async_timed!("database_query", {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            42
        });

        info!(result, "Query completed");

        // Measure with span
        let data = measure!("fetch_data", async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            vec![1, 2, 3]
        });

        info!(?data, "Data fetched");
    })
    .await;

    Ok(())
}
