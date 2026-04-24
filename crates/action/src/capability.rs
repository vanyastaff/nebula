//! Trigger-specific capability interfaces for action contexts.

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
    accessor::{CredentialAccessor, EventEmitter, LogLevel, Logger, MetricsEmitter, ResourceAccessor},
    id::ExecutionId,
};

use crate::ActionError;

/// Object-safe scheduling capability injected into trigger contexts.
pub trait TriggerScheduler: Send + Sync {
    /// Schedule the next trigger run after the given delay.
    fn schedule_after(
        &self,
        delay: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + '_>>;
}

/// Object-safe execution emission capability injected into trigger contexts.
pub trait ExecutionEmitter: Send + Sync {
    /// Start a new execution for this trigger's workflow with the given input.
    fn emit(
        &self,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<ExecutionId, ActionError>> + Send + '_>>;
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

/// No-op scheduler used when runtime does not inject trigger scheduling.
#[derive(Debug, Default)]
pub struct NoopTriggerScheduler;

impl TriggerScheduler for NoopTriggerScheduler {
    fn schedule_after(
        &self,
        _delay: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + '_>> {
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
    fn emit(
        &self,
        _input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<ExecutionId, ActionError>> + Send + '_>> {
        Box::pin(async {
            Err(ActionError::fatal(
                "execution emitter capability is not configured in TriggerContext",
            ))
        })
    }
}

// ── Default capability accessors ───────────────────────────────────────────
//
// Wired into `ActionRuntimeContext::new` / `TriggerRuntimeContext::new` so
// every freshly-constructed context starts with fail-closed capabilities that
// the runtime then replaces via `.with_*` builders. Exposed publicly so
// non-action crates (engine pipelines, test harnesses) can share the same
// fail-closed defaults instead of cloning local stubs.

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// No-op resource accessor — fails closed on every `acquire_any`.
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

/// No-op credential accessor — fails closed on every `resolve_any`.
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
        Box::pin(async move {
            Err(CoreError::CredentialNotConfigured(format!(
                "credential accessor is not configured (requested `{key_str}`)"
            )))
        })
    }
    fn try_resolve_any(
        &self,
        _key: &CredentialKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        Box::pin(async { Ok(None) })
    }
}

/// No-op logger — silently drops every log line.
#[derive(Debug, Default)]
pub struct NoopLogger;

impl Logger for NoopLogger {
    fn log(&self, _level: LogLevel, _message: &str) {}
    fn log_with_fields(&self, _level: LogLevel, _message: &str, _fields: &[(&str, &str)]) {}
}

/// No-op metrics emitter — silently drops every sample.
#[derive(Debug, Default)]
pub struct NoopMetricsEmitter;

impl MetricsEmitter for NoopMetricsEmitter {
    fn counter(&self, _name: &str, _value: u64, _labels: &[(&str, &str)]) {}
    fn gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
    fn histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
}

/// No-op event emitter — silently drops every event.
#[derive(Debug, Default)]
pub struct NoopEventEmitter;

impl EventEmitter for NoopEventEmitter {
    fn emit(&self, _topic: &str, _payload: serde_json::Value) {}
}

/// Default resource accessor — [`NoopResourceAccessor`].
#[must_use]
pub fn default_resource_accessor() -> Arc<dyn ResourceAccessor> {
    Arc::new(NoopResourceAccessor)
}

/// Default credential accessor — [`NoopCredentialAccessor`].
#[must_use]
pub fn default_credential_accessor() -> Arc<dyn CredentialAccessor> {
    Arc::new(NoopCredentialAccessor)
}

/// Default logger — [`NoopLogger`].
#[must_use]
pub fn default_action_logger() -> Arc<dyn Logger> {
    Arc::new(NoopLogger)
}

/// Default metrics emitter — [`NoopMetricsEmitter`].
#[must_use]
pub fn default_metrics_emitter() -> Arc<dyn MetricsEmitter> {
    Arc::new(NoopMetricsEmitter)
}

/// Default event emitter — [`NoopEventEmitter`].
#[must_use]
pub fn default_event_emitter() -> Arc<dyn EventEmitter> {
    Arc::new(NoopEventEmitter)
}

/// Default trigger scheduler — [`NoopTriggerScheduler`].
#[must_use]
pub fn default_trigger_scheduler() -> Arc<dyn TriggerScheduler> {
    Arc::new(NoopTriggerScheduler)
}

/// Default execution emitter — [`NoopExecutionEmitter`].
#[must_use]
pub fn default_execution_emitter() -> Arc<dyn ExecutionEmitter> {
    Arc::new(NoopExecutionEmitter)
}

