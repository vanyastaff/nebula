//! Proptest: algebraic laws of the opt-in keyed secret commitment
//! (`FieldValue::canonical_bytes_committing`).
//!
//! The committing path must (1) be byte-identical to the default canon on any
//! secret-free value — the key path is only entered for a `SecretLiteral` — and
//! (2) be a deterministic, injective PRF over secrets under a fixed key.

use nebula_schema::{CommitmentKey, FieldKey, FieldValue, FieldValues, SecretValue};
use proptest::prelude::*;
use serde_json::{Value, json};

/// Bounded secret-free JSON (same domain as `FieldValue::from_json`).
fn json_strategy() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
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

fn key() -> CommitmentKey {
    CommitmentKey::for_testing([42u8; 32])
}

proptest! {
    /// On any secret-free value the committing path is byte-identical to the
    /// default canon: the key is only consulted for a `SecretLiteral`.
    #[test]
    fn committing_equals_default_when_secret_free(v in json_strategy()) {
        let fv = FieldValue::from_json(v);
        let default = fv.canonical_bytes().expect("secret-free, finite");
        let committed = fv.canonical_bytes_committing(&key()).expect("secret-free, finite");
        prop_assert_eq!(default, committed);
    }

    /// A committed secret is deterministic under a fixed key.
    #[test]
    fn secret_commit_is_deterministic(s in "[a-zA-Z0-9]{0,32}") {
        let fv = FieldValue::SecretLiteral(SecretValue::string(s));
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
        let ca = FieldValue::SecretLiteral(SecretValue::string(a))
            .canonical_bytes_committing(&k).expect("commit");
        let cb = FieldValue::SecretLiteral(SecretValue::string(b))
            .canonical_bytes_committing(&k).expect("commit");
        prop_assert_ne!(ca, cb);
    }

    /// The default (rejecting) path is unaffected by the new policy threading: a
    /// secret-bearing store still has no canon.
    #[test]
    fn default_still_rejects_secret_in_store(s in "[a-z]{0,16}") {
        let mut values = FieldValues::new();
        values.set(
            FieldKey::new("k").expect("valid key"),
            FieldValue::SecretLiteral(SecretValue::string(s)),
        );
        prop_assert!(values.canonical_bytes().is_err(), "secret store has no default canon");
        prop_assert!(values.canonical_bytes_committing(&key()).is_ok(), "but commits under a key");
    }
}
