use nebula_log::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Auto-detect best configuration
    nebula_log::auto_init()?;

    // Note: fields come BEFORE the message
    info!(port = 8080, "Server starting");
    debug!(request_count = 42, "Debug information");
    warn!(retry_count = 3, "Operation failed, retrying");
    error!(error_code = "DB_001", "Database connection failed");

    Ok(())
}
