//! Context system -- base trait + capabilities (spec 23).

pub mod capability;

pub use capability::*;
use tokio_util::sync::CancellationToken;

use crate::{
    accessor::Clock,
    obs::TraceId,
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
}

/// Shared identity fields. Domain contexts embed and delegate.
pub struct BaseContext {
    scope: Scope,
    principal: Principal,
    cancellation: CancellationToken,
    clock: Box<dyn Clock>,
    trace_id: Option<TraceId>,
}

impl BaseContext {
    /// Create a builder for BaseContext.
    pub fn builder() -> BaseContextBuilder {
        BaseContextBuilder::default()
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
}

/// Builder for BaseContext.
#[derive(Default)]
pub struct BaseContextBuilder {
    scope: Option<Scope>,
    principal: Option<Principal>,
    cancellation: Option<CancellationToken>,
    clock: Option<Box<dyn Clock>>,
    trace_id: Option<TraceId>,
}

impl BaseContextBuilder {
    /// Set the scope.
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }
    /// Set the principal.
    pub fn principal(mut self, p: Principal) -> Self {
        self.principal = Some(p);
        self
    }
    /// Set the cancellation token.
    pub fn cancellation(mut self, t: CancellationToken) -> Self {
        self.cancellation = Some(t);
        self
    }
    /// Set the clock.
    pub fn clock(mut self, c: impl Clock + 'static) -> Self {
        self.clock = Some(Box::new(c));
        self
    }
    /// Set the trace ID.
    pub fn trace_id(mut self, t: TraceId) -> Self {
        self.trace_id = Some(t);
        self
    }

    /// Build the BaseContext.
    pub fn build(self) -> BaseContext {
        BaseContext {
            scope: self.scope.unwrap_or_default(),
            principal: self.principal.unwrap_or(Principal::System),
            cancellation: self.cancellation.unwrap_or_default(),
            clock: self
                .clock
                .unwrap_or_else(|| Box::new(crate::accessor::SystemClock)),
            trace_id: self.trace_id,
        }
    }
}
