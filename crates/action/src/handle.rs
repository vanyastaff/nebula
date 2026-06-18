//! `ActionHandle` enum + object-safe per-variant sub-traits.
//!
//! The engine cannot dispatch on the
//! `Sized` [`Action`](crate::Action) trait directly because Variant A makes
//! it object-unsafe. Instead, the engine works against an enum
//! [`ActionHandle`] whose variants wrap `Box<dyn XxxHandle>` trait objects
//! — one per execution-shape kind:
//!
//! - [`StatelessHandle`] — one-shot JSON in / JSON out.
//! - [`StatefulHandle`] — iterative with mutable JSON state.
//! - [`TriggerHandle`] — start/stop trigger lifecycle.
//! - [`ResourceHandle`] — graph-scoped resource configure/cleanup.
//! - [`ControlHandle`] — flow-control nodes desugared to a stateless surface.
//!
//! The engine produces an `ActionHandle` per execution via
//! [`ActionFactory::instantiate`](crate::ActionFactory::instantiate).

use std::{any::Any, fmt};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    context::{ActionContext, TriggerContext},
    error::ActionError,
    metadata::ActionMetadata,
    result::ActionResult,
    trigger::{TriggerEvent, TriggerEventOutcome},
};

// ── Sub-traits ──────────────────────────────────────────────────────────────

/// Object-safe stateless dispatch surface.
///
/// Mirrors the typed [`StatelessAction`](crate::StatelessAction) but works
/// on `Value` to/from for engine erasure. Implementors are typically
/// generic wrappers produced by an [`ActionFactory`](crate::ActionFactory).
#[async_trait]
pub trait StatelessHandle: Send + Sync + 'static {
    /// Action metadata (key, version, ports, schemas).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute one-shot with JSON input.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] on validation, retryable, or fatal failure.
    async fn dispatch(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

/// Object-safe stateful dispatch surface.
#[async_trait]
pub trait StatefulHandle: Send + Sync + 'static {
    /// Action metadata.
    fn metadata(&self) -> &ActionMetadata;

    /// Build initial state as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] if state initialization fails (e.g.,
    /// serialization error in the underlying typed action).
    fn init_state(&self) -> Result<Value, ActionError>;

    /// Attempt to migrate state from a previous version.
    fn migrate_state(&self, _old: Value) -> Option<Value> {
        None
    }

    /// Execute one iteration with mutable JSON state.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] on validation, retryable, or fatal failure.
    async fn dispatch(
        &self,
        input: &Value,
        state: &mut Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

/// Object-safe trigger dispatch surface (start / stop / handle_event).
#[async_trait]
pub trait TriggerHandle: Send + Sync + 'static {
    /// Action metadata.
    fn metadata(&self) -> &ActionMetadata;

    /// Start the trigger.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if trigger cannot be started.
    async fn start(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError>;

    /// Stop the trigger.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if trigger cannot be stopped cleanly.
    async fn stop(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError>;

    /// Whether this trigger accepts externally pushed events.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Handle an external event pushed to this trigger.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] by default — triggers that don't accept
    /// external events should never have this called.
    async fn handle_event(
        &self,
        event: TriggerEvent,
        ctx: &dyn TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        let _ = (event, ctx);
        Err(ActionError::fatal(
            "trigger does not accept external events",
        ))
    }
}

/// Object-safe resource dispatch surface (configure/cleanup lifecycle).
#[async_trait]
pub trait ResourceHandle: Send + Sync + 'static {
    /// Action metadata.
    fn metadata(&self) -> &ActionMetadata;

    /// Configure the resource for this scope.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the resource cannot be configured.
    async fn configure(
        &self,
        config: Value,
        ctx: &dyn ActionContext,
    ) -> Result<Box<dyn Any + Send + Sync>, ActionError>;

    /// Clean up the resource instance when the scope ends.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if cleanup fails.
    async fn cleanup(
        &self,
        instance: Box<dyn Any + Send + Sync>,
        ctx: &dyn ActionContext,
    ) -> Result<(), ActionError>;
}

/// Object-safe control dispatch surface (flow-control desugared to stateless).
#[async_trait]
pub trait ControlHandle: Send + Sync + 'static {
    /// Action metadata (with `ActionCategory::Control` or `Terminal` stamped).
    fn metadata(&self) -> &ActionMetadata;

    /// Evaluate the control decision and emit an [`ActionResult<Value>`].
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] on validation or fatal failure.
    async fn dispatch(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

// ── Top-level enum ──────────────────────────────────────────────────────────

/// Top-level action-handle enum — engine dispatches on the variant.
///
/// Each variant wraps a `Box<dyn XxxHandle>`. The engine produces these
/// per-execution via [`ActionFactory::instantiate`](crate::ActionFactory::instantiate).
#[non_exhaustive]
pub enum ActionHandle {
    /// One-shot stateless execution.
    Stateless(Box<dyn StatelessHandle>),
    /// Iterative execution with mutable JSON state.
    Stateful(Box<dyn StatefulHandle>),
    /// Workflow trigger (start/stop lifecycle).
    Trigger(Box<dyn TriggerHandle>),
    /// Graph-scoped resource (configure/cleanup).
    Resource(Box<dyn ResourceHandle>),
    /// Flow-control node (If / Switch / Router / Filter / NoOp / Stop / Fail).
    Control(Box<dyn ControlHandle>),
}

impl ActionHandle {
    /// Get metadata regardless of variant.
    #[must_use]
    pub fn metadata(&self) -> &ActionMetadata {
        match self {
            Self::Stateless(h) => h.metadata(),
            Self::Stateful(h) => h.metadata(),
            Self::Trigger(h) => h.metadata(),
            Self::Resource(h) => h.metadata(),
            Self::Control(h) => h.metadata(),
        }
    }

    /// Whether this is a stateless action handle.
    #[must_use]
    pub fn is_stateless(&self) -> bool {
        matches!(self, Self::Stateless(_))
    }

    /// Whether this is a stateful action handle.
    #[must_use]
    pub fn is_stateful(&self) -> bool {
        matches!(self, Self::Stateful(_))
    }

    /// Whether this is a trigger action handle.
    #[must_use]
    pub fn is_trigger(&self) -> bool {
        matches!(self, Self::Trigger(_))
    }

    /// Whether this is a resource action handle.
    #[must_use]
    pub fn is_resource(&self) -> bool {
        matches!(self, Self::Resource(_))
    }

    /// Whether this is a control action handle.
    #[must_use]
    pub fn is_control(&self) -> bool {
        matches!(self, Self::Control(_))
    }
}

impl fmt::Debug for ActionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (tag, key) = match self {
            Self::Stateless(h) => ("Stateless", h.metadata().base.key.as_str()),
            Self::Stateful(h) => ("Stateful", h.metadata().base.key.as_str()),
            Self::Trigger(h) => ("Trigger", h.metadata().base.key.as_str()),
            Self::Resource(h) => ("Resource", h.metadata().base.key.as_str()),
            Self::Control(h) => ("Control", h.metadata().base.key.as_str()),
        };
        f.debug_tuple(tag).field(&key).finish()
    }
}
