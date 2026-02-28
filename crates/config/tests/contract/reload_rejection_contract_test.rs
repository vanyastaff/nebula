use super::helpers::write_temp_file;
use nebula_config::{ConfigBuilder, ConfigError, FunctionValidator};
use std::sync::Arc;

#[tokio::test]
async fn invalid_reload_candidate_is_rejected() {
    let path = write_temp_file("reload_reject", "json", r#"{"feature":{"enabled":true}}"#);

    let validator = FunctionValidator::new(|value| {
        let enabled = value
            .get("feature")
            .and_then(|f| f.get("enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if enabled {
            Ok(())
        } else {
            Err(ConfigError::validation("feature.enabled must remain true"))
        }
    });

    let config = ConfigBuilder::new()
        .with_source(nebula_config::ConfigSource::File(path.clone()))
        .with_validator(Arc::new(validator))
        .build()
        .await
        .expect("initial valid config should build");

    std::fs::write(&path, r#"{"feature":{"enabled":false}}"#)
        .expect("should overwrite fixture file");

    let reload_error = config.reload().await.expect_err("reload must be rejected");
    assert_eq!(
        reload_error.contract_category(),
        nebula_config::core::error::ContractErrorCategory::ValidationFailed
    );
}
