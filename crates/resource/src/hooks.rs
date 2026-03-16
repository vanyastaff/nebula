//! Lifecycle hooks for extending resource operations with custom logic.
//!
//! Hooks run before and after lifecycle operations (acquire, release, create,
//! cleanup). Before-hooks can cancel operations; after-hook errors are logged
//! but never propagated.
//!
//! Hooks are executed in priority order (lower number = earlier).

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use smallvec::SmallVec;

use crate::context::Context;
use crate::error::Error;

/// Inline capacity for hook snapshots — covers the common case without a heap allocation.
pub(crate) const HOOKS_INLINE: usize = 4;

// ---------------------------------------------------------------------------
// HookEvent
// ---------------------------------------------------------------------------

/// Events that trigger lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    /// Before/after acquiring a resource instance.
    ///
    /// Before-hooks can cancel the acquisition by returning
    /// [`HookResult::Cancel`].
    Acquire,
    /// Before/after releasing a resource instance.
    ///
    /// **Note:** Release happens inside a `Drop` impl (via
    /// [`ReleaseHookGuard`](crate::manager::ReleaseHookGuard-internal)),
    /// so the before-hook result is **ignored** — a release cannot be
    /// cancelled. Before-hooks are still invoked for observability
    /// (logging, metrics), but returning [`HookResult::Cancel`] has no
    /// effect. If you need cancellable release semantics, use an
    /// explicit release method instead of relying on guard drop.
    Release,
    /// Before/after creating a new resource instance.
    ///
    /// Before-hooks can cancel the creation by returning
    /// [`HookResult::Cancel`], which causes the acquire to fail with
    /// the error from the hook.
    Create,
    /// After `Resource::create()` succeeds and before the first acquire.
    ///
    /// Use for one-time initialisation that must happen on the new instance
    /// but only once (e.g. `SET search_path`, `SET timezone`, registering
    /// the connection in a registry).  Cannot cancel the creation.
    PostCreate,
    /// Before/after cleaning up (permanently destroying) a resource instance.
    ///
    /// **Note:** Cleanup is irrevocable — the before-hook result is
    /// **ignored** and cannot prevent the cleanup from proceeding.
    /// Before-hooks are called for observability only.
    Cleanup,
    /// Before handing an idle instance to the caller.
    ///
    /// Called after [`is_reusable`](crate::resource::Resource::is_reusable)
    /// returns `true` but before `prepare` and the guard is constructed.
    /// Use for conditional pings (`idle_for > threshold`), lazy reconnects,
    /// or metrics recording at the instance level.  Before-hooks can cancel
    /// (i.e. discard the instance and retry) by returning
    /// [`HookResult::Cancel`].
    PreAcquire,
    /// After `Resource::recycle()` succeeds and before the instance is pushed
    /// back to the idle queue.
    ///
    /// Use to flush pending state, reset per-execution fields set by
    /// `prepare`, or record per-release metrics.  Errors are logged but
    /// never propagated.
    PostRecycle,
    /// After the [`Guard`](crate::guard::Guard) has been dropped and the
    /// instance returned to the idle queue (or cleaned up).
    ///
    /// Use for audit logging, release-latency metrics, or any observability
    /// that must happen after the instance is fully released.  Cannot cancel
    /// or affect the release outcome.
    PostRelease,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Acquire => write!(f, "Acquire"),
            Self::Release => write!(f, "Release"),
            Self::Create => write!(f, "Create"),
            Self::PostCreate => write!(f, "PostCreate"),
            Self::Cleanup => write!(f, "Cleanup"),
            Self::PreAcquire => write!(f, "PreAcquire"),
            Self::PostRecycle => write!(f, "PostRecycle"),
            Self::PostRelease => write!(f, "PostRelease"),
        }
    }
}

// ---------------------------------------------------------------------------
// HookFilter
// ---------------------------------------------------------------------------

/// Filter that determines which resources a hook applies to.
#[derive(Debug, Clone)]
pub enum HookFilter {
    /// Hook applies to all resources.
    All,
    /// Hook applies only to the named resource.
    Resource(String),
    /// Hook applies to resources matching a prefix.
    Prefix(String),
}

impl HookFilter {
    /// Check whether this filter matches the given resource ID.
    #[must_use]
    pub fn matches(&self, resource_id: &str) -> bool {
        match self {
            Self::All => true,
            Self::Resource(id) => id == resource_id,
            Self::Prefix(prefix) => resource_id.starts_with(prefix.as_str()),
        }
    }
}

