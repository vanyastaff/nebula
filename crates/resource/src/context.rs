//! Resource execution context backed by `nebula-core` types.
//!
//! [`ResourceContext`] replaces the former `Ctx` / `BasicCtx` pair with a
//! concrete struct that implements [`nebula_core::Context`],
//! [`nebula_core::HasResources`], and [`nebula_core::HasCredentials`].

use std::{any::Any, future::Future, pin::Pin, sync::Arc};

use nebula_core::{
    ScopeLevel,
    accessor::{Clock, CredentialAccessor, ResourceAccessor, SystemClock},
    context::{BaseContext, Context, HasCredentials, HasResources},
    scope::{Principal, Scope},
};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// No-op accessor stubs (used by `minimal` constructor)
// ---------------------------------------------------------------------------

/// No-op [`ResourceAccessor`] for contexts that don't need resource access.
struct NoopResourceAccessor;

impl ResourceAccessor for NoopResourceAccessor {
    fn has(&self, _key: &nebula_core::ResourceKey) -> bool {
        false
    }
    fn acquire_any(
        &self,
        _key: &nebula_core::ResourceKey,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Box<dyn Any + Send + Sync>, nebula_core::CoreError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Err(nebula_core::CoreError::CredentialNotConfigured(
                "resource capability is not configured in ResourceContext".to_owned(),
            ))
        })
    }
    fn try_acquire_any(
        &self,
        _key: &nebula_core::ResourceKey,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<Box<dyn Any + Send + Sync>>, nebula_core::CoreError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }
}

/// No-op [`CredentialAccessor`] for contexts that don't need credential access.
struct NoopCredentialAccessor;

impl CredentialAccessor for NoopCredentialAccessor {
    fn has(&self, _key: &nebula_core::CredentialKey) -> bool {
        false
    }
    fn resolve_any(
        &self,
        _key: &nebula_core::CredentialKey,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Box<dyn Any + Send + Sync>, nebula_core::CoreError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Err(nebula_core::CoreError::CredentialNotConfigured(
                "credential capability is not configured in ResourceContext".to_owned(),
            ))
        })
    }
    fn try_resolve_any(
        &self,
        _key: &nebula_core::CredentialKey,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<Box<dyn Any + Send + Sync>>, nebula_core::CoreError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }
}

// ---------------------------------------------------------------------------
// ResourceContext
// ---------------------------------------------------------------------------

/// Execution context for the resource subsystem.
///
/// Embeds a [`BaseContext`] for identity / scope / cancellation and holds
/// optional accessor arcs for resource-to-resource and credential resolution.
pub struct ResourceContext {
    base: BaseContext,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
}

impl ResourceContext {
    /// Creates a new `ResourceContext` with full accessor support.
    pub fn new(
        base: BaseContext,
        resources: Arc<dyn ResourceAccessor>,
        credentials: Arc<dyn CredentialAccessor>,
    ) -> Self {
        Self {
            base,
            resources,
            credentials,
        }
    }

    /// Creates a minimal context for cases that only need scope + cancellation
    /// (e.g., daemon loops, warmup). Uses no-op accessors internally.
    ///
    /// Infallible by construction — the principal is supplied directly
    /// ([`Principal::System`]) via `BaseContextBuilder::build_with`, so there
    /// is no `.build().expect(...)` panic to harden on a hot path.
    pub fn minimal(scope: Scope, cancellation: CancellationToken) -> Self {
        let base = BaseContext::builder(scope)
            .cancellation(cancellation)
            .clock(SystemClock)
            .build_with(Principal::System);
        Self {
            base,
            resources: Arc::new(NoopResourceAccessor),
            credentials: Arc::new(NoopCredentialAccessor),
        }
    }

    /// Returns the most specific [`ScopeLevel`] derivable from the scope bag.
    ///
    /// Resolution order (most specific wins):
    /// `Execution > Workflow > Workspace > Organization > Global`.
    pub fn scope_level(&self) -> ScopeLevel {
        scope_to_level(self.scope())
    }

    /// Convenience: returns the cancellation token (mirrors the old `Ctx::cancel_token` API).
    pub fn cancel_token(&self) -> &CancellationToken {
        self.cancellation()
    }

    /// Convenience: returns the execution ID from the scope, if present.
    pub fn execution_id(&self) -> Option<nebula_core::ExecutionId> {
        self.scope().execution_id
    }

