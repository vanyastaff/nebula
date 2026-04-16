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
                .variant("none", "None", Field::hidden("none_payload"))
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

#[test]
fn telegram_schema_validates_resource_operation_flow() {
    let schema = telegram_send_message_schema();
    let mut values = FieldValues::new();
    values.set("resource", json!("message"));
    values.set("operation", json!("sendMessage"));
    values.set("text", json!("Hello from Nebula"));
    values.set("api_key", json!("sk_test_1234567890abcdef"));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(!report.has_errors());
}

#[test]
fn http_schema_rejects_invalid_url() {
    let schema = http_request_schema();
    let mut values = FieldValues::new();
    values.set("method", json!("GET"));
    values.set("url", json!("not-a-url"));
    values.set("auth", json!({ "mode": "none" }));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert_eq!(report.errors()[0].key, "url");
}

#[test]
fn oauth_schema_list_rules_are_enforced() {
    let schema = oauth2_credential_schema();
    let mut values = FieldValues::new();
    values.set("grant_type", json!("client_credentials"));
    values.set("client_secret", json!("top-secret-value"));
    values.set("scopes", json!([]));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert_eq!(report.errors()[0].key, "scopes");
}
