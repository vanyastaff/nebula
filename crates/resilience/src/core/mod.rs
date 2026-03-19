//! Core types for the resilience library.
//!
//! - [`types`] — `CallError<E>`, `ConfigError`, `CallResult<T, E>`
//! - [`error`] — `ResilienceError` (used by fallback/cancellation patterns)
//! - [`metrics`] — `MetricsCollector`, `MetricSnapshot`
//! - [`policy_source`] — `PolicySource<C>`, `LoadSignal`, `ConstantLoad`
//! - [`signals`] — load signal types
//! - [`cancellation`] — `CancellationContext`, `ShutdownCoordinator`

pub mod cancellation;
mod error;
mod metrics;
pub mod policy_source;
mod result;
pub mod signals;
pub mod types;

// Primary re-exports
pub use error::{CircuitBreakerOpenState, ErrorClass, ErrorContext, ResilienceError};
pub use metrics::{MetricKind, MetricSnapshot, Metrics, MetricsCollector};
pub use policy_source::PolicySource;
pub use result::{ResilienceResult, ResultExt};
pub use signals::{ConstantLoad, LoadSignal};
pub use types::{CallError, CallResult};

pub use cancellation::{
    CancellableFuture, CancellationContext, CancellationExt, ShutdownCoordinator,
};
