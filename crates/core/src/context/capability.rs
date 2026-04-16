//! Capability traits for context composition (spec 23).

use crate::accessor::{CredentialAccessor, EventEmitter, Logger, MetricsEmitter, ResourceAccessor};

/// Capability: access to managed resources.
pub trait HasResources: crate::context::Context {
    /// Get the resource accessor.
    fn resources(&self) -> &dyn ResourceAccessor;
}

/// Capability: access to credentials.
pub trait HasCredentials: crate::context::Context {
    /// Get the credential accessor.
    fn credentials(&self) -> &dyn CredentialAccessor;
}

/// Capability: structured logging.
pub trait HasLogger: crate::context::Context {
    /// Get the logger.
    fn logger(&self) -> &dyn Logger;
}

/// Capability: metrics emission.
///
/// Returns `&dyn MetricsEmitter` -- the core abstraction for metrics.
/// `nebula_telemetry::metrics::MetricsRegistry` is the concrete implementation
/// used at runtime; this trait intentionally depends only on the core abstraction
/// so that library crates stay decoupled from the telemetry backend.
pub trait HasMetrics: crate::context::Context {
    /// Get the metrics emitter.
    fn metrics(&self) -> &dyn MetricsEmitter;
}

/// Capability: event bus.
pub trait HasEventBus: crate::context::Context {
    /// Get the event emitter.
    fn eventbus(&self) -> &dyn EventEmitter;
}
