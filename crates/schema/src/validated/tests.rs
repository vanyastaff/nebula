use serde_json::json;

use super::*;
use crate::{Field, FieldKey, Schema, field_key};

fn reference_path(pointer: &str) -> ValuePath {
    ValuePath::from_pointer(pointer).expect("test pointer is canonical RFC6901")
}

#[test]
fn clone_is_cheap_via_arc() {
    let s = Schema::builder()
        .add(Field::string(FieldKey::new("x").unwrap()))
        .build()
        .unwrap();
    let c = s.clone();
    assert!(Arc::ptr_eq(&s.0, &c.0));
}

#[test]
fn find_returns_top_level() {
    let s = Schema::builder()
        .add(Field::string(FieldKey::new("x").unwrap()))
        .build()
        .unwrap();
    assert!(s.find(&FieldKey::new("x").unwrap()).is_some());
    assert!(s.find(&FieldKey::new("y").unwrap()).is_none());
}

// ── Sum-type union (SchemaKind::Union) ─────────────────────────────────────

fn sample_union(tagging: SerdeTagging) -> ValidSchema {
    ValidSchema::union(
        Field::mode(field_key!("auth"))
            .variant(
                "oauth",
                "OAuth",
                Field::object(field_key!("oauth"))
                    .add(Field::string(field_key!("token")).required()),
            )
            .variant_empty("none", "None"),
        tagging,
    )
    .expect("sample union builds")
}

#[test]
fn union_kind_and_tagging_accessors() {
    let u = sample_union(SerdeTagging::External);
    assert_eq!(u.kind(), SchemaKind::Union);
    assert_eq!(u.serde_tagging(), Some(&SerdeTagging::External));
    // The variants live as the sole root Field::Mode (the marker design),
    // not in a parallel store.
    assert_eq!(u.fields().len(), 1);
    assert!(matches!(u.fields()[0], Field::Mode(_)));
}

// ── first_undeclared_path (closed-set walk) ───────────────────────────────

#[test]
fn first_undeclared_none_when_every_key_declared() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("host")))
        .add(Field::number(field_key!("port")))
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"host": "h", "port": 5432})).unwrap();
    assert_eq!(schema.first_undeclared_path(&values), None);
}

#[test]
fn first_undeclared_flags_top_level_key() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("host")))
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"host": "h", "password": "x"})).unwrap();
    let path = schema
        .first_undeclared_path(&values)
        .expect("undeclared top-level key must be reported");
    assert_eq!(path.to_string(), "/password");
}

#[test]
fn first_undeclared_recurses_into_nested_object() {
    let schema = Schema::builder()
        .add(Field::object(field_key!("tls")).add(Field::string(field_key!("ca"))))
        .build()
        .unwrap();
    // `ca` is declared; `secret_key` inside the nested object is not — serde
    // would silently drop it, but the closed-set walk must surface it.
    let values =
        AuthoredValue::from_data(json!({"tls": {"ca": "c", "secret_key": "leak"}})).unwrap();
    let path = schema
        .first_undeclared_path(&values)
        .expect("undeclared nested key must be reported");
    assert!(
        path.to_string().contains("secret_key"),
        "path should name the nested key, got: {path}"
    );
}

#[test]
fn first_undeclared_recurses_into_list_items() {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("hosts"))
                .item(Field::object(field_key!("host")).add(Field::string(field_key!("name")))),
        )
        .build()
        .unwrap();
    let values =
        AuthoredValue::from_data(json!({"hosts": [{"name": "a"}, {"name": "b", "bad": 1}]}))
            .unwrap();
    let path = schema
        .first_undeclared_path(&values)
        .expect("undeclared key inside a list item must be reported");
    assert!(
        path.to_string().contains("bad"),
        "path should name the list-item key, got: {path}"
    );
}

#[test]
fn first_undeclared_recurses_into_union_variant_payload() {
    // The `oauth` variant declares only `token`; an inlined `leak` in its
    // payload must be surfaced (the union's deeper-than-top-level case).
    let union = sample_union(SerdeTagging::External);
    let values = union
        .values_from_wire(json!({"oauth": {"token": "t", "leak": "x"}}))
        .expect("external data-variant wire ingests");
    let path = union
        .first_undeclared_path(&values)
        .expect("undeclared key in a union variant payload must be reported");
    assert!(
        path.to_string().contains("leak"),
        "path should name the variant-payload key, got: {path}"
    );
    // A clean payload has nothing undeclared.
    let clean = union
        .values_from_wire(json!({"oauth": {"token": "t"}}))
        .unwrap();
    assert_eq!(union.first_undeclared_path(&clean), None);
}

