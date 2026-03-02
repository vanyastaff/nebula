//! Schema contract tests for nebula-core (Phase 2: Compatibility Contracts).
//!
//! These tests freeze the JSON serialization of boundary types and CoreError
//! codes. Changing the output format is a breaking change — update expected
//! values intentionally and document in MIGRATION.md.

use nebula_core::prelude::{
    ExecutionId, InterfaceVersion, NodeId, OrganizationId, ProjectId, ProjectType, RoleScope,
    ScopeLevel, WorkflowId,
};
use nebula_core::types::{Priority, Status};

#[test]
fn status_serialization_contract() {
    let cases = [
        (Status::Active, "\"Active\""),
        (Status::Inactive, "\"Inactive\""),
        (Status::InProgress, "\"InProgress\""),
        (Status::Completed, "\"Completed\""),
        (Status::Failed, "\"Failed\""),
        (Status::Pending, "\"Pending\""),
        (Status::Cancelled, "\"Cancelled\""),
        (Status::Suspended, "\"Suspended\""),
        (Status::Error, "\"Error\""),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, expected,
            "Status serialization changed for {:?}",
            value
        );
    }
}

#[test]
fn priority_serialization_contract() {
    let cases = [
        (Priority::Low, "\"Low\""),
        (Priority::Normal, "\"Normal\""),
        (Priority::High, "\"High\""),
        (Priority::Critical, "\"Critical\""),
        (Priority::Emergency, "\"Emergency\""),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, expected,
            "Priority serialization changed for {:?}",
            value
        );
    }
}

#[test]
fn project_type_serialization_contract() {
    let cases = [
        (ProjectType::Personal, "\"personal\""),
        (ProjectType::Team, "\"team\""),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, expected,
            "ProjectType serialization changed for {:?}",
            value
        );
    }
}

#[test]
fn role_scope_serialization_contract() {
    let cases = [
        (RoleScope::Global, "\"global\""),
        (RoleScope::Project, "\"project\""),
        (RoleScope::Credential, "\"credential\""),
        (RoleScope::Workflow, "\"workflow\""),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, expected,
            "RoleScope serialization changed for {:?}",
            value
        );
    }
}

#[test]
fn interface_version_serialization_contract() {
    let v = InterfaceVersion::new(1, 2);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#"{"major":1,"minor":2}"#);
}

#[test]
fn scope_level_serialization_contract() {
    let org_id = OrganizationId::nil();
    let proj_id = ProjectId::nil();
    let wf_id = WorkflowId::nil();
    let exec_id = ExecutionId::nil();
    let node_id = NodeId::nil();
    let nil_str = "00000000-0000-0000-0000-000000000000";

    let cases = [
        (ScopeLevel::Global, "\"Global\""),
        (
            ScopeLevel::Organization(org_id),
            &format!(r#"{{"Organization":"{}"}}"#, nil_str),
        ),
        (
            ScopeLevel::Project(proj_id),
            &format!(r#"{{"Project":"{}"}}"#, nil_str),
        ),
        (
            ScopeLevel::Workflow(wf_id),
            &format!(r#"{{"Workflow":"{}"}}"#, nil_str),
        ),
        (
            ScopeLevel::Execution(exec_id),
            &format!(r#"{{"Execution":"{}"}}"#, nil_str),
        ),
        (
            ScopeLevel::Action(exec_id, node_id),
            &format!(r#"{{"Action":["{}","{}"]}}"#, nil_str, nil_str),
        ),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json, *expected,
            "ScopeLevel serialization changed for {:?}",
            value
        );
    }
}

#[test]
fn id_types_serialize_as_uuid_string() {
    let id = ExecutionId::nil();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""00000000-0000-0000-0000-000000000000""#);
}

#[test]
fn core_error_code_stability() {
    use nebula_core::CoreError;

    let cases: [(CoreError, &str); 5] = [
        (CoreError::validation("x"), "VALIDATION_ERROR"),
        (CoreError::not_found("User", "123"), "NOT_FOUND_ERROR"),
        (CoreError::invalid_input("bad"), "INVALID_INPUT_ERROR"),
        (CoreError::internal("oops"), "INTERNAL_ERROR"),
        (
            serde_json::from_str::<serde_json::Value>("{")
                .unwrap_err()
                .into(),
            "SERIALIZATION_ERROR",
        ),
    ];
    for (err, expected_code) in cases {
        assert_eq!(
            err.error_code(),
            expected_code,
            "CoreError::error_code() changed"
        );
    }
}
