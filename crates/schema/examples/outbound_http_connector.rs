//! A dense **outbound HTTP connector** example: auth and body as `mode` branches, header/query
//! lists, retry object, optional HMAC signing, and a string list filter — useful to stress-test
//! UI generation and validation without a real third-party spec.
//!
//! Run: `cargo run -p nebula-schema --example outbound_http_connector`

#![expect(
    clippy::print_stderr,
    reason = "example: errors are reported to stderr"
)]

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples_include/outbound_http_connector_shared.rs"
));

use nebula_schema::AuthoredValue;
use serde_json::json;

fn main() {
    let schema = build_outbound_http_connector_schema();
    eprintln!("Schema: {} top-level field(s)", schema.fields().len());

    let full = json!({
        "base_url": "https://hooks.partner.example",
        "http_method": "POST",
        "path": "/v2/events/ingest",
        "auth": { "mode": "api_key_header", "value": {
            "header_name": "X-Api-Key",
            "api_key_value": "supersecretkeyatleast8chars"
        }},
        "headers": [
            { "name": "X-Correlation-Id", "value": "ulid-here" }
        ],
        "body": { "mode": "json", "value": "{\n  \"hello\": \"world\"\n}" },
        "query": { "params": [ { "name": "debug", "value": "0" } ] },
        "timeout_ms": 15000,
        "retry": { "max_attempts": 5, "initial_backoff_ms": 200 },
        "request_signing": { "mode": "hmac_sha256", "value": {
            "secret": "signingkeymustbeeightplus",
            "header_name": "X-Signature"
        }},
        "include_event_types": [ "order.paid", "user.created" ]
    });

    let values = AuthoredValue::from_data(full).expect("ingest");
    let resolved = schema
        .validate(values)
        .expect("full connector payload should validate")
        .resolve_data()
        .expect("full connector payload completes");
    assert_eq!(
        resolved.get(&field_key!("http_method")),
        Some(&json!("POST"))
    );
    assert_eq!(
        resolved.to_wire_json()["auth"]["value"],
        json!({"header_name": "X-Api-Key"})
    );

    let minimal = json!({
        "base_url": "https://api.example.com",
        "http_method": "GET",
        "path": "/health",
        "auth": { "mode": "none" },
        "body": { "mode": "none" },
        "query": { "params": [] },
        "request_signing": { "mode": "none" },
    });
    let values = AuthoredValue::from_data(minimal).expect("ingest");
    let resolved = schema
        .validate(values)
        .expect("minimal GET without lists")
        .resolve_data()
        .expect("minimal connector payload completes");
    assert_eq!(resolved.get(&field_key!("path")), Some(&json!("/health")));

    eprintln!("OK: outbound HTTP connector example payloads validated");
}
