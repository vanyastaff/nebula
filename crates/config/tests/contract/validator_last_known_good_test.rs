use super::helpers::{assert_contract_category, assert_validation_failed, write_temp_file};
use nebula_config::{ConfigBuilder, ConfigSource};
use nebula_validator::foundation::{Validate, ValidationError};
use std::sync::Arc;

#[derive(Clone)]
struct RequirePortAtLeast1024;

impl Validate<serde_json::Value> for RequirePortAtLeast1024 {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        let port = input
            .get("service")
            .and_then(|service| service.get("port"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if port >= 1024 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "validation_failed",
                "service.port must be >= 1024",
            ))
        }
    }
}

#[tokio::test]
async fn failed_reload_preserves_last_known_good_with_nebula_validator() {
    let path = write_temp_file(
        "validator_lkg",
        "toml",
        "[service]\nversion = \"1.0.0\"\nport = 8080\n",
    );

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .with_validator(Arc::new(RequirePortAtLeast1024))
        .build()
        .await
        .expect("initial valid config should be active");

    std::fs::write(&path, "[service]\nversion = \"2.0.0\"\nport = 80\n")
        .expect("should write invalid candidate");

    let err = config.reload().await.expect_err("reload must fail");
    assert_validation_failed(&err);
    assert_contract_category(&err, "validation_failed");

    let version: String = config
        .get("service.version")
        .await
        .expect("last-known-good version should stay active");
    let port: u16 = config
        .get("service.port")
        .await
        .expect("last-known-good port should stay active");

    assert_eq!(version, "1.0.0");
    assert_eq!(port, 8080);
}