    /// Clones the context for the type-erased acquire dispatch path
    /// ([`Manager::acquire_any`](crate::Manager::acquire_any)).
    ///
    /// Forwards the caller's scope, principal, cancellation, and trace/span
    /// identifiers, **plus** the resource and credential accessor arcs, so a
    /// `Provider::create`/`prepare` reached through the erased `acquire_any`
    /// surface observes the same identity, trace correlation, and
    /// nested-resource/credential access it would on a typed acquire. Earlier
    /// this returned a [`minimal`](Self::minimal) context, which silently
    /// clobbered the principal to [`Principal::System`], dropped the trace, and
    /// substituted no-op accessors — breaking authz checks, span parentage, and
    /// nested `ctx.resource::<R>()` calls during creation.
    ///
    /// The clock is not forwarded — it is not part of the identity and
    /// `BaseContext` is not `Clone` (its clock is boxed); a fresh `SystemClock`
    /// is used.
    pub fn clone_for_acquire(&self) -> Self {
        let mut builder = BaseContext::builder(self.scope().clone())
            .cancellation(self.cancellation().clone())
            .clock(SystemClock);
        if let Some(trace_id) = self.trace_id() {
            builder = builder.trace_id(trace_id);
        }
        if let Some(span_id) = self.span_id() {
            builder = builder.span_id(span_id);
        }
        Self {
            base: builder.build_with(self.principal().clone()),
            resources: Arc::clone(&self.resources),
            credentials: Arc::clone(&self.credentials),
        }
    }
}

impl std::fmt::Debug for ResourceContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceContext")
            .field("scope", self.scope())
            .field("principal", self.principal())
            .finish_non_exhaustive()
    }
}

// --- Context delegation ---------------------------------------------------

impl Context for ResourceContext {
    fn scope(&self) -> &Scope {
        self.base.scope()
    }
    fn principal(&self) -> &Principal {
        self.base.principal()
    }
    fn cancellation(&self) -> &CancellationToken {
        self.base.cancellation()
    }
    fn clock(&self) -> &dyn Clock {
        self.base.clock()
    }
    fn trace_id(&self) -> Option<nebula_core::obs::TraceId> {
        self.base.trace_id()
    }
    fn span_id(&self) -> Option<nebula_core::obs::SpanId> {
        self.base.span_id()
    }
}

// --- Capability impls -----------------------------------------------------

impl HasResources for ResourceContext {
    fn resources(&self) -> &dyn ResourceAccessor {
        &*self.resources
    }
}

impl HasCredentials for ResourceContext {
    fn credentials(&self) -> &dyn CredentialAccessor {
        &*self.credentials
    }
}

// ---------------------------------------------------------------------------
// Helper: Scope → ScopeLevel
// ---------------------------------------------------------------------------

/// Converts a [`Scope`] bag to the most specific [`ScopeLevel`].
pub fn scope_to_level(scope: &Scope) -> ScopeLevel {
    if let Some(id) = scope.execution_id {
        ScopeLevel::Execution(id)
    } else if let Some(id) = scope.workflow_id {
        ScopeLevel::Workflow(id)
    } else if let Some(id) = scope.workspace_id {
        ScopeLevel::Workspace(id)
    } else if let Some(id) = scope.org_id {
        ScopeLevel::Organization(id)
    } else {
        ScopeLevel::Global
    }
}