// ---------------------------------------------------------------------------
// HookResult
// ---------------------------------------------------------------------------

/// Result of a before-hook execution.
pub enum HookResult {
    /// Continue with the operation.
    Continue,
    /// Cancel the operation with an error.
    Cancel(Error),
}

// ---------------------------------------------------------------------------
// ResourceHook trait
// ---------------------------------------------------------------------------

/// Trait for resource lifecycle hooks.
///
/// Hooks are called before and after lifecycle operations.
/// Before-hooks can cancel operations by returning [`HookResult::Cancel`].
/// After-hooks cannot cancel but are called for observability.
#[async_trait]
pub trait ResourceHook: Send + Sync {
    /// Human-readable name for this hook.
    fn name(&self) -> &str;

    /// Priority (lower = runs first). Default: 100.
    fn priority(&self) -> u32 {
        100
    }

    /// Which events this hook responds to.
    fn events(&self) -> Vec<HookEvent>;

    /// Which resources this hook applies to.
    fn filter(&self) -> HookFilter {
        HookFilter::All
    }

    /// Called before the operation.
    ///
    /// Returning [`HookResult::Cancel`] cancels the operation for
    /// [`Acquire`](HookEvent::Acquire) and [`Create`](HookEvent::Create).
    /// For [`Release`](HookEvent::Release) and
    /// [`Cleanup`](HookEvent::Cleanup), the result is ignored (the
    /// operation proceeds regardless) because these occur in
    /// irrevocable contexts (e.g. `Drop`).
    async fn before(&self, event: &HookEvent, resource_id: &str, ctx: &Context) -> HookResult;

    /// Called after the operation completes (success or failure).
    async fn after(&self, event: &HookEvent, resource_id: &str, ctx: &Context, success: bool);
}

// ---------------------------------------------------------------------------
// HookRegistry
// ---------------------------------------------------------------------------

/// Registry for managing lifecycle hooks.
///
/// Hooks are stored sorted by priority (lower first). Registration is
/// protected by an `RwLock` so it can happen concurrently with reads.
pub struct HookRegistry {
    hooks: parking_lot::RwLock<Vec<Arc<dyn ResourceHook>>>,
}

impl HookRegistry {
    /// Create a new empty hook registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hooks: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Register a hook. Hooks are sorted by priority (lower first).
    pub fn register(&self, hook: Arc<dyn ResourceHook>) {
        let mut hooks = self.hooks.write();
        hooks.push(hook);
        hooks.sort_by_key(|h| h.priority());
    }

    /// Return a snapshot of all registered hooks (cloned `Arc`s).
    ///
    /// Useful for passing hooks into spawned tasks without holding the
    /// registry lock across `.await` points.
    #[must_use]
    pub fn snapshot(&self) -> SmallVec<[Arc<dyn ResourceHook>; HOOKS_INLINE]> {
        self.hooks.read().iter().cloned().collect()
    }

    /// Run all matching before-hooks in priority order.
    ///
    /// Short-circuits on the first [`HookResult::Cancel`] result.
    pub async fn run_before(
        &self,
        event: &HookEvent,
        resource_id: &str,
        ctx: &Context,
    ) -> crate::error::Result<()> {
        // Snapshot the hooks under the lock, then release before awaiting.
        let hooks: SmallVec<[Arc<dyn ResourceHook>; HOOKS_INLINE]> = {
            let guard = self.hooks.read();
            guard
                .iter()
                .filter(|h| h.events().contains(event) && h.filter().matches(resource_id))
                .cloned()
                .collect()
        };

        for hook in &hooks {
            match hook.before(event, resource_id, ctx).await {
                HookResult::Continue => {}
                HookResult::Cancel(err) => {
                    tracing::warn!(
                        hook = hook.name(),
                        resource_id,
                        event = %event,
                        "Before-hook cancelled operation"
                    );
                    return Err(err);
                }
            }
        }

        Ok(())
    }

