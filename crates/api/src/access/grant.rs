//! API access grant model.

use std::collections::BTreeSet;

use nebula_core::Permission;
use thiserror::Error;

/// Effective API access granted to an authenticated caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// A first-party identity whose permissions are resolved outside PAT scopes.
    UnrestrictedIdentity,
    /// A personal access token with complete API access.
    PatFullAccess,
    /// A personal access token restricted to a fixed permission set.
    PatScoped(BTreeSet<Permission>),
    /// Internal system access for trusted platform calls.
    SystemInternal,
}

impl Grant {
    /// Require `permission` for this grant.
    ///
    /// Unrestricted identity, full-access PAT, and system grants allow every
    /// permission. Scoped PATs allow only permissions present in their set.
    pub fn require(&self, permission: Permission) -> Result<(), AccessDenied> {
        match self {
            Self::UnrestrictedIdentity | Self::PatFullAccess | Self::SystemInternal => Ok(()),
            Self::PatScoped(permissions) if permissions.contains(&permission) => Ok(()),
            Self::PatScoped(_) => Err(AccessDenied::new(permission)),
        }
    }
}

/// Error returned when an access grant does not allow a required permission.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("access denied for permission {permission:?}")]
pub struct AccessDenied {
    permission: Permission,
}

impl AccessDenied {
    /// Create an access-denied error for `permission`.
    #[must_use]
    pub const fn new(permission: Permission) -> Self {
        Self { permission }
    }

    /// Permission that was denied.
    #[must_use]
    pub const fn permission(&self) -> Permission {
        self.permission
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nebula_core::Permission;

    use super::{AccessDenied, Grant};

    #[test]
    fn unrestricted_grants_allow_required_permission() {
        for grant in [
            Grant::UnrestrictedIdentity,
            Grant::PatFullAccess,
            Grant::SystemInternal,
        ] {
            assert_eq!(grant.require(Permission::WorkflowDelete), Ok(()));
        }
    }

    #[test]
    fn scoped_pat_allows_only_included_permissions() {
        let grant = Grant::PatScoped(BTreeSet::from([Permission::WorkflowRead]));

        assert_eq!(grant.require(Permission::WorkflowRead), Ok(()));
        assert_eq!(
            grant.require(Permission::WorkflowWrite),
            Err(AccessDenied::new(Permission::WorkflowWrite))
        );
    }
}
