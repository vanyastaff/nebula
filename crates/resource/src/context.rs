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
    pub fn minimal(scope: Scope, cancellation: CancellationToken) -> Self {
        let base = BaseContext::builder()
            .scope(scope)
            .cancellation(cancellation)
            .clock(SystemClock)
            .build();
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
fn scope_to_level(scope: &Scope) -> ScopeLevel {
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
}
