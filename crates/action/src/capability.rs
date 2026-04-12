//! Capability module interfaces for action and trigger contexts.
//!
//! These traits are object-safe boundaries injected by runtime/engine so
//! action code can access resources and logging without coupling to concrete
//! manager implementations.
//!
//! Credential accessor types (`CredentialAccessor`, `CredentialAccessError`,
//! `ScopedCredentialAccessor`, etc.) live in [`nebula_credential`] and are
//! imported directly by consumers — `nebula-action` does not re-export them.
//! Action authors interact with credentials through
//! [`CredentialGuard`](nebula_credential::CredentialGuard) returned by
//! [`ActionContext::credential`](crate::ActionContext::credential).

use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
    },
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::id::ExecutionId;

use crate::ActionError;

/// Object-safe resource accessor injected into [`crate::ActionContext`].
#[async_trait]
pub trait ResourceAccessor: Send + Sync {
    /// Acquire a resource by key.
    ///
    /// Returns a type-erased instance that action code can downcast.
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError>;

    /// Check whether a resource exists for the given key.
    async fn exists(&self, key: &str) -> bool;
}

/// Log severity for action-scoped logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionLogLevel {
    /// Trace-level diagnostic event.
    Trace,
    /// Debug-level diagnostic event.
    Debug,
    /// Informational event.
    Info,
    /// Warning event.
    Warn,
    /// Error event.
    Error,
}

/// Object-safe logging capability injected into action contexts.
pub trait ActionLogger: Send + Sync {
    /// Emit a message at the given level.
    fn log(&self, level: ActionLogLevel, message: &str);
}

/// Object-safe scheduling capability injected into trigger contexts.
#[async_trait]
pub trait TriggerScheduler: Send + Sync {
    /// Schedule the next trigger run after the given delay.
    async fn schedule_after(&self, delay: Duration) -> Result<(), ActionError>;
}

/// Object-safe execution emission capability injected into trigger contexts.
#[async_trait]
pub trait ExecutionEmitter: Send + Sync {
    /// Start a new execution for this trigger's workflow with the given input.
    async fn emit(&self, input: serde_json::Value) -> Result<ExecutionId, ActionError>;
}

/// Shared health state for a running trigger. Adapter writes,
/// runtime reads. Lock-free via atomics — no allocations per cycle.
///
/// Lives on `TriggerContext` as `Arc<TriggerHealth>`. Not behind a
/// trait — health shape is universal, trait dispatch adds nothing.
///
/// All fields use `Relaxed` ordering (eventual consistency is
/// sufficient for monitoring — exact cross-field consistency is
/// not needed).
pub struct TriggerHealth {
    /// Epoch millis of last main operation. 0 = never.
    last_active_at: AtomicU64,
    /// Epoch millis of last successful event emission. 0 = never.
    last_success_at: AtomicU64,
    /// Consecutive cycles without progress.
    idle_streak: AtomicU32,
    /// Consecutive errors.
    error_streak: AtomicU32,
    /// Total events emitted since start.
    total_emitted: AtomicU64,
    /// Total cycles since start.
    total_cycles: AtomicU64,
}

impl TriggerHealth {
    /// Create a new health state with all counters at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_active_at: AtomicU64::new(0),
            last_success_at: AtomicU64::new(0),
            idle_streak: AtomicU32::new(0),
            error_streak: AtomicU32::new(0),
            total_emitted: AtomicU64::new(0),
            total_cycles: AtomicU64::new(0),
        }
    }

    /// Record a completed cycle with successful event emission.
    pub fn record_success(&self, emitted: u64) {
        let now = now_millis();
        self.last_active_at.store(now, Relaxed);
        self.last_success_at.store(now, Relaxed);
        self.total_emitted.fetch_add(emitted, Relaxed);
        self.total_cycles.fetch_add(1, Relaxed);
        self.idle_streak.store(0, Relaxed);
        self.error_streak.store(0, Relaxed);
    }

    /// Record a completed cycle with no events (idle).
    pub fn record_idle(&self) {
        self.last_active_at.store(now_millis(), Relaxed);
        self.total_cycles.fetch_add(1, Relaxed);
        self.idle_streak.fetch_add(1, Relaxed);
        self.error_streak.store(0, Relaxed);
    }

    /// Record a failed cycle (retryable error).
    pub fn record_error(&self) {
        self.last_active_at.store(now_millis(), Relaxed);
        self.total_cycles.fetch_add(1, Relaxed);
        self.error_streak.fetch_add(1, Relaxed);
    }

    /// Read a point-in-time snapshot for dashboards / API.
    ///
    /// Not atomic-consistent across fields (each field is read
    /// independently), but sufficient for monitoring.
    #[must_use]
    pub fn snapshot(&self) -> TriggerHealthSnapshot {
        TriggerHealthSnapshot {
            last_active_at: self.last_active_at.load(Relaxed),
            last_success_at: self.last_success_at.load(Relaxed),
            idle_streak: self.idle_streak.load(Relaxed),
            error_streak: self.error_streak.load(Relaxed),
            total_emitted: self.total_emitted.load(Relaxed),
            total_cycles: self.total_cycles.load(Relaxed),
        }
    }
}

