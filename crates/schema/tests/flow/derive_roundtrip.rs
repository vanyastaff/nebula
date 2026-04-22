//! `#[derive(Schema)]` smoke: struct-level `#[schema(custom = ...)]` + validate.

use nebula_schema::{FieldValues, HasSchema, Schema};
use serde::Deserialize;
use serde_json::json;

#[derive(Schema, Deserialize)]
#[schema(custom = "phase3_engine_stub")]
struct Demo {
    #[allow(dead_code)]
    name: String,
}

#[test]
fn derive_schema_custom_and_validate() {
    let schema = Demo::schema();
    assert_eq!(schema.fields().len(), 1);
    assert_eq!(schema.root_rules().len(), 1);

    let values = FieldValues::from_json(json!({"name": "ada"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn serde_default_on_struct_aligns_with_empty_json() {
    fn default_seven() -> i64 {
        7
    }

    /// `#[derive(Schema)]` does not inject serde defaults; pair
    /// `#[param(default = ...)]` with `#[serde(default = "...")]` when `{}`
    /// must deserialize to the same wire shape you validate against.
    #[derive(Schema, Deserialize)]
    struct WithSerdeDefault {
        #[serde(default = "default_seven")]
        #[param(default = 7)]
        n: i64,
    }

    let schema = WithSerdeDefault::schema();
    let typed: WithSerdeDefault = serde_json::from_value(json!({})).unwrap();
    assert_eq!(typed.n, 7);

    let values = FieldValues::from_json(json!({"n": 7})).unwrap();
    assert!(schema.validate(&values).is_ok());
}
