use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

// Test helper: render a canonical RFC-6901 field pointer (`/a/b/0`) back into
// the schema's dotted/bracketed display (`a.b[0]`) so historical path
// assertions keep their original form after the nebula-error migration.
fn field_dotted(e: &nebula_schema::ValidationError) -> String {
    let Some(pointer) = e.field.as_deref() else {
        return String::new();
    };
    let mut out = String::new();
    for seg in pointer.trim_start_matches('/').split('/') {
        if seg.is_empty() {
            continue;
        }
        let unescaped = seg.replace("~1", "/").replace("~0", "~");
        if unescaped.chars().all(|c| c.is_ascii_digit()) {
            out.push('[');
            out.push_str(&unescaped);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(&unescaped);
        }
    }
    out
}

fn telegram_send_message_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::select(field_key!("resource"))
                .option("message", "Message")
                .option("chat", "Chat")
                .required(),
        )
        .add(
            Field::select(field_key!("operation"))
                .option("sendMessage", "Send Message")
                .option("sendPhoto", "Send Photo")
                .required(),
        )
        .add(
            Field::string(field_key!("text"))
                .min_length(1)
                .max_length(4096)
                .active_when(nebula_validator::Rule::predicate(
                    nebula_validator::Predicate::eq("operation", json!("sendMessage")).unwrap(),
                )),
        )
        .add(
            Field::secret(field_key!("api_key"))
                .required()
                .min_length(20)
                .reveal_last(4),
        )
        .build()
        .expect("valid telegram schema")
}

fn http_request_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::select(field_key!("method"))
                .option("GET", "GET")
                .option("POST", "POST")
                .option("PUT", "PUT")
                .required(),
        )
        .add(Field::string(field_key!("url")).required().url())
        .add(
            Field::mode(field_key!("auth"))
                // "hidden" role: use VisibilityMode::Never (hidden variant removed)
                .variant(
                    "none",
                    "None",
                    Field::string(field_key!("none_payload"))
                        .visible(nebula_schema::VisibilityMode::Never),
                )
                .variant(
                    "bearer",
                    "Bearer",
                    Field::secret(field_key!("token")).required().min_length(8),
                )
                .default_variant("none"),
        )
        .build()
        .expect("valid http schema")
}

fn oauth2_credential_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::select(field_key!("grant_type"))
                .option("client_credentials", "Client Credentials")
                .option("authorization_code", "Authorization Code")
                .required(),
        )
        .add(
            Field::secret(field_key!("client_secret"))
                .required()
                .multiline()
                .reveal_last(4),
        )
        .add(
            Field::list(field_key!("scopes"))
                .item(Field::string(field_key!("scope")).min_length(1))
                .min_items(1)
                .max_items(20),
        )
        .build()
        .expect("valid oauth2 schema")
}

fn nested_object_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::object(field_key!("config"))
                .add(Field::string(field_key!("host")).required())
                .add(Field::number(field_key!("port")).required()),
        )
        .build()
        .expect("valid nested schema")
}

#[test]
fn telegram_schema_validates_resource_operation_flow() {
    let schema = telegram_send_message_schema();
    let values = FieldValues::from_json(json!({
        "resource": "message",
        "operation": "sendMessage",
        "text": "Hello from Nebula",
        "api_key": "sk_test_1234567890abcdef"
    }))
    .unwrap();

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn http_schema_rejects_invalid_url() {
    let schema = http_request_schema();
    let values = FieldValues::from_json(json!({
        "method": "GET",
        "url": "not-a-url",
        "auth": { "mode": "none" }
    }))
    .unwrap();

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(report.errors().any(|e| field_dotted(e) == "url"));
}

#[test]
fn oauth_schema_list_rules_are_enforced() {
    let schema = oauth2_credential_schema();
    let values = FieldValues::from_json(json!({
        "grant_type": "client_credentials",
        "client_secret": "top-secret-value",
        "scopes": []
    }))
    .unwrap();

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(
        report
            .errors()
            .any(|e| field_dotted(e) == "scopes" && e.code == "items.min")
    );
}

#[test]
fn mode_variant_payload_is_validated() {
    let schema = http_request_schema();
    let values = FieldValues::from_json(json!({
        "method": "GET",
        "url": "https://example.com",
        "auth": { "mode": "bearer" }
    }))
    .unwrap();

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    // The bearer token is required — the error path is auth.value (the mode payload slot)
    assert!(
        report.errors().any(|e| field_dotted(e).contains("auth")),
        "expected required error under auth, got: {:?}",
        report
            .errors()
            .map(|e| (&e.code, field_dotted(e)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn object_children_are_validated() {
    let schema = nested_object_schema();
    let values = FieldValues::from_json(json!({
        "config": { "host": "localhost" }
    }))
    .unwrap();

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(report.errors().any(|e| field_dotted(e) == "config.port"));
}
