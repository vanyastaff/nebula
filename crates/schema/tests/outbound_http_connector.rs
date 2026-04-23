//! See `examples_include/outbound_http_connector_shared.rs`.

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples_include/outbound_http_connector_shared.rs"
));

use nebula_schema::FieldValues;
use serde_json::json;

#[test]
fn outbound_schema_builds() {
    let s = build_outbound_http_connector_schema();
    assert!(s.fields().len() >= 10);
}

#[test]
fn minimal_get_validates() {
    let s = build_outbound_http_connector_schema();
    let v = json!({
        "base_url": "https://api.example.com",
        "http_method": "GET",
        "path": "/v1/ok",
        "auth": { "mode": "none" },
        "body": { "mode": "none" },
        "query": { "params": [] },
        "request_signing": { "mode": "none" },
    });
    assert!(s.validate(&FieldValues::from_json(v).unwrap()).is_ok());
}

#[test]
fn post_with_bearer_and_json_body_validates() {
    let s = build_outbound_http_connector_schema();
    let v = json!({
        "base_url": "https://partner.example",
        "http_method": "POST",
        "path": "/in",
        "auth": { "mode": "bearer", "value": { "token": "bearersecrettokenthing" } },
        "body": { "mode": "json", "value": { "json_body": "{}" } },
        "query": { "params": [] },
        "request_signing": { "mode": "none" },
    });
    assert!(s.validate(&FieldValues::from_json(v).unwrap()).is_ok());
}

#[test]
fn form_body_validates() {
    let s = build_outbound_http_connector_schema();
    let v = json!({
        "base_url": "https://form.example",
        "http_method": "POST",
        "path": "/form",
        "auth": { "mode": "none" },
        "body": { "mode": "form_urlencoded", "value": [
            { "name": "a", "value": "1" }
        ] },
        "query": { "params": [] },
        "request_signing": { "mode": "none" },
    });
    let values = FieldValues::from_json(v).expect("ingest");
    assert!(s.validate(&values).is_ok());
}