    /// Run all matching after-hooks in priority order.
    ///
    /// Errors from individual hooks are logged but never propagated.
    pub async fn run_after(
        &self,
        event: &HookEvent,
        resource_id: &str,
        ctx: &Context,
        success: bool,
    ) {
        // Snapshot the hooks under the lock, then release before awaiting.
        let hooks: SmallVec<[Arc<dyn ResourceHook>; HOOKS_INLINE]> = {
            let guard = self.hooks.read();
            guard
                .iter()
                .filter(|h| h.events().contains(event) && h.filter().matches(resource_id))
                .cloned()
                .collect()
        };

        for hook in &hooks {
            hook.after(event, resource_id, ctx, success).await;
        }
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.hooks.read().len();
        f.debug_struct("HookRegistry")
            .field("hook_count", &count)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Built-in hooks
// ---------------------------------------------------------------------------

/// Audit hook that logs all lifecycle events via `tracing::info!`.
///
/// Priority 10 (runs early). Listens to all events, applies to all resources.
pub struct AuditHook;

#[async_trait]
impl ResourceHook for AuditHook {
    fn name(&self) -> &str {
        "audit"
    }

    fn priority(&self) -> u32 {
        10
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![
            HookEvent::Acquire,
            HookEvent::Release,
            HookEvent::Create,
            HookEvent::Cleanup,
        ]
    }

    async fn before(&self, event: &HookEvent, resource_id: &str, _ctx: &Context) -> HookResult {
        tracing::info!(
            hook = "audit",
            resource_id,
            event = %event,
            phase = "before",
            "Lifecycle hook"
        );
        let _ = (event, resource_id);
        HookResult::Continue
    }

    async fn after(&self, event: &HookEvent, resource_id: &str, _ctx: &Context, success: bool) {
        tracing::info!(
            hook = "audit",
            resource_id,
            event = %event,
            phase = "after",
            success,
            "Lifecycle hook"
        );
        let _ = (event, resource_id, success);
    }
}

// ---------------------------------------------------------------------------

/// Hook that logs a warning when resource acquisition takes longer than a
/// configurable threshold.
///
/// Priority 90 (runs late, after most other hooks). Listens only to
/// [`HookEvent::Acquire`].
pub struct SlowAcquireHook {
    threshold: std::time::Duration,
    /// Stores the acquire start time so `after` can measure elapsed.
    /// Uses a sharded map keyed by (resource_id, execution_id).
    timers: parking_lot::Mutex<std::collections::HashMap<String, Instant>>,
}

impl SlowAcquireHook {
    /// Create a new slow-acquire hook with the given duration threshold.
    #[must_use]
    pub fn new(threshold: std::time::Duration) -> Self {
        Self {
            threshold,
            timers: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Build a map key from resource ID and execution ID.
    fn timer_key(resource_id: &str, ctx: &Context) -> String {
        format!("{}:{}", resource_id, ctx.execution_id)
    }
}

#[async_trait]
impl ResourceHook for SlowAcquireHook {
    fn name(&self) -> &str {
        "slow-acquire"
    }

    fn priority(&self) -> u32 {
        90
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Acquire]
    }

    async fn before(&self, _event: &HookEvent, resource_id: &str, ctx: &Context) -> HookResult {
        let key = Self::timer_key(resource_id, ctx);
        self.timers.lock().insert(key, Instant::now());
        HookResult::Continue
    }

    async fn after(&self, _event: &HookEvent, resource_id: &str, ctx: &Context, _success: bool) {
        let key = Self::timer_key(resource_id, ctx);
        if let Some(start) = self.timers.lock().remove(&key) {
            let elapsed = start.elapsed();
            if elapsed > self.threshold {
                tracing::warn!(
                    hook = "slow-acquire",
                    resource_id,
                    elapsed_ms = elapsed.as_millis() as u64,
                    threshold_ms = self.threshold.as_millis() as u64,
                    "Resource acquisition exceeded threshold"
                );
                let _ = (resource_id, elapsed);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_filter_all_matches_everything() {
        let filter = HookFilter::All;
        assert!(filter.matches("postgres"));
        assert!(filter.matches("redis"));
        assert!(filter.matches(""));
    }

    #[test]
    fn hook_filter_resource_matches_exact() {
        let filter = HookFilter::Resource("postgres".to_string());
        assert!(filter.matches("postgres"));
        assert!(!filter.matches("redis"));
        assert!(!filter.matches("postgres-replica"));
    }

    #[test]
    fn hook_filter_prefix_matches_prefix() {
        let filter = HookFilter::Prefix("db-".to_string());
        assert!(filter.matches("db-postgres"));
        assert!(filter.matches("db-redis"));
        assert!(!filter.matches("cache-redis"));
    }

    // -----------------------------------------------------------------------
    // Shared test hook structs (extracted to module level to avoid nesting)
    // -----------------------------------------------------------------------

    struct PriorityHook {
        name: String,
        prio: u32,
    }

    #[async_trait]
    impl ResourceHook for PriorityHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> u32 {
            self.prio
        }
        fn events(&self) -> Vec<HookEvent> {
            vec![HookEvent::Acquire]
        }
        async fn before(
            &self,
            _event: &HookEvent,
            _resource_id: &str,
            _ctx: &Context,
        ) -> HookResult {
            HookResult::Continue
        }
        async fn after(
            &self,
            _event: &HookEvent,
            _resource_id: &str,
            _ctx: &Context,
            _success: bool,
        ) {
        }
    }

    struct CancelHook;

    #[async_trait]
    impl ResourceHook for CancelHook {
        fn name(&self) -> &str {
            "canceller"
        }
        fn priority(&self) -> u32 {
            10
        }
        fn events(&self) -> Vec<HookEvent> {
            vec![HookEvent::Acquire]
        }
        async fn before(
            &self,
            _event: &HookEvent,
            _resource_id: &str,
            _ctx: &Context,
        ) -> HookResult {
            HookResult::Cancel(Error::Unavailable {
                resource_key: nebula_core::resource_key!("test"),
                reason: "cancelled by hook".to_string(),
                retryable: false,
            })
        }
        async fn after(
            &self,
            _event: &HookEvent,
            _resource_id: &str,
            _ctx: &Context,
            _success: bool,
        ) {
        }
    }

    struct TrackingHook {
        called: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl ResourceHook for TrackingHook {
        fn name(&self) -> &str {
            "tracker"
        }
        fn priority(&self) -> u32 {
            20
        }
        fn events(&self) -> Vec<HookEvent> {
            vec![HookEvent::Acquire]
        }
        async fn before(
            &self,
            _event: &HookEvent,
            _resource_id: &str,
            _ctx: &Context,
        ) -> HookResult {
            self.called.store(true, std::sync::atomic::Ordering::SeqCst);
            HookResult::Continue
        }
        async fn after(
            &self,
            _event: &HookEvent,
            _resource_id: &str,
            _ctx: &Context,
            _success: bool,
        ) {
        }
    }

    #[test]
    fn hook_registry_sorts_by_priority() {
        let registry = HookRegistry::new();
        registry.register(Arc::new(PriorityHook {
            name: "high".into(),
            prio: 200,
        }));
        registry.register(Arc::new(PriorityHook {
            name: "low".into(),
            prio: 10,
        }));
        registry.register(Arc::new(PriorityHook {
            name: "mid".into(),
            prio: 100,
        }));

        let hooks = registry.hooks.read();
        assert_eq!(hooks[0].name(), "low");
        assert_eq!(hooks[1].name(), "mid");
        assert_eq!(hooks[2].name(), "high");
    }

    #[tokio::test]
    async fn run_before_short_circuits_on_cancel() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let registry = HookRegistry::new();
        registry.register(Arc::new(CancelHook));
        let tracker = Arc::new(TrackingHook {
            called: AtomicBool::new(false),
        });
        registry.register(Arc::clone(&tracker) as Arc<dyn ResourceHook>);

        let ctx = Context::new(
            crate::scope::Scope::Global,
            nebula_core::WorkflowId::nil(),
            nebula_core::ExecutionId::nil(),
        );
        let result = registry.run_before(&HookEvent::Acquire, "test", &ctx).await;

        assert!(result.is_err());
        assert!(
            !tracker.called.load(Ordering::SeqCst),
            "hook after cancelling hook should not be called"
        );
    }

    #[test]
    fn audit_hook_has_correct_properties() {
        let hook = AuditHook;
        assert_eq!(hook.name(), "audit");
        assert_eq!(hook.priority(), 10);
        assert_eq!(hook.events().len(), 4);
    }

    #[test]
    fn slow_acquire_hook_has_correct_properties() {
        let hook = SlowAcquireHook::new(std::time::Duration::from_secs(1));
        assert_eq!(hook.name(), "slow-acquire");
        assert_eq!(hook.priority(), 90);
        assert_eq!(hook.events(), vec![HookEvent::Acquire]);
    }

    #[test]
    fn hook_registry_debug_shows_count() {
        let registry = HookRegistry::new();
        let debug = format!("{registry:?}");
        assert!(debug.contains("hook_count: 0"));
    }
}
