//! Smoke tests for [`IdempotencyKey`].
//!
//! Per Tech Spec §15.12 F2 — `TriggerAction::idempotency_key()` returns
//! `Option<IdempotencyKey>`; this type is the concrete return.

use nebula_action::IdempotencyKey;

#[test]
fn new_round_trips_string() {
    let k = IdempotencyKey::new("delivery-abc-123");
    assert_eq!(k.as_str(), "delivery-abc-123");
}

#[test]
fn equality_is_string_equality() {
    let a = IdempotencyKey::new("x");
    let b = IdempotencyKey::new("x");
    let c = IdempotencyKey::new("y");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn hash_matches_string() {
    use std::collections::HashSet;
    let mut s: HashSet<IdempotencyKey> = HashSet::new();
    s.insert(IdempotencyKey::new("x"));
    assert!(s.contains(&IdempotencyKey::new("x")));
    assert!(!s.contains(&IdempotencyKey::new("y")));
}

#[test]
fn debug_redacts_nothing() {
    // IdempotencyKey is NOT a secret — engine logs / metrics may include it.
    // (Per Tech Spec §15.12 F2, key is a stable transport-level dedup id,
    // not a credential.) This test pins the contract.
    let k = IdempotencyKey::new("delivery-abc-123");
    let debug = format!("{k:?}");
    assert!(debug.contains("delivery-abc-123"));
}
