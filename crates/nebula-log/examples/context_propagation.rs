//! Example showing context propagation across async boundaries

use nebula_log::prelude::*;
use nebula_log::{with_context, Context};

#[tokio::main]
async fn main() -> Result<()> {
    nebula_log::auto_init()?;

    // Set global context
    let _ctx = with_context!(service = "api-gateway", environment = "staging");

    // Simulate handling multiple requests concurrently
    let handles = (0..3)
        .map(|i| tokio::spawn(async move { handle_user_request(format!("user-{}", i)).await }));

    for handle in handles {
        handle.await??;
    }

    Ok(())
}

async fn handle_user_request(user_id: String) -> Result<()> {
    // Set request-specific context
    let ctx = Context::current()
        .with_request_id(format!("req-{}", uuid::Uuid::new_v4()))
        .with_user_id(&user_id);

    // This context is scoped to this async task
    ctx.scope(|| {
        info!("Processing user request");

        // Context is preserved across function calls
        process_user_data();

        info!("User request completed");
    });

    Ok(())
}

fn process_user_data() {
    let ctx = Context::current();

    debug!(
        user_id = ?ctx.user_id,
        request_id = ?ctx.request_id,
        "Processing user data"
    );
}
