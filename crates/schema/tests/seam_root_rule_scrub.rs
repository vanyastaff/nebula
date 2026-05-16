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
use nebula_schema::{Field, FieldValue, FieldValues, Schema, field_key};
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

    // (2) A Field::Object `cfg` whose only child is a Field::Secret. The
    //     container node MUST stay present (so a legal `Set("/cfg")` presence
    //     guard still resolves — dropping it is the fail-open class), but its
    //     secret child is stripped, so the node value carries no plaintext and
    //     the secret pointer is unreadable.
    let nested = Field::object(field_key!("cfg")).add(Field::secret(field_key!("the_secret")));
    let nested_fields = vec![Field::from(nested)];
    let nested_values =
        FieldValues::from_json(json!({ "cfg": { "the_secret": PLAINTEXT } })).unwrap();
    let ctx = root_predicate_context_for(&nested_fields, &nested_values);

    let cfg = ctx.get(&ValidatorPath::parse("/cfg").unwrap());
    assert!(
        cfg.is_some(),
        "the container node must stay present so a legal `Set(\"/cfg\")` guard \
         does not fail open"
    );
    assert!(
        !cfg.unwrap().to_string().contains(PLAINTEXT),
        "the present container node must be secret-stripped (no plaintext blob)"
    );
    for ptr in ["/cfg/the_secret", "/the_secret"] {
        assert!(
            ctx.get(&ValidatorPath::parse(ptr).unwrap()).is_none(),
            "scrubbed root context must not expose the secret at {ptr}"
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

// A legal root guard keyed on a non-secret WHOLE Field::Object's presence
// (`Set("/cfg")`). This resolved under the pre-scrub `from_json` (it stored the
// object node blob). A leaf-only scrub drops `/cfg` → `Set` is false →
// `Not(Set)` true → `Any` passes → the guard silently never fires (fail-OPEN).
// This asserts the secret-free whole-object node is still addressable.
#[test]
fn legal_whole_object_presence_root_guard_fires_after_scrub() {
    let schema = Schema::builder()
        .add(Field::object(field_key!("cfg")).add(Field::string(field_key!("region"))))
        .add(Field::string(field_key!("dpa")))
        .root_rule(Rule::any([
            Rule::not(Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/cfg").unwrap(),
            ))),
            Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/dpa").unwrap(),
            )),
        ]))
        .build()
        .expect("schema builds");

    // cfg present, dpa missing → guard MUST fire.
    let missing_dpa = FieldValues::from_json(json!({ "cfg": { "region": "eu" } })).unwrap();
    schema.validate(&missing_dpa).expect_err(
        "cfg present without dpa must be rejected (whole-object guard fires post-scrub)",
    );

    // cfg present, dpa present → satisfied.
    let with_dpa =
        FieldValues::from_json(json!({ "cfg": { "region": "eu" }, "dpa": "signed" })).unwrap();
    schema.validate(&with_dpa).expect("cfg + dpa must pass");

    // cfg absent → antecedent false → vacuously satisfied.
    let no_cfg = FieldValues::from_json(json!({})).unwrap();
    schema.validate(&no_cfg).expect("absent cfg must pass");
}

// Same fail-open class for a non-secret WHOLE Field::List presence guard.
#[test]
fn legal_whole_list_presence_root_guard_fires_after_scrub() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("items")).item(Field::string(field_key!("name"))))
        .add(Field::string(field_key!("dpa")))
        .root_rule(Rule::any([
            Rule::not(Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/items").unwrap(),
            ))),
            Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/dpa").unwrap(),
            )),
        ]))
        .build()
        .expect("schema builds");

    let missing_dpa = FieldValues::from_json(json!({ "items": ["a"] })).unwrap();
    schema.validate(&missing_dpa).expect_err(
        "items present without dpa must be rejected (whole-list guard fires post-scrub)",
    );

    let with_dpa = FieldValues::from_json(json!({ "items": ["a"], "dpa": "x" })).unwrap();
    schema.validate(&with_dpa).expect("items + dpa must pass");

    let no_items = FieldValues::from_json(json!({})).unwrap();
    schema.validate(&no_items).expect("absent items must pass");
}

