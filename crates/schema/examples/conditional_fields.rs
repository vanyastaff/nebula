//! Conditional fields with `active_when`: a secret is only visible and required
//! when the user picks a matching select option.
//!
//! Run:
//! `cargo run -p nebula-schema --example conditional_fields`

#![expect(
    clippy::print_stderr,
    reason = "example: errors are reported to stderr"
)]

use nebula_schema::prelude::*;
use serde_json::json;

fn main() {
    let schema = Schema::builder()
        .add(
            Field::select(field_key!("auth_type"))
                .option("api_key", "API key")
                .option("oauth2", "OAuth2")
                .required(),
        )
        .add(
            Field::secret(field_key!("api_key")).active_when(
                Rule::predicate(Predicate::eq("auth_type", json!("api_key")).expect("predicate"))
                    .expect("bounded API key visibility rule"),
            ),
        )
        .add(
            Field::string(field_key!("client_id")).active_when(
                Rule::predicate(Predicate::eq("auth_type", json!("oauth2")).expect("predicate"))
                    .expect("bounded OAuth visibility rule"),
            ),
        )
        .build()
        .expect("schema should lint");

    // API key path: secret must be present when active.
    let wire = json!({
        "auth_type": "api_key",
        "api_key": "s3cr3t",
    });
    let values = AuthoredValue::from_data(wire).expect("wire");
    let resolved = schema
        .validate(values)
        .expect("values should validate for api_key flow")
        .resolve_data()
        .expect("api_key flow should satisfy conditional policies");
    assert_eq!(resolved.to_wire_json(), json!({"auth_type": "api_key"}));

    // OAuth path: client_id required when branch is active.
    let wire = json!({
        "auth_type": "oauth2",
        "client_id": "my-app",
    });
    let values = AuthoredValue::from_data(wire).expect("wire");
    let resolved = schema
        .validate(values)
        .expect("values should validate for oauth2 flow")
        .resolve_data()
        .expect("oauth2 flow should satisfy conditional policies");
    assert_eq!(
        resolved.to_wire_json(),
        json!({"auth_type": "oauth2", "client_id": "my-app"})
    );

    eprintln!(
        "OK: conditional schema has {} field(s)",
        schema.fields().len()
    );
}
