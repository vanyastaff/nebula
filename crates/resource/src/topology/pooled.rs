//! Pool topology — N interchangeable stateful instances with checkout/recycle/destroy.

use std::future::Future;

use crate::{ctx::Ctx, resource::Resource};

/// Synchronous broken-check result.
///
/// Used in the `Drop` path to decide whether an instance should be returned
/// to the idle pool or destroyed immediately. Because it runs in `Drop`,
/// this check must be synchronous and O(1) — no I/O, no async.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum BrokenCheck {
    /// Instance is healthy and can be returned to the pool.
    Healthy,
    /// Instance is broken with the given reason and should be destroyed.
    Broken(String),
}

impl BrokenCheck {
    /// Returns `true` if the instance is broken.
    pub fn is_broken(&self) -> bool {
        matches!(self, Self::Broken(_))
    }
}

/// Decision after an async recycle check.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecycleDecision {
    /// Return the instance to the idle pool.
    Keep,
    /// Destroy the instance instead of recycling it.
    Drop,
}

/// Metrics available during the recycle check.
///
/// These are maintained by the pool manager and passed to
/// [`Pooled::recycle`] so the implementation can make informed
/// keep-or-drop decisions.
#[derive(Debug, Clone)]
pub struct InstanceMetrics {
    /// Number of errors observed during checkouts of this instance.
    pub error_count: u64,
    /// Number of times this instance has been checked out.
    pub checkout_count: u64,
    /// When this instance was created.
    pub created_at: std::time::Instant,
}

impl InstanceMetrics {
    /// Returns the age of this instance.
    pub fn age(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }
}

/// Pool topology — N interchangeable stateful instances with
/// checkout/recycle/destroy.
///
/// Implementors extend [`Resource`] with pool-aware lifecycle hooks:
/// a sync broken check (for the `Drop` path), an async recycle step,
/// and an optional per-checkout prepare step.
///
/// # Acquire bounds
///
/// [`Manager::acquire_pooled`](crate::Manager::acquire_pooled) requires:
/// - `R: Clone + Send + Sync + 'static`
/// - `R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static`
/// - `R::Lease: Into<R::Runtime> + Send + 'static`
///
/// If `Runtime` and `Lease` are the same type, the blanket
/// `impl<T> From<T> for T` satisfies both conversion bounds automatically.
pub trait Pooled: Resource {
    /// Sync O(1) broken check. Called in the `Drop` path — NO async, NO I/O.
    ///
    /// The default implementation reports all instances as healthy.
    fn is_broken(&self, _runtime: &Self::Runtime) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    /// Async recycle check performed when an instance is returned to the pool.
    ///
    /// Implementations can inspect [`InstanceMetrics`] to decide whether to
    /// keep or drop the instance. The default keeps everything.
    fn recycle(
        &self,
        _runtime: &Self::Runtime,
        _metrics: &InstanceMetrics,
    ) -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send {
        async { Ok(RecycleDecision::Keep) }
    }

    /// Prepares an instance for a specific execution context.
    ///
    /// Called after checkout, before the caller receives the lease.
    /// Use this for operations like `SET search_path` or `USE database`.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if preparation fails. The pool manager will
    /// destroy the instance and try another one.
    fn prepare(
        &self,
        _runtime: &Self::Runtime,
        _ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

/// Configuration types for pool topology.
pub mod config {
    use std::time::Duration;

    /// Pool checkout ordering strategy.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub enum PoolStrategy {
        /// Last-in, first-out — reuses the most recently returned instance.
        #[default]
        Lifo,
        /// First-in, first-out — spreads load evenly across instances.
        Fifo,
    }

    /// Strategy for pre-warming the pool at startup.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub enum WarmupStrategy {
        /// No warmup — instances created on demand.
        #[default]
        None,
        /// Create `min_size` instances one at a time.
        Sequential,
        /// Create `min_size` instances concurrently.
        Parallel,
        /// Create instances with a delay between each.
        Staggered {
            /// Delay between successive instance creations.
            interval: Duration,
        },
    }

    /// Pool configuration.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// Minimum number of idle instances to maintain.
        pub min_size: u32,
        /// Maximum number of instances (idle + in-use).
        pub max_size: u32,
        /// How long an idle instance can sit before eviction.
        pub idle_timeout: Option<Duration>,
        /// Maximum total lifetime of an instance.
        pub max_lifetime: Option<Duration>,
        /// Timeout for creating a new instance.
        pub create_timeout: Duration,
        /// Checkout ordering strategy.
        pub strategy: PoolStrategy,
        /// Warmup strategy at pool startup.
        pub warmup: WarmupStrategy,
        /// Whether to run a health check on every checkout.
        pub test_on_checkout: bool,
        /// Interval between background maintenance sweeps.
        pub maintenance_interval: Duration,
        /// Maximum number of concurrent instance creations.
        pub max_concurrent_creates: u32,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                min_size: 1,
                max_size: 10,
                idle_timeout: Some(Duration::from_mins(5)),
                max_lifetime: Some(Duration::from_mins(30)),
                create_timeout: Duration::from_secs(30),
                strategy: PoolStrategy::default(),
                warmup: WarmupStrategy::default(),
                test_on_checkout: false,
                maintenance_interval: Duration::from_secs(30),
                max_concurrent_creates: 3,
            }
        }
    }
}
