//! Typed ULID identifiers for Nebula entities.
//!
//! Convention: `FooId` = system-generated (ULID), `FooKey` = author-defined (string).

use domain_key::define_ulid;

// System-generated identifiers
define_ulid!(pub OrgIdDomain => OrgId, prefix = "org");
define_ulid!(pub WorkspaceIdDomain => WorkspaceId, prefix = "ws");
define_ulid!(pub WorkflowIdDomain => WorkflowId, prefix = "wf");
define_ulid!(pub WorkflowVersionIdDomain => WorkflowVersionId, prefix = "wfv");
define_ulid!(pub ExecutionIdDomain => ExecutionId, prefix = "exe");

define_ulid!(pub AttemptIdDomain => AttemptId, prefix = "att");
define_ulid!(pub InstanceIdDomain => InstanceId, prefix = "nbl");
define_ulid!(pub TriggerIdDomain => TriggerId, prefix = "trg");
define_ulid!(pub TriggerEventIdDomain => TriggerEventId, prefix = "evt");
define_ulid!(pub UserIdDomain => UserId, prefix = "usr");
define_ulid!(pub ServiceAccountIdDomain => ServiceAccountId, prefix = "svc");
define_ulid!(pub CredentialIdDomain => CredentialId, prefix = "cred");
define_ulid!(pub ResourceIdDomain => ResourceId, prefix = "res");
define_ulid!(pub SessionIdDomain => SessionId, prefix = "sess");
// OrganizationId duplicates OrgId with the same "org" prefix.
#[deprecated(note = "Use OrgId instead")]
pub type OrganizationId = OrgId;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_id_new_is_unique() {
        let a = ExecutionId::new();
        let b = ExecutionId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn id_display_has_prefix() {
        let id = ExecutionId::new();
        let s = id.to_string();
        assert!(s.starts_with("exe_"), "expected 'exe_' prefix, got: {s}");
    }

    #[test]
    fn id_parse_roundtrip() {
        let id = WorkflowId::new();
        let s = id.to_string();
        let parsed: WorkflowId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn id_serde_json_roundtrip() {
        let id = ExecutionId::new();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: ExecutionId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn id_parse_invalid_string_returns_error() {
        let result: Result<ExecutionId, _> = "not-a-ulid".parse();
        assert!(result.is_err());
    }

    #[test]
    fn id_copy_semantics_both_copies_usable() {
        let id1 = CredentialId::new();
        let id2 = id1; // Copy
        assert_eq!(id1, id2);
    }

    #[test]
    fn id_ordering_is_deterministic() {
        let a = WorkspaceId::new();
        let b = WorkspaceId::new();
        assert_ne!(a, b, "two freshly created IDs must differ");
        // Ordering must be consistent: a < b or a > b (never equal).
        let ord = a.cmp(&b);
        assert_ne!(ord, std::cmp::Ordering::Equal);
        // Reverse comparison must be the mirror.
        assert_eq!(b.cmp(&a), ord.reverse());
    }

    #[test]
    fn id_hash_is_consistent() {
        use std::collections::HashSet;
        let id = SessionId::new();
        let mut set = HashSet::new();
        set.insert(id);
        assert!(set.contains(&id));
    }

    #[test]
    fn different_id_types_are_incompatible() {
        // Type safety: ExecutionId and WorkflowId are distinct newtypes.
        // Passing one where the other is expected is a compile error:
        //   accepts_execution(wf);  // won't compile
        //   accepts_workflow(exec); // won't compile
        fn accepts_execution(_id: ExecutionId) {}
        fn accepts_workflow(_id: WorkflowId) {}

        let exec = ExecutionId::new();
        let wf = WorkflowId::new();
        accepts_execution(exec);
        accepts_workflow(wf);
    }

    #[test]
    #[expect(
        deprecated,
        reason = "test exercises OrganizationId which is deprecated but still supported"
    )]
    fn all_id_types_create_successfully() {
        let _ = OrgId::new();
        let _ = WorkspaceId::new();
        let _ = WorkflowId::new();
        let _ = WorkflowVersionId::new();
        let _ = ExecutionId::new();
        let _ = AttemptId::new();
        let _ = InstanceId::new();
        let _ = TriggerId::new();
        let _ = TriggerEventId::new();
        let _ = UserId::new();
        let _ = ServiceAccountId::new();
        let _ = CredentialId::new();
        let _ = ResourceId::new();
        let _ = SessionId::new();
        let _ = OrganizationId::new();
    }
}
