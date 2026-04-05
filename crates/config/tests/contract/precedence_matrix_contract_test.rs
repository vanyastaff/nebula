use super::helpers::{StaticFixtureLoader, fixture_path};
use nebula_config::{ConfigBuilder, ConfigSource};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct PrecedenceFixture {
    defaults: Value,
    sources: Vec<FixtureSource>,
    expected: Value,
}

#[derive(Debug, Deserialize)]
struct FixtureSource {
    kind: String,
    id: String,
    payload: Value,
}

#[tokio::test]
async fn precedence_matrix_resolves_deterministically() {
    let fixture_raw = std::fs::read_to_string(fixture_path("compat/precedence_v1.json"))
        .expect("precedence fixture should exist");
    let fixture: PrecedenceFixture =
        serde_json::from_str(&fixture_raw).expect("precedence fixture must be valid JSON");

    let mut builder = ConfigBuilder::new().with_defaults(fixture.defaults);
    let mut loader = StaticFixtureLoader::default();

    for source in &fixture.sources {
        let config_source = match source.kind.as_str() {
            "file" => ConfigSource::File(source.id.clone().into()),
            "file_auto" => ConfigSource::FileAuto(source.id.clone().into()),
            "env_with_prefix" => ConfigSource::EnvWithPrefix(source.id.clone()),
            other => panic!("unsupported fixture source kind: {other}"),
        };
        builder = builder.with_source(config_source.clone());
        loader = loader.with_payload(config_source, source.payload.clone());
    }

    let config = builder
        .with_loader(Arc::new(loader))
        .build()
        .await
        .expect("fixture config should build");

    let actual = config
        .get_raw(None)
        .await
        .expect("resolved config should be readable");
    assert_eq!(actual, fixture.expected);
}
