//! seam: `ValidValues` is mintable ONLY via the `nebula-schema`
//! pipeline (`ValidSchema::validate`). Moving condition evaluation into
//! `nebula-validator` must not add a back-door constructor. This pins the
//! proof-token custody contract referenced by.

use nebula_schema::{Field, FieldKey, FieldValue, FieldValues, Schema};
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

#[test]
fn valid_values_only_minted_by_validate() {
    // `name` requires `min_length(3)`, so a too-short value makes `validate`
    // return `Err` and a long-enough value makes it `Ok` — a checkable
    // validity boundary that proves minting is gated on validation.
    let schema = Schema::builder()
        .add(Field::string(fk("name")).min_length(3))
        .build()
        .expect("schema builds");

    // The ONLY way to obtain a `ValidValues` is `ValidSchema::validate`.
    let good = FieldValues::from_json(json!({ "name": "alice" })).unwrap();
    let vv = schema
        .validate(&good)
        .expect("valid input mints the proof token");

    // The token carries the schema it was minted from (public accessor only —
    // `ValidValues`'s inner field is `pub(crate)`, unreachable from this
    // external integration-test crate).
    assert_eq!(
        vv.schema().fields().len(),
        schema.fields().len(),
        "the proof token carries the ValidSchema it was minted from"
    );

    // The token round-trips the exact input it was minted from, so a
    // `ValidValues` cannot exist detached from the values that passed
    // `validate()`.
    assert_eq!(
        vv.raw().get(&fk("name")),
        Some(&FieldValue::Literal(json!("alice"))),
        "the proof token carries the values it was minted from"
    );

    // Invalid input yields NO token (`Err`), so a `ValidValues` cannot exist
    // without having passed `validate()`.
    let bad = FieldValues::from_json(json!({ "name": "ab" })).unwrap();
    assert!(
        schema.validate(&bad).is_err(),
        "invalid input must not mint a proof token"
    );
}
