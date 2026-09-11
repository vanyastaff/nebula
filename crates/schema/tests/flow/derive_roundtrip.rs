//! `#[derive(Schema)]` smoke: struct-level `#[schema(custom = ...)]` + validate.

use std::assert_matches;

use nebula_schema::{AuthoredValue, HasSchema, Schema, validated::PendingValidation};
use serde::Deserialize;
use serde_json::json;

#[derive(Schema, Deserialize)]
#[schema(custom = "phase3_engine_stub")]
struct Demo {
    #[expect(dead_code, reason = "the custom rule prevents typed extraction")]
    name: String,
}

#[test]
fn derive_schema_custom_and_validate() {
    let schema = Demo::schema().unwrap();
    assert_eq!(schema.fields().len(), 1);
    assert_eq!(schema.root_rules().len(), 1);

    let values = AuthoredValue::from_data(json!({"name": "ada"})).unwrap();
    let valid = schema.validate(values).unwrap();
    assert_eq!(valid.values().to_json(), json!({"name": "ada"}));
    assert_matches!(valid.pending(), [PendingValidation::Rule { .. }]);
    let report = valid.resolve_data().unwrap_err();
    assert_eq!(report.errors().count(), 1);
    assert_eq!(
        report.errors().next().unwrap().code(),
        "evaluation_unavailable"
    );
    assert_eq!(report.errors().next().unwrap().path().to_string(), "");
}

#[test]
fn serde_default_on_struct_aligns_with_empty_json() {
    fn default_seven() -> i64 {
        7
    }

    /// `#[derive(Schema)]` does not inject serde defaults; pair
    /// `#[field(default = ...)]` with `#[serde(default = "...")]` when `{}`
    /// must deserialize to the same wire shape you validate against.
    #[derive(Schema, Deserialize)]
    struct WithSerdeDefault {
        #[serde(default = "default_seven")]
        #[field(default = 7)]
        n: i64,
    }

    let schema = WithSerdeDefault::schema().unwrap();
    let typed: WithSerdeDefault = serde_json::from_value(json!({})).unwrap();
    assert_eq!(typed.n, 7);

    let values = AuthoredValue::from_data(json!({"n": 7})).unwrap();
    let valid = schema.validate(values).unwrap();
    assert_eq!(valid.values().to_json(), json!({"n": 7}));
    let resolved = valid.resolve_data().unwrap();
    let roundtrip: WithSerdeDefault = resolved.into_typed().unwrap();
    assert_eq!(roundtrip.n, typed.n);
}
