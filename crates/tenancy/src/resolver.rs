//! `Principal` → `Scope` resolution.
//!
//! Generalised from the credential-specific `ScopeLayer`/`ScopeResolver`
//! (`current_owner() -> Option<&str>`): instead of an owner-id string this
//! resolves the *authenticated tenant identity* to the port's plain-data
//! [`Scope`]. The result feeds the scoping decorators (`src/decorator/`)
//! which inject it into every storage call so the engine/api can never
//! pass an arbitrary scope.
//!
//! `nebula-tenancy` owns this **policy** — it does not own [`Scope`] (that
//! is port/Core-level, spec §3 tension resolution).

use nebula_core::id::{OrgId, WorkspaceId};
use nebula_core::scope::Principal as ActorPrincipal;
use nebula_storage_port::Scope;

use crate::error::TenancyError;

/// Authenticated tenant identity presented by an inbound request.
///
/// This is the **security-boundary input**: the actor (`nebula-core`'s
/// actor-only [`ActorPrincipal`]) plus the tenant binding it authenticated
/// against. Carrying the binding here — rather than reusing the actor-only
/// enum — is what lets [`ScopeResolver`] produce a *non-optional*
/// `Scope { workspace_id, org_id }`. A scope that cannot be `None` cannot
/// be forgotten at a call site (spec §6.1 confused-deputy mitigation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// The acting identity (user / service account / workflow / system).
    pub actor: ActorPrincipal,
    /// Organization the actor authenticated against. Always present —
    /// every tenant-scoped row is keyed by org.
    pub org_id: OrgId,
    /// Workspace the actor authenticated against, when the request is
    /// workspace-scoped. `None` for org-level principals (e.g. an org
    /// admin listing workspaces).
    pub workspace_id: Option<WorkspaceId>,
}

impl Principal {
    /// Construct a workspace-scoped principal.
    pub fn workspace(actor: ActorPrincipal, org_id: OrgId, workspace_id: WorkspaceId) -> Self {
        Self {
            actor,
            org_id,
            workspace_id: Some(workspace_id),
        }
    }

    /// Construct an org-scoped principal (no workspace binding).
    pub fn org(actor: ActorPrincipal, org_id: OrgId) -> Self {
        Self {
            actor,
            org_id,
            workspace_id: None,
        }
    }
}

/// Resolves an authenticated [`Principal`] to the port [`Scope`].
///
/// Implementations are the single authority that decides which tenant a
/// request runs as. The composition root resolves once per request and
/// hands the scoping decorators the resulting [`Scope`]; nothing
/// downstream can override it.
pub trait ScopeResolver: Send + Sync {
    /// Resolve `principal` to the workspace+org [`Scope`] every
    /// tenant-scoped storage call is keyed by.
    ///
    /// # Errors
    ///
    /// - [`TenancyError::MissingWorkspace`] if the principal has no
    ///   workspace binding (fail-closed — never widen to org-only).
    /// - [`TenancyError::Unauthorized`] if the principal is not
    ///   authorized for the tenant it presented.
    fn resolve(&self, principal: &Principal) -> Result<Scope, TenancyError>;
}

/// Default resolver: trusts the binding the authentication layer already
/// stamped onto the [`Principal`] and projects it into a [`Scope`].
///
/// Authentication/RBAC happens *upstream* (it is `nebula-core`'s
/// `TenantContext::require` + the API tenancy middleware). By the time a
/// `Principal` reaches this resolver its org/workspace are already proven;
/// the resolver's job is the **fail-closed projection** into the
/// non-optional port `Scope`, rejecting an absent workspace rather than
/// silently downgrading isolation.
#[derive(Debug, Clone, Default)]
pub struct BindingScopeResolver;

impl ScopeResolver for BindingScopeResolver {
    fn resolve(&self, principal: &Principal) -> Result<Scope, TenancyError> {
        let workspace_id = principal
            .workspace_id
            .as_ref()
            .ok_or(TenancyError::MissingWorkspace)?;
        Ok(Scope::new(
            workspace_id.to_string(),
            principal.org_id.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::id::{OrgId, UserId, WorkspaceId};
    use nebula_core::scope::Principal as ActorPrincipal;

    use super::*;

    #[test]
    fn resolves_workspace_principal_to_scope() {
        let org = OrgId::new();
        let ws = WorkspaceId::new();
        let principal = Principal::workspace(ActorPrincipal::User(UserId::new()), org, ws);

        let scope = BindingScopeResolver
            .resolve(&principal)
            .expect("workspace principal resolves");

        assert_eq!(scope.org_id, org.to_string());
        assert_eq!(scope.workspace_id, ws.to_string());
    }

    #[test]
    fn org_only_principal_is_denied_for_workspace_scope() {
        let principal = Principal::org(ActorPrincipal::System, OrgId::new());

        let err = BindingScopeResolver
            .resolve(&principal)
            .expect_err("org-only principal must fail closed");

        assert_eq!(err, TenancyError::MissingWorkspace);
    }

    #[test]
    fn distinct_orgs_resolve_to_distinct_scopes() {
        let ws = WorkspaceId::new();
        let a = Principal::workspace(ActorPrincipal::System, OrgId::new(), ws);
        let b = Principal::workspace(ActorPrincipal::System, OrgId::new(), ws);

        let sa = BindingScopeResolver.resolve(&a).expect("a resolves");
        let sb = BindingScopeResolver.resolve(&b).expect("b resolves");

        // Same workspace id but different org ⇒ different scope. The
        // decorator keys every row on (workspace, org), so these two
        // never observe each other's data.
        assert_ne!(sa, sb);
        assert_eq!(sa.workspace_id, sb.workspace_id);
        assert_ne!(sa.org_id, sb.org_id);
    }
}
