//! Lifecycle hooks for extending resource operations with custom logic.
//!
//! Hooks run before and after lifecycle operations (acquire, release, create,
//! cleanup). Before-hooks can cancel operations; after-hook errors are logged
//! but never propagated.
//!
//! Hooks are executed in priority order (lower number = earlier).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use crate::context::Context;
use crate::error::Error;

// ---------------------------------------------------------------------------
// HookEvent
// ---------------------------------------------------------------------------

/// Events that trigger lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    /// Before/after acquiring a resource instance.
    Acquire,
    /// Before/after releasing a resource instance.
    Release,
    /// Before/after creating a new resource instance.
    Create,
    /// Before/after cleaning up a resource instance.
    Cleanup,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Acquire => write!(f, "Acquire"),
            Self::Release => write!(f, "Release"),
            Self::Create => write!(f, "Create"),
            Self::Cleanup => write!(f, "Cleanup"),
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

    /// Called before the operation. Can cancel by returning [`HookResult::Cancel`].
    fn before<'a>(
        &'a self,
        event: &'a HookEvent,
        resource_id: &'a str,
        ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>>;

    /// Called after the operation completes (success or failure).
    fn after<'a>(
        &'a self,
        event: &'a HookEvent,
        resource_id: &'a str,
        ctx: &'a Context,
        success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
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
        let hooks: Vec<Arc<dyn ResourceHook>> = {
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
                    #[cfg(feature = "tracing")]
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
        let hooks: Vec<Arc<dyn ResourceHook>> = {
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

    fn before<'a>(
        &'a self,
        event: &'a HookEvent,
        resource_id: &'a str,
        _ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(feature = "tracing")]
            tracing::info!(
                hook = "audit",
                resource_id,
                event = %event,
                phase = "before",
                "Lifecycle hook"
            );
            let _ = (event, resource_id);
            HookResult::Continue
        })
    }

    fn after<'a>(
        &'a self,
        event: &'a HookEvent,
        resource_id: &'a str,
        _ctx: &'a Context,
        success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(feature = "tracing")]
            tracing::info!(
                hook = "audit",
                resource_id,
                event = %event,
                phase = "after",
                success,
                "Lifecycle hook"
            );
            let _ = (event, resource_id, success);
        })
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

    fn before<'a>(
        &'a self,
        _event: &'a HookEvent,
        resource_id: &'a str,
        ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async move {
            let key = Self::timer_key(resource_id, ctx);
            self.timers.lock().insert(key, Instant::now());
            HookResult::Continue
        })
    }

    fn after<'a>(
        &'a self,
        _event: &'a HookEvent,
        resource_id: &'a str,
        ctx: &'a Context,
        _success: bool,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let key = Self::timer_key(resource_id, ctx);
            if let Some(start) = self.timers.lock().remove(&key) {
                let elapsed = start.elapsed();
                if elapsed > self.threshold {
                    #[cfg(feature = "tracing")]
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
        })
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
        fn before<'a>(
            &'a self,
            _event: &'a HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
        ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
            Box::pin(async { HookResult::Continue })
        }
        fn after<'a>(
            &'a self,
            _event: &'a HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
            _success: bool,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    struct CancelHook;

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
        fn before<'a>(
            &'a self,
            _event: &'a HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
        ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
            Box::pin(async {
                HookResult::Cancel(Error::Unavailable {
                    resource_id: "test".to_string(),
                    reason: "cancelled by hook".to_string(),
                    retryable: false,
                })
            })
        }
        fn after<'a>(
            &'a self,
            _event: &'a HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
            _success: bool,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    struct TrackingHook {
        called: std::sync::atomic::AtomicBool,
    }

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
        fn before<'a>(
            &'a self,
            _event: &'a HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
        ) -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
            Box::pin(async {
                self.called
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                HookResult::Continue
            })
        }
        fn after<'a>(
            &'a self,
            _event: &'a HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
            _success: bool,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
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

        let ctx = Context::new(crate::scope::Scope::Global, "wf", "ex");
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
