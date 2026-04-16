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

    // Organization, Workspace, Workflow, Execution all serialize with prefixed ULIDs
    let org_id = OrgId::new();
    let org_scope = ScopeLevel::Organization(org_id);
    let org_json = serde_json::to_string(&org_scope).unwrap();
    assert!(
        org_json.contains("org_"),
        "org scope should contain org_ prefix"
    );
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
        (
            CoreError::ScopeViolation {
                actor: "a".into(),
                target: "b".into(),
            },
            "CORE:SCOPE_VIOLATION",
        ),
        (
            CoreError::DependencyCycle {
                path: vec!["a", "b"],
            },
            "CORE:DEPENDENCY_CYCLE",
        ),
        (
            CoreError::DependencyMissing {
                name: "x",
                required_by: "y",
            },
            "CORE:DEPENDENCY_MISSING",
        ),
    ];
    for (err, expected_code) in errors {
        assert_eq!(err.code().as_str(), expected_code, "CoreError code changed");
    }
}
