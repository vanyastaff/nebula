use super::helpers::write_temp_file;
use nebula_config::{ConfigBuilder, ConfigError, ConfigSource, FunctionValidator};
use std::sync::Arc;

#[tokio::test]
async fn failed_reload_keeps_last_known_good_snapshot() {
    let path = write_temp_file("lkg", "toml", "[app]\nversion = \"1.0.0\"\nport = 8080\n");

    let validator = FunctionValidator::new(|value| {
        let valid = value
            .get("app")
            .and_then(|app| app.get("port"))
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|port| port >= 1024);
        if valid {
            Ok(())
        } else {
            Err(ConfigError::validation("app.port must be >= 1024"))
        }
    });

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .with_validator(Arc::new(validator))
        .build()
        .await
        .expect("initial valid config should build");

    let before: String = config
        .get("app.version")
        .await
        .expect("version should exist");
    assert_eq!(before, "1.0.0");

    std::fs::write(&path, "[app]\nversion = \"2.0.0\"\nport = 80\n")
        .expect("should overwrite file with invalid candidate");

    assert!(config.reload().await.is_err());

    let version: String = config
        .get("app.version")
        .await
        .expect("last-known-good version should remain active");
    let port: u16 = config
        .get("app.port")
        .await
        .expect("last-known-good port should remain active");

    assert_eq!(version, "1.0.0");
    assert_eq!(port, 8080);
}
