use nebula_storage_port::dto::{
    MembershipRoleParseError, OrgMembershipRole, WorkspaceMembershipRole,
};
use nebula_storage_port::store::MembershipStore;

#[test]
fn persisted_roles_use_the_existing_identity_vocabulary() {
    for (wire, role, privileged) in [
        ("OrgMember", OrgMembershipRole::Member, false),
        ("OrgBilling", OrgMembershipRole::Billing, false),
        ("OrgAdmin", OrgMembershipRole::Admin, true),
        ("OrgOwner", OrgMembershipRole::Owner, true),
    ] {
        assert_eq!(OrgMembershipRole::parse(wire), Ok(role));
        assert_eq!(role.as_str(), wire);
        assert_eq!(role.is_privileged(), privileged);
    }
    for (wire, role) in [
        ("WorkspaceViewer", WorkspaceMembershipRole::Viewer),
        ("WorkspaceRunner", WorkspaceMembershipRole::Runner),
        ("WorkspaceEditor", WorkspaceMembershipRole::Editor),
        ("WorkspaceAdmin", WorkspaceMembershipRole::Admin),
    ] {
        assert_eq!(WorkspaceMembershipRole::parse(wire), Ok(role));
        assert_eq!(role.as_str(), wire);
    }
}

#[test]
fn unknown_and_cross_scope_roles_fail_closed_without_retaining_input() {
    for value in [
        "",
        "owner",
        "OrgOwner ",
        "untrusted-secret",
        "WorkspaceAdmin",
    ] {
        assert_eq!(
            OrgMembershipRole::parse(value),
            Err(MembershipRoleParseError)
        );
    }
    for value in [
        "",
        "admin",
        "WorkspaceAdmin ",
        "untrusted-secret",
        "OrgOwner",
    ] {
        assert_eq!(
            WorkspaceMembershipRole::parse(value),
            Err(MembershipRoleParseError)
        );
    }
    let error = OrgMembershipRole::parse("untrusted-secret").unwrap_err();
    assert!(!format!("{error:?} {error}").contains("untrusted-secret"));
}

// These methods must remain callable through the runtime's object-safe handle.
async fn _read_snapshot(store: &dyn MembershipStore) {
    let _ = store
        .get_tenant_membership(
            "org",
            Some("workspace"),
            nebula_storage_port::dto::PrincipalKind::User,
            "user",
        )
        .await;
}