impl Default for TriggerHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TriggerHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerHealth")
            .field("total_cycles", &self.total_cycles.load(Relaxed))
            .field("total_emitted", &self.total_emitted.load(Relaxed))
            .field("idle_streak", &self.idle_streak.load(Relaxed))
            .field("error_streak", &self.error_streak.load(Relaxed))
            .finish()
    }
}

/// Point-in-time health snapshot — plain data, serializable.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TriggerHealthSnapshot {
    /// Epoch millis of last main operation. 0 = never.
    pub last_active_at: u64,
    /// Epoch millis of last successful event emission. 0 = never.
    pub last_success_at: u64,
    /// Consecutive cycles without progress.
    pub idle_streak: u32,
    /// Consecutive errors.
    pub error_streak: u32,
    /// Total events emitted since start.
    pub total_emitted: u64,
    /// Total cycles since start.
    pub total_cycles: u64,
}

/// Current time as epoch milliseconds.
#[must_use]
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// No-op logger used when runtime does not inject a logger capability.
#[derive(Debug, Default)]
pub struct NoopActionLogger;

impl ActionLogger for NoopActionLogger {
    fn log(&self, _level: ActionLogLevel, _message: &str) {}
}

/// No-op scheduler used when runtime does not inject trigger scheduling.
#[derive(Debug, Default)]
pub struct NoopTriggerScheduler;

#[async_trait]
impl TriggerScheduler for NoopTriggerScheduler {
    async fn schedule_after(&self, _delay: Duration) -> Result<(), ActionError> {
        Err(ActionError::fatal(
            "trigger scheduler capability is not configured in TriggerContext",
        ))
    }
}

/// No-op emitter used when runtime does not inject execution emission.
#[derive(Debug, Default)]
pub struct NoopExecutionEmitter;

#[async_trait]
impl ExecutionEmitter for NoopExecutionEmitter {
    async fn emit(&self, _input: serde_json::Value) -> Result<ExecutionId, ActionError> {
        Err(ActionError::fatal(
            "execution emitter capability is not configured in TriggerContext",
        ))
    }
}

/// No-op resource accessor used when runtime does not inject resources.
#[derive(Debug, Default)]
pub struct NoopResourceAccessor;

#[async_trait]
impl ResourceAccessor for NoopResourceAccessor {
    async fn acquire(&self, _key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        Err(ActionError::fatal(
            "resource capability is not configured in ActionContext",
        ))
    }

    async fn exists(&self, _key: &str) -> bool {
        false
    }
}

/// Default resource accessor capability.
#[must_use]
pub fn default_resource_accessor() -> Arc<dyn ResourceAccessor> {
    Arc::new(NoopResourceAccessor)
}

/// Default action logger capability.
#[must_use]
pub fn default_action_logger() -> Arc<dyn ActionLogger> {
    Arc::new(NoopActionLogger)
}

/// Default trigger scheduler capability.
#[must_use]
pub fn default_trigger_scheduler() -> Arc<dyn TriggerScheduler> {
    Arc::new(NoopTriggerScheduler)
}

/// Default execution emitter capability.
#[must_use]
pub fn default_execution_emitter() -> Arc<dyn ExecutionEmitter> {
    Arc::new(NoopExecutionEmitter)
}