// The whole-container fix must NOT re-open the secret leak: a secret-bearing
// container node stays present (so a legal presence guard resolves) but is
// secret-STRIPPED — the secret leaf is gone and the node blob carries no
// plaintext. Closing the presence-guard fail-open and keeping the blob leak
// closed are the SAME mechanism (prune secrets, then key as `from_json`).
#[test]
fn secret_bearing_container_blob_stays_unreadable_after_whole_container_fix() {
    const PLAINTEXT: &str = "s3cr3t-stays-closed";
    let fields = vec![Field::from(
        Field::object(field_key!("cfg"))
            .add(Field::secret(field_key!("api_key")))
            .add(Field::string(field_key!("region"))),
    )];
    let values =
        FieldValues::from_json(json!({ "cfg": { "api_key": PLAINTEXT, "region": "eu" } })).unwrap();
    let ctx = root_predicate_context_for(&fields, &values);

    // Container node present (legal `Set("/cfg")` resolves) but secret-stripped
    // (`Contains("/cfg", "<secret>")` cannot read the plaintext); secret leaf
    // gone; the non-secret sibling leaf still resolves (no over-scrub).
    let cfg = ctx.get(&ValidatorPath::parse("/cfg").unwrap());
    assert!(
        cfg.is_some(),
        "secret-bearing container node must stay present (presence guard must \
         not fail open)"
    );
    assert!(
        !cfg.unwrap().to_string().contains(PLAINTEXT),
        "the present container node must be secret-stripped"
    );
    assert!(
        ctx.get(&ValidatorPath::parse("/cfg/api_key").unwrap())
            .is_none(),
        "secret leaf must not be emitted"
    );
    assert_eq!(
        ctx.get(&ValidatorPath::parse("/cfg/region").unwrap()),
        Some(&json!("eu")),
        "non-secret sibling leaf must still resolve"
    );
    assert!(
        !format!("{ctx:?}").contains(PLAINTEXT),
        "redacted Debug must never carry the plaintext"
    );
}

// Parity documentation: list-item / array-element leaves were NOT addressable
// under the pre-scrub `from_json` (it stores arrays whole and never descends
// them). The scrub preserves that exactly — the whole list resolves, an
// indexed element leaf does not. This is intentional parity, not a regression.
#[test]
fn list_item_leaf_non_addressable_matches_from_json_parity() {
    let fields = vec![Field::from(Field::list(field_key!("items")).item(
        Field::object(field_key!("row")).add(Field::string(field_key!("region"))),
    ))];
    let values = FieldValues::from_json(json!({ "items": [ { "region": "eu" } ] })).unwrap();
    let ctx = root_predicate_context_for(&fields, &values);

    assert!(
        ctx.get(&ValidatorPath::parse("/items").unwrap()).is_some(),
        "the whole secret-free list node must resolve (from_json parity)"
    );
    assert!(
        ctx.get(&ValidatorPath::parse("/items/0/region").unwrap())
            .is_none(),
        "an array-element leaf is non-addressable, exactly as under from_json"
    );
}

