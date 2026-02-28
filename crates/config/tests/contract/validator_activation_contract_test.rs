use nebula_config::ConfigBuilder;
use nebula_validator::foundation::{Validate, ValidationError};

#[derive(Clone)]
struct RequireModeProd;

impl Validate<serde_json::Value> for RequireModeProd {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        let mode = input
            .get("service")
            .and_then(|service| service.get("mode"))
            .and_then(serde_json::Value::as_str);

        if mode == Some("prod") {
            Ok(())
        } else {
            Err(ValidationError::new(
                "validation_failed",
                "service.mode must be prod",
            ))
        }
    }
}

#[tokio::test]
async fn valid_candidate_activates_with_nebula_validator_bridge() {
    let config = ConfigBuilder::new()
        .with_defaults_json(serde_json::json!({
            "service": {
                "mode": "prod"
            }
        }))
        .with_validator(std::sync::Arc::new(RequireModeProd))
        .build()
        .await
        .expect("valid candidate should activate");

    let mode: String = config.get("service.mode").await.expect("path should exist");
    assert_eq!(mode, "prod");
}
