use serde_json::json;

use super::*;
use crate::Property;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

/// Gradual, kind-blind slice check: an empty producer slice is the `Any`
/// escape. This is **test-only** — production code calls
/// [`explain_assignable`], which distinguishes concrete records from Any.
/// Retained here to exercise the shared per-field matching logic (type
/// mismatch, cardinality, nesting) and the gradual empty-producer escape
/// directly on `&[Property]` without building a `ValidSchema` per case. Mirrors
/// the binary mapping: `Yes`/`Unknown` ⇒ `Ok`, `No` ⇒ first incompatibility.
fn is_assignable(producer: &[Property], consumer: &[Property]) -> Result<(), SchemaIncompat> {
    match explain_slice(producer, consumer, false) {
        Assignability::Yes | Assignability::Unknown(_) => Ok(()),
        Assignability::No(incompats) => match incompats.into_iter().next() {
            Some(first) => Err(first),
            None => Ok(()),
        },
    }
}

/// Test-only collect-all over raw slices, with explicit `strict` control.
fn explain_slice(producer: &[Property], consumer: &[Property], strict: bool) -> Assignability {
    let mut acc = Explain::default();
    collect_fields(producer, consumer, strict, &mut acc);
    acc.into_verdict()
}

// Keep every schema test on the real direction-typed, tri-state entry point.
fn explain_assignable(producer: &ValidSchema, consumer: &ValidSchema) -> Assignability {
    super::explain_assignable(
        &OutputSchema::new(producer.clone()),
        &InputSchema::new(consumer.clone()),
    )
}

// ── Compatible: producer has all required consumer fields + extra ──────

#[test]
fn compatible_with_extra_producer_field() {
    let producer = [
        Property::string(fk("name")).required().into(),
        Property::number(fk("score")).into(),
        Property::boolean(fk("extra")).into(), // producer-only, ignored
    ];
    let consumer = [
        Property::string(fk("name")).required().into(),
        Property::number(fk("score")).into(),
    ];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Missing hard-required field ────────────────────────────────────────

#[test]
fn missing_required_field_returns_error() {
    let producer = [Property::number(fk("score")).into()];
    let consumer = [
        Property::string(fk("name")).required().into(),
        Property::number(fk("score")).into(),
    ];
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::MissingRequiredField { key: fk("name") })
    );
}

// ── Type mismatch on shared field ──────────────────────────────────────

#[test]
fn type_mismatch_on_shared_field_returns_error() {
    let producer = [Property::number(fk("value")).required().into()];
    let consumer = [Property::string(fk("value")).required().into()];
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::FieldTypeMismatch {
            key: fk("value"),
            producer: "number",
            consumer: "string",
        })
    );
}

// ── Optional consumer field absent from producer ───────────────────────

#[test]
fn optional_consumer_field_absent_is_ok() {
    let producer = [Property::string(fk("name")).required().into()];
    let consumer = [
        Property::string(fk("name")).required().into(),
        Property::number(fk("optional_score")).into(), // optional, absent in producer
    ];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── When-required consumer field absent treated as optional ───────────

#[test]
fn when_required_absent_is_ok() {
    use crate::Rule;
    use nebula_validator::Predicate;

    let rule = Rule::predicate(Predicate::eq("mode", json!("advanced")).unwrap()).unwrap();
    let producer = [Property::string(fk("name")).required().into()];
    let consumer = [
        Property::string(fk("name")).required().into(),
        Property::string(fk("advanced_opt"))
            .required_when(rule)
            .into(),
    ];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Nested Object: consumer-object requires a field producer-object lacks

#[test]
fn nested_object_missing_required_returns_error() {
    let producer = [Property::object(fk("config"))
        .property(Property::string(fk("host")).required())
        // "port" absent from producer's config
        .into()];
    let consumer = [Property::object(fk("config"))
        .property(Property::string(fk("host")).required())
        .property(Property::number(fk("port")).required())
        .into()];
    // Both outer fields are Object — recurse; inner check finds "port"
    // missing → NestedIncompat wrapping MissingRequiredField.
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::NestedIncompat {
            key: fk("config"),
            inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("port") }),
        })
    );
}

// ── Nested Object: inner scalar field type mismatch ───────────────────

#[test]
fn nested_object_field_type_mismatch_returns_nested_incompat() {
    let producer = [Property::object(fk("config"))
        .property(Property::number(fk("port")).required()) // number in producer
        .into()];
    let consumer = [Property::object(fk("config"))
        .property(Property::string(fk("port")).required()) // string in consumer
        .into()];
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::NestedIncompat {
            key: fk("config"),
            inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                key: fk("port"),
                producer: "number",
                consumer: "string",
            }),
        })
    );
}

// ── Nested Object: fully nested-compatible ─────────────────────────────

