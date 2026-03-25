//! Integration tests for the normalization engine.
//!
//! Covers: default backfilling, user values preserved,
//! mode default-variant selection, nested mode defaults.

use nebula_parameter::{Parameter, ParameterCollection};
use serde_json::json;

fn make_values(pairs: &[(&str, serde_json::Value)]) -> nebula_parameter::ParameterValues {
    let mut v = nebula_parameter::ParameterValues::new();
    for (k, val) in pairs {
        v.set(*k, val.clone());
    }
    v
}

// ── Default backfilling ──────────────────────────────────────────────────────

#[test]
fn default_is_applied_for_missing_field() {
    let collection = ParameterCollection::new().add(
        Parameter::string("region")
            .label("Region")
            .default(json!("us-east-1")),
    );
    let normalized = collection.normalize(&make_values(&[]));

    assert_eq!(normalized.get("region"), Some(&json!("us-east-1")));
}

#[test]
fn existing_value_is_not_overwritten_by_default() {
    let collection = ParameterCollection::new().add(
        Parameter::string("region")
            .label("Region")
            .default(json!("us-east-1")),
    );
    let normalized = collection.normalize(&make_values(&[("region", json!("eu-west-1"))]));

    assert_eq!(normalized.get("region"), Some(&json!("eu-west-1")));
}

#[test]
fn field_without_default_remains_absent() {
    let collection = ParameterCollection::new().add(Parameter::string("comment").label("Comment"));
    let normalized = collection.normalize(&make_values(&[]));

    assert_eq!(normalized.get("comment"), None);
}

#[test]
fn multiple_defaults_all_applied() {
    let collection = ParameterCollection::new()
        .add(
            Parameter::string("host")
                .label("Host")
                .default(json!("localhost")),
        )
        .add(
            Parameter::integer("port")
                .label("Port")
                .default(json!(5432)),
        )
        .add(Parameter::boolean("ssl").label("SSL").default(json!(false)));

    let normalized = collection.normalize(&make_values(&[]));

    assert_eq!(normalized.get("host"), Some(&json!("localhost")));
    assert_eq!(normalized.get("port"), Some(&json!(5432)));
    assert_eq!(normalized.get("ssl"), Some(&json!(false)));
}

#[test]
fn partial_values_only_missing_fields_get_defaults() {
    let collection = ParameterCollection::new()
        .add(
            Parameter::string("host")
                .label("Host")
                .default(json!("localhost")),
        )
        .add(
            Parameter::integer("port")
                .label("Port")
                .default(json!(5432)),
        );

    // user already provided host
    let normalized = collection.normalize(&make_values(&[("host", json!("db.example.com"))]));

    assert_eq!(normalized.get("host"), Some(&json!("db.example.com")));
    assert_eq!(normalized.get("port"), Some(&json!(5432)));
}

// ── Mode field defaults ──────────────────────────────────────────────────────

#[test]
fn mode_default_variant_key_is_injected_when_absent() {
    let collection = ParameterCollection::new().add(
        Parameter::mode("auth")
            .variant(Parameter::boolean("none").label("None"))
            .variant(
                Parameter::string("token")
                    .label("Token")
                    .label("Token Value"),
            )
            .default_variant("none"),
    );

    let normalized = collection.normalize(&make_values(&[]));
    let auth = normalized
        .get("auth")
        .expect("auth field should be present");
    assert_eq!(auth.get("mode").and_then(|v| v.as_str()), Some("none"));
}

#[test]
fn mode_nested_default_applied_when_variant_selected() {
    let collection = ParameterCollection::new().add(
        Parameter::mode("auth")
            .variant(
                Parameter::string("token")
                    .label("Token")
                    .default(json!("my-default-token")),
            )
            .default_variant("token"),
    );

    // auth object present but nested value absent
    let normalized = collection.normalize(&make_values(&[("auth", json!({ "mode": "token" }))]));

    let auth = normalized.get("auth").expect("auth present");
    assert_eq!(
        auth.get("value").and_then(|v| v.as_str()),
        Some("my-default-token")
    );
}

#[test]
fn mode_nested_value_not_overwritten_by_default() {
    let collection = ParameterCollection::new().add(
        Parameter::mode("auth")
            .variant(
                Parameter::string("token")
                    .label("Token")
                    .default(json!("default-token")),
            )
            .default_variant("token"),
    );

    let normalized = collection.normalize(&make_values(&[(
        "auth",
        json!({ "mode": "token", "value": "user-token" }),
    )]));

    let auth = normalized.get("auth").expect("auth present");
    assert_eq!(
        auth.get("value").and_then(|v| v.as_str()),
        Some("user-token")
    );
}

// ── Extra keys preserved ─────────────────────────────────────────────────────

#[test]
fn normalize_does_not_discard_extra_keys_in_values() {
    // normalize is not validation — extra keys should survive
    let collection = ParameterCollection::new().add(Parameter::string("name").label("Name"));
    let normalized = collection.normalize(&make_values(&[
        ("name", json!("Alice")),
        ("unrecognised", json!(true)),
    ]));

    assert_eq!(normalized.get("unrecognised"), Some(&json!(true)));
}
