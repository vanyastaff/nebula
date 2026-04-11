use std::{collections::BTreeMap, fs, path::PathBuf};

use nebula_log::{Config, DestinationFailurePolicy, Fields, Format, WriterConfig};
use serde_json::Value;

#[test]
fn config_schema_snapshot_default_roundtrip() {
    let expected = load_fixture_json("schema_v1_default_snapshot.json");
    let actual = serde_json::to_value(stable_default_config()).expect("serialize default config");
    assert_eq!(actual, expected, "default schema snapshot drifted");

    let roundtrip: Config =
        serde_json::from_value(expected.clone()).expect("deserialize default snapshot");
    roundtrip
        .ensure_compatible()
        .expect("default snapshot must be schema compatible");
    let reserialized =
        serde_json::to_value(roundtrip).expect("re-serialize default roundtrip config");
    assert_eq!(
        reserialized, expected,
        "default schema roundtrip shape drifted"
    );
}

#[test]
fn config_schema_snapshot_non_default_roundtrip() {
    let expected = load_fixture_json("schema_v1_non_default_snapshot.json");
    let actual =
        serde_json::to_value(representative_non_default_config()).expect("serialize non-default");
    assert_eq!(actual, expected, "non-default schema snapshot drifted");

    let roundtrip: Config =
        serde_json::from_value(expected.clone()).expect("deserialize non-default snapshot");
    roundtrip
        .ensure_compatible()
        .expect("non-default snapshot must be schema compatible");
    let reserialized =
        serde_json::to_value(roundtrip).expect("re-serialize non-default roundtrip config");
    assert_eq!(
        reserialized, expected,
        "non-default schema roundtrip shape drifted"
    );
}

#[test]
fn config_schema_snapshot_rejects_unsupported_version_fixture() {
    let raw = load_fixture_text("schema_v999_unsupported_snapshot.json");
    let config: Config =
        serde_json::from_str(&raw).expect("unsupported schema fixture must parse as Config");
    let err = config
        .ensure_compatible()
        .expect_err("unsupported schema fixture must fail compatibility check");
    assert!(format!("{err}").contains("Unsupported config schema version"));
}

fn stable_default_config() -> Config {
    let mut cfg = Config::default();
    // Avoid environment/runtime-dependent defaults in snapshot contract.
    cfg.display.source = false;
    cfg.display.colors = false;
    cfg
}

fn representative_non_default_config() -> Config {
    let mut cfg = stable_default_config();
    cfg.level = "debug,hyper=warn".to_string();
    cfg.format = Format::Json;
    cfg.writer = WriterConfig::Multi {
        policy: DestinationFailurePolicy::PrimaryWithFallback,
        writers: vec![WriterConfig::Stdout, WriterConfig::Stderr],
    };
    cfg.display.time_format = Some("%Y-%m-%dT%H:%M:%S%.3fZ".to_string());
    cfg.display.source = true;
    cfg.display.target = true;
    cfg.display.thread_ids = true;
    cfg.display.thread_names = true;
    cfg.display.span_list = false;
    cfg.display.flatten = false;
    cfg.reloadable = true;

    let mut custom = BTreeMap::new();
    custom.insert("shard".to_string(), serde_json::json!(3));
    custom.insert("tenant".to_string(), serde_json::json!("acme"));
    cfg.fields = Fields {
        service: Some("nebula-api".to_string()),
        env: Some("prod".to_string()),
        version: Some("1.2.3".to_string()),
        instance: Some("i-abc123".to_string()),
        region: Some("us-east-1".to_string()),
        custom,
    };

    cfg
}

fn load_fixture_text(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("read schema snapshot fixture")
}

fn load_fixture_json(name: &str) -> Value {
    serde_json::from_str(&load_fixture_text(name)).expect("schema snapshot fixture must be valid")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("config")
        .join(name)
}
