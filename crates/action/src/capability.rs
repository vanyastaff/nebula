//! Trigger-specific capability interfaces and default (no-op) accessor factories.
//!
//! Action execution contexts ([`ActionContext`](crate::ActionContext),
//! [`TriggerContext`](crate::TriggerContext)) compose capabilities from
//! `nebula-core` (`Logger`, `ResourceAccessor`, `CredentialAccessor`). This
//! module adds the two trigger-only interfaces that core does not model
//! ([`TriggerScheduler`], [`ExecutionEmitter`]), the shared [`TriggerHealth`]
//! atomics block, and `default_*` constructors that hand out no-op `Arc`s
//! so contexts can be built before the runtime has wired real capabilities.

use std::{
    any::Any,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
    },
    time::Duration,
};

use nebula_core::{
    CoreError, CredentialKey, ResourceKey,
    accessor::{CredentialAccessor, LogLevel, Logger, ResourceAccessor},
    id::ExecutionId,
};

use crate::ActionError;

/// Dyn-safe async return (mirrors `nebula-core::accessor::BoxFuture`).
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── Trigger-only capabilities ──────────────────────────────────────────────

/// Schedule the next invocation of a trigger.
///
/// Dyn-safe: `Arc<dyn TriggerScheduler>` is how the runtime wires this into
/// [`TriggerContext`]. The explicit `Pin<Box<dyn Future>>` return (instead of
/// `async fn`) preserves that dyn-compatibility without the `async_trait`
/// proc-macro — Rust 1.94 native async-in-traits requires the caller to
/// know the concrete `Self`, which breaks dyn dispatch.
pub trait TriggerScheduler: Send + Sync {
    /// Schedule the next trigger run after the given delay.
    fn schedule_after(&self, delay: Duration) -> BoxFut<'_, Result<(), ActionError>>;
}

/// Start a new workflow execution with a typed input payload.
///
/// Dyn-safe (see [`TriggerScheduler`] for the Rust 1.94 rationale).
pub trait ExecutionEmitter: Send + Sync {
    /// Start a new execution for this trigger's workflow with the given input.
    fn emit(&self, input: serde_json::Value) -> BoxFut<'_, Result<ExecutionId, ActionError>>;
}

// ── Trigger health atomics ─────────────────────────────────────────────────

/// Shared health state for a running trigger. Adapter writes, runtime reads.
/// Lock-free via atomics — no allocations per cycle.
///
/// All fields use `Relaxed` ordering (eventual consistency is sufficient for
/// monitoring — exact cross-field consistency is not needed).
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
    /// Not atomic-consistent across fields (each field is read independently),
    /// but sufficient for monitoring.
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

// ── No-op default capabilities ─────────────────────────────────────────────

/// No-op scheduler used when runtime does not inject trigger scheduling.
#[derive(Debug, Default)]
pub struct NoopTriggerScheduler;

impl TriggerScheduler for NoopTriggerScheduler {
    fn schedule_after(&self, _delay: Duration) -> BoxFut<'_, Result<(), ActionError>> {
        Box::pin(async {
            Err(ActionError::fatal(
                "trigger scheduler capability is not configured in TriggerContext",
            ))
        })
    }
}

/// No-op emitter used when runtime does not inject execution emission.
#[derive(Debug, Default)]
pub struct NoopExecutionEmitter;

impl ExecutionEmitter for NoopExecutionEmitter {
    fn emit(&self, _input: serde_json::Value) -> BoxFut<'_, Result<ExecutionId, ActionError>> {
        Box::pin(async {
            Err(ActionError::fatal(
                "execution emitter capability is not configured in TriggerContext",
            ))
        })
    }
}

/// No-op resource accessor — hands out errors for every key.
#[derive(Debug, Default)]
pub struct NoopResourceAccessor;

impl ResourceAccessor for NoopResourceAccessor {
    fn has(&self, _key: &ResourceKey) -> bool {
        false
    }

    fn acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
        let key_str = key.as_str().to_owned();
        Box::pin(async move {
            Err(CoreError::CredentialNotConfigured(format!(
                "resource accessor is not configured (requested `{key_str}`)"
            )))
        })
    }

    fn try_acquire_any(
        &self,
        _key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        Box::pin(async { Ok(None) })
    }
}

/// No-op credential accessor — hands out errors for every key.
#[derive(Debug, Default)]
pub struct NoopCredentialAccessor;

impl CredentialAccessor for NoopCredentialAccessor {
    fn has(&self, _key: &CredentialKey) -> bool {
        false
    }

    fn resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
        let key_str = key.as_str().to_owned();
        Box::pin(async move { Err(CoreError::CredentialNotFound { key: key_str }) })
    }

    fn try_resolve_any(
        &self,
        _key: &CredentialKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        Box::pin(async { Ok(None) })
    }
}

/// No-op logger — discards every record.
#[derive(Debug, Default)]
pub struct NoopLogger;

impl Logger for NoopLogger {
    fn log(&self, _level: LogLevel, _message: &str) {}
    fn log_with_fields(&self, _level: LogLevel, _message: &str, _fields: &[(&str, &str)]) {}
}

/// Default trigger scheduler (no-op).
#[must_use]
pub fn default_trigger_scheduler() -> Arc<dyn TriggerScheduler> {
    Arc::new(NoopTriggerScheduler)
}

/// Default execution emitter (no-op).
#[must_use]
pub fn default_execution_emitter() -> Arc<dyn ExecutionEmitter> {
    Arc::new(NoopExecutionEmitter)
}

/// Default resource accessor (no-op).
#[must_use]
pub fn default_resource_accessor() -> Arc<dyn ResourceAccessor> {
    Arc::new(NoopResourceAccessor)
}

/// Default credential accessor (no-op).
#[must_use]
pub fn default_credential_accessor() -> Arc<dyn CredentialAccessor> {
    Arc::new(NoopCredentialAccessor)
}

/// Default action logger (no-op).
#[must_use]
pub fn default_action_logger() -> Arc<dyn Logger> {
    Arc::new(NoopLogger)
}
