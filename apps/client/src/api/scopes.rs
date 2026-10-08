//! The scopes a personal access token can carry: the server's vocabulary
//! (`crates/api/src/access/scope.rs`, `permission_scope` and `parse_pat_grant`). The server rejects
//! any other scope, and `full_access` combined with another, with 400.

/// Every scope by area, in the order the token form lists them.
pub(crate) const TOKEN_SCOPES: [(&str, &[&str]); 8] = [
    (
        "Workflows",
        &[
            "workflows:read",
            "workflows:write",
            "workflows:delete",
            "workflows:execute",
        ],
    ),
    (
        "Executions",
        &[
            "executions:read",
            "executions:cancel",
            "executions:terminate",
            "executions:restart",
        ],
    ),
    (
        "Credentials",
        &[
            "credentials:read",
            "credentials:write",
            "credentials:delete",
            "credentials:reconcile",
        ],
    ),
    (
        "Resources",
        &["resources:read", "resources:write", "resources:delete"],
    ),
    (
        "Workspace members",
        &["workspace_members:read", "workspace_members:manage"],
    ),
    (
        "Organization members",
        &["members:read", "members:invite", "members:remove"],
    ),
    ("Organization", &["orgs:read", "orgs:update", "orgs:delete"]),
    ("Service accounts", &["service_accounts:manage"]),
];

/// Every permission the account has; the server accepts it only on its own.
pub(crate) const FULL_ACCESS: &str = "full_access";

/// Whether the server would mint a token with these scopes: at least one, each known, and
/// `full_access` alone. The reason, when it would not.
pub(crate) fn check_scopes(scopes: &[String]) -> Result<(), String> {
    if scopes.is_empty() {
        return Err("scopes: Choose at least one scope.".to_owned());
    }
    if scopes.iter().any(|scope| scope == FULL_ACCESS) {
        return if scopes.len() == 1 {
            Ok(())
        } else {
            Err(format!(
                "scopes: `{FULL_ACCESS}` cannot be combined with other scopes."
            ))
        };
    }
    match scopes.iter().find(|scope| {
        !TOKEN_SCOPES
            .iter()
            .any(|(_, known)| known.contains(&scope.as_str()))
    }) {
        Some(unknown) => Err(format!("scopes: Unknown scope `{unknown}`.")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_server_s_vocabulary_is_accepted_and_nothing_else() {
        assert!(check_scopes(&scopes(&["workflows:execute", "executions:cancel"])).is_ok());
        assert!(check_scopes(&scopes(&[FULL_ACCESS])).is_ok());
        assert!(check_scopes(&scopes(&["executions:write"])).is_err());
        assert!(check_scopes(&scopes(&[FULL_ACCESS, "workflows:read"])).is_err());
        assert!(check_scopes(&[]).is_err());
    }
}