#[test]
fn nested_object_fully_compatible_is_ok() {
    let producer = [Property::object(fk("config"))
        .property(Property::string(fk("host")).required())
        .property(Property::number(fk("port")).required())
        .into()];
    let consumer = [Property::object(fk("config"))
        .property(Property::string(fk("host")).required())
        .property(Property::number(fk("port")).required())
        .into()];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── List: compatible item types ────────────────────────────────────────

#[test]
fn list_compatible_item_types_is_ok() {
    let producer = [Property::list(fk("tags"))
        .item(Property::string(fk("tag")))
        .into()];
    let consumer = [Property::list(fk("tags"))
        .item(Property::string(fk("tag")))
        .into()];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── List: mismatched item types → NestedIncompat ───────────────────────

#[test]
fn list_mismatched_item_types_returns_nested_incompat() {
    let producer = [Property::list(fk("values"))
        .item(Property::string(fk("item")))
        .required()
        .into()];
    let consumer = [Property::list(fk("values"))
        .item(Property::number(fk("item")))
        .required()
        .into()];
    // Outer NestedIncompat is keyed by the list field (`values`); the inner
    // mismatch is keyed by the *item* (`item`), not the list key.
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::NestedIncompat {
            key: fk("values"),
            inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                key: fk("item"),
                producer: "string",
                consumer: "number",
            }),
        })
    );
}

// ── Any escape: empty producer ─────────────────────────────────────────

#[test]
fn empty_producer_satisfies_typed_consumer() {
    let consumer = [Property::string(fk("name")).required().into()];
    assert_eq!(is_assignable(&[], &consumer), Ok(()));
}

// ── Any escape: Dynamic producer field vs typed required consumer ──────

#[test]
fn dynamic_producer_field_satisfies_typed_required_consumer() {
    let producer = [Property::dynamic(fk("name")).into()];
    let consumer = [Property::string(fk("name")).required().into()];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Any escape: empty consumer accepts everything ──────────────────────

#[test]
fn empty_consumer_accepts_any_producer() {
    let producer = [Property::string(fk("name")).required().into()];
    assert_eq!(is_assignable(&producer, &[]), Ok(()));
}

// ── Notice consumer field is ignored ──────────────────────────────────

#[test]
fn notice_consumer_field_ignored() {
    let producer = [Property::string(fk("name")).required().into()];
    let consumer = [
        Property::string(fk("name")).required().into(),
        Property::notice(fk("tip")).into(), // absent in producer, but ignored
    ];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Extra producer fields ignored (width subtyping) ───────────────────

#[test]
fn extra_producer_fields_ignored() {
    let producer = [
        Property::string(fk("name")).required().into(),
        Property::number(fk("extra_a")).into(),
        Property::boolean(fk("extra_b")).into(),
    ];
    let consumer = [Property::string(fk("name")).required().into()];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Cardinality: File single→multiple is a mismatch ───────────────────

#[test]
fn file_single_to_multiple_returns_cardinality_mismatch() {
    // producer: single file; consumer expects multiple files
    let producer = [Property::file(fk("attachment")).required().into()];
    let consumer = [Property::file(fk("attachment"))
        .multiple()
        .required()
        .into()];
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::CardinalityMismatch {
            key: fk("attachment"),
            producer_multiple: false,
            consumer_multiple: true,
        })
    );
}

// ── Cardinality: File multiple→multiple is compatible ─────────────────

#[test]
fn file_multiple_to_multiple_is_ok() {
    let producer = [Property::file(fk("attachment"))
        .multiple()
        .required()
        .into()];
    let consumer = [Property::file(fk("attachment"))
        .multiple()
        .required()
        .into()];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Cardinality: Select cardinality mismatch ──────────────────────────

#[test]
fn select_cardinality_mismatch_returns_error() {
    // producer: multi-select; consumer: single-select
    let producer = [Property::select(fk("tags")).multiple().required().into()];
    let consumer = [Property::select(fk("tags")).required().into()];
    assert_eq!(
        is_assignable(&producer, &consumer),
        Err(SchemaIncompat::CardinalityMismatch {
            key: fk("tags"),
            producer_multiple: true,
            consumer_multiple: false,
        })
    );
}

// ── Cardinality: Select same cardinality is compatible ────────────────

#[test]
fn select_same_cardinality_is_ok() {
    let producer = [Property::select(fk("tags")).multiple().required().into()];
    let consumer = [Property::select(fk("tags")).multiple().required().into()];
    assert_eq!(is_assignable(&producer, &consumer), Ok(()));
}

// ── Kind-aware entry point: records and Any stay distinct ──────

/// Build a single-required-field record `ValidSchema`.
fn required_record(key: &str) -> ValidSchema {
    crate::Schema::builder()
        .property(Property::string(fk(key)).required())
        .build()
        .unwrap()
}

/// The defining fix: an empty **record** producer (`()`) does NOT satisfy a
/// consumer that hard-requires a field — unlike the gradual `Any`, an empty
/// record provably emits nothing.
#[test]
fn empty_record_producer_does_not_satisfy_required_consumer() {
    let producer = ValidSchema::empty(); // Record, zero fields
    let consumer = required_record("name");
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::MissingRequiredField {
            key: fk("name")
        }]),
    );
}

/// An opaque producer cannot prove the required record contract.
#[test]
fn any_producer_is_unknown_for_required_consumer() {
    let producer = ValidSchema::any();
    let consumer = required_record("name");
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
    );
}

