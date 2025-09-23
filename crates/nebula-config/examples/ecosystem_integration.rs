//! Example demonstrating nebula-config integration with the Nebula ecosystem

use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;

use nebula_config::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with nebula-log
    nebula_log::auto_init()?;

    info!("Starting configuration ecosystem integration example");

    // Example 1: Basic configuration loading with structured logging
    info!("=== Basic Configuration Loading ===");

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::Env)
        .with_defaults(serde_json::json!({
            "app": {
                "name": "nebula-config-example",
                "version": "1.0.0",
                "port": 8080,
                "debug": false
            },
            "database": {
                "host": "localhost",
                "port": 5432,
                "name": "nebula_db",
                "ssl": true
            },
            "features": {
                "enabled": ["logging", "metrics"],
                "experimental": {
                    "ai_mode": false,
                    "cache_size": 1024
                }
            }
        }))?
        .build()
        .await?;

    info!(
        config_sources = config.sources().len(),
        "Configuration loaded successfully"
    );

    // Example 2: Working with NebulaValue for dynamic configuration
    info!("=== NebulaValue Integration ===");

    // Get configuration as NebulaValue
    let app_config = config.get_value("app").await?;
    info!(
        app_name = %app_config.get_path("name").unwrap_or(&NebulaValue::null()),
        app_port = %app_config.get_path("port").unwrap_or(&NebulaValue::null()),
        "Application configuration loaded"
    );

    // Set dynamic configuration using NebulaValue
    let dynamic_value = NebulaValue::Object({
        let obj = nebula_value::Object::new();
        let _ = obj.insert("runtime_mode".to_string(), NebulaValue::string("development"));
        let _ = obj.insert("request_timeout".to_string(), NebulaValue::int(30));
        let _ = obj.insert("retry_count".to_string(), NebulaValue::int(3));
        obj
    });

    config.set_value("runtime", dynamic_value).await?;

    info!(
        runtime_mode = %config.get_value("runtime.runtime_mode").await?,
        timeout = %config.get_value("runtime.request_timeout").await?,
        "Dynamic configuration set"
    );

    // Example 3: Typed configuration with NebulaValue serialization
    info!("=== Typed Configuration ===");

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    struct DatabaseConfig {
        host: String,
        port: u16,
        name: String,
        ssl: bool,
        pool_size: Option<u32>,
    }

    // Get typed configuration
    let mut db_config: DatabaseConfig = config.get("database").await.unwrap_or_else(|_| {
        warn!("Failed to load database config, using defaults");
        DatabaseConfig {
            host: "localhost".to_string(),
            port: 5432,
            name: "default_db".to_string(),
            ssl: false,
            pool_size: Some(10),
        }
    });

    info!(
        db_host = %db_config.host,
        db_port = db_config.port,
        db_ssl = db_config.ssl,
        "Database configuration loaded"
    );

    // Update configuration
    db_config.pool_size = Some(20);
    config.set_typed("database", &db_config).await?;

    info!(
        new_pool_size = db_config.pool_size.unwrap_or(0),
        "Database pool size updated"
    );

    // Example 4: Configuration validation and error handling
    info!("=== Error Handling & Validation ===");

    // Try to get a non-existent configuration
    match config.get::<String>("non_existent.key").await {
        Ok(value) => info!(value = %value, "Found unexpected configuration"),
        Err(err) => {
            // ConfigError automatically converts to NebulaError
            let nebula_error: NebulaError = err.into();
            warn!(
                error_type = "configuration_missing",
                error = %nebula_error,
                "Expected configuration error occurred"
            );
        }
    }

    // Example 5: Flat configuration map for debugging
    info!("=== Configuration Debugging ===");

    let flat_map = config.flatten().await;
    debug!(
        total_keys = flat_map.len(),
        config_keys = ?flat_map.keys().collect::<Vec<_>>(),
        "Configuration flattened for debugging"
    );

    // Show some key configuration values
    for (key, value) in flat_map.iter().take(5) {
        debug!(
            config_key = %key,
            config_value = %value,
            "Configuration entry"
        );
    }

    // Example 6: Configuration merging
    info!("=== Configuration Merging ===");

    let override_config = NebulaValue::Object({
        let obj = nebula_value::Object::new();
        let mut app_obj = nebula_value::Object::new();
        let _ = app_obj.insert("debug".to_string(), NebulaValue::bool(true));
        let _ = app_obj.insert("log_level".to_string(), NebulaValue::string("debug"));
        let _ = obj.insert("app".to_string(), NebulaValue::Object(app_obj));
        obj
    });

    config.merge(override_config).await?;

    let debug_enabled: bool = config.get("app.debug").await?;
    let log_level: String = config.get("app.log_level").await.unwrap_or_else(|_| "info".to_string());

    info!(
        debug_mode = debug_enabled,
        log_level = %log_level,
        "Configuration merged with overrides"
    );

    // Example 7: Environment variable integration
    info!("=== Environment Variables ===");

    // Set some environment variables for demonstration
    unsafe {
        std::env::set_var("NEBULA_APP_PORT", "9090");
        std::env::set_var("NEBULA_DATABASE_HOST", "production-db.example.com");
    }

    // Reload configuration to pick up environment changes
    let env_config = ConfigBuilder::new()
        .with_source(ConfigSource::EnvWithPrefix("NEBULA".to_string()))
        .build()
        .await?;

    if let Ok(env_port) = env_config.get::<u16>("app.port").await {
        info!(
            env_port = env_port,
            "Environment variable override detected"
        );
    }

    if let Ok(env_db_host) = env_config.get::<String>("database.host").await {
        info!(
            env_db_host = %env_db_host,
            "Environment database host override"
        );
    }

    // Example 8: Configuration validation
    info!("=== Configuration Validation ===");

    // Validate configuration structure
    let validation_result = validate_config_structure(&config).await;
    match validation_result {
        Ok(_) => info!("Configuration validation passed"),
        Err(err) => warn!(error = %err, "Configuration validation failed"),
    }

    // Example 9: Configuration watching simulation
    info!("=== Configuration Watching ===");

    // In a real scenario, you would set up file watching
    // Here we'll just simulate configuration changes
    info!("Simulating configuration changes...");

    for i in 1..=3 {
        sleep(Duration::from_millis(100)).await;

        let dynamic_update = NebulaValue::Object({
            let obj = nebula_value::Object::new();
            let _ = obj.insert("iteration".to_string(), NebulaValue::int(i));
            let _ = obj.insert("timestamp".to_string(), NebulaValue::string(
                chrono::Utc::now().to_rfc3339()
            ));
            obj
        });

        config.set_value(&format!("simulation.update_{}", i), dynamic_update).await?;

        info!(
            iteration = i,
            "Configuration updated dynamically"
        );
    }

    info!("Configuration ecosystem integration example completed successfully");
    Ok(())
}

/// Validate configuration structure
async fn validate_config_structure(config: &Config) -> ConfigResult<()> {
    // Check required fields exist
    let required_fields = [
        "app.name",
        "app.port",
        "database.host",
        "database.port",
    ];

    for field in &required_fields {
        let value = config.get_value(field).await?;
        if matches!(value, NebulaValue::Null) {
            return Err(ConfigError::validation_with_field(
                "Required field is null",
                field.to_string(),
            ));
        }
    }

    // Validate port ranges
    let app_port = config.get::<u16>("app.port").await?;
    if app_port < 1024 || app_port > 65535 {
        return Err(ConfigError::validation_with_field(
            "Port must be between 1024 and 65535",
            "app.port",
        ));
    }

    // Validate database configuration
    let db_port = config.get::<u16>("database.port").await?;
    if db_port == 0 {
        return Err(ConfigError::validation_with_field(
            "Database port cannot be zero",
            "database.port",
        ));
    }

    debug!("Configuration validation completed successfully");
    Ok(())
}