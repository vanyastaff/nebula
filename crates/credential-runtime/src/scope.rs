//! Tenant scoping: `TenantScope` is a mandatory operation argument; it
//! derives the `owner_id` string the storage `ScopeLayer` keys on, and
//! supplies a per-call `ScopeResolver`. Confused-deputy (spec §6 #1) is
//! closed by type: no operation is callable without a `&TenantScope`.

use nebula_storage::credential::ScopeResolver;

/// Tenant identity for a credential operation. `owner_id` =
/// `"{org}/{workspace}"` — the value persisted in
/// `StoredCredential.metadata["owner_id"]` and matched by `ScopeLayer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantScope {
    owner_id: String,
}

impl TenantScope {
    /// Construct from organization + workspace identifiers.
    #[must_use]
    pub fn new(org: impl AsRef<str>, workspace: impl AsRef<str>) -> Self {
        Self {
            owner_id: format!("{}/{}", org.as_ref(), workspace.as_ref()),
        }
    }

    /// The scope key persisted/matched by the storage `ScopeLayer`.
    #[must_use]
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// A `ScopeResolver` pinned to this scope, for the per-call layered
    /// store stack.
    #[must_use]
    pub fn resolver(&self) -> FixedScopeResolver {
        FixedScopeResolver {
            owner: self.owner_id.clone(),
        }
    }
}

/// `ScopeResolver` that always reports one fixed owner — constructed
/// per operation from the caller's `TenantScope`.
#[derive(Debug)]
pub struct FixedScopeResolver {
    owner: String,
}

impl ScopeResolver for FixedScopeResolver {
    fn current_owner(&self) -> Option<&str> {
        Some(&self.owner)
    }
}

#[cfg(test)]
mod tests {
    use super::TenantScope;

    #[test]
    fn owner_id_is_org_slash_workspace() {
        let s = TenantScope::new("org-1", "ws-2");
        assert_eq!(s.owner_id(), "org-1/ws-2");
    }

    #[test]
    fn scope_resolver_returns_owner() {
        use nebula_storage::credential::ScopeResolver;
        let s = TenantScope::new("o", "w");
        let r = s.resolver();
        assert_eq!(r.current_owner(), Some("o/w"));
    }
}
