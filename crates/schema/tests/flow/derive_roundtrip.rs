//! `#[derive(Schema)]` smoke: derive, validate and resolve.

use nebula_schema::{AuthoredValue, HasSchema, Schema};
use serde::Deserialize;
use serde_json::json;

#[derive(Schema, Deserialize)]
struct Demo {
    #[expect(dead_code, reason = "only schema validation is exercised")]
    name: String,
}

#[test]
fn derive_schema_validates_and_resolves() {
    let schema = Demo::schema().unwrap();
    assert_eq!(schema.properties().len(), 1);
    assert!(schema.root_rules().is_empty());

    let values = AuthoredValue::from_data(json!({"name": "ada"})).unwrap();
    let valid = schema.validate(values).unwrap();
    assert_eq!(valid.values().to_json(), json!({"name": "ada"}));
    assert_eq!(valid.pending(), []);
    assert_eq!(
        valid.resolve_data().unwrap().into_json(),
        json!({"name": "ada"})
    );
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
