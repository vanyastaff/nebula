//! Pool internal types: `PoolInner`, `PoolState`, `Entry`, `PoolResilienceState`, etc.
//!
//! ## Lock ordering in `PoolInner`
//!
//! When acquiring multiple locks simultaneously, always in this order:
//!
//! 1. `state`              — idle queue + stats
//! 2. `latency_state`      — histogram (independent, but if both needed — `state` first)
//! 3. `maintenance_handle` — write-once at spawn, take-once at shutdown
//!
//! `PoolResilienceState` contains only lock-free types (atomics, `Copy`).
//! Can be read at any time without lock ordering.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use nebula_core::ResourceKey;
use nebula_resilience::{CircuitBreaker, Gate};
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::events::EventBus;
use crate::hooks::HookRegistry;
use crate::lifecycle::Lifecycle;
use crate::resource::Resource;

use super::InstanceMetadata;
use super::config::{PoolConfig, PoolResiliencePolicy};
use super::stats::{LatencyHistogram, LatencyPercentiles, PoolStats};

// ---------------------------------------------------------------------------
// CounterGuard — RAII waiter-count tracker
// ---------------------------------------------------------------------------

/// RAII guard that increments an [`AtomicUsize`] on construction and
/// decrements it on drop.
///
/// Used in [`Pool::acquire_inner`](super::Pool::acquire_inner) to track the number of callers
/// currently waiting for a semaphore permit.
pub(super) struct CounterGuard(Arc<AtomicUsize>);

impl CounterGuard {
    pub(super) fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(Arc::clone(counter))
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// PoolResilienceState
// ---------------------------------------------------------------------------

/// Runtime resilience state for a pool.
///
/// Constructed from [`PoolResiliencePolicy`] during pool construction and boxed
/// behind `Option<Box<PoolResilienceState>>` on [`PoolInner`] to eliminate all
/// overhead for pools that don't use resilience features.
pub(super) struct PoolResilienceState {
    /// Circuit breaker for `Resource::create()`.
    pub(super) create_breaker: Option<CircuitBreaker>,
    /// Circuit breaker for `Resource::recycle()`.
    pub(super) recycle_breaker: Option<CircuitBreaker>,
}

impl PoolResilienceState {
    /// Build from a [`PoolResiliencePolicy`].
    ///
    /// Returns `Ok(None)` when the policy is fully default (all `None`),
    /// eliminating any Box allocation for the common case.
    pub(super) fn from_policy(policy: &PoolResiliencePolicy) -> Result<Option<Self>> {
        if policy.is_empty() {
            return Ok(None);
        }
        let create_breaker = if let Some(cfg) = &policy.create_breaker {
            Some(
                CircuitBreaker::new(cfg.clone()).map_err(|e| Error::Configuration {
                    message: format!("create_breaker config invalid: {e}"),
                    source: None,
                })?,
            )
        } else {
            None
        };
        let recycle_breaker = if let Some(cfg) = &policy.recycle_breaker {
            Some(
                CircuitBreaker::new(cfg.clone()).map_err(|e| Error::Configuration {
                    message: format!("recycle_breaker config invalid: {e}"),
                    source: None,
                })?,
            )
        } else {
            None
        };
        Ok(Some(Self {
            create_breaker,
            recycle_breaker,
        }))
    }
}

// ---------------------------------------------------------------------------
// IdleResult
// ---------------------------------------------------------------------------

/// Outcome of a single pop from the idle queue.
pub(super) enum IdleResult<T> {
    /// Non-expired entry popped; statistics already committed under lock.
    Valid(Entry<T>, u64),
    /// Expired entry popped; needs async cleanup, no stats committed.
    Expired(T),
    /// Queue empty; caller must create a new instance.
    Miss,
}

// ---------------------------------------------------------------------------
// EntryMeta
// ---------------------------------------------------------------------------

/// Metadata pulled from an [`Entry`] at acquire time, carried through to the
/// release path so that [`InstanceMetadata`] can be rebuilt without a second
/// state-lock acquisition.
///
/// `None` for freshly created instances.
#[derive(Debug, Clone, Copy)]
pub(super) struct EntryMeta {
    pub(super) created_at: Instant,
    /// Incremented each time the instance is checked out (the +1 for the
    /// current acquisition has already been applied).
    pub(super) acquire_count: usize,
}

impl EntryMeta {
    pub(super) fn to_instance_metadata(self, idle_since: Instant) -> InstanceMetadata {
        InstanceMetadata {
            created_at: self.created_at,
            idle_since,
            acquire_count: self.acquire_count,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry<T>
// ---------------------------------------------------------------------------

/// A pool entry wrapping a resource instance with lifecycle tracking.
pub(super) struct Entry<T> {
    pub(super) instance: T,
    pub(super) created_at: Instant,
    pub(super) last_used: Instant,
    /// How many times this entry has been acquired.
    pub(super) acquire_count: usize,
    /// Current lifecycle state of this entry.
    #[expect(
        dead_code,
        reason = "tracked for observability and future drain/shutdown logic"
    )]
    pub(super) lifecycle: Lifecycle,
}

impl<T> Entry<T> {
    pub(super) fn new(instance: T) -> Self {
        let now = Instant::now();
        Self {
            instance,
            created_at: now,
            last_used: now,
            acquire_count: 0,
            lifecycle: Lifecycle::Ready,
        }
    }

