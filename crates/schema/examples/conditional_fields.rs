//! Conditional fields with `active_when`: a secret is only visible and required
//! when the user picks a matching select option.
//!
//! Run:
//! `cargo run -p nebula-schema --example conditional_fields`

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
            Field::secret(field_key!("api_key")).active_when(Rule::predicate(
                Predicate::eq("auth_type", json!("api_key")).expect("predicate"),
            )),
        )
        .add(
            Field::string(field_key!("client_id")).active_when(Rule::predicate(
                Predicate::eq("auth_type", json!("oauth2")).expect("predicate"),
            )),
        )
        .build()
        .expect("schema should lint");

    // API key path: secret must be present when active.
    let wire = json!({
        "auth_type": "api_key",
        "api_key": "s3cr3t",
    });
    let values = FieldValues::from_json(wire).expect("wire");
    schema
        .validate(&values)
        .expect("values should validate for api_key flow");

    // OAuth path: client_id required when branch is active.
    let wire = json!({
        "auth_type": "oauth2",
        "client_id": "my-app",
    });
    let values = FieldValues::from_json(wire).expect("wire");
    schema
        .validate(&values)
        .expect("values should validate for oauth2 flow");

    eprintln!(
        "OK: conditional schema has {} field(s)",
        schema.fields().len()
    );
}
