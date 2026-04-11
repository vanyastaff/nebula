use nebula_config::ConfigBuilder;
use nebula_validator::foundation::{Validate, ValidationError};

use super::helpers::assert_validation_failed;

#[derive(Clone)]
struct SecretRejectingValidator;

impl Validate<serde_json::Value> for SecretRejectingValidator {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        let secret = input
            .get("credentials")
            .and_then(|credentials| credentials.get("password"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if secret.is_empty() {
            Ok(())
        } else {
            Err(ValidationError::new(
                "validation_failed",
                "credential payload must not include inline password",
            )
            .with_param("password", secret.to_string()))
        }
    }
}

#[tokio::test]
async fn diagnostics_do_not_leak_sensitive_values_from_validator_context() {
    let result = ConfigBuilder::new()
        .with_defaults(serde_json::json!({
            "credentials": {
                "password": "super-secret-value"
            }
        }))
        .with_validator(std::sync::Arc::new(SecretRejectingValidator))
        .build()
        .await;

    let err = result.expect_err("build should fail");
    assert_validation_failed(&err);

    let text = err.to_string();
    assert!(!text.contains("super-secret-value"));
    assert!(text.contains("credential payload"));
}