/// An `Any` consumer accepts any producer (it requires nothing).
#[test]
fn typed_producer_satisfies_any_consumer() {
    let producer = required_record("name");
    let consumer = ValidSchema::any();
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// An empty record accepts other records under width-subtyping.
#[test]
fn empty_record_consumer_accepts_typed_producer() {
    let producer = required_record("name");
    let consumer = ValidSchema::empty();
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// Two compatible concrete records pass through the kind-aware entry point,
/// matching the slice-level result (no behavior drift on the typed path).
#[test]
fn compatible_records_via_schema_entry_is_ok() {
    let producer = crate::Schema::builder()
        .property(Property::string(fk("name")).required())
        .property(Property::number(fk("extra")))
        .build()
        .unwrap();
    let consumer = required_record("name");
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// Two incompatible concrete records still produce the same first
/// incompatibility through the kind-aware entry point.
#[test]
fn incompatible_records_via_schema_entry_returns_error() {
    let producer = required_record("name");
    let consumer = required_record("other");
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::MissingRequiredField {
            key: fk("other")
        }]),
    );
}

/// The universal consumer accepts even an opaque producer.
#[test]
fn both_any_schemas_are_compatible() {
    assert_eq!(
        explain_assignable(&ValidSchema::any(), &ValidSchema::any()),
        Assignability::Yes,
    );
}

// ── Strict-mode recursion: `strict` must reach nested Object / List leaves ──
// These drive the collect-all core via `explain_slice` and contrast strict
// vs gradual at depth, so a dropped `strict` argument in the Object/List
// recursion arms goes RED.

/// An empty nested-object producer does NOT satisfy a consumer whose nested
/// object hard-requires a field — under strict mode, one level deep. Gradual
/// mode escapes it, proving the assertion is `strict`-specific.
#[test]
fn strict_rejects_empty_nested_object_producer() {
    let producer = [Property::object(fk("config")).into()]; // empty inner object
    let consumer = [Property::object(fk("config"))
        .property(Property::string(fk("host")).required())
        .into()];

    assert_eq!(
        explain_slice(&producer, &consumer, true),
        Assignability::No(vec![SchemaIncompat::NestedIncompat {
            key: fk("config"),
            inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("host") }),
        }]),
        "strict mode must recurse into the empty producer object and fail the required field"
    );
    assert_eq!(
        explain_slice(&producer, &consumer, false),
        Assignability::Yes,
        "gradual mode escapes the empty nested producer — confirms the strict flag is what bites"
    );
}

/// A list whose item is an empty object does NOT satisfy a consumer list
/// whose item object hard-requires a field — strict propagates List → item →
/// Object. Gradual mode escapes it. The inner context key is the *item*
/// (`item`), the outer is the list (`items`).
#[test]
fn strict_rejects_empty_nested_list_item_producer() {
    let producer = [Property::list(fk("items"))
        .item(Property::object(fk("item")))
        .into()];
    let consumer = [Property::list(fk("items"))
        .item(Property::object(fk("item")).property(Property::string(fk("id")).required()))
        .into()];

    assert_eq!(
        explain_slice(&producer, &consumer, true),
        Assignability::No(vec![SchemaIncompat::NestedIncompat {
            key: fk("items"),
            inner: Box::new(SchemaIncompat::NestedIncompat {
                key: fk("item"),
                inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("id") }),
            }),
        }]),
        "strict mode must recurse List -> item -> Object and fail the required field"
    );
    assert_eq!(
        explain_slice(&producer, &consumer, false),
        Assignability::Yes,
        "gradual mode escapes the empty list-item object — confirms strict propagation"
    );
}

// ── Ternary `explain_assignable`: Yes / No(all) / Unknown(reasons) ─────────

/// `explain_assignable` collects ALL incompatibilities, not just the first —
/// the property that makes it a CI/diagnostics channel rather than a gate.
#[test]
fn explain_collects_all_incompatibilities() {
    let producer = required_record("present");
    let consumer = crate::Schema::builder()
        .property(Property::string(fk("missing_a")).required())
        .property(Property::string(fk("missing_b")).required())
        .build()
        .unwrap();
    match explain_assignable(&producer, &consumer) {
        Assignability::No(incompats) => {
            assert_eq!(
                incompats.len(),
                2,
                "both missing required fields are reported"
            );
            assert!(incompats.contains(&SchemaIncompat::MissingRequiredField {
                key: fk("missing_a")
            }));
            assert!(incompats.contains(&SchemaIncompat::MissingRequiredField {
                key: fk("missing_b")
            }));
        },
        other => panic!("expected No(2), got {other:?}"),
    }
}

/// An `Any` producer is unknown, never proof of a concrete contract.
#[test]
fn explain_any_producer_is_unknown_opaque() {
    let consumer = required_record("name");
    assert_eq!(
        explain_assignable(&ValidSchema::any(), &consumer),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
    );
}

/// An `Any` consumer is provably `Yes` (it accepts anything).
#[test]
fn explain_any_consumer_is_yes() {
    let producer = required_record("name");
    assert_eq!(
        explain_assignable(&producer, &ValidSchema::any()),
        Assignability::Yes,
    );
}

/// No declared properties does not remove the consumer's object requirement.
#[test]
fn explain_any_producer_into_empty_record_consumer_is_unknown() {
    assert_eq!(
        explain_assignable(&ValidSchema::any(), &ValidSchema::empty()),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
    );
}

