//! Schema contract tests for nebula-core (Phase 2: Compatibility Contracts).
//!
//! These tests verify the serialization format of boundary types after
//! the ULID migration. ID types now use prefixed ULID format.

use nebula_core::{
    CoreError,
    prelude::{ExecutionId, OrgId, ScopeLevel, WorkflowId},
};

#[test]
fn scope_level_serialization_contract() {
    let global_json = serde_json::to_string(&ScopeLevel::Global).unwrap();
    assert_eq!(global_json, "\"Global\"");

    // Organization serializes as {"Organization":"org_<ULID>"}
    let org_id = OrgId::new();
    let org_scope = ScopeLevel::Organization(org_id);
    let org_value: serde_json::Value = serde_json::to_value(&org_scope).unwrap();
    let org_obj = org_value
        .as_object()
        .expect("Organization scope must serialize as JSON object");
    assert!(
        org_obj.contains_key("Organization"),
        "key must be 'Organization'"
    );
    let org_id_str = org_obj["Organization"]
        .as_str()
        .expect("value must be a string");
    assert!(
        org_id_str.starts_with("org_"),
        "org ID must have org_ prefix, got: {org_id_str}"
    );
    // Verify the ID round-trips
    let parsed_org: OrgId = org_id_str.parse().expect("org ID in JSON must parse back");
    assert_eq!(parsed_org, org_id);
}

#[test]
fn id_types_serialize_as_prefixed_ulid() {
    let id = ExecutionId::new();
    let json = serde_json::to_string(&id).unwrap();
    // Should be a quoted string starting with exe_
    assert!(
        json.starts_with("\"exe_"),
        "expected exe_ prefix, got: {json}"
    );
    assert!(json.ends_with('"'));
}

#[test]
fn id_parse_roundtrip() {
    let id = WorkflowId::new();
    let json = serde_json::to_string(&id).unwrap();
    let parsed: WorkflowId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, parsed);
}

#[test]
fn core_error_code_stability() {
    use nebula_error::Classify;

    let errors = [
        (CoreError::invalid_id("x", "exe"), "CORE:INVALID_ID"),
        (CoreError::invalid_key("x", "action"), "CORE:INVALID_KEY"),
        (CoreError::scope_violation("a", "b"), "CORE:SCOPE_VIOLATION"),
        (
            CoreError::dependency_cycle(vec!["a", "b"]),
            "CORE:DEPENDENCY_CYCLE",
        ),
        (
            CoreError::dependency_missing("x", "y"),
            "CORE:DEPENDENCY_MISSING",
        ),
    ];
    for (err, expected_code) in errors {
        assert_eq!(err.code().as_str(), expected_code, "CoreError code changed");
    }
}
