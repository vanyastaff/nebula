//! Proptest: algebraic laws of `FieldValue::canonical_bytes`.
//!
//! The canon is the basis for content-addressing / dedup, so its two
//! load-bearing properties — determinism and injectivity — are checked against
//! randomly generated values rather than only hand-picked cases.

use nebula_schema::{FieldValue, FieldValues};
use proptest::prelude::*;
use serde_json::{Value, json};

/// A bounded recursive strategy for JSON values (the input domain of
/// `FieldValue::from_json`). Kept shallow so the recursive collections stay cheap.
fn json_strategy() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        // Integral floats exercise the `1.0`-normalizes-to-`1` path.
        (-1000i64..1000).prop_map(|n| json!(n as f64)),
        "[a-z]{0,6}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::hash_map("[a-z]{1,4}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    /// `canonical_bytes` is a pure, **succeeding** function over the secret-free,
    /// finite-float domain, prefixed with the domain separator + version.
    #[test]
    fn canon_is_deterministic(v in json_strategy()) {
        let fv = FieldValue::from_json(v);
        let once = fv.canonical_bytes().expect("json_strategy is secret-free and finite");
        let twice = fv.canonical_bytes().expect("deterministic");
        prop_assert_eq!(&once, &twice);
        prop_assert!(once.starts_with(b"nbschema-value-v"), "carries the domain separator");
        prop_assert_eq!(&once[16..18], &[0x00, 0x01], "carries VALUE_CANON_VERSION = 1");
    }

    /// Cross-shape injectivity over random content: a single-element list and a
    /// single-key object wrapping the SAME value never collide (distinct tags).
    #[test]
    fn canon_distinguishes_list_from_object(v in json_strategy()) {
        let as_list = FieldValue::from_json(json!([v]));
        let as_object = FieldValue::from_json(json!({ "k": v }));
        prop_assert_ne!(
            as_list.canonical_bytes().unwrap(),
            as_object.canonical_bytes().unwrap()
        );
    }

    /// Object key insertion order does not affect the canon: a value built from a
    /// JSON object equals the same object built from its key-reversed form.
    #[test]
    fn canon_ignores_object_key_order(
        pairs in prop::collection::vec(("[a-z]{1,4}", any::<i64>()), 1..6)
    ) {
        // Collapse to unique keys (first occurrence wins) so forward and reverse
        // describe the SAME mapping in opposite insertion orders — otherwise a
        // duplicate key would make "last inserted wins" diverge between the two.
        let mut unique: Vec<(String, i64)> = Vec::new();
        for (k, n) in &pairs {
            if !unique.iter().any(|(seen, _)| seen == k) {
                unique.push((k.clone(), *n));
            }
        }
        let mut forward = serde_json::Map::new();
        for (k, n) in &unique {
            forward.insert(k.clone(), json!(n));
        }
        let mut reverse = serde_json::Map::new();
        for (k, n) in unique.iter().rev() {
            reverse.insert(k.clone(), json!(n));
        }
        let a = FieldValue::from_json(Value::Object(forward));
        let b = FieldValue::from_json(Value::Object(reverse));
        prop_assert_eq!(a.canonical_bytes().unwrap(), b.canonical_bytes().unwrap());
    }

    /// Injectivity sample: two values that differ only in one integer field have
    /// distinct canons (the canon never silently collapses distinct data).
    #[test]
    fn canon_separates_distinct_values(a in any::<i64>(), b in any::<i64>()) {
        prop_assume!(a != b);
        let va = FieldValues::from_json(json!({"n": a})).unwrap();
        let vb = FieldValues::from_json(json!({"n": b})).unwrap();
        prop_assert_ne!(va.canonical_bytes().unwrap(), vb.canonical_bytes().unwrap());
    }
}