/// An untyped producer list item against a *typed* consumer item is opaque:
/// strict ⇒ `Unknown(NestedUnknown { items, OpaqueProducer })`, gradual ⇒
/// `Yes` (producer-side escape preserved).
///
/// Driven via `explain_slice` on raw `Property`s, not `explain_assignable`: a
/// built `ValidSchema` can never carry an item-less list (the builder lint
/// rejects it — `lint/mod.rs` `missing_item_schema`), so this case is
/// unreachable through the public `ValidSchema` API. The `collect_pair` core
/// is kept total/correct for it regardless (mirrors the record-level
/// empty-producer rule), and the gradual slice form does reach it.
#[test]
fn explain_untyped_producer_list_item_is_unknown_for_typed_consumer() {
    let p = [Property::list(fk("items")).into()];
    let c = [Property::list(fk("items"))
        .item(Property::string(fk("item")))
        .into()];

    assert_eq!(
        explain_slice(&p, &c, true),
        Assignability::Unknown(vec![UnknownReason::NestedUnknown {
            key: fk("items"),
            inner: Box::new(UnknownReason::OpaqueProducer),
        }]),
    );
    assert_eq!(explain_slice(&p, &c, false), Assignability::Yes);
}

/// A dynamic producer has no statically known concrete shape.
#[test]
fn explain_dynamic_field_is_unknown_not_ok() {
    let producer = crate::Schema::builder()
        .property(Property::dynamic(fk("name")))
        .build()
        .unwrap();
    let consumer = required_record("name");
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::DynamicLoaderBacked { key: fk("name") }]),
    );
}

/// A `Property::Unknown` on either side is opaque: even two of the *same* future
/// kind are `Unknown(OpaqueFieldKind)`, never a misleading `Yes` — this
/// version cannot prove an unrecognized kind's value contract.
#[test]
fn explain_unknown_field_pair_is_unknown_not_yes() {
    let producer: ValidSchema = serde_json::from_value(
        json!({"policy_version": 2, "fields": [{"type": "richtext", "key": "bio"}]}),
    )
    .expect("Unknown producer schema");
    let consumer: ValidSchema = serde_json::from_value(
        json!({"policy_version": 2, "fields": [{"type": "richtext", "key": "bio"}]}),
    )
    .expect("Unknown consumer schema");
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind { key: fk("bio") }]),
    );
}

/// Two *distinct* unknown kinds at the same key are `Unknown`, not a hard
/// `No`/`FieldTypeMismatch`: `type_name()` collapses both to "unknown", but
/// the pair is genuinely undecidable, not provably incompatible.
#[test]
fn explain_distinct_unknown_kinds_are_unknown_not_mismatch() {
    let producer: ValidSchema = serde_json::from_value(
        json!({"policy_version": 2, "fields": [{"type": "richtext", "key": "bio"}]}),
    )
    .expect("Unknown producer schema");
    let consumer: ValidSchema = serde_json::from_value(
        json!({"policy_version": 2, "fields": [{"type": "gallery", "key": "bio"}]}),
    )
    .expect("Unknown consumer schema");
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind { key: fk("bio") }]),
    );
}

/// An `Unknown` against a *known* field is also undecidable — not a hard
/// mismatch — because this version cannot reason about the unknown side.
#[test]
fn explain_unknown_vs_known_field_is_unknown_not_mismatch() {
    let producer: ValidSchema = serde_json::from_value(
        json!({"policy_version": 2, "fields": [{"type": "richtext", "key": "bio"}]}),
    )
    .expect("Unknown producer schema");
    let consumer = crate::Schema::builder()
        .property(Property::string(fk("bio")))
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind { key: fk("bio") }]),
    );
}

/// Number int→float widens (provably `Yes`); float→int narrows
/// (`Unknown(NumberWidening)`, not a hard `No`).
#[test]
fn explain_number_widening_is_directional() {
    let int_producer = crate::Schema::builder()
        .property(Property::number(fk("n")).integer())
        .build()
        .unwrap();
    let float_consumer = crate::Schema::builder()
        .property(Property::number(fk("n")))
        .build()
        .unwrap();
    // int -> float: safe widening.
    assert_eq!(
        explain_assignable(&int_producer, &float_consumer),
        Assignability::Yes,
    );
    // float -> int: possible precision loss, undecidable.
    assert_eq!(
        explain_assignable(&float_consumer, &int_producer),
        Assignability::Unknown(vec![UnknownReason::NumberWidening { key: fk("n") }]),
    );
}

/// A definite incompatibility dominates an undecidable one: `No` ▸ `Unknown`.
#[test]
fn explain_no_dominates_unknown() {
    let producer = crate::Schema::builder()
        .property(Property::dynamic(fk("d")))
        .build()
        .unwrap();
    let consumer = crate::Schema::builder()
        .property(Property::dynamic(fk("d")))
        .property(Property::string(fk("required_missing")).required())
        .build()
        .unwrap();
    match explain_assignable(&producer, &consumer) {
        Assignability::No(v) => assert_eq!(
            v,
            vec![SchemaIncompat::MissingRequiredField {
                key: fk("required_missing")
            }]
        ),
        other => panic!("a provable incompatibility must dominate Unknown, got {other:?}"),
    }
}

