//! Sentry integration test example

use nebula_log::prelude::*;
use std::env;

#[tokio::main]
async fn main() -> nebula_log::Result<()> {
    // Set Sentry DSN for testing
    unsafe {
        env::set_var("SENTRY_DSN", "https://2c82298d59b7fa61a293e1305d5aaaa2@o1200386.ingest.us.sentry.io/4509795981787136");
        env::set_var("SENTRY_ENV", "test");
        env::set_var("SENTRY_TRACES_SAMPLE_RATE", "1.0"); // 100% sampling for testing
    }

    // Initialize logger with Sentry
    let _guard = nebula_log::auto_init()?;

    info!("Sentry test started - this should appear in both console and Sentry");

    // Test different log levels
    trace!("This is a trace message");
    debug!(user_id = "test-123", "Debug message with user context");
    info!(request_id = "req-456", endpoint = "/api/test", "Processing test request");
    warn!(retry_count = 3, error_type = "timeout", "Operation failed, retrying");

    // Test error logging - this should definitely appear in Sentry
    error!(
        error_code = "TEST_001",
        component = "sentry_test",
        "Test error for Sentry integration - this is intentional!"
    );

    // Simulate a panic (commented out to avoid crashing)
    // panic!("Test panic for Sentry - this would be captured automatically");

    // Test manual error reporting
    let custom_error = anyhow::anyhow!("Custom test error for Sentry");
    error!(error = ?custom_error, "Manual error reporting test");

    // Give Sentry time to send events
    println!("Waiting for Sentry events to be sent...");
    std::thread::sleep(std::time::Duration::from_secs(2));

    info!("Sentry test completed - check your Sentry dashboard!");

    Ok(())
}
