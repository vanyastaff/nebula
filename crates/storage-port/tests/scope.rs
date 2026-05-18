use nebula_storage_port::{FencingToken, Scope};

#[test]
fn scope_equality_and_serde() {
    let s = Scope::new("ws_1", "org_1");
    let j = serde_json::to_string(&s).expect("scope serializes");
    let back: Scope = serde_json::from_str(&j).expect("scope deserializes");
    assert_eq!(s, back);
}

#[test]
fn fencing_token_is_monotone_comparable() {
    assert!(FencingToken::from_generation(1) < FencingToken::from_generation(2));
}
