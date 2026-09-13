//! Proptest: algebraic laws of `AuthoredValue::canonical_bytes`.
//!
//! The canon is the basis for content-addressing / dedup, so its two
//! load-bearing properties — determinism and injectivity — are checked against
//! randomly generated values rather than only hand-picked cases.

use indexmap::IndexMap;
use nebula_schema::{AuthoredValue, Expression, ProgramSyntax};
use proptest::prelude::*;
use serde_json::{Value, json};

/// A bounded recursive strategy for JSON values (the input domain of
/// `AuthoredValue::from_data`). Kept shallow so recursive collections stay cheap.
fn json_strategy() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        // Integral floats exercise the `1.0`-normalizes-to-`1` path.
        (-1000i32..1000).prop_map(|n| json!(f64::from(n))),
        ".{0,6}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::hash_map(".{0,4}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn canon_separates_syntax_and_survives_authored_wire(source in any::<String>()) {
        let mut canons = Vec::new();
        for syntax in [ProgramSyntax::Auto, ProgramSyntax::Expression, ProgramSyntax::Template] {
            let authored = AuthoredValue::Expression(Expression::with_syntax(source.clone(), syntax));
            let bytes = authored.canonical_bytes().unwrap();
            prop_assert!(!canons.contains(&bytes));
            let decoded: AuthoredValue = serde_json::from_slice(&serde_json::to_vec(&authored).unwrap()).unwrap();
            prop_assert_eq!(decoded.canonical_bytes().unwrap(), bytes.clone());
            prop_assert_eq!(decoded.content_id().unwrap(), authored.content_id().unwrap());
            canons.push(bytes);
        }
    }

    /// `canonical_bytes` is a pure, **succeeding** function over the secret-free,
    /// finite-float domain, prefixed with the domain separator + version.
    #[test]
    fn canon_is_deterministic(v in json_strategy()) {
        let fv = AuthoredValue::from_data(v).expect("bounded JSON data");
        let once = fv.canonical_bytes().expect("json_strategy is secret-free and finite");
        let twice = fv.canonical_bytes().expect("deterministic");
        prop_assert_eq!(&once, &twice);
        prop_assert!(once.starts_with(b"nbschema-value-v"), "carries the domain separator");
        prop_assert_eq!(&once[16..18], &[0x00, 0x02], "carries VALUE_CANON_VERSION = 2");
    }

    /// Cross-shape injectivity over random content: a single-element list and a
    /// single-key object wrapping the SAME value never collide (distinct tags).
    #[test]
    fn canon_distinguishes_list_from_object(v in json_strategy()) {
        let as_list = AuthoredValue::from_data(json!([v])).unwrap();
        let as_object = AuthoredValue::from_data(json!({ "k": v })).unwrap();
        prop_assert_ne!(
            as_list.canonical_bytes().unwrap(),
            as_object.canonical_bytes().unwrap()
        );
    }

    /// Object key insertion order does not affect the canon: a value built from a
    /// JSON object equals the same object built from its key-reversed form.
    #[test]
    fn canon_ignores_object_key_order(
        pairs in prop::collection::vec((".{0,4}", any::<i64>()), 1..6)
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
        let mut forward = IndexMap::new();
        for (k, n) in &unique {
            forward.insert(k.clone(), AuthoredValue::from_data(json!(n)).unwrap());
        }
        let mut reverse = IndexMap::new();
        for (k, n) in unique.iter().rev() {
            reverse.insert(k.clone(), AuthoredValue::from_data(json!(n)).unwrap());
        }
        let a = AuthoredValue::Object(forward);
        let b = AuthoredValue::Object(reverse);
        prop_assert_eq!(a.canonical_bytes().unwrap(), b.canonical_bytes().unwrap());
    }

    /// Injectivity sample: two values that differ only in one integer field have
    /// distinct canons (the canon never silently collapses distinct data).
    #[test]
    fn canon_separates_distinct_values(a in any::<i64>(), b in any::<i64>()) {
        prop_assume!(a != b);
        let va = AuthoredValue::from_data(json!({"n": a})).unwrap();
        let vb = AuthoredValue::from_data(json!({"n": b})).unwrap();
        prop_assert_ne!(va.canonical_bytes().unwrap(), vb.canonical_bytes().unwrap());
    }
}