#[test]
fn union_root_mode_is_required_no_fail_open() {
    // A sum-type value is never optional: the root mode is RequiredMode::Always,
    // so an absent value FAILS rather than validating clean (the fail-open the
    // parallel-Vec design would have allowed).
    let u = sample_union(SerdeTagging::External);
    let empty = AuthoredValue::from_data(serde_json::json!({})).unwrap();
    assert!(
        u.validate(empty).is_err(),
        "an absent sum-type value must fail the required root mode"
    );
}

#[test]
fn union_roundtrips_external_and_adjacent() {
    for tagging in [
        SerdeTagging::External,
        SerdeTagging::Adjacent {
            tag: "type".to_owned(),
            content: "data".to_owned(),
        },
    ] {
        let u = sample_union(tagging);
        let json = serde_json::to_string(&u).expect("serialize union");
        let back: ValidSchema = serde_json::from_str(&json).expect("deserialize union");
        assert_eq!(u, back, "a union must round-trip including its tagging");
    }
}

#[test]
fn union_partial_eq_distinguishes_tagging() {
    // Same variants, different tagging => different schema. Without serde_tagging
    // in PartialEq these would falsely compare equal (a type-DAG cache defect).
    let external = sample_union(SerdeTagging::External);
    let adjacent = sample_union(SerdeTagging::Adjacent {
        tag: "type".to_owned(),
        content: "data".to_owned(),
    });
    assert_ne!(external, adjacent);
}

#[test]
fn union_rejects_default_variant() {
    // A tagged union has no default variant — serde always requires the
    // discriminant, and a default would let mode validation accept a value
    // with no selector, breaking the tagged-union contract.
    let mode = Field::mode(field_key!("auth"))
        .variant(
            "oauth",
            "OAuth",
            Field::object(field_key!("oauth")).add(Field::string(field_key!("token")).required()),
        )
        .default_variant("oauth");
    let err = ValidSchema::union(mode, SerdeTagging::External).unwrap_err();
    assert!(
        format!("{err:?}").contains("default_variant"),
        "the union constructor must reject a default variant, got {err:?}"
    );
}

#[test]
fn union_deserialize_fails_closed() {
    let union_wire = serde_json::to_value(sample_union(SerdeTagging::External)).unwrap();
    let record_wire = serde_json::to_value(
        Schema::builder()
            .add(Field::string(field_key!("x")))
            .build()
            .unwrap(),
    )
    .unwrap();

    // Missing serde_tagging on a union.
    let mut missing_tag = union_wire.clone();
    missing_tag.as_object_mut().unwrap().remove("serde_tagging");
    assert!(serde_json::from_value::<ValidSchema>(missing_tag).is_err());

    // Root field is not a mode field.
    let mut not_mode = union_wire.clone();
    not_mode["fields"] = record_wire["fields"].clone();
    assert!(serde_json::from_value::<ValidSchema>(not_mode).is_err());

    // More than one root field. (Last use of `union_wire` — move, don't clone.)
    let mut multi = union_wire;
    let doubled: Vec<serde_json::Value> = {
        let mode_fields = multi["fields"].as_array().unwrap();
        mode_fields.iter().chain(mode_fields).cloned().collect()
    };
    multi["fields"] = serde_json::Value::Array(doubled);
    assert!(serde_json::from_value::<ValidSchema>(multi).is_err());

    // A non-union (Record) must not carry serde_tagging.
    let mut stray_tag = record_wire;
    stray_tag
        .as_object_mut()
        .unwrap()
        .insert("serde_tagging".to_owned(), serde_json::json!("external"));
    assert!(serde_json::from_value::<ValidSchema>(stray_tag).is_err());
}

#[test]
fn find_by_path_handles_nested_object_and_mode_variant() {
    let schema = Schema::builder()
        .add(Field::object(FieldKey::new("user").unwrap()).add(Field::string(field_key!("email"))))
        .add(Field::mode(FieldKey::new("auth").unwrap()).variant(
            "token",
            "Token",
            Field::string(field_key!("value")),
        ))
        .build()
        .unwrap();

    assert!(
        schema
            .find_by_path(&FieldPath::parse("user.email").unwrap())
            .is_some()
    );
    assert!(
        schema
            .find_by_path(&FieldPath::parse("auth.token").unwrap())
            .is_some()
    );
    assert!(
        schema
            .find_by_path(&FieldPath::parse("user.missing").unwrap())
            .is_none()
    );
}

