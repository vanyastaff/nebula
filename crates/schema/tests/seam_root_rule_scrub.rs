//! ADR-0052 P2 item-2: root-rule predicates run against a context scrubbed of
//! `Field::Secret` (by schema type, recursively) — BUT legal non-secret nested
//! values (object / list-item / mode-variant) remain addressable so a
//! legitimate root guard does NOT fail open.
//!
//! Two guards:
//!  - `legal_non_secret_nested_root_predicate_still_fires_after_scrub` proves a
//!    legitimate nested-object root predicate still resolves post-scrub (it
//!    would also resolve under the old `from_json`, so it is a true regression
//!    guard against the scrub silently nuking legal nested context).
//!  - `root_predicate_cannot_read_scrubbed_secret_plaintext` unit-tests the new
//!    `root_predicate_context_for` directly: the secret's path is absent, no
//!    pushed value carries the plaintext, and a container object that has a
//!    secret descendant is never emitted as a node (so a `Contains("/cfg", …)`
//!    over the parent blob cannot read the secret).

use nebula_schema::context::root_predicate_context_for;
use nebula_schema::{Field, FieldValues, Schema, field_key};
use nebula_validator::Rule;
use nebula_validator::foundation::FieldPath as ValidatorPath;
use serde_json::json;

#[test]
fn legal_non_secret_nested_root_predicate_still_fires_after_scrub() {
    // Root guard: if `/policy/region == "eu"` then `dpa` must be set.
    // Encoded as the implication `¬P ∨ Q` = Any[ Not(Eq(region,"eu")), Set(dpa) ].
    // `region` lives inside a nested non-secret Field::Object and `dpa` is a
    // sibling non-secret String — NO secret anywhere, so the scrub must keep
    // `/policy/region` addressable (else the guard fails open for all input).
    let schema = Schema::builder()
        .add(Field::object(field_key!("policy")).add(Field::string(field_key!("region"))))
        .add(Field::string(field_key!("dpa")))
        .root_rule(Rule::any([
            Rule::not(Rule::Predicate(nebula_validator::Predicate::Eq(
                ValidatorPath::parse("/policy/region").unwrap(),
                json!("eu"),
            ))),
            Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/dpa").unwrap(),
            )),
        ]))
        .build()
        .expect("schema with a legal non-secret nested root predicate must build");

    // region == "eu" and dpa missing → guard MUST fire (Err). If the scrub
    // wiped /policy/region the predicate would see absent → Not(Eq)=true →
    // Any passes → fail-OPEN. This asserts it does NOT.
    let missing_dpa = FieldValues::from_json(json!({ "policy": { "region": "eu" } })).unwrap();
    let report = schema
        .validate(&missing_dpa)
        .expect_err("eu region without dpa must be rejected (root guard fires post-scrub)");
    assert!(
        report.errors().count() > 0,
        "expected a root-rule failure, got no errors"
    );

    // region == "eu" and dpa present → guard satisfied → Ok.
    let with_dpa =
        FieldValues::from_json(json!({ "policy": { "region": "eu" }, "dpa": "signed" })).unwrap();
    schema
        .validate(&with_dpa)
        .expect("eu region with dpa present must pass");

    // region != "eu" → antecedent false → guard vacuously satisfied → Ok.
    let non_eu = FieldValues::from_json(json!({ "policy": { "region": "us" } })).unwrap();
    schema
        .validate(&non_eu)
        .expect("non-eu region must pass regardless of dpa");
}

#[test]
fn root_predicate_cannot_read_scrubbed_secret_plaintext() {
    const PLAINTEXT: &str = "s3cr3t-root-plaintext";

    // (1) Top-level Field::Secret with a pre-resolve plaintext literal: the
    //     scrubbed root context must NOT expose it under any pointer.
    let secret_fields = vec![Field::from(Field::secret(field_key!("api_key")))];
    let secret_values = FieldValues::from_json(json!({ "api_key": PLAINTEXT })).unwrap();
    let ctx = root_predicate_context_for(&secret_fields, &secret_values);
    assert!(
        ctx.get(&ValidatorPath::parse("/api_key").unwrap())
            .is_none(),
        "secret-typed field must be absent from the scrubbed root context"
    );
    assert!(
        !format!("{ctx:?}").contains(PLAINTEXT),
        "redacted Debug must never carry the plaintext"
    );

    // (2) A Field::Object `cfg` whose child is a Field::Secret. The walker
    //     visits only the secret LEAF (skipped); the parent object node must
    //     never be emitted, or a `Contains("/cfg", "<substr>")` predicate could
    //     read the secret out of the serialized blob.
    let nested = Field::object(field_key!("cfg")).add(Field::secret(field_key!("the_secret")));
    let nested_fields = vec![Field::from(nested)];
    let nested_values =
        FieldValues::from_json(json!({ "cfg": { "the_secret": PLAINTEXT } })).unwrap();
    let ctx = root_predicate_context_for(&nested_fields, &nested_values);

    for ptr in ["/cfg", "/cfg/the_secret", "/the_secret"] {
        assert!(
            ctx.get(&ValidatorPath::parse(ptr).unwrap()).is_none(),
            "scrubbed root context must not expose {ptr} (secret descendant)"
        );
    }
    assert!(
        !format!("{ctx:?}").contains(PLAINTEXT),
        "no pushed value (and no Debug) may carry the secret plaintext"
    );

    // (3) Structural guarantee: a sibling NON-secret leaf next to the secret
    //     IS still emitted (proves the scrub is type-targeted, not blanket —
    //     it does not over-remove legal context, which would fail open).
    let mixed = Field::object(field_key!("cfg2"))
        .add(Field::secret(field_key!("token")))
        .add(Field::string(field_key!("region")));
    let mixed_fields = vec![Field::from(mixed)];
    let mixed_values =
        FieldValues::from_json(json!({ "cfg2": { "token": PLAINTEXT, "region": "eu" } })).unwrap();
    let ctx = root_predicate_context_for(&mixed_fields, &mixed_values);
    assert_eq!(
        ctx.get(&ValidatorPath::parse("/cfg2/region").unwrap()),
        Some(&json!("eu")),
        "non-secret sibling leaf must remain addressable (no over-scrub fail-open)"
    );
    assert!(
        ctx.get(&ValidatorPath::parse("/cfg2/token").unwrap())
            .is_none(),
        "secret sibling must still be scrubbed"
    );
    assert!(
        !format!("{ctx:?}").contains(PLAINTEXT),
        "redacted Debug must never carry the plaintext"
    );
}
