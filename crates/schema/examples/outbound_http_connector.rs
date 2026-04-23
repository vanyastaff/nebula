//! A dense **outbound HTTP connector** example: auth and body as `mode` branches, header/query
//! lists, retry object, optional HMAC signing, and a string list filter — useful to stress-test
//! UI generation and validation without a real third-party spec.
//!
//! Run: `cargo run -p nebula-schema --example outbound_http_connector`

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples_include/outbound_http_connector_shared.rs"
));

use nebula_schema::FieldValues;
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
        "body": { "mode": "json", "value": { "json_body": "{\n  \"hello\": \"world\"\n}" } },
        "query": { "params": [ { "name": "debug", "value": "0" } ] },
        "timeout_ms": 15000,
        "retry": { "max_attempts": 5, "initial_backoff_ms": 200 },
        "request_signing": { "mode": "hmac_sha256", "value": {
            "secret": "signingkeymustbeeightplus",
            "header_name": "X-Signature"
        }},
        "include_event_types": [ "order.paid", "user.created" ]
    });

    let v = FieldValues::from_json(full).expect("ingest");
    schema
        .validate(&v)
        .expect("full connector payload should validate");

    let minimal = json!({
        "base_url": "https://api.example.com",
        "http_method": "GET",
        "path": "/health",
        "auth": { "mode": "none" },
        "body": { "mode": "none" },
        "query": { "params": [] },
        "request_signing": { "mode": "none" },
    });
    let v = FieldValues::from_json(minimal).expect("ingest");
    schema.validate(&v).expect("minimal GET without lists");

    eprintln!("OK: outbound HTTP connector example payloads validated");
}
