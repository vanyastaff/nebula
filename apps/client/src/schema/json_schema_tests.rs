//! Credential type schemas as forms, read from the server's own credential types.

use super::{json_schema::UNION_FIELD, *};
use serde_json::json;

/// The `schema` of one credential type in the server snapshot the demo serves.
fn type_schema(key: &str) -> Value {
    let snapshot: Value =
        serde_json::from_str(include_str!("../demo/credential_types.json")).unwrap();
    snapshot["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|kind| kind["key"] == key)
        .unwrap()["schema"]
        .clone()
}

fn keys(form: &Form) -> Vec<&str> {
    form.fields.iter().map(|field| field.key.as_str()).collect()
}

#[test]
fn a_username_comes_before_its_password_and_the_password_is_secret() {
    let form = Form::from_json_schema(&type_schema("basic_auth"));

    assert!(!form.tagged_union);
    assert_eq!(keys(&form), ["username", "password"]);
    assert!(matches!(form.fields[0].kind, Kind::Text { .. }));
    assert!(matches!(form.fields[1].kind, Kind::Secret { .. }));
    assert!(
        form.fields
            .iter()
            .all(|field| field.required == Condition::Always)
    );
}

#[test]
fn a_secret_may_be_written_as_an_expression() {
    let form = Form::from_json_schema(&type_schema("api_key"));

    let key = &form.fields[0];
    assert_eq!(key.key, "api_key");
    assert!(matches!(key.kind, Kind::Secret { .. }));
    assert_eq!(key.expression, ExpressionMode::Allowed);
}

#[test]
fn a_union_of_grant_types_is_one_type_field_with_nothing_chosen() {
    let form = Form::from_json_schema(&type_schema("oauth2"));

    assert!(form.tagged_union);
    assert_eq!(keys(&form), [UNION_FIELD]);
    let Kind::Mode { variants, .. } = &form.fields[0].kind else {
        panic!("the union is a mode field");
    };
    let tags: Vec<&str> = variants
        .iter()
        .map(|variant| variant.key.as_str())
        .collect();
    assert_eq!(tags, ["authorization_code", "client_credentials"]);
    // `oneOf` declares no default, so the form does not pretend one was picked.
    assert_eq!(form.fields[0].initial(), Value::Null);
}

#[test]
fn credential_data_writes_literals_expressions_and_the_chosen_tag() {
    let basic = Form::from_json_schema(&type_schema("basic_auth"));
    let entries = json!({
        "username": {"type": "literal", "value": "etl"},
        "password": {"type": "expression", "expr": "{{ $env.SFTP_PASSWORD }}"},
    });
    assert_eq!(
        Value::Object(credential_data(&basic, entries.as_object().unwrap())),
        json!({"username": "etl", "password": {"$expr": "{{ $env.SFTP_PASSWORD }}"}})
    );

    let oauth = Form::from_json_schema(&type_schema("oauth2"));
    let entries = json!({
        UNION_FIELD: {"type": "literal", "value": {
            "mode": "client_credentials",
            "value": {"client_id": "app", "token_url": "https://id.example/token"},
        }},
    });
    assert_eq!(
        Value::Object(credential_data(&oauth, entries.as_object().unwrap())),
        json!({"client_credentials": {"client_id": "app", "token_url": "https://id.example/token"}})
    );
}
