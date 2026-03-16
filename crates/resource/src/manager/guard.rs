//! Type-erased guard types for resource management.
//!
//! This module contains the guard abstractions that allow the [`Manager`](crate::manager::Manager)
//! to store and return resources of different concrete types through a unified interface.

use std::any::Any;
use std::sync::Arc;

use smallvec::SmallVec;

use nebula_core::ResourceKey;

use crate::context::Context;
use crate::events::EventBus;
use crate::hooks::{HookEvent, HOOKS_INLINE};

// ---------------------------------------------------------------------------
// Type-erased guard
// ---------------------------------------------------------------------------

/// Trait for type-erased resource guards.
///
/// Provides `&dyn Any` access to the inner instance while the concrete
/// `TypedGuard<R>` holds the real `Guard` that returns the instance
/// to the pool on drop.
pub trait AnyGuardTrait: Send {
    /// Access the inner instance as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Access the inner instance as `&mut dyn Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Mark the wrapped instance as tainted so pool return skips recycle.
    fn taint(&mut self);

    /// Returns true when the wrapped instance is tainted.
    fn is_tainted(&self) -> bool;
}

/// Type-erased guard returned by [`Manager::acquire`](crate::manager::Manager::acquire).
///
/// When dropped, the underlying `Guard` returns the instance
/// to the pool. Use [`as_any`](AnyGuardTrait::as_any) and
/// `downcast_ref` to access the concrete instance.
pub type AnyGuard = Box<dyn AnyGuardTrait>;

impl std::fmt::Debug for dyn AnyGuardTrait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyGuard").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// ResourceHandle
// ---------------------------------------------------------------------------

/// Opaque handle wrapping a type-erased resource guard.
///
/// Returned by the engine's `ResourceProvider` adapter (via
/// `ActionContext::resource()`). When dropped, the underlying guard
/// returns the instance to its pool.
pub struct ResourceHandle {
    guard: AnyGuard,
}

impl ResourceHandle {
    /// Wrap an [`AnyGuard`] in a handle.
    pub fn new(guard: AnyGuard) -> Self {
        Self { guard }
    }

    /// Access the inner resource instance by type.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.guard.as_any().downcast_ref()
    }

    /// Mutably access the inner resource instance by type.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.guard.as_any_mut().downcast_mut()
    }

    /// Mark this resource handle as tainted.
    pub fn taint(&mut self) {
        self.guard.taint();
    }

    /// Returns true when this resource was marked tainted.
    #[must_use]
    pub fn is_tainted(&self) -> bool {
        self.guard.is_tainted()
    }
}

// ---------------------------------------------------------------------------
// TypedResourceGuard
// ---------------------------------------------------------------------------

/// Type-safe guard returned by [`Manager::acquire_typed`](crate::manager::Manager::acquire_typed).
///
/// Holds the same underlying pool guard as [`AnyGuard`], but exposes the
/// instance as a concrete type so you avoid manual downcasting.
///
/// Provides fallible typed access via [`get`](Self::get) / [`get_mut`](Self::get_mut)
/// without panicking on type mismatch.
///
/// # Example
///
/// ```rust,ignore
/// let guard = manager.acquire_typed::<TelegramBotResource>(&ctx).await?;
/// let bot = guard.get().expect("typed guard must match resource type");
/// ```
pub struct TypedResourceGuard<T: Send + Sync + 'static> {
    pub(crate) guard: AnyGuard,
    pub(crate) _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> TypedResourceGuard<T> {
    /// Access the resource instance. The type is guaranteed by [`Manager::acquire_typed`](crate::manager::Manager::acquire_typed).
    #[must_use]
    pub fn get(&self) -> Option<&T> {
        self.guard.as_any().downcast_ref()
    }

    /// Mutably access the resource instance.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.guard.as_any_mut().downcast_mut()
    }

    /// Mark this typed resource guard as tainted.
    pub fn taint(&mut self) {
        self.guard.taint();
    }

    /// Returns true when this typed resource was marked tainted.
    #[must_use]
    pub fn is_tainted(&self) -> bool {
        self.guard.is_tainted()
    }
}

impl<T: Send + Sync + 'static> std::fmt::Debug for TypedResourceGuard<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedResourceGuard")
            .field("type", &std::any::type_name::<T>())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// ReleaseHookGuard — fires Release hooks on drop
// ---------------------------------------------------------------------------

/// Wrapper around an [`AnyGuard`] that fires Release hooks when dropped.
///
/// **Semantics of `before(Release)`:** Because resource release happens
/// inside a `Drop` impl, the before-hook's [`HookResult`](crate::hooks::HookResult) is ignored —
/// a release **cannot** be cancelled. Before-hooks are still called for
/// observability (logging, metrics) but returning
/// [`HookResult::Cancel`](crate::hooks::HookResult::Cancel) has no
/// effect. If you need cancellable release semantics, use an explicit
/// `release()` method instead of relying on guard drop.
pub(crate) struct ReleaseHookGuard {
    pub(crate) inner: Option<AnyGuard>,
    pub(crate) resource_id: ResourceKey,
    pub(crate) hooks: SmallVec<[Arc<dyn crate::hooks::ResourceHook>; HOOKS_INLINE]>,
    pub(crate) event_bus: Arc<EventBus>,
    pub(crate) ctx: Context,
}

impl AnyGuardTrait for ReleaseHookGuard {
    fn as_any(&self) -> &dyn Any {
        self.inner.as_ref().expect("guard used after drop").as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.inner
            .as_mut()
            .expect("guard used after drop")
            .as_any_mut()
    }

    fn taint(&mut self) {
        if let Some(inner) = self.inner.as_mut() {
            inner.taint();
        }
    }

    fn is_tainted(&self) -> bool {
        self.inner.as_ref().is_some_and(|inner| inner.is_tainted())
    }
}

impl Drop for ReleaseHookGuard {
    fn drop(&mut self) {
        let inner = self.inner.take();
        if inner.is_none() {
            return;
        }

        let resource_id = self.resource_id.clone();
        let hooks = self.hooks.clone();
        let event_bus = Arc::clone(&self.event_bus);
        let ctx = self.ctx.clone();

        // Fire Release hooks in a spawned task since Drop is sync.
        tokio::spawn(async move {
            // Run before-hooks for Release (result ignored — can't cancel a drop).
            let id = resource_id.as_ref();
            for hook in &hooks {
                if hook.events().contains(&HookEvent::Release) && hook.filter().matches(id) {
                    let _ = hook.before(&HookEvent::Release, id, &ctx).await;
                }
            }

            // Drop the inner guard (returns instance to pool).
            drop(inner);

            // Run after-hooks for Release.
            for hook in &hooks {
                if hook.events().contains(&HookEvent::Release) && hook.filter().matches(id) {
                    hook.after(&HookEvent::Release, id, &ctx, true).await;
                }
            }

            let _ = &event_bus;
        });
    }
}