// A legal PRESENCE guard (`Set`) on a SECRET-BEARING list. `Set`/`Empty` read
// no value and are explicitly blessed on secrets, so this guard is legal and
// resolved under the pre-scrub `from_json` (it keyed the whole array under
// `/creds`). The scrub must keep `/creds` present (secrets stripped from the
// items) so the guard still fires — dropping it is the fail-open class — while
// the stripped array carries no secret plaintext.
#[test]
fn legal_presence_guard_on_secret_bearing_list_fires_and_leaks_nothing() {
    const SECRET: &str = "s3cr3t-list-item-key";
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("creds")).item(
                Field::object(field_key!("cred"))
                    .add(Field::string(field_key!("region")))
                    .add(Field::secret(field_key!("api_key"))),
            ),
        )
        .add(Field::string(field_key!("dpa")))
        .root_rule(Rule::any([
            Rule::not(Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/creds").unwrap(),
            ))),
            Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/dpa").unwrap(),
            )),
        ]))
        .build()
        .expect("schema builds");

    // creds present, dpa missing → guard MUST fire (this is the exact
    // fail-open: a secret-bearing list silently dropped from the context).
    let missing_dpa =
        FieldValues::from_json(json!({ "creds": [ { "region": "eu", "api_key": SECRET } ] }))
            .unwrap();
    schema.validate(&missing_dpa).expect_err(
        "creds present without dpa must be rejected (secret-bearing list presence guard fires)",
    );

    // creds present, dpa present → satisfied.
    let with_dpa = FieldValues::from_json(
        json!({ "creds": [ { "region": "eu", "api_key": SECRET } ], "dpa": "signed" }),
    )
    .unwrap();
    schema.validate(&with_dpa).expect("creds + dpa must pass");

    // creds absent → antecedent false → vacuously satisfied.
    let no_creds = FieldValues::from_json(json!({})).unwrap();
    schema.validate(&no_creds).expect("absent creds must pass");

    // And the stripped list node leaks no secret plaintext.
    let fields = vec![Field::from(
        Field::list(field_key!("creds")).item(
            Field::object(field_key!("cred"))
                .add(Field::string(field_key!("region")))
                .add(Field::secret(field_key!("api_key"))),
        ),
    )];
    let ctx = root_predicate_context_for(&fields, &missing_dpa);
    let creds = ctx.get(&ValidatorPath::parse("/creds").unwrap());
    assert!(
        creds.is_some(),
        "the secret-bearing list node must stay present (presence guard must \
         not fail open)"
    );
    assert!(
        !creds.unwrap().to_string().contains(SECRET),
        "the stripped list node must not carry the item secret plaintext"
    );
    assert!(
        !format!("{ctx:?}").contains(SECRET),
        "redacted Debug must never carry the plaintext"
    );
}

