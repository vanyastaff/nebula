use nebula_schema::{Field, FieldValues, Schema};
use serde_json::json;

fn telegram_send_message_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::select("resource")
                .option("message", "Message")
                .option("chat", "Chat")
                .required(),
        )
        .add(
            Field::select("operation")
                .option("sendMessage", "Send Message")
                .option("sendPhoto", "Send Photo")
                .required(),
        )
        .add(
            Field::string("text")
                .min_length(1)
                .max_length(4096)
                .active_when(nebula_validator::Rule::Eq {
                    field: "operation".to_owned(),
                    value: json!("sendMessage"),
                }),
        )
        .add(
            Field::secret("api_key")
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
            Field::select("method")
                .option("GET", "GET")
                .option("POST", "POST")
                .option("PUT", "PUT")
                .required(),
        )
        .add(Field::string("url").required().url())
        .add(
            Field::mode("auth")
                // "hidden" role: use VisibilityMode::Never (hidden variant removed)
                .variant(
                    "none",
                    "None",
                    Field::string("none_payload")
                        .visible(nebula_schema::VisibilityMode::Never),
                )
                .variant(
                    "bearer",
                    "Bearer",
                    Field::secret("token").required().min_length(8),
                )
                .default_variant("none"),
        )
        .build()
        .expect("valid http schema")
}

fn oauth2_credential_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::select("grant_type")
                .option("client_credentials", "Client Credentials")
                .option("authorization_code", "Authorization Code")
                .required(),
        )
        .add(
            Field::secret("client_secret")
                .required()
                .multiline()
                .reveal_last(4),
        )
        .add(
            Field::list("scopes")
                .item(Field::string("scope").min_length(1))
                .min_items(1)
                .max_items(20),
        )
        .build()
        .expect("valid oauth2 schema")
}

fn nested_object_schema() -> nebula_schema::ValidSchema {
    Schema::builder()
        .add(
            Field::object("config")
                .add(Field::string("host").required())
                .add(Field::number("port").required()),
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
    assert!(report.errors().any(|e| e.path.to_string() == "url"));
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
            .any(|e| e.path.to_string() == "scopes" && e.code == "items.min")
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
        report.errors().any(|e| e.path.to_string().contains("auth")),
        "expected required error under auth, got: {:?}",
        report
            .errors()
            .map(|e| (&e.code, e.path.to_string()))
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
    assert!(report.errors().any(|e| e.path.to_string() == "config.port"));
}
