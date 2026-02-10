//! Unique identifiers for Nebula entities.
//!
//! This module provides strongly-typed UUID identifiers for various Nebula
//! entities using [`domain-key`](https://crates.io/crates/domain-key) `Uuid<D>`
//! wrappers. Each identifier type is parameterized by a unique domain marker,
//! providing compile-time type safety that prevents mixing different ID types.
//!
//! All ID types are `Copy` (16 bytes, stack-allocated) and support:
//! - `v4()` for random UUID generation
//! - `nil()` for zero-valued default
//! - `parse(&str)` for string parsing
//! - Full serde support (serializes as UUID string)
//! - `Display`, `FromStr`, `Eq`, `Ord`, `Hash`

use domain_key::define_uuid;

// Re-export for downstream parse error handling
pub use domain_key::UuidParseError;

// Entity identifiers — UUID-based, Copy, 16 bytes each
define_uuid!(UserIdDomain => UserId);
define_uuid!(TenantIdDomain => TenantId);
define_uuid!(ExecutionIdDomain => ExecutionId);
define_uuid!(WorkflowIdDomain => WorkflowId);
define_uuid!(NodeIdDomain => NodeId);
define_uuid!(ActionIdDomain => ActionId);
define_uuid!(ResourceIdDomain => ResourceId);
define_uuid!(CredentialIdDomain => CredentialId);
define_uuid!(ProjectIdDomain => ProjectId);
define_uuid!(RoleIdDomain => RoleId);
define_uuid!(OrganizationIdDomain => OrganizationId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_id_v4_creates_non_nil_uuid() {
        let id = UserId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn tenant_id_v4_creates_non_nil_uuid() {
        let id = TenantId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn execution_id_v4_creates_non_nil_uuid() {
        let id = ExecutionId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn workflow_id_v4_creates_non_nil_uuid() {
        let id = WorkflowId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn node_id_v4_creates_non_nil_uuid() {
        let id = NodeId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn action_id_v4_creates_non_nil_uuid() {
        let id = ActionId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn resource_id_v4_creates_non_nil_uuid() {
        let id = ResourceId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn credential_id_v4_creates_non_nil_uuid() {
        let id = CredentialId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn project_id_v4_creates_non_nil_uuid() {
        let id = ProjectId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn role_id_v4_creates_non_nil_uuid() {
        let id = RoleId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn organization_id_v4_creates_non_nil_uuid() {
        let id = OrganizationId::v4();
        assert!(!id.is_nil());
    }

    #[test]
    fn id_nil_creates_zero_valued_uuid() {
        let id = ProjectId::nil();
        assert!(id.is_nil());
        assert_eq!(id.to_string(), "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn id_parse_valid_uuid_string_succeeds() {
        let id = ProjectId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(!id.is_nil());
        assert_eq!(id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn id_parse_invalid_string_returns_error() {
        let result = ProjectId::parse("not-a-uuid");
        assert!(result.is_err());
    }

    #[test]
    fn id_copy_semantics_both_copies_usable() {
        let id1 = ProjectId::v4();
        let id2 = id1; // Copy, not move
        assert_eq!(id1, id2); // Both still usable
    }

    #[test]
    fn id_display_outputs_uuid_string() {
        let id = ProjectId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(format!("{}", id), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn id_from_uuid_roundtrips() {
        let raw = uuid::Uuid::new_v4();
        let typed = ProjectId::new(raw);
        let back: uuid::Uuid = typed.get();
        assert_eq!(raw, back);
    }

    #[test]
    fn id_from_bytes_roundtrips() {
        let bytes = [42u8; 16];
        let id = ProjectId::from_bytes(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn id_serde_json_roundtrip() {
        let id = ProjectId::v4();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: ProjectId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn id_domain_returns_type_name() {
        let id = ProjectId::nil();
        assert_eq!(id.domain(), "ProjectId");
    }

    #[test]
    fn different_id_types_are_incompatible() {
        // This test verifies type safety at the type level.
        // ProjectId and UserId are distinct types — passing one where the
        // other is expected would be a compile error.
        fn accepts_project(_id: ProjectId) {}
        fn accepts_user(_id: UserId) {}

        let project = ProjectId::v4();
        let user = UserId::v4();
        accepts_project(project);
        accepts_user(user);
        // accepts_project(user); // Would not compile
        // accepts_user(project); // Would not compile
    }

    #[test]
    fn id_try_from_str_succeeds() {
        let id = ProjectId::try_from("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(!id.is_nil());
    }

    #[test]
    fn id_try_from_string_succeeds() {
        let s = String::from("550e8400-e29b-41d4-a716-446655440000");
        let id = ProjectId::try_from(s).unwrap();
        assert!(!id.is_nil());
    }

    #[test]
    fn id_ordering_is_consistent() {
        let a = ProjectId::nil();
        let b = ProjectId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(a < b);
    }

    #[test]
    fn id_hash_is_consistent() {
        use std::collections::HashSet;
        let id = ProjectId::v4();
        let mut set = HashSet::new();
        set.insert(id);
        assert!(set.contains(&id));
    }
}