/// Identical `Mode`s are provably assignable (`Yes`): every producer variant
/// has a matching consumer variant with a compatible payload.
#[test]
fn explain_mode_identical_is_yes() {
    let mode_schema = || {
        crate::Schema::builder()
            .property(Property::mode(fk("m")).variant("v", "V", Property::string(fk("x"))))
            .build()
            .unwrap()
    };
    let producer = mode_schema();
    let consumer = mode_schema();
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// Sum-type containment (the dual of records): a producer `Mode` that can emit
/// a variant the consumer does not declare is a provable `No` — the consumer
/// would face an unhandled case. This is the soundness fix: it was previously a
/// blanket `Unknown` that the binary check let through.
#[test]
fn explain_mode_extra_producer_variant_is_unhandled() {
    let producer = crate::Schema::builder()
        .property(
            Property::mode(fk("auth"))
                .variant("api_key", "API key", Property::string(fk("key")))
                .variant("oauth", "OAuth", Property::string(fk("token"))),
        )
        .build()
        .unwrap();
    // Consumer handles only `api_key` — it cannot handle the producer's `oauth`.
    let consumer = crate::Schema::builder()
        .property(Property::mode(fk("auth")).variant(
            "api_key",
            "API key",
            Property::string(fk("key")),
        ))
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::UnhandledVariant {
            key: fk("auth"),
            variant: "oauth".to_owned(),
        }]),
    );
}

/// The reverse direction is sound: a consumer that accepts *more* variants
/// than the producer can emit is still satisfied — the extra arms are never
/// taken.
#[test]
fn explain_mode_extra_consumer_variant_is_yes() {
    let producer = crate::Schema::builder()
        .property(Property::mode(fk("auth")).variant(
            "api_key",
            "API key",
            Property::string(fk("key")),
        ))
        .build()
        .unwrap();
    let consumer = crate::Schema::builder()
        .property(
            Property::mode(fk("auth"))
                .variant("api_key", "API key", Property::string(fk("key")))
                .variant("oauth", "OAuth", Property::string(fk("token"))),
        )
        .build()
        .unwrap();
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// Variant payloads are checked covariantly (producer payload → consumer
/// payload): a shared variant whose payload type mismatches is a nested `No`
/// keyed by the mode field then the payload field.
#[test]
fn explain_mode_variant_payload_mismatch_is_nested() {
    let producer = crate::Schema::builder()
        .property(Property::mode(fk("m")).variant("v", "V", Property::number(fk("x"))))
        .build()
        .unwrap();
    let consumer = crate::Schema::builder()
        .property(Property::mode(fk("m")).variant("v", "V", Property::string(fk("x"))))
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::NestedIncompat {
            key: fk("m"),
            inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                key: fk("x"),
                producer: "number",
                consumer: "string",
            }),
        }]),
    );
}

/// An undecidable payload (a `Dynamic` variant field) bubbles up as a nested
/// `Unknown`, not a blanket Mode `Unknown` — variance itself is decided.
#[test]
fn explain_mode_dynamic_payload_bubbles_unknown() {
    let producer = crate::Schema::builder()
        .property(Property::mode(fk("m")).variant("v", "V", Property::dynamic(fk("x"))))
        .build()
        .unwrap();
    let consumer = crate::Schema::builder()
        .property(Property::mode(fk("m")).variant("v", "V", Property::string(fk("x"))))
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::NestedUnknown {
            key: fk("m"),
            inner: Box::new(UnknownReason::DynamicLoaderBacked { key: fk("x") }),
        }]),
    );
}

/// An undecidable field nested inside an `Object` keeps its path: the reason
/// is `NestedUnknown { config, DynamicLoaderBacked { d } }`, not a bare
/// `DynamicLoaderBacked { d }` that a sibling object's `d` could alias.
#[test]
fn explain_nested_object_unknown_preserves_path() {
    let producer = crate::Schema::builder()
        .property(Property::object(fk("config")).property(Property::dynamic(fk("d"))))
        .build()
        .unwrap();
    // Consumer's nested `d` is optional, so the only finding is the Unknown
    // (no MissingRequiredField to dominate it).
    let consumer = crate::Schema::builder()
        .property(Property::object(fk("config")).property(Property::string(fk("d"))))
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::NestedUnknown {
            key: fk("config"),
            inner: Box::new(UnknownReason::DynamicLoaderBacked { key: fk("d") }),
        }]),
    );
}

/// An undecidable field nested inside a `List` item keeps both the list key
/// and the item key: `NestedUnknown { items, NestedUnknown { item, .. } }`.
#[test]
fn explain_nested_list_unknown_preserves_path() {
    let producer = crate::Schema::builder()
        .property(
            Property::list(fk("items"))
                .item(Property::object(fk("item")).property(Property::dynamic(fk("d")))),
        )
        .build()
        .unwrap();
    let consumer = crate::Schema::builder()
        .property(
            Property::list(fk("items"))
                .item(Property::object(fk("item")).property(Property::string(fk("d")))),
        )
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::Unknown(vec![UnknownReason::NestedUnknown {
            key: fk("items"),
            inner: Box::new(UnknownReason::NestedUnknown {
                key: fk("item"),
                inner: Box::new(UnknownReason::DynamicLoaderBacked { key: fk("d") }),
            }),
        }]),
    );
}

