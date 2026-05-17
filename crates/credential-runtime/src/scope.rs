//! Tenant scoping: `TenantScope` is a mandatory operation argument; it
//! derives the `owner_id` string the storage `ScopeLayer` keys on, and
//! supplies a per-call `ScopeResolver`. Confused-deputy (spec §6 #1) is
//! closed by type: no operation is callable without a `&TenantScope`.

use nebula_credential::ScopeResolver;

/// Tenant identity for a credential operation. `owner_id` =
/// `"{org}/{workspace}"` — the value persisted in
/// `StoredCredential.metadata["owner_id"]` and matched by `ScopeLayer`.
///
/// An optional `session_id` carries the interactive-flow session: the
/// `PendingStateStore` binds pending acquisitions on
/// `(kind, owner, session, token)`, so a session is **required** for the
/// interactive paths (`resolve`/`acquire` returning `Pending`,
/// `continue_resolve`). CRUD and the non-interactive capability ops do
/// not consult it; `new` leaves it `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantScope {
    owner_id: String,
    session_id: Option<String>,
}

impl TenantScope {
    /// Construct from organization + workspace identifiers. The session
    /// is `None`; attach one with [`with_session`](Self::with_session)
    /// before driving an interactive acquisition.
    #[must_use]
    pub fn new(org: impl AsRef<str>, workspace: impl AsRef<str>) -> Self {
        Self {
            owner_id: format!("{}/{}", org.as_ref(), workspace.as_ref()),
            session_id: None,
        }
    }

    /// Attach the interactive-flow session id. Required for the
    /// pending-store `(kind, owner, session, token)` binding that the
    /// interactive `resolve`/`continue_resolve` paths depend on; CRUD
    /// and the non-interactive ops ignore it.
    #[must_use]
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// The scope key persisted/matched by the storage `ScopeLayer`.
    /// Unaffected by [`with_session`](Self::with_session) — owner
    /// derivation is org/workspace only.
    #[must_use]
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// The interactive-flow session id, if one was attached via
    /// [`with_session`](Self::with_session).
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
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
    fn new_scope_has_no_session() {
        let s = TenantScope::new("org-1", "ws-2");
        assert_eq!(s.session_id(), None);
    }

    #[test]
    fn with_session_threads_session_without_changing_owner() {
        let s = TenantScope::new("org-1", "ws-2").with_session("sess-7");
        assert_eq!(s.session_id(), Some("sess-7"));
        // Owner derivation is unchanged by the session.
        assert_eq!(s.owner_id(), "org-1/ws-2");
    }

    #[test]
    fn scope_resolver_returns_owner() {
        use nebula_credential::ScopeResolver;
        let s = TenantScope::new("o", "w");
        let r = s.resolver();
        assert_eq!(r.current_owner(), Some("o/w"));
    }
}
