use nebula_log::Config;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Production configuration with JSON output
    let mut config = Config::production();

    // Add service metadata
    config.fields.service = Some("api-gateway".to_string());
    config.fields.env = Some("production".to_string());
    config.fields.version = Some(env!("CARGO_PKG_VERSION").to_string());
    config
        .fields
        .custom
        .insert("datacenter".to_string(), json!("us-west-2"));

    // Enable hot reload
    config.reloadable = true;

    let _guard = nebula_log::init_with(config)?;

    tracing::info!(
        endpoint = "/api/v1/users",
        method = "GET",
        "Request received"
    );

    Ok(())
}