#[test]
fn find_by_path_handles_list_object_children() {
    let schema = Schema::builder()
        .add(Field::list(FieldKey::new("items").unwrap()).item(
            Field::object(FieldKey::new("item").unwrap()).add(Field::string(field_key!("name"))),
        ))
        .build()
        .unwrap();

    let field = schema
        .find_by_path(&FieldPath::parse("items.name").unwrap())
        .expect("list item child should be indexed");
    assert_eq!(field.key().as_str(), "name");
    assert!(
        schema
            .find_by_path(&FieldPath::parse("items[0].name").unwrap())
            .is_none(),
        "schema paths use the canonical anonymous list-item path"
    );
    assert!(
        schema
            .find_by_path(&FieldPath::parse("items.missing").unwrap())
            .is_none()
    );
}

#[test]
fn root_rule_runs_after_fields() {
    use nebula_validator::{Predicate, Rule};
    use serde_json::json;

    let schema = Schema::builder()
        .add(Field::string(FieldKey::new("tier").unwrap()))
        .root_rule(Rule::predicate(Predicate::eq("tier", json!("pro")).unwrap()).unwrap())
        .build()
        .unwrap();

    let bad = AuthoredValue::from_data(json!({"tier": "free"})).unwrap();
    assert!(schema.validate(bad).is_err());

    let ok = AuthoredValue::from_data(json!({"tier": "pro"})).unwrap();
    assert!(schema.validate(ok).is_ok());
}

#[test]
fn root_rule_error_preserves_validator_field_path() {
    use nebula_validator::{Predicate, Rule};
    use serde_json::json;

    let schema = Schema::builder()
        .add(
            Field::object(FieldKey::new("config").unwrap())
                .add(Field::string(FieldKey::new("tier").unwrap())),
        )
        .root_rule(Rule::predicate(Predicate::eq("/config/tier", json!("pro")).unwrap()).unwrap())
        .build()
        .unwrap();

    let bad = AuthoredValue::from_data(json!({"config": {"tier": "free"}})).unwrap();
    let report = schema.validate(bad).unwrap_err();
    assert!(
        report
            .errors()
            .any(|e| e.path().to_string() == "/config/tier"),
        "expected root-rule error at config.tier, got: {report:?}"
    );
}

#[test]
fn valid_schema_serde_roundtrips_root_rules() {
    use nebula_validator::{Predicate, Rule};
    use serde_json::json;

    let schema = Schema::builder()
        .add(Field::string(FieldKey::new("x").unwrap()))
        .root_rule(Rule::predicate(Predicate::eq("x", json!("a")).unwrap()).unwrap())
        .build()
        .unwrap();

    let wire = serde_json::to_value(&schema).unwrap();
    let back: ValidSchema = serde_json::from_value(wire).unwrap();
    assert_eq!(schema.root_rules(), back.root_rules());
    assert_eq!(schema.fields().len(), back.fields().len());
}

#[test]
fn deserialize_bare_any_is_accepted() {
    use serde_json::json;

    let decoded: ValidSchema = serde_json::from_value(json!({"kind": "any"})).unwrap();
    assert_eq!(decoded.kind(), SchemaKind::Any);
    assert!(decoded.fields().is_empty());
}

#[test]
fn deserialize_any_carrying_fields_is_rejected() {
    use serde_json::json;

    // Build a real typed schema, then mistag it as `Any` while keeping its
    // `fields`. Accepting this would silently drop every field constraint.
    let typed = Schema::builder()
        .add(Field::string(field_key!("x")).required())
        .build()
        .unwrap();
    let mut wire = serde_json::to_value(&typed).unwrap();
    wire["kind"] = json!("any");

    let result: Result<ValidSchema, _> = serde_json::from_value(wire);
    assert!(
        result.is_err(),
        "a schema tagged `kind: any` that still carries fields must be rejected, not \
             silently coerced to the unconstrained `Any`"
    );
}

