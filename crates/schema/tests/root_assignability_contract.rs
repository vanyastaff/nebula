//! Root-shape assignability must prove a producer fits the consumer contract.

use nebula_schema::{
    Assignability, Field, InputSchema, OutputSchema, ScalarKind, ScalarSchema, Schema,
    SchemaIncompat, SchemaKind, SerdeTagging, UnknownReason, ValidSchema, explain_assignable,
    field_key,
};
use nebula_validator::{Predicate, Rule};
use proptest::prelude::*;
use serde_json::{Number, json};
use std::assert_matches;

fn verdict(producer: ValidSchema, consumer: ValidSchema) -> Assignability {
    explain_assignable(&OutputSchema::new(producer), &InputSchema::new(consumer))
}

fn record() -> ValidSchema {
    Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .build()
        .unwrap()
}

fn union() -> ValidSchema {
    ValidSchema::union(
        Field::mode(field_key!("choice")).variant(
            "text",
            "Text",
            Field::string(field_key!("text")),
        ),
        SerdeTagging::External,
    )
    .unwrap()
}

fn scalar(schema: ScalarSchema) -> ValidSchema {
    ValidSchema::scalar(schema).unwrap()
}

fn integer(minimum: impl Into<Number>, maximum: impl Into<Number>) -> ValidSchema {
    scalar(ScalarSchema::integer(minimum, maximum).unwrap())
}

fn number(minimum: impl Into<Number>, maximum: impl Into<Number>) -> ValidSchema {
    scalar(ScalarSchema::number(minimum, maximum).unwrap())
}

fn concrete_roots() -> Vec<ValidSchema> {
    vec![
        scalar(ScalarSchema::null()),
        scalar(ScalarSchema::boolean()),
        scalar(ScalarSchema::string()),
        integer(-10, 10),
        number(-10, 10),
        ValidSchema::empty(),
        record(),
        union(),
    ]
}

#[test]
fn any_producer_does_not_prove_an_empty_record_consumer() {
    assert_eq!(
        verdict(ValidSchema::any(), ValidSchema::empty()),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
    );
}

#[test]
fn union_producer_does_not_prove_an_empty_record_consumer() {
    assert_eq!(
        verdict(union(), ValidSchema::empty()),
        Assignability::No(vec![SchemaIncompat::KindMismatch {
            producer: SchemaKind::Union,
            consumer: SchemaKind::Record,
        }]),
    );
}

#[test]
fn any_consumer_accepts_every_existing_root_shape() {
    for producer in concrete_roots().into_iter().chain([ValidSchema::any()]) {
        assert_eq!(verdict(producer, ValidSchema::any()), Assignability::Yes);
    }
}

