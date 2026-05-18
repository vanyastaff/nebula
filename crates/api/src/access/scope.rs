//! PAT scope vocabulary and parsing.

use std::collections::BTreeSet;

use nebula_core::Permission;
use thiserror::Error;

use crate::access::Grant;

/// Scope string that grants complete API access to a PAT.
pub const FULL_ACCESS_SCOPE: &str = "full_access";

/// Placeholder scope for core permissions that are not part of this API scope vocabulary.
pub const UNSUPPORTED_PERMISSION_SCOPE: &str = "__unsupported_permission__";

/// Error returned when parsing API access scopes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScopeParseError {
    /// No scopes were supplied.
    #[error("at least one scope is required")]
    Empty,
    /// The scope is not in the supported API vocabulary.
    #[error("unknown scope {0}")]
    Unknown(String),
    /// `full_access` cannot be combined with any other scope.
    #[error("{FULL_ACCESS_SCOPE} cannot be combined with other scopes")]
    FullAccessMixed,
}

/// Return the canonical API scope string for `permission`.
///
/// Permissions not included in the PAT scope vocabulary map to
/// [`UNSUPPORTED_PERMISSION_SCOPE`].
#[must_use]
pub fn permission_scope(permission: Permission) -> &'static str {
    match permission {
        Permission::WorkflowRead => "workflows:read",
        Permission::WorkflowWrite => "workflows:write",
        Permission::WorkflowDelete => "workflows:delete",
        Permission::WorkflowExecute => "workflows:execute",
        Permission::ExecutionRead => "executions:read",
        Permission::ExecutionCancel => "executions:cancel",
        Permission::ExecutionTerminate => "executions:terminate",
        Permission::ExecutionRestart => "executions:restart",
        Permission::CredentialRead => "credentials:read",
        Permission::CredentialWrite => "credentials:write",
        Permission::CredentialDelete => "credentials:delete",
        Permission::ResourceRead => "resources:read",
        Permission::ResourceWrite => "resources:write",
        Permission::ResourceDelete => "resources:delete",
        Permission::MemberRead => "members:read",
        Permission::MemberInvite => "members:invite",
        Permission::MemberRemove => "members:remove",
        Permission::OrgRead => "orgs:read",
        Permission::OrgUpdate => "orgs:update",
        Permission::OrgDelete => "orgs:delete",
        Permission::ServiceAccountManage => "service_accounts:manage",
        _ => UNSUPPORTED_PERMISSION_SCOPE,
    }
}

/// Parse a canonical API scope string into a core permission.
pub fn permission_from_scope(scope: &str) -> Result<Permission, ScopeParseError> {
    match scope {
        "workflows:read" => Ok(Permission::WorkflowRead),
        "workflows:write" => Ok(Permission::WorkflowWrite),
        "workflows:delete" => Ok(Permission::WorkflowDelete),
        "workflows:execute" => Ok(Permission::WorkflowExecute),
        "executions:read" => Ok(Permission::ExecutionRead),
        "executions:cancel" => Ok(Permission::ExecutionCancel),
        "executions:terminate" => Ok(Permission::ExecutionTerminate),
        "executions:restart" => Ok(Permission::ExecutionRestart),
        "credentials:read" => Ok(Permission::CredentialRead),
        "credentials:write" => Ok(Permission::CredentialWrite),
        "credentials:delete" => Ok(Permission::CredentialDelete),
        "resources:read" => Ok(Permission::ResourceRead),
        "resources:write" => Ok(Permission::ResourceWrite),
        "resources:delete" => Ok(Permission::ResourceDelete),
        "members:read" => Ok(Permission::MemberRead),
        "members:invite" => Ok(Permission::MemberInvite),
        "members:remove" => Ok(Permission::MemberRemove),
        "orgs:read" => Ok(Permission::OrgRead),
        "orgs:update" => Ok(Permission::OrgUpdate),
        "orgs:delete" => Ok(Permission::OrgDelete),
        "service_accounts:manage" => Ok(Permission::ServiceAccountManage),
        unknown => Err(ScopeParseError::Unknown(unknown.to_owned())),
    }
}