/// `No` dominates `Unknown` even when they arise in DIFFERENT nested
/// containers: a missing-required in object `a` wins over a dynamic field in
/// sibling object `b`, and the `Unknown` is dropped.
#[test]
fn explain_no_dominates_unknown_across_containers() {
    let producer = crate::Schema::builder()
        .property(Property::object(fk("a"))) // empty — can't satisfy a's required child
        .property(Property::object(fk("b")).property(Property::dynamic(fk("d"))))
        .build()
        .unwrap();
    let consumer = crate::Schema::builder()
        .property(Property::object(fk("a")).property(Property::string(fk("need")).required()))
        .property(Property::object(fk("b")).property(Property::string(fk("d"))))
        .build()
        .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::NestedIncompat {
            key: fk("a"),
            inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("need") }),
        }]),
        "No dominates across containers; the sibling Unknown is dropped"
    );
}

#[test]
fn unknown_reason_display_is_human_readable() {
    assert_eq!(
        UnknownReason::OpaqueProducer.to_string(),
        "producer side is opaque (shape unknown)"
    );
    assert_eq!(
        UnknownReason::DynamicLoaderBacked { key: fk("tok") }.to_string(),
        "field `tok` is dynamic/computed (resolved at runtime)"
    );
    // Nested reasons render their path prefix.
    assert_eq!(
        UnknownReason::NestedUnknown {
            key: fk("cfg"),
            inner: Box::new(UnknownReason::NumberWidening { key: fk("n") }),
        }
        .to_string(),
        "in field `cfg`: field `n` narrows float to integer (possible precision loss)"
    );
}

// ── Directional types: `OutputSchema::explain_successor_of` ─────────

/// Output-vs-output evolution: a new output that keeps every field old
/// consumers required (and adds more) is a compatible successor; one that
/// drops a required field is not. The new output is the producer, the old
/// output the consumer-expectation.
#[test]
fn output_schema_compatible_successor() {
    let prev = OutputSchema::new(required_record("result"));
    let wider = OutputSchema::new(
        crate::Schema::builder()
            .property(Property::string(fk("result")).required())
            .property(Property::string(fk("extra")).required())
            .build()
            .unwrap(),
    );
    assert_eq!(
        wider.explain_successor_of(&prev),
        Assignability::Yes,
        "adding fields keeps old consumers satisfied"
    );

    let narrower = OutputSchema::new(required_record("other"));
    assert_eq!(
        narrower.explain_successor_of(&prev),
        Assignability::No(vec![SchemaIncompat::MissingRequiredField {
            key: fk("result")
        }]),
        "dropping a field old consumers required is a breaking successor"
    );

    // Opaque evolution is unproven, not compatible. Dropping required
    // declarations remains a definite conflict.
    let any_new = OutputSchema::new(ValidSchema::any());
    assert_eq!(
        any_new.explain_successor_of(&prev),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        "a new `Any` output is not provably breaking"
    );
    let empty_new = OutputSchema::new(ValidSchema::empty());
    assert_eq!(
        empty_new.explain_successor_of(&prev),
        Assignability::No(vec![SchemaIncompat::MissingRequiredField {
            key: fk("result")
        }]),
        "an empty-record new output drops every field old consumers required"
    );
    // An old Any output imposes no structural constraints.
    let any_prev = OutputSchema::new(ValidSchema::any());
    assert_eq!(
        narrower.explain_successor_of(&any_prev),
        Assignability::Yes,
        "an `Any` old output constrains nothing"
    );
}

// ── Sum-type union (SchemaKind::Union) assignability ───────────────────────

/// An external-tagged union whose variants each carry a string payload.
fn ext_union(variant_keys: &[&str]) -> ValidSchema {
    let mut mode = Property::mode(fk("u"));
    for key in variant_keys {
        mode = mode.variant(*key, *key, Property::string(fk("x")));
    }
    ValidSchema::union(mode, SerdeTagging::External).expect("union builds")
}

/// Identical unions are provably assignable.
#[test]
fn union_identical_is_yes() {
    let producer = ext_union(&["a", "b"]);
    let consumer = ext_union(&["a", "b"]);
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// Sum-type containment via the SAME `collect_mode_variants` rule: a producer
/// union that can emit a variant the consumer does not accept is a provable
/// `No` (`UnhandledVariant`), keyed by the root mode.
#[test]
fn union_extra_producer_variant_is_unhandled() {
    let producer = ext_union(&["a", "b"]);
    let consumer = ext_union(&["a"]);
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::UnhandledVariant {
            key: fk("u"),
            variant: "b".to_owned(),
        }]),
    );
}

/// The reverse is sound: a consumer union accepting MORE variants than the
/// producer can emit is satisfied.
#[test]
fn union_extra_consumer_variant_is_yes() {
    let producer = ext_union(&["a"]);
    let consumer = ext_union(&["a", "b"]);
    assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
}

/// Variant payloads are checked covariantly: a shared variant whose payload
/// type mismatches is a nested `No` keyed by the root mode then the payload.
#[test]
fn union_variant_payload_mismatch_is_nested_no() {
    let producer = ValidSchema::union(
        Property::mode(fk("u")).variant("a", "a", Property::number(fk("x"))),
        SerdeTagging::External,
    )
    .unwrap();
    let consumer = ValidSchema::union(
        Property::mode(fk("u")).variant("a", "a", Property::string(fk("x"))),
        SerdeTagging::External,
    )
    .unwrap();
    assert_eq!(
        explain_assignable(&producer, &consumer),
        Assignability::No(vec![SchemaIncompat::NestedIncompat {
            key: fk("u"),
            inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                key: fk("x"),
                producer: "number",
                consumer: "string",
            }),
        }]),
    );
}