#[test]
fn any_producer_is_unknown_for_every_concrete_root() {
    for consumer in concrete_roots() {
        assert_eq!(
            verdict(ValidSchema::any(), consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        );
    }
}

#[test]
fn different_concrete_root_kinds_are_not_assignable() {
    for producer in concrete_roots() {
        for consumer in concrete_roots() {
            if producer.kind() != consumer.kind() {
                assert_eq!(
                    verdict(producer.clone(), consumer.clone()),
                    Assignability::No(vec![SchemaIncompat::KindMismatch {
                        producer: producer.kind(),
                        consumer: consumer.kind(),
                    }]),
                );
            }
        }
    }
}

#[test]
fn disjoint_scalar_kinds_are_not_assignable() {
    let scalars = [
        ScalarSchema::null(),
        ScalarSchema::boolean(),
        ScalarSchema::string(),
        ScalarSchema::integer(-10, 10).unwrap(),
        ScalarSchema::number(-10, 10).unwrap(),
    ];
    for producer in &scalars {
        for consumer in &scalars {
            if producer.kind() == consumer.kind() {
                assert_eq!(
                    verdict(scalar(producer.clone()), scalar(consumer.clone())),
                    Assignability::Yes
                );
            } else if !matches!(
                (producer.kind(), consumer.kind()),
                (ScalarKind::Integer, ScalarKind::Number)
                    | (ScalarKind::Number, ScalarKind::Integer)
            ) {
                assert_eq!(
                    verdict(scalar(producer.clone()), scalar(consumer.clone())),
                    Assignability::No(vec![SchemaIncompat::ScalarKindMismatch {
                        producer: producer.kind(),
                        consumer: consumer.kind(),
                    }]),
                );
            }
        }
    }
}

#[test]
fn integer_to_number_widens_but_reverse_remains_unknown() {
    assert_eq!(
        verdict(integer(-10, 10), number(-20, 20)),
        Assignability::Yes
    );
    assert_eq!(
        verdict(number(-10, 10), integer(-20, 20)),
        Assignability::Unknown(vec![UnknownReason::ScalarNarrowing {
            producer: ScalarKind::Number,
            consumer: ScalarKind::Integer,
        }]),
    );
}

#[test]
fn numeric_bounds_distinguish_containment_overlap_and_disjointness() {
    assert_eq!(verdict(integer(1, 9), integer(0, 10)), Assignability::Yes);
    assert_matches!(
        verdict(integer(0, 10), integer(1, 9)),
        Assignability::Unknown(_)
    );
    assert_matches!(
        verdict(integer(0, 10), integer(10, 20)),
        Assignability::Unknown(_)
    );
    assert_eq!(
        verdict(integer(0, 9), integer(10, 20)),
        Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint])
    );
    assert_eq!(
        verdict(integer(10, 20), integer(0, 9)),
        Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint])
    );
    assert_eq!(
        verdict(integer(0, 0), integer(1, 1)),
        Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint])
    );
}

#[test]
fn mixed_numeric_bounds_do_not_round_adjacent_large_integers() {
    let exact_float = Number::from_f64(9_007_199_254_740_992.0).unwrap();
    let next_integer = 9_007_199_254_740_993_u64;
    assert_eq!(
        verdict(
            integer(next_integer, next_integer),
            number(0, exact_float.clone())
        ),
        Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint]),
    );
    assert_matches!(
        verdict(integer(0, next_integer), number(0, exact_float.clone())),
        Assignability::Unknown(_),
    );
    assert_eq!(
        verdict(integer(0, next_integer - 1), number(0, exact_float)),
        Assignability::Yes,
    );

    let two_to_64 = Number::from_f64(18_446_744_073_709_551_616.0).unwrap();
    assert_eq!(
        verdict(integer(0, u64::MAX), number(0, two_to_64.clone())),
        Assignability::Yes
    );
    assert_eq!(
        verdict(number(two_to_64.clone(), two_to_64), integer(0, u64::MAX)),
        Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint]),
    );
    assert_eq!(
        verdict(integer(i64::MIN, u64::MAX), number(i64::MIN, u64::MAX)),
        Assignability::Yes
    );
}

#[test]
fn unproven_root_rules_are_not_a_compatible_contract() {
    let consumer = ScalarSchema::string().root_rule(Rule::max_length(4));
    assert_eq!(
        verdict(scalar(ScalarSchema::string()), scalar(consumer)),
        Assignability::Unknown(vec![UnknownReason::UnprovenRootRules])
    );

    let consumer = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .root_rule(
            Rule::predicate(Predicate::eq("/name", json!("private-rule-marker")).unwrap())
                .expect("bounded root predicate"),
        )
        .build()
        .unwrap();
    let result = verdict(record(), consumer);
    assert_eq!(
        result,
        Assignability::Unknown(vec![UnknownReason::UnprovenRootRules])
    );
    assert!(!format!("{result:?}").contains("private-rule-marker"));
}

#[test]
fn identical_value_rules_can_be_proven_without_rule_execution() {
    let rule = Rule::max_length(4);
    let producer = ScalarSchema::string().root_rule(rule.clone());
    let consumer = ScalarSchema::string().root_rule(rule);
    assert_eq!(
        verdict(scalar(producer), scalar(consumer)),
        Assignability::Yes
    );
}

