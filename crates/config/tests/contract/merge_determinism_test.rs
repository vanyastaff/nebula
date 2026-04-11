use std::sync::Arc;

use nebula_config::{ConfigBuilder, ConfigSource};
use serde_json::json;

use super::helpers::StaticFixtureLoader;

async fn build_snapshot() -> serde_json::Value {
    let file_source = ConfigSource::File("contract-file.toml".into());
    let env_source = ConfigSource::EnvWithPrefix("CONTRACT".to_string());
    let override_source = ConfigSource::EnvWithPrefix("CONTRACT_RUNTIME".to_string());

    let loader = StaticFixtureLoader::default()
        .with_payload(
            file_source.clone(),
            json!({"a": 2, "shared": {"b": "file"}}),
        )
        .with_payload(env_source.clone(), json!({"shared": {"b": "env", "c": 3}}))
        .with_payload(override_source.clone(), json!({"shared": {"c": 4}}));

    ConfigBuilder::new()
        .with_defaults(json!({"a": 1, "shared": {"b": "default"}}))
        .with_source(file_source)
        .with_source(env_source)
        .with_source(override_source)
        .with_loader(Arc::new(loader))
        .build()
        .await
        .expect("config should build")
        .get_raw(None)
        .await
        .expect("snapshot should be readable")
}

#[tokio::test]
async fn identical_inputs_produce_identical_merged_outputs() {
    let expected = build_snapshot().await;
    for _ in 0..5 {
        let actual = build_snapshot().await;
        assert_eq!(actual, expected);
    }
}