#[test]
fn deserialize_any_carrying_root_rules_is_rejected() {
    use nebula_validator::{Predicate, Rule};
    use serde_json::json;

    let with_rules = Schema::builder()
        .add(Field::string(field_key!("x")))
        .root_rule(Rule::predicate(Predicate::eq("x", json!("a")).unwrap()).unwrap())
        .build()
        .unwrap();
    let mut wire = serde_json::to_value(&with_rules).unwrap();
    wire["kind"] = json!("any");
    // Drop fields so only `root_rules` remains to prove the rule branch fires.
    wire["fields"] = json!([]);

    let result: Result<ValidSchema, _> = serde_json::from_value(wire);
    assert!(
        result.is_err(),
        "a schema tagged `kind: any` that still carries root_rules must be rejected"
    );
}

#[test]
fn list_field_custom_rules_are_enforced() {
    use nebula_validator::Rule;
    use serde_json::json;

    let schema = Schema::builder()
        .add(
            Field::list(FieldKey::new("tags").unwrap())
                .item(Field::string(FieldKey::new("tag").unwrap()))
                .with_rule(Rule::max_items(1)),
        )
        .build()
        .unwrap();

    let values = AuthoredValue::from_data(json!({"tags": ["a", "b"]})).unwrap();
    let report = schema.validate(values).expect_err("list rule must fail");
    assert!(
        report.has_errors(),
        "expected list-level custom rule to produce an error"
    );
}

#[test]
fn object_field_custom_rules_are_enforced() {
    use nebula_validator::Rule;
    use serde_json::json;

    let schema = Schema::builder()
        .add(
            Field::object(FieldKey::new("config").unwrap())
                .add(Field::boolean(FieldKey::new("enabled").unwrap()))
                .with_rule(Rule::one_of([json!({"enabled": true})]).unwrap()),
        )
        .build()
        .unwrap();

    let values = AuthoredValue::from_data(json!({"config": {"enabled": false}})).unwrap();
    let report = schema.validate(values).expect_err("object rule must fail");
    assert!(report.errors().any(|e| e.code() == "one_of"));
}

// ── walk_reference_path / single_field (ADR-0100 TypeDAG, W0 U5) ───────────

#[test]
fn select_field_reference_fails_open() {
    // `Select` may carry an array (`multiple`) or an arbitrary option value —
    // never a safe scalar terminal, so a reference into it is opaque.
    let schema = Schema::builder()
        .add(Field::select(field_key!("country")))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&reference_path("/country")),
        PathWalk::Opaque
    );
}

#[test]
fn file_field_reference_fails_open() {
    let schema = Schema::builder()
        .add(Field::file(field_key!("attachment")))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&reference_path("/attachment")),
        PathWalk::Opaque
    );
}

#[test]
fn notice_field_reference_fails_open() {
    // `Notice` produces no runtime value at all.
    let schema = Schema::builder()
        .add(Field::notice(field_key!("banner")))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&reference_path("/banner")),
        PathWalk::Opaque
    );
}

#[test]
fn untyped_list_item_reference_fails_open() {
    // An item that is itself opaque (e.g. what a `Vec<serde_json::Value>`
    // derives to: an empty `Field::Object`) is opaque, not resolved — a
    // valid numeric index still fails open, it never becomes
    // `NonIndexOnList` just because the item is opaque.
    let opaque_item = Schema::builder()
        .add(Field::list(field_key!("items")).item(Field::object(field_key!("item"))))
        .build()
        .unwrap();
    assert_eq!(
        opaque_item.walk_reference_path(&reference_path("/items/0")),
        PathWalk::Opaque
    );

    // `item: None` (truly untyped) is rejected by the builder's own lint
    // (`missing_item_schema` — a `List` must always declare an item schema
    // to pass `SchemaBuilder::build`), so it can never reach a real
    // `ValidSchema` through the public API. `walk_step` still has to
    // handle it defensively (the field is a plain `Option`) — construct
    // the otherwise-unreachable shape directly via the crate-private
    // `from_inner` to prove that defensive arm, rather than asserting on
    // dead code.
    let untyped = ValidSchema::from_inner(ValidSchemaInner {
        root: RootShape::record(
            vec![Field::from(Field::list(field_key!("items")))],
            Vec::new(),
        ),
        index: IndexMap::new(),
        flags: SchemaFlags::default(),
        has_contextual_rules: false,
    });
    assert_eq!(
        untyped.walk_reference_path(&reference_path("/items/0")),
        PathWalk::Opaque
    );
}

#[test]
fn missing_object_key_is_opaque_not_an_error() {
    // A declared-absent key under a non-empty `Object` fails open — `HasSchema`
    // is unsealed, so a non-empty `Object` cannot be trusted as exhaustive.
    let schema = Schema::builder()
        .add(Field::object(field_key!("contact")).add(Field::string(field_key!("email"))))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&reference_path("/contact/phone")),
        PathWalk::Opaque
    );
    // The declared key still resolves.
    assert!(matches!(
        schema.walk_reference_path(&reference_path("/contact/email")),
        PathWalk::Resolved(Field::String(_))
    ));
}

