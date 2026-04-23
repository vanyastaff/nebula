// Fictional “outbound HTTP connector” integration: URL, method, auth modes, custom headers,
// body modes (JSON / form), HMAC signing, retry policy, and a filter list — for exercising
// object/list/mode composition without tying to a specific vendor.
//
// Included from `examples/outbound_http_connector.rs` and `tests/outbound_http_connector.rs`.

use nebula_schema::{Field, Schema, ValidSchema, field_key};

fn auth_mode() -> Field {
    Field::mode(field_key!("auth"))
        .label("How to authenticate the outbound request")
        .variant_empty("none", "No auth")
        .variant(
            "bearer",
            "Authorization: Bearer …",
            Field::secret(field_key!("token"))
                .required()
                .min_length(4)
                .label("Access token"),
        )
        .variant(
            "api_key_header",
            "API key in a header",
            Field::object(field_key!("api_key_in_header"))
                .add(
                    Field::string(field_key!("header_name"))
                        .label("Header name")
                        .default(serde_json::json!("X-API-Key"))
                        .required(),
                )
                .add(
                    Field::secret(field_key!("api_key_value"))
                        .label("Key value")
                        .required(),
                ),
        )
        .variant(
            "basic",
            "HTTP Basic",
            Field::object(field_key!("basic_auth"))
                .add(Field::string(field_key!("username")).required().label("Username"))
                .add(
                    Field::secret(field_key!("password"))
                        .required()
                        .min_length(1)
                        .label("Password"),
                ),
        )
        .default_variant("none")
        .into()
}

fn body_mode() -> Field {
    Field::mode(field_key!("body"))
        .label("Entity body")
        .variant_empty("none", "Empty")
        .variant(
            "json",
            "JSON (code editor)",
            Field::code(field_key!("json_body"))
                .label("JSON")
                .language("json")
                .description("Serialized as the raw request body (UTF-8)"),
        )
        .variant(
            "form_urlencoded",
            "Form (application/x-www-form-urlencoded)",
            // As the sole `mode` payload, wire is a **JSON array** of objects (not `{form_fields: …}`).
            Field::list(field_key!("form_fields"))
                .label("Fields")
                .item(
                    Field::object(field_key!("field"))
                        .add(
                            Field::string(field_key!("name"))
                                .required()
                                .max_length(256),
                        )
                        .add(Field::string(field_key!("value")).max_length(4096)),
                )
                .min_items(1)
                .max_items(64),
        )
        .default_variant("none")
        .into()
}

fn signing_mode() -> Field {
    Field::mode(field_key!("request_signing"))
        .label("Request signing (optional, e.g. webhooks HMAC)")
        .variant_empty("none", "None")
        .variant(
            "hmac_sha256",
            "HMAC-SHA256 header",
            Field::object(field_key!("hmac_sha256"))
                .add(
                    Field::secret(field_key!("secret"))
                        .required()
                        .min_length(8)
                        .label("Shared secret"),
                )
                .add(
                    Field::string(field_key!("header_name"))
                        .label("Header to set")
                        .default(serde_json::json!("X-Integration-Signature"))
                        .max_length(128),
                )
                .description("Signature = hex(HMAC-SHA256(secret, raw_body_bytes)) — policy is illustrative"),
        )
        .default_variant("none")
        .into()
}

/// A heavier integration shape: many branches, optional subtrees, nested lists.
pub fn build_outbound_http_connector_schema() -> ValidSchema {
    Schema::builder()
        .add(
            Field::string(field_key!("base_url"))
                .label("Base URL")
                .description("e.g. https://api.partner.example; path is appended (no trailing slash required)")
                .required()
                .url(),
        )
        .add(
            Field::select(field_key!("http_method"))
                .label("Method")
                .option("GET", "GET")
                .option("POST", "POST")
                .option("PUT", "PUT")
                .option("PATCH", "PATCH")
                .option("DELETE", "DELETE")
                .required(),
        )
        .add(
            Field::string(field_key!("path"))
                .label("Path")
                .description("Appended to base_url; may contain `{template}` segments")
                .required()
                .max_length(512),
        )
        .add(auth_mode())
        .add(
            Field::list(field_key!("headers"))
                .label("Extra headers")
                .item(
                    Field::object(field_key!("header"))
                        .add(
                            Field::string(field_key!("name"))
                                .required()
                                .max_length(128)
                                .description("Header name (ASCII)"),
                        )
                        .add(Field::string(field_key!("value")).max_length(4096)),
                )
                .max_items(32),
        )
        .add(body_mode())
        .add(
            Field::object(field_key!("query"))
                .label("Query string parameters")
                .add(
                    Field::list(field_key!("params"))
                        .item(
                            Field::object(field_key!("param"))
                                .add(Field::string(field_key!("name")).required().max_length(128))
                                .add(Field::string(field_key!("value"))),
                        )
                        .max_items(40),
                ),
        )
        .add(
            Field::number(field_key!("timeout_ms"))
                .label("Request timeout (ms)")
                .min_int(100)
                .default_int(30_000),
        )
        .add(
            Field::object(field_key!("retry"))
                .label("Retry on transport errors")
                .add(
                    Field::number(field_key!("max_attempts"))
                        .label("Max attempts")
                        .min_int(1)
                        .max_int(8)
                        .default_int(3),
                )
                .add(
                    Field::number(field_key!("initial_backoff_ms"))
                        .label("First backoff (ms)")
                        .min_int(100)
                        .default_int(500),
                ),
        )
        .add(signing_mode())
        .add(
            Field::list(field_key!("include_event_types"))
                .label("Send only for these event types (empty = all)")
                .item(
                    Field::string(field_key!("_item"))
                        .min_length(1)
                        .max_length(64),
                )
                .max_items(50),
        )
        .build()
        .expect("outbound HTTP connector example schema lints")
}