#[test]
fn structural_no_dominates_unproven_root_rules() {
    let consumer = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .root_rule(
            Rule::predicate(Predicate::eq("/name", json!("private-rule-marker")).unwrap())
                .expect("bounded root predicate"),
        )
        .build()
        .unwrap();
    assert_eq!(
        verdict(ValidSchema::empty(), consumer),
        Assignability::No(vec![SchemaIncompat::MissingRequiredField {
            key: field_key!("name")
        }]),
    );
}

#[test]
fn root_identity_does_not_collapse_the_type_dag() {
    let roots: Vec<_> = concrete_roots()
        .into_iter()
        .chain([ValidSchema::any()])
        .collect();
    for (index, root) in roots.iter().enumerate() {
        for other in &roots[index + 1..] {
            assert_ne!(root, other);
        }
    }
    assert_ne!(integer(0, 9), integer(0, 10));
    assert_ne!(
        scalar(ScalarSchema::string()),
        scalar(ScalarSchema::string().root_rule(Rule::max_length(4)))
    );
}

#[test]
fn successor_verdict_cannot_hide_an_opaque_or_narrowed_output() {
    let previous = OutputSchema::new(integer(0, 10));
    assert_eq!(
        OutputSchema::new(ValidSchema::any()).explain_successor_of(&previous),
        Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
    );
    assert_eq!(
        OutputSchema::new(integer(1, 9)).explain_successor_of(&previous),
        Assignability::Yes
    );
    assert_matches!(
        OutputSchema::new(integer(0, 11)).explain_successor_of(&previous),
        Assignability::Unknown(_)
    );
}

proptest! {
    #[test]
    fn signed_integer_interval_relation_matches_exact_order(
        producer_bounds in any::<(i64, i64)>(),
        consumer_bounds in any::<(i64, i64)>(),
    ) {
        let (p0, p1) = producer_bounds;
        let (c0, c1) = consumer_bounds;
        let (pmin, pmax) = (p0.min(p1), p0.max(p1));
        let (cmin, cmax) = (c0.min(c1), c0.max(c1));
        let actual = verdict(integer(pmin, pmax), integer(cmin, cmax));
        if pmax < cmin || pmin > cmax {
            prop_assert_eq!(actual, Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint]));
        } else if pmin >= cmin && pmax <= cmax {
            prop_assert_eq!(actual, Assignability::Yes);
        } else {
            prop_assert_eq!(actual, Assignability::Unknown(vec![UnknownReason::ScalarNarrowing {
                producer: ScalarKind::Integer,
                consumer: ScalarKind::Integer,
            }]));
        }
    }

    #[test]
    fn mixed_signed_unsigned_domains_match_wide_integer_order(
        producer_bounds in any::<(i64, i64)>(),
        consumer_bounds in any::<(u64, u64)>(),
    ) {
        let (p0, p1) = producer_bounds;
        let (c0, c1) = consumer_bounds;
        let (pmin, pmax) = (p0.min(p1), p0.max(p1));
        let (cmin, cmax) = (c0.min(c1), c0.max(c1));
        let actual = verdict(integer(pmin, pmax), number(cmin, cmax));
        if i128::from(pmax) < i128::from(cmin) || i128::from(pmin) > i128::from(cmax) {
            prop_assert_eq!(actual, Assignability::No(vec![SchemaIncompat::ScalarBoundsDisjoint]));
        } else if i128::from(pmin) >= i128::from(cmin) && i128::from(pmax) <= i128::from(cmax) {
            prop_assert_eq!(actual, Assignability::Yes);
        } else {
            prop_assert_eq!(actual, Assignability::Unknown(vec![UnknownReason::ScalarNarrowing {
                producer: ScalarKind::Integer,
                consumer: ScalarKind::Number,
            }]));
        }
    }
}

#[test]
fn empty_record_consumer_preserves_record_width_subtyping() {
    assert_eq!(verdict(record(), ValidSchema::empty()), Assignability::Yes);
    assert_eq!(
        verdict(ValidSchema::empty(), record()),
        Assignability::No(vec![SchemaIncompat::MissingRequiredField {
            key: field_key!("name"),
        }]),
    );
}