// Blob-bypass via the public UNVALIDATED `FieldValues::set`: a secret-bearing
// `Field::Object` whose value is a `Literal` object blob carrying an
// UNDECLARED sibling key holding secret-shaped plaintext. The secret rides the
// *defined* container path `/cfg` (the schema builds; `secret.predicate_on_value`
// only flags secret leaves, not container-path predicates), so the runtime
// scrub must drop attacker-controlled undeclared keys inside a secret-bearing
// container — they are NOT `from_json`-parity data there.
#[test]
fn unvalidated_blob_undeclared_sibling_cannot_smuggle_secret_via_container_path() {
    const SECRET: &str = "s3cr3t-via-undeclared-blob-key";

    // (i) A schema with a root rule keyed on the DEFINED container path builds.
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("cfg"))
                .add(Field::secret(field_key!("api_key")))
                .add(Field::string(field_key!("region"))),
        )
        .add(Field::string(field_key!("flag")))
        .root_rule(Rule::any([
            Rule::not(Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/cfg").unwrap(),
            ))),
            Rule::Predicate(nebula_validator::Predicate::Set(
                ValidatorPath::parse("/flag").unwrap(),
            )),
        ]))
        .build();
    assert!(
        schema.is_ok(),
        "a root rule on the defined container path must build (the dangling-ref \
         lint does not reject it; the runtime scrub is the boundary)"
    );

    // (ii)-(iv) Object blob with an undeclared `leak` sibling, set via the
    // unvalidated public setter.
    let fields = vec![Field::from(
        Field::object(field_key!("cfg"))
            .add(Field::secret(field_key!("api_key")))
            .add(Field::string(field_key!("region"))),
    )];
    let mut values = FieldValues::new();
    values.set(
        field_key!("cfg"),
        FieldValue::Literal(json!({ "api_key": SECRET, "leak": SECRET, "region": "eu" })),
    );
    let ctx = root_predicate_context_for(&fields, &values);

    let cfg = ctx.get(&ValidatorPath::parse("/cfg").unwrap());
    assert!(
        cfg.is_some(),
        "container node stays present (presence guard must not fail open)"
    );
    assert!(
        !cfg.unwrap().to_string().contains(SECRET),
        "undeclared blob sibling MUST be stripped — no plaintext on the \
         defined container path: got {cfg:?}"
    );
    for ptr in ["/cfg/leak", "/cfg/api_key"] {
        assert!(
            ctx.get(&ValidatorPath::parse(ptr).unwrap()).is_none(),
            "secret pointer {ptr} must not resolve"
        );
    }
    assert_eq!(
        ctx.get(&ValidatorPath::parse("/cfg/region").unwrap()),
        Some(&json!("eu")),
        "the declared non-secret sibling still resolves"
    );
    assert!(
        !format!("{ctx:?}").contains(SECRET),
        "redacted Debug must never carry the plaintext"
    );

    // Symmetric: secret-bearing LIST whose item blob carries an undeclared
    // secret sibling.
    let list_fields = vec![Field::from(
        Field::list(field_key!("creds")).item(
            Field::object(field_key!("cred"))
                .add(Field::string(field_key!("region")))
                .add(Field::secret(field_key!("api_key"))),
        ),
    )];
    let mut list_values = FieldValues::new();
    list_values.set(
        field_key!("creds"),
        FieldValue::Literal(json!([{ "api_key": SECRET, "leak": SECRET, "region": "eu" }])),
    );
    let ctx = root_predicate_context_for(&list_fields, &list_values);
    let creds = ctx.get(&ValidatorPath::parse("/creds").unwrap());
    assert!(creds.is_some(), "list node stays present");
    assert!(
        !creds.unwrap().to_string().contains(SECRET),
        "undeclared sibling inside a secret-bearing list item must be stripped: got {creds:?}"
    );
    assert!(
        !format!("{ctx:?}").contains(SECRET),
        "redacted Debug must never carry the plaintext"
    );
}

// A Mode submission may OMIT `mode` and rely on `default_variant`
// (`validate_literal_value` resolves it via `.or(default_variant)`). For a
// secret-bearing mode (the scrub branch), the default-variant payload must
// still reach root predicates — dropping it because `mode` is absent would
// fail a legal root guard open.
#[test]
fn mode_default_variant_payload_survives_scrub_when_mode_omitted() {
    const SECRET: &str = "s3cr3t-oauth-client";
    let fields = vec![Field::from(
        Field::mode(field_key!("auth"))
            .default_variant("oauth")
            .variant(
                "oauth",
                "OAuth",
                Field::object(field_key!("o"))
                    .add(Field::string(field_key!("client_id")))
                    .add(Field::secret(field_key!("client_secret"))),
            ),
    )];
    // `mode` OMITTED — `default_variant = "oauth"` applies.
    let mut values = FieldValues::new();
    values.set(
        field_key!("auth"),
        FieldValue::Literal(json!({ "value": { "client_id": "abc", "client_secret": SECRET } })),
    );
    let ctx = root_predicate_context_for(&fields, &values);

    assert_eq!(
        ctx.get(&ValidatorPath::parse("/auth/value/client_id").unwrap()),
        Some(&json!("abc")),
        "default-variant payload must survive the scrub when `mode` is omitted"
    );
    assert!(
        ctx.get(&ValidatorPath::parse("/auth/value/client_secret").unwrap())
            .is_none(),
        "the secret inside the default-variant payload must be scrubbed"
    );
    assert!(
        !format!("{ctx:?}").contains(SECRET),
        "redacted Debug must never carry the plaintext"
    );
}