/// Scope levels to probe for acquire/lookup, most specific first.
///
/// When an execution-scoped context carries `org_id` / `workspace_id`, registry
/// rows registered at those levels remain reachable without falling through to
/// an unrelated Global row.
pub fn scope_levels_for_acquire(scope: &Scope) -> Vec<ScopeLevel> {
    [
        scope.execution_id.map(ScopeLevel::Execution),
        scope.workflow_id.map(ScopeLevel::Workflow),
        scope.workspace_id.map(ScopeLevel::Workspace),
        scope.org_id.map(ScopeLevel::Organization),
        Some(ScopeLevel::Global),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Builds a minimal [`Scope`] bag containing only the given level's id field.
pub fn minimal_scope_for_level(level: &ScopeLevel) -> Scope {
    let mut scope = Scope::default();
    match level {
        ScopeLevel::Global => {},
        ScopeLevel::Organization(id) => scope.org_id = Some(*id),
        ScopeLevel::Workspace(id) => scope.workspace_id = Some(*id),
        ScopeLevel::Workflow(id) => scope.workflow_id = Some(*id),
        ScopeLevel::Execution(id) => scope.execution_id = Some(*id),
    }
    scope
}

#[cfg(test)]
mod tests {
    use nebula_core::{ExecutionId, context::Context, scope::Scope};
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[test]
    fn resource_context_implements_context_traits() {
        let scope = Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        };
        let ctx = ResourceContext::minimal(scope, CancellationToken::new());
        assert!(ctx.scope().execution_id.is_some());
    }

    #[test]
    fn resource_context_scope_level_global_when_empty() {
        let scope = Scope::default();
        let ctx = ResourceContext::minimal(scope, CancellationToken::new());
        assert_eq!(ctx.scope_level(), ScopeLevel::Global);
    }

    #[test]
    fn resource_context_scope_level_execution() {
        let eid = ExecutionId::new();
        let scope = Scope {
            execution_id: Some(eid),
            ..Default::default()
        };
        let ctx = ResourceContext::minimal(scope, CancellationToken::new());
        assert_eq!(ctx.scope_level(), ScopeLevel::Execution(eid));
    }

    #[test]
    fn scope_levels_for_acquire_includes_ancestors() {
        let org = nebula_core::OrgId::new();
        let eid = ExecutionId::new();
        let scope = Scope {
            execution_id: Some(eid),
            org_id: Some(org),
            ..Default::default()
        };
        let levels = scope_levels_for_acquire(&scope);
        assert_eq!(levels[0], ScopeLevel::Execution(eid));
        assert_eq!(levels[1], ScopeLevel::Organization(org));
        assert_eq!(levels.last(), Some(&ScopeLevel::Global));
    }

    #[test]
    fn resource_context_execution_id_convenience() {
        let eid = ExecutionId::new();
        let scope = Scope {
            execution_id: Some(eid),
            ..Default::default()
        };
        let ctx = ResourceContext::minimal(scope, CancellationToken::new());
        assert_eq!(ctx.execution_id(), Some(eid));
    }

    #[test]
    fn resource_context_has_resources_and_credentials() {
        use nebula_core::context::{HasCredentials, HasResources};
        let scope = Scope::default();
        let ctx = ResourceContext::minimal(scope, CancellationToken::new());
        // Noop accessors should return false for has()
        let _ = ctx.resources();
        let _ = ctx.credentials();
    }

    #[test]
    fn clone_for_acquire_forwards_principal_and_accessors() {
        use nebula_core::{context::HasResources, id::UserId, scope::Principal};

        // Accessor that reports every key as present, distinguishable from the
        // `NoopResourceAccessor` (which reports `false`) — proves the real
        // accessor arc is forwarded across the erased-acquire boundary.
        struct AlwaysHasAccessor;
        impl ResourceAccessor for AlwaysHasAccessor {
            fn has(&self, _key: &nebula_core::ResourceKey) -> bool {
                true
            }
            fn acquire_any(
                &self,
                _key: &nebula_core::ResourceKey,
            ) -> Pin<
                Box<
                    dyn Future<Output = Result<Box<dyn Any + Send + Sync>, nebula_core::CoreError>>
                        + Send
                        + '_,
                >,
            > {
                Box::pin(async {
                    Err(nebula_core::CoreError::RegistryInvariant(
                        "test accessor never resolves",
                    ))
                })
            }
            fn try_acquire_any(
                &self,
                _key: &nebula_core::ResourceKey,
            ) -> Pin<
                Box<
                    dyn Future<
                            Output = Result<
                                Option<Box<dyn Any + Send + Sync>>,
                                nebula_core::CoreError,
                            >,
                        > + Send
                        + '_,
                >,
            > {
                Box::pin(async { Ok(None) })
            }
        }

        let user = Principal::User(UserId::new());
        let base = BaseContext::builder(Scope::default())
            .principal(user.clone())
            .build()
            .expect("principal is set");
        let ctx = ResourceContext::new(
            base,
            Arc::new(AlwaysHasAccessor),
            Arc::new(NoopCredentialAccessor),
        );

        let cloned = ctx.clone_for_acquire();

        // Identity is forwarded, not clobbered to `Principal::System`.
        assert_eq!(
            cloned.principal(),
            &user,
            "clone_for_acquire must forward the caller's principal"
        );
        // The resource accessor arc is forwarded, not replaced by the no-op —
        // so a `Provider::create` that resolves a nested resource still works.
        let nested = nebula_core::resource_key!("nested.dep");
        assert!(
            cloned.resources().has(&nested),
            "clone_for_acquire must forward the resource accessor"
        );
    }
}
