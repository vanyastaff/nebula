use nebula_config::core::error::ContractErrorCategory;
use nebula_config::{ConfigBuilder, ConfigError};
use serde_json::json;

#[tokio::test]
async fn missing_path_maps_to_missing_path_category() {
    let config = ConfigBuilder::new()
        .with_defaults(json!({"server":{"port":8080}}))
        .build()
        .await
        .expect("config should build");

    let err = config
        .get::<String>("server.host")
        .await
        .expect_err("missing path should error");
    assert!(matches!(err, ConfigError::PathError { .. }));
    assert_eq!(err.contract_category(), ContractErrorCategory::MissingPath);
}

#[tokio::test]
async fn type_mismatch_maps_to_type_mismatch_category() {
    let config = ConfigBuilder::new()
        .with_defaults(json!({"server":{"port":8080}}))
        .build()
        .await
        .expect("config should build");

    let err = config
        .get::<String>("server.port")
        .await
        .expect_err("type mismatch should error");
    assert!(matches!(err, ConfigError::TypeError { .. }));
    assert_eq!(err.contract_category(), ContractErrorCategory::TypeMismatch);
}