/// A union and a plain record are a `KindMismatch` in both directions — a
/// union value carries a discriminant a record cannot, and vice versa.
#[test]
fn union_and_record_are_kind_mismatch() {
    let union = ext_union(&["a"]);
    let record = required_record("name");
    assert_eq!(
        explain_assignable(&union, &record),
        Assignability::No(vec![SchemaIncompat::KindMismatch {
            producer: SchemaKind::Union,
            consumer: SchemaKind::Record,
        }]),
    );
    assert_eq!(
        explain_assignable(&record, &union),
        Assignability::No(vec![SchemaIncompat::KindMismatch {
            producer: SchemaKind::Record,
            consumer: SchemaKind::Union,
        }]),
    );
}

/// Gradual escapes still win over the union dispatch (the verified ordering):
/// an `Any` producer feeding a union consumer is `Unknown` (it *might* emit a
/// valid tagged value), NOT a hard `KindMismatch`.
#[test]
fn any_producer_into_union_consumer_is_unknown() {
    assert_eq!(
        explain_assignable(&ValidSchema::any(), &ext_union(&["a"])),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
    );
}

/// Only Any is universal; an empty record remains a record contract.
#[test]
fn union_producer_into_any_is_yes_but_empty_record_is_no() {
    assert_eq!(
        explain_assignable(&ext_union(&["a"]), &ValidSchema::any()),
        Assignability::Yes,
    );
    assert_eq!(
        explain_assignable(&ext_union(&["a"]), &ValidSchema::empty()),
        Assignability::No(vec![SchemaIncompat::KindMismatch {
            producer: SchemaKind::Union,
            consumer: SchemaKind::Record,
        }]),
    );
}

/// Two unions with identical variants but DIFFERENT serde tagging are not
/// assignable — their wire shapes (`{"a": ..}` vs `{tag, content}`) differ and
/// are not interconvertible, so the consumer cannot deserialize the producer.
#[test]
fn union_tagging_mismatch_is_rejected() {
    let external = ext_union(&["a"]); // SerdeTagging::External
    let adjacent = ValidSchema::union(
        Property::mode(fk("u")).variant("a", "a", Property::string(fk("x"))),
        SerdeTagging::Adjacent {
            tag: "t".to_owned(),
            content: "c".to_owned(),
        },
    )
    .unwrap();
    match explain_assignable(&external, &adjacent) {
        Assignability::No(v) => assert!(
            matches!(v.first(), Some(SchemaIncompat::UnionTaggingMismatch { .. })),
            "expected a tagging mismatch, got {v:?}"
        ),
        other => panic!("expected No(UnionTaggingMismatch), got {other:?}"),
    }
}

/// Findings keep producer-variant order: an earlier variant's payload mismatch
/// precedes a later unhandled variant (the first-incompatibility contract).
#[test]
fn union_findings_preserve_producer_variant_order() {
    let producer = ValidSchema::union(
        Property::mode(fk("u"))
            .variant("a", "a", Property::number(fk("x")))
            .variant("b", "b", Property::string(fk("y"))),
        SerdeTagging::External,
    )
    .unwrap();
    // Consumer handles only `a`, and types its payload as a string (so `a`
    // mismatches) — `b` is unhandled.
    let consumer = ext_union(&["a"]);
    match explain_assignable(&producer, &consumer) {
        Assignability::No(v) => {
            assert!(
                matches!(v[0], SchemaIncompat::NestedIncompat { .. }),
                "earlier variant `a`'s payload mismatch must come first, got {v:?}"
            );
            assert!(
                matches!(&v[1], SchemaIncompat::UnhandledVariant { variant, .. } if variant == "b"),
                "later unhandled variant `b` must come second, got {v:?}"
            );
        },
        other => panic!("expected No, got {other:?}"),
    }
}

/// Output evolution (sum-type variance): a new output union that ADDS a
/// variant is a breaking successor (old consumers can't handle it); one that
/// DROPS a variant is compatible (it just emits fewer cases).
#[test]
fn union_output_successor_variance() {
    let old = OutputSchema::new(ext_union(&["a", "b"]));
    let adds = OutputSchema::new(ext_union(&["a", "b", "c"]));
    let drops = OutputSchema::new(ext_union(&["a"]));
    assert_eq!(
        adds.explain_successor_of(&old),
        Assignability::No(vec![SchemaIncompat::UnhandledVariant {
            key: fk("u"),
            variant: "c".to_owned(),
        }]),
        "adding an output variant is breaking — old consumers can't handle it"
    );
    assert_eq!(
        drops.explain_successor_of(&old),
        Assignability::Yes,
        "dropping an output variant is compatible — fewer cases emitted"
    );
}

/// A record output cannot evolve into a union output (or vice versa).
#[test]
fn record_to_union_successor_is_kind_mismatch() {
    let old = OutputSchema::new(required_record("result"));
    let new = OutputSchema::new(ext_union(&["a"]));
    assert_eq!(
        new.explain_successor_of(&old),
        Assignability::No(vec![SchemaIncompat::KindMismatch {
            producer: SchemaKind::Union,
            consumer: SchemaKind::Record,
        }]),
    );
}

