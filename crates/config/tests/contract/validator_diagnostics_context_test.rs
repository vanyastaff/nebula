use nebula_config::{ConfigBuilder, ConfigError};
use nebula_validator::foundation::{Validate, ValidationError};

#[derive(Clone)]
struct ContextAwareValidator;

impl Validate<serde_json::Value> for ContextAwareValidator {
    fn validate(&self, _input: &serde_json::Value) -> Result<(), ValidationError> {
        Err(
            ValidationError::new("validation_failed", "service.port is invalid")
                .with_field("service.port"),
        )
    }
}

#[tokio::test]
async fn diagnostics_include_source_and_path_context() {
    let result = ConfigBuilder::new()
        .with_defaults_json(serde_json::json!({"service":{"port":0}}))
        .with_validator(std::sync::Arc::new(ContextAwareValidator))
        .build()
        .await;

    let err = result.expect_err("validation must fail");
    match err {
        ConfigError::ValidationError { message, field } => {
            assert!(message.contains("service.port is invalid"));
            assert_eq!(field.as_deref(), Some("service.port"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}
