use std::sync::Arc;

use nebula_config::{ConfigBuilder, ConfigSource};
use nebula_validator::foundation::{Validate, ValidationError};

use super::helpers::{assert_contract_category, assert_validation_failed, write_temp_file};

#[derive(Clone)]
struct RequireFeatureEnabled;

impl Validate<serde_json::Value> for RequireFeatureEnabled {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        let enabled = input
            .get("feature")
            .and_then(|feature| feature.get("enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if enabled {
            Ok(())
        } else {
            Err(ValidationError::new(
                "validation_failed",
                "feature.enabled must be true",
            ))
        }
    }
}

#[tokio::test]
async fn invalid_reload_candidate_is_rejected_by_nebula_validator() {
    let path = write_temp_file(
        "validator_reload_reject",
        "toml",
        "[feature]\nenabled = true\n",
    );

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .with_validator(Arc::new(RequireFeatureEnabled))
        .build()
        .await
        .expect("initial valid snapshot should build");

    std::fs::write(&path, "[feature]\nenabled = false\n")
        .expect("should write invalid reload candidate");

    let err = config.reload().await.expect_err("reload must fail");
    assert_validation_failed(&err);
    assert_contract_category(&err, "validation_failed");
}
