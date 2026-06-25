//! Context system -- base trait + capabilities (spec 23).

pub mod capability;

pub use capability::*;
use tokio_util::sync::CancellationToken;

use crate::{
    accessor::Clock,
    obs::{SpanId, TraceId},
    scope::{Principal, Scope},
};

/// Base context trait -- identity, tenancy, lifecycle, clock.
pub trait Context: Send + Sync {
    /// Get the scope.
    fn scope(&self) -> &Scope;
    /// Get the principal (who is acting).
    fn principal(&self) -> &Principal;
    /// Get the cancellation token.
    fn cancellation(&self) -> &CancellationToken;
    /// Get the clock.
    fn clock(&self) -> &dyn Clock;
    /// Get the trace ID, if available.
    fn trace_id(&self) -> Option<TraceId> {
        None
    }
    /// Get the span ID, if available.
    fn span_id(&self) -> Option<SpanId> {
        None
    }
}

/// Shared identity fields. Domain contexts embed and delegate.
///
/// `BaseContext` is intentionally not `Clone` because it holds a
/// `Box<dyn Clock>` which is not cloneable. If cloning is needed,
/// wrap the clock in `Arc<dyn Clock>` instead of boxing it.
pub struct BaseContext {
    scope: Scope,
    principal: Principal,
    cancellation: CancellationToken,
    clock: Box<dyn Clock>,
    trace_id: Option<TraceId>,
    span_id: Option<SpanId>,
}

impl BaseContext {
    /// Create a builder for `BaseContext`.
    ///
    /// `scope` is required at construction time — every context must declare
    /// the tenancy scope it operates within. Background tasks and system
    /// operations should pass `Scope::default()` explicitly.
    ///
    /// # Required fields
    ///
    /// - `scope` — provided here.
    /// - `principal` — set via [`.principal()`](BaseContextBuilder::principal) before calling
    ///   [`.build()`](BaseContextBuilder::build). Use [`Principal::System`] for background tasks.
    pub fn builder(scope: Scope) -> BaseContextBuilder {
        BaseContextBuilder {
            scope,
            principal: None,
            cancellation: None,
            clock: None,
            trace_id: None,
            span_id: None,
        }
    }
}

impl Context for BaseContext {
    fn scope(&self) -> &Scope {
        &self.scope
    }
    fn principal(&self) -> &Principal {
        &self.principal
    }
    fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
    fn clock(&self) -> &dyn Clock {
        &*self.clock
    }
    fn trace_id(&self) -> Option<TraceId> {
        self.trace_id
    }
    fn span_id(&self) -> Option<SpanId> {
        self.span_id
    }
}

/// Builder for [`BaseContext`].
///
/// Constructed via [`BaseContext::builder(scope)`](BaseContext::builder).
/// `scope` is fixed at construction; `principal` must be set before calling
/// [`build`](Self::build). All other fields are optional with sensible defaults.
#[must_use = "call .build() to construct the BaseContext"]
pub struct BaseContextBuilder {
    scope: Scope,
    principal: Option<Principal>,
    cancellation: Option<CancellationToken>,
    clock: Option<Box<dyn Clock>>,
    trace_id: Option<TraceId>,
    span_id: Option<SpanId>,
}

impl BaseContextBuilder {
    /// Set the principal (who is acting).
    ///
    /// Required. Use [`Principal::System`] for background / daemon contexts.
    pub fn principal(mut self, p: Principal) -> Self {
        self.principal = Some(p);
        self
    }

    /// Set the cancellation token.
    ///
    /// Defaults to a fresh [`CancellationToken`] if not provided.
    pub fn cancellation(mut self, t: CancellationToken) -> Self {
        self.cancellation = Some(t);
        self
    }

    /// Set the clock implementation.
    ///
    /// Defaults to [`SystemClock`](crate::accessor::SystemClock) if not provided.
    pub fn clock(mut self, c: impl Clock + 'static) -> Self {
        self.clock = Some(Box::new(c));
        self
    }

    /// Set the trace ID for distributed tracing correlation.
    pub fn trace_id(mut self, t: TraceId) -> Self {
        self.trace_id = Some(t);
        self
    }

    /// Set the span ID for distributed tracing correlation.
    pub fn span_id(mut self, s: SpanId) -> Self {
        self.span_id = Some(s);
        self
    }

    /// Build the [`BaseContext`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::CoreError::RegistryInvariant`] if `principal` was not set.
    /// Pass [`Principal::System`] explicitly for background tasks.
    pub fn build(self) -> Result<BaseContext, crate::error::CoreError> {
        let principal = self
            .principal
            .ok_or(crate::error::CoreError::RegistryInvariant(
                "BaseContext requires a principal; use Principal::System for background tasks",
            ))?;
        Ok(BaseContext {
            scope: self.scope,
            principal,
            cancellation: self.cancellation.unwrap_or_default(),
            clock: self
                .clock
                .unwrap_or_else(|| Box::new(crate::accessor::SystemClock)),
            trace_id: self.trace_id,
            span_id: self.span_id,
        })
    }

    /// Build the [`BaseContext`] with the given principal, infallibly.
    ///
    /// The principal is supplied here rather than via
    /// [`principal`](Self::principal), so the only failure path of
    /// [`build`](Self::build) (a missing principal) is eliminated by
    /// construction — this cannot return an error. Any principal previously
    /// staged via [`principal`](Self::principal) is overridden by the argument.
    ///
    /// Prefer this on hot / background paths (warmup, maintenance, type-erased
    /// acquire dispatch) where a `.build().expect(...)` would otherwise sit and
    /// harden into a real panic if the builder contract ever widens.
    pub fn build_with(self, principal: Principal) -> BaseContext {
        BaseContext {
            scope: self.scope,
            principal,
            cancellation: self.cancellation.unwrap_or_default(),
            clock: self
                .clock
                .unwrap_or_else(|| Box::new(crate::accessor::SystemClock)),
            trace_id: self.trace_id,
            span_id: self.span_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::CoreError, scope::Principal};

    #[test]
    fn build_without_principal_returns_registry_invariant_error() {
        let result = BaseContext::builder(Scope::default()).build();
        assert!(
            matches!(result, Err(CoreError::RegistryInvariant(_))),
            "missing principal must yield RegistryInvariant",
        );
    }

    #[test]
    fn build_with_scope_and_principal_succeeds_and_carries_both() {
        use crate::id::{OrgId, WorkspaceId};

        let org_id = OrgId::new();
        let ws_id = WorkspaceId::new();
        let scope = Scope {
            org_id: Some(org_id),
            workspace_id: Some(ws_id),
            ..Scope::default()
        };

        let ctx = BaseContext::builder(scope)
            .principal(Principal::System)
            .build()
            .expect("scope + principal must produce a valid BaseContext");

        assert_eq!(ctx.scope().org_id, Some(org_id));
        assert_eq!(ctx.scope().workspace_id, Some(ws_id));
        assert_eq!(ctx.principal(), &Principal::System);
    }

    #[test]
    fn build_with_user_principal_stores_and_returns_it() {
        use crate::id::UserId;

        let user_id = UserId::new();

        let ctx = BaseContext::builder(Scope::default())
            .principal(Principal::User(user_id))
            .build()
            .expect("user principal must produce a valid BaseContext");

        assert_eq!(
            ctx.principal(),
            &Principal::User(user_id),
            "stored principal must match the one supplied to the builder"
        );
        assert_ne!(
            ctx.principal(),
            &Principal::System,
            "user principal must not compare equal to Principal::System"
        );
    }
}
