//! Example showing context propagation across async boundaries
//!
//! Unlike the old thread-local approach, contexts now survive across `.await`
//! points in multi-thread Tokio runtimes via `tokio::task_local!`.

use anyhow::Result;
use nebula_log::Context;
use nebula_log::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    nebula_log::auto_init()?;

    // Simulate handling multiple requests concurrently
    let handles: Vec<_> = (0..3)
        .map(|i| tokio::spawn(handle_user_request(format!("user-{}", i))))
        .collect();

    for handle in handles {
        handle.await??;
    }

    Ok(())
}

async fn handle_user_request(user_id: String) -> Result<()> {
    // Build request-specific context
    let ctx = (*Context::current())
        .clone()
        .with_request_id(format!("req-{}", uuid::Uuid::new_v4()))
        .with_user_id(&user_id);

    // Context survives across .await points
    ctx.scope(async {
        info!("Processing user request");

        // Context is preserved in nested async calls
        process_user_data().await;

        info!("User request completed");
    })
    .await;

    Ok(())
}

async fn process_user_data() {
    let ctx = Context::current();

    debug!(
        user_id = ?ctx.user_id,
        request_id = ?ctx.request_id,
        "Processing user data"
    );

    // Simulated async work — context persists
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    debug!(
        user_id = ?ctx.user_id,
        "Still have context after await"
    );
}