#[test]
fn non_index_on_list_hard_rejected() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("items")).item(Field::string(field_key!("item"))))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&reference_path("/items/first")),
        PathWalk::Unresolved(PathResolveError::NonIndexOnList {
            segment: "first".to_owned()
        })
    );
}

#[test]
fn descend_past_scalar_hard_rejected() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&reference_path("/name/first")),
        PathWalk::Unresolved(PathResolveError::DescendPastLeaf {
            segment: "first".to_owned()
        })
    );
}

#[test]
fn root_reference_path_resolves_concrete_record() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .build()
        .unwrap();
    assert_eq!(
        schema.walk_reference_path(&ValuePath::root()),
        PathWalk::ResolvedRoot
    );
}

#[test]
fn root_reference_path_resolves_concrete_scalar() {
    let schema = ValidSchema::scalar(ScalarSchema::string()).unwrap();
    assert_eq!(
        schema.walk_reference_path(&ValuePath::root()),
        PathWalk::ResolvedRoot
    );
    assert_eq!(
        schema.walk_reference_path(&reference_path("/child")),
        PathWalk::Unresolved(PathResolveError::DescendPastLeaf {
            segment: "child".to_owned()
        })
    );
}

#[test]
fn any_and_union_roots_are_opaque() {
    assert_eq!(
        ValidSchema::any().walk_reference_path(&ValuePath::root()),
        PathWalk::Opaque
    );
    assert_eq!(
        ValidSchema::any().walk_reference_path(&reference_path("/anything")),
        PathWalk::Opaque
    );
    let u = sample_union(SerdeTagging::External);
    assert_eq!(u.walk_reference_path(&ValuePath::root()), PathWalk::Opaque);
    assert_eq!(
        u.walk_reference_path(&reference_path("/oauth")),
        PathWalk::Opaque
    );
}

#[test]
fn walk_agrees_with_find_by_path_on_plain_paths() {
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("contact")).add(Field::string(field_key!("email")).required()),
        )
        .build()
        .unwrap();
    let walked = schema.walk_reference_path(&reference_path("/contact/email"));
    let found = schema
        .find_by_path(
            &FieldPath::root()
                .join(field_key!("contact"))
                .join(field_key!("email")),
        )
        .expect("plain path resolves via find_by_path");
    assert_eq!(walked, PathWalk::Resolved(found));
}

#[test]
fn single_field_rekeys_producer_leaf_so_explain_assignable_can_pair_it() {
    // The producer leaf's own key (`email`) differs from the consumer
    // parameter key (`recipient`) it is being checked against; `single_field`
    // must re-key both to the SAME key so `explain_assignable`'s key-based
    // pairing matches them up (a `FieldTypeMismatch`/`Yes`, never a spurious
    // `MissingRequiredField` from a key mismatch).
    let producer_leaf: Field = Field::string(field_key!("email")).required().into();
    let consumer_field: Field = Field::string(field_key!("recipient")).required().into();

    let output = crate::OutputSchema::new(ValidSchema::single_field(
        field_key!("recipient"),
        producer_leaf,
    ));
    let input = crate::InputSchema::new(ValidSchema::single_field(
        field_key!("recipient"),
        consumer_field,
    ));

    assert_eq!(
        crate::explain_assignable(&output, &input),
        crate::Assignability::Yes
    );
}

#[test]
fn single_field_type_mismatch_is_reported_under_the_shared_key() {
    let producer_leaf: Field = Field::number(field_key!("age")).into();
    let consumer_field: Field = Field::string(field_key!("name")).required().into();

    let output =
        crate::OutputSchema::new(ValidSchema::single_field(field_key!("name"), producer_leaf));
    let input = crate::InputSchema::new(ValidSchema::single_field(
        field_key!("name"),
        consumer_field,
    ));

    match crate::explain_assignable(&output, &input) {
        crate::Assignability::No(incompats) => {
            assert!(incompats.iter().any(|i| matches!(
                i,
                crate::SchemaIncompat::FieldTypeMismatch { key, .. } if key.as_str() == "name"
            )));
        },
        other => panic!("expected No(FieldTypeMismatch), got {other:?}"),
    }
}
