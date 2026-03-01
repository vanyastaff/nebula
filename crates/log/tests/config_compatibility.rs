use nebula_log::{Config, DestinationFailurePolicy, WriterConfig};
use std::fs;
use std::path::PathBuf;

#[test]
fn writer_policy_serialization_shape_is_stable() {
    let cfg = WriterConfig::Multi {
        policy: DestinationFailurePolicy::PrimaryWithFallback,
        writers: vec![WriterConfig::Stdout],
    };

    let json = serde_json::to_string(&cfg).expect("serialize writer config");
    assert!(json.contains("primary_with_fallback"));
}

#[test]
fn loads_supported_config_fixture() {
    let fixture = fixture_path("v1-basic.json");
    let content = fs::read_to_string(fixture).expect("read fixture");
    let config: Config = serde_json::from_str(&content).expect("parse config fixture");
    config
        .ensure_compatible()
        .expect("fixture schema should be compatible");
}

#[test]
fn rejects_unsupported_schema_version() {
    let config = Config {
        schema_version: 999,
        ..Config::default()
    };
    let err = config
        .ensure_compatible()
        .expect_err("unsupported schema version must fail");
    assert!(format!("{err}").contains("Unsupported config schema version"));
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("config")
        .join(name)
}
