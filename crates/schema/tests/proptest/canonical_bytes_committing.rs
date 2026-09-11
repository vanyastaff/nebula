//! Proptest: algebraic laws of the opt-in keyed secret commitment
//! (`AuthoredValue::canonical_bytes_committing`).
//!
//! The committing path must (1) be byte-identical to the default canon on any
//! secret-free value — the key path is only entered for a `Secret` — and
//! (2) be a deterministic, injective PRF over secrets under a fixed key.

use nebula_schema::{AuthoredValue, CommitmentKey, Expression, ProgramSyntax, SecretValue};
use proptest::prelude::*;
use serde_json::{Value, json};

/// Bounded secret-free JSON (same domain as `AuthoredValue::from_data`).
fn json_strategy() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
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

fn key() -> CommitmentKey {
    CommitmentKey::for_testing([42u8; 32])
}

proptest! {
    #[test]
    fn syntax_distinguishes_commitments_even_beside_secrets(source in any::<String>(), secret in any::<String>()) {
        let key = key();
        let mut commitments = Vec::new();
        for syntax in [ProgramSyntax::Auto, ProgramSyntax::Expression, ProgramSyntax::Template] {
            let expression = AuthoredValue::Expression(Expression::with_syntax(source.clone(), syntax));
            prop_assert_eq!(expression.canonical_bytes().unwrap(), expression.canonical_bytes_committing(&key).unwrap());
            let tree = AuthoredValue::List(vec![expression, AuthoredValue::Secret(SecretValue::string(secret.clone()))]);
            let commitment = tree.content_id_committing(&key).unwrap();
            prop_assert!(!commitments.contains(&commitment));
            prop_assert_eq!(&commitment, &tree.content_id_committing(&key).unwrap());
            commitments.push(commitment);
        }
    }

    /// On any secret-free value the committing path is byte-identical to the
    /// default canon: the key is only consulted for a `Secret`.
    #[test]
    fn committing_equals_default_when_secret_free(v in json_strategy()) {
        let fv = AuthoredValue::from_data(v).expect("bounded JSON data");
        let default = fv.canonical_bytes().expect("secret-free, finite");
        let committed = fv.canonical_bytes_committing(&key()).expect("secret-free, finite");
        prop_assert_eq!(default, committed);
    }

    /// A committed secret is deterministic under a fixed key.
    #[test]
    fn secret_commit_is_deterministic(s in "[a-zA-Z0-9]{0,32}") {
        let fv = AuthoredValue::Secret(SecretValue::string(s));
        let k = key();
        prop_assert_eq!(
            fv.canonical_bytes_committing(&k).expect("commit"),
            fv.canonical_bytes_committing(&k).expect("commit"),
        );
    }

    /// Distinct secrets commit to distinct bytes under one key (PRF injectivity;
    /// a collision here is a 2^-256 event and treated as failure).
    #[test]
    fn distinct_secrets_commit_differently(a in "[a-z]{1,16}", b in "[a-z]{1,16}") {
        prop_assume!(a != b);
        let k = key();
        let ca = AuthoredValue::Secret(SecretValue::string(a))
            .canonical_bytes_committing(&k).expect("commit");
        let cb = AuthoredValue::Secret(SecretValue::string(b))
            .canonical_bytes_committing(&k).expect("commit");
        prop_assert_ne!(ca, cb);
    }

    /// The default (rejecting) path is unaffected by the new policy threading: a
    /// secret-bearing store still has no canon.
    #[test]
    fn default_still_rejects_secret_in_store(s in "[a-z]{0,16}") {
        let mut values = AuthoredValue::object();
        values.insert("k", AuthoredValue::Secret(SecretValue::string(s))).unwrap();
        let error = values.canonical_bytes().unwrap_err();
        prop_assert_eq!(error.code(), "secret.not_hashable");
        let committed = values.canonical_bytes_committing(&key()).expect("explicit keyed commitment");
        prop_assert!(committed.starts_with(b"nbschema-value-v\x00\x02"));
        prop_assert_eq!(committed.len(), 56, "object framing plus one full-width secret commitment");
    }
}