/// Parse PAT scope strings into an effective API access grant.
pub fn parse_pat_grant(scopes: &[String]) -> Result<Grant, ScopeParseError> {
    if scopes.is_empty() {
        return Err(ScopeParseError::Empty);
    }

    let has_full_access = scopes.iter().any(|scope| scope == FULL_ACCESS_SCOPE);
    if has_full_access {
        return if scopes.len() == 1 {
            Ok(Grant::PatFullAccess)
        } else {
            Err(ScopeParseError::FullAccessMixed)
        };
    }

    scopes
        .iter()
        .map(|scope| permission_from_scope(scope))
        .collect::<Result<BTreeSet<_>, _>>()
        .map(Grant::PatScoped)
}

/// Validate PAT scope strings for a new token request.
pub fn validate_new_pat_scopes(scopes: &[String]) -> Result<(), ScopeParseError> {
    parse_pat_grant(scopes).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nebula_core::Permission;

    use super::{
        FULL_ACCESS_SCOPE, ScopeParseError, UNSUPPORTED_PERMISSION_SCOPE, parse_pat_grant,
        permission_from_scope, permission_scope, validate_new_pat_scopes,
    };
    use crate::access::Grant;

    #[test]
    fn parses_pat_grant_from_permission_scopes() {
        assert_eq!(
            parse_pat_grant(&[
                "workflows:read".to_string(),
                "executions:cancel".to_string(),
                "resources:write".to_string(),
                "members:read".to_string(),
            ]),
            Ok(Grant::PatScoped(BTreeSet::from([
                Permission::WorkflowRead,
                Permission::ExecutionCancel,
                Permission::ResourceWrite,
                Permission::MemberRead,
            ])))
        );
    }

    #[test]
    fn full_access_scope_alone_grants_full_pat_access() {
        assert_eq!(
            parse_pat_grant(&[FULL_ACCESS_SCOPE.to_string()]),
            Ok(Grant::PatFullAccess)
        );
    }

    #[test]
    fn duplicate_scopes_collapse() {
        assert_eq!(
            parse_pat_grant(&["workflows:read".to_string(), "workflows:read".to_string(),]),
            Ok(Grant::PatScoped(BTreeSet::from([Permission::WorkflowRead])))
        );
    }

    #[test]
    fn rejects_empty_unknown_and_mixed_full_access_scopes() {
        assert_eq!(parse_pat_grant(&[]), Err(ScopeParseError::Empty));
        assert_eq!(
            parse_pat_grant(&["bogus".to_string()]),
            Err(ScopeParseError::Unknown("bogus".to_string()))
        );
        assert_eq!(
            parse_pat_grant(&[FULL_ACCESS_SCOPE.to_string(), "workflows:read".to_string(),]),
            Err(ScopeParseError::FullAccessMixed)
        );
    }

    #[test]
    fn validation_matches_parser_errors_without_returning_grant() {
        assert_eq!(validate_new_pat_scopes(&["orgs:read".to_string()]), Ok(()));
        assert_eq!(validate_new_pat_scopes(&[]), Err(ScopeParseError::Empty));
    }

    #[test]
    fn maps_representative_permissions_to_scope_strings() {
        assert_eq!(
            permission_scope(Permission::WorkflowExecute),
            "workflows:execute"
        );
        assert_eq!(
            permission_scope(Permission::CredentialDelete),
            "credentials:delete"
        );
        assert_eq!(
            permission_scope(Permission::ResourceDelete),
            "resources:delete"
        );
        assert_eq!(
            permission_scope(Permission::ServiceAccountManage),
            "service_accounts:manage"
        );
        assert_eq!(
            permission_scope(Permission::WorkspaceMemberRead),
            UNSUPPORTED_PERMISSION_SCOPE
        );
    }

    #[test]
    fn maps_scope_strings_to_permissions() {
        assert_eq!(
            permission_from_scope("workflows:write"),
            Ok(Permission::WorkflowWrite)
        );
        assert_eq!(
            permission_from_scope("executions:terminate"),
            Ok(Permission::ExecutionTerminate)
        );
        assert_eq!(
            permission_from_scope("members:remove"),
            Ok(Permission::MemberRemove)
        );
        assert_eq!(
            permission_from_scope(FULL_ACCESS_SCOPE),
            Err(ScopeParseError::Unknown(FULL_ACCESS_SCOPE.to_string()))
        );
    }
}