// ── explain_field_assignable (field-granularity, W0 U5) ────────────────────

/// A producer leaf's own key differs from the consumer field it is being
/// checked against (`email` vs. `recipient`) — `explain_field_assignable`
/// must re-key both to the same synthetic key internally so the pairing
/// succeeds, exactly as it would if the two keys already matched.
#[test]
fn field_assignable_pairs_leaves_under_different_keys() {
    let producer_leaf: Property = Property::string(fk("email")).required().into();
    let consumer_leaf: Property = Property::string(fk("recipient")).required().into();

    assert_eq!(
        explain_field_assignable(&producer_leaf, &consumer_leaf),
        Assignability::Yes
    );
}

#[test]
fn record_root_is_assignable_to_matching_object_field() {
    let producer = OutputSchema::new(required_record("name"));
    let consumer: Property = Property::object(fk("payload"))
        .property(Property::string(fk("name")).required())
        .into();

    assert_eq!(
        explain_root_field_assignable(&producer, &consumer),
        Assignability::Yes
    );
}

#[test]
fn record_root_is_not_assignable_to_scalar_field() {
    let producer = OutputSchema::new(required_record("name"));
    let consumer: Property = Property::string(fk("payload")).into();

    assert!(matches!(
        explain_root_field_assignable(&producer, &consumer),
        Assignability::No(incompatibilities)
            if matches!(
                incompatibilities.first(),
                Some(SchemaIncompat::FieldTypeMismatch {
                    producer: "object",
                    consumer: "string",
                    ..
                })
            )
    ));
}

#[test]
fn scalar_root_uses_concrete_scalar_kind() {
    let string_root = OutputSchema::new(
        ValidSchema::scalar(ScalarSchema::string()).expect("string root is valid"),
    );
    let string_field: Property = Property::string(fk("payload")).into();
    let number_field: Property = Property::number(fk("payload")).into();

    assert_eq!(
        explain_root_field_assignable(&string_root, &string_field),
        Assignability::Yes
    );
    assert!(matches!(
        explain_root_field_assignable(&string_root, &number_field),
        Assignability::No(_)
    ));
}

/// A provable type mismatch between the two leaves is `No`, carrying a
/// `FieldTypeMismatch` under the consumer's own key.
#[test]
fn field_assignable_type_mismatch_is_no() {
    let producer_leaf: Property = Property::number(fk("age")).into();
    let consumer_leaf: Property = Property::string(fk("name")).required().into();

    match explain_field_assignable(&producer_leaf, &consumer_leaf) {
        Assignability::No(incompats) => {
            assert!(incompats.iter().any(|i| matches!(
                i,
                SchemaIncompat::FieldTypeMismatch { key, .. } if key.as_str() == "name"
            )));
        },
        other => panic!("expected No(FieldTypeMismatch), got {other:?}"),
    }
}

/// A float producer feeding an integer consumer is `Unknown` (possible
/// precision loss, not a provable conflict) — the same `NumberWidening`
/// leniency the schema-level check applies.
#[test]
fn field_assignable_float_to_int_is_unknown() {
    let producer_leaf: Property = Property::number(fk("amount")).into();
    let consumer_leaf: Property = Property::integer(fk("qty")).required().into();

    assert_eq!(
        explain_field_assignable(&producer_leaf, &consumer_leaf),
        Assignability::Unknown(vec![UnknownReason::NumberWidening { key: fk("qty") }])
    );
}

/// A `Property::Unknown` producer leaf at a key DIFFERENT from the required
/// consumer leaf it is checked against must still report
/// `Unknown(OpaqueFieldKind)` — not a spurious `No(MissingRequiredField)`.
///
/// `ValidSchema::single_field`'s re-key (`rekeyed`) cannot rewrite an
/// `Unknown` field's private key, so without the early `Property::Unknown`
/// check in `explain_field_assignable`, the producer's synthetic
/// one-field schema would keep the leaf's ORIGINAL key (`"bio"`) instead
/// of the consumer's key (`"recipient_email"`) — the two synthetic
/// schemas would fail to pair up, and `collect_fields` would read that as
/// the required consumer field being entirely absent from the producer.
#[test]
fn field_assignable_unknown_producer_leaf_mismatched_key_is_unknown_not_missing() {
    let unknown_schema: ValidSchema =
        serde_json::from_value(json!({"fields": [{"type": "richtext", "key": "bio"}]}))
            .expect("Unknown producer schema");
    let producer_leaf = unknown_schema.properties()[0].clone();
    assert!(
        matches!(producer_leaf, Property::Unknown(_)),
        "sanity: leaf must be Property::Unknown"
    );
    assert_eq!(producer_leaf.key().as_str(), "bio");

    let consumer_leaf: Property = Property::string(fk("recipient_email")).required().into();

    assert_eq!(
        explain_field_assignable(&producer_leaf, &consumer_leaf),
        Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind {
            key: fk("recipient_email")
        }]),
        "an Unknown producer leaf at a different key must report OpaqueFieldKind, not \
         silently fail to pair up as a spurious MissingRequiredField"
    );
}