    /// Return an entry to the pool, preserving the original `created_at`.
    pub(super) fn returned(instance: T, created_at: Instant, acquire_count: usize) -> Self {
        Self {
            instance,
            created_at,
            last_used: Instant::now(),
            acquire_count,
            lifecycle: Lifecycle::Idle,
        }
    }

    pub(super) fn is_expired(&self, config: &PoolConfig) -> bool {
        self.created_at.elapsed() > config.lifetime.max_lifetime
            || self.last_used.elapsed() > config.lifetime.idle_timeout
    }
}

// ---------------------------------------------------------------------------
// PoolState<T>
// ---------------------------------------------------------------------------

/// Combined pool state: idle queue + statistics under a single lock.
pub(super) struct PoolState<T> {
    pub(super) idle: VecDeque<Entry<T>>,
    pub(super) stats: PoolStats,
    /// Set to `true` after `shutdown()` to prevent Guard drops from
    /// reinserting instances into the idle queue.
    pub(super) shutdown: bool,
}

// ---------------------------------------------------------------------------
// LatencyState
// ---------------------------------------------------------------------------

/// Latency histogram state, kept in a **separate** lock from [`PoolState`].
///
/// Separating histogram recording from the idle-queue mutex means that
/// concurrent workers can record latency samples in parallel with other
/// threads checking out or returning instances.
pub(super) struct LatencyState {
    pub(super) histogram: LatencyHistogram,
    pub(super) seq: u64,
    pub(super) percentiles_cache: Option<LatencyPercentiles>,
}

// ---------------------------------------------------------------------------
// PoolInner<R>
// ---------------------------------------------------------------------------

/// Inner shared state for the pool.
pub(super) struct PoolInner<R: Resource> {
    pub(super) resource: Arc<R>,
    pub(super) config: R::Config,
    pub(super) pool_config: PoolConfig,
    /// Cached resource key — avoids calling `resource.metadata()` on the hot path.
    pub(super) resource_key: ResourceKey,
    pub(super) state: Mutex<crate::poison::Poison<PoolState<R::Instance>>>,
    /// Separate lock for latency histogram — decoupled from the idle-queue mutex.
    pub(super) latency_state: Mutex<LatencyState>,
    /// Semaphore limits concurrent active (checked-out) instances.
    pub(super) semaphore: Semaphore,
    /// Cooperative shutdown barrier.
    pub(super) gate: Gate,
    /// Cancellation token for background tasks.
    pub(super) cancel: CancellationToken,
    /// Optional event bus for emitting lifecycle events.
    pub(super) event_bus: Option<Arc<EventBus>>,
    /// Number of callers currently waiting to acquire an instance.
    pub(super) waiting_count: Arc<AtomicUsize>,
    /// Lock-free active instance counter for adaptive backpressure.
    pub(super) active_count: AtomicUsize,
    /// Optional hook registry for lifecycle hooks.
    pub(super) hooks: Option<Arc<HookRegistry>>,
    /// Handle for the background maintenance task.
    pub(super) maintenance_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Runtime resilience state (circuit breakers + timeouts).
    ///
    /// `None` when all resilience fields are default — zero overhead on the
    /// hot path for the common case that doesn't use circuit breakers.
    pub(super) resilience: Option<Box<PoolResilienceState>>,
    /// Pre-built synthetic context for background maintenance operations.
    pub(super) maintenance_ctx: Context,
    /// Optional context enricher called immediately before `Resource::create()`.
    pub(super) context_enricher: Option<Arc<dyn Fn(Context) -> Context + Send + Sync>>,
}
