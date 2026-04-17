use nebula_schema::{ExecutionMode, Field, FieldValues, Schema};
use serde_json::json;

fn telegram_send_message_schema() -> Schema {
    Schema::new()
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
}

fn http_request_schema() -> Schema {
    Schema::new()
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
}

fn oauth2_credential_schema() -> Schema {
    Schema::new()
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
}

fn nested_object_schema() -> Schema {
    Schema::new().add(
        Field::object("config")
            .add(Field::string("host").required())
            .add(Field::number("port").required()),
    )
}

#[test]
fn telegram_schema_validates_resource_operation_flow() {
    let schema = telegram_send_message_schema();
    let mut values = FieldValues::new();
    values.set_raw("resource", json!("message"));
    values.set_raw("operation", json!("sendMessage"));
    values.set_raw("text", json!("Hello from Nebula"));
    values.set_raw("api_key", json!("sk_test_1234567890abcdef"));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(!report.has_errors());
}

#[test]
fn http_schema_rejects_invalid_url() {
    let schema = http_request_schema();
    let mut values = FieldValues::new();
    values.set_raw("method", json!("GET"));
    values.set_raw("url", json!("not-a-url"));
    values.set_raw("auth", json!({ "mode": "none" }));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert_eq!(report.errors()[0].key, "url");
}

#[test]
fn oauth_schema_list_rules_are_enforced() {
    let schema = oauth2_credential_schema();
    let mut values = FieldValues::new();
    values.set_raw("grant_type", json!("client_credentials"));
    values.set_raw("client_secret", json!("top-secret-value"));
    values.set_raw("scopes", json!([]));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert_eq!(report.errors()[0].key, "scopes");
}

#[test]
fn mode_variant_payload_is_validated() {
    let schema = http_request_schema();
    let mut values = FieldValues::new();
    values.set_raw("method", json!("GET"));
    values.set_raw("url", json!("https://example.com"));
    values.set_raw("auth", json!({ "mode": "bearer" }));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert_eq!(report.errors()[0].key, "auth.value");
}

#[test]
fn object_children_are_validated() {
    let schema = nested_object_schema();
    let mut values = FieldValues::new();
    values.set_raw("config", json!({ "host": "localhost" }));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert_eq!(report.errors()[0].key, "config.port");
}
