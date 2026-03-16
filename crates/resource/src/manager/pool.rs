//! Type-erased pool adapters used internally by the [`Manager`](crate::manager::Manager).
//!
//! These types bridge the generic `Pool<R>` to a `dyn AnyPool` trait object
//! so the manager can store pools of different resource types in a single map.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use nebula_core::ResourceKey;

use crate::context::Context;
use crate::error::Result;
use crate::guard::Guard;
use super::guard::{AnyGuard, AnyGuardTrait};
use crate::pool::Pool;
use crate::resource::Resource;
use crate::scope::Scope;

// ---------------------------------------------------------------------------
// Type-erased guard (typed inner)
// ---------------------------------------------------------------------------

/// Concrete guard wrapping a typed `Guard`.
pub(crate) struct TypedGuard<R: Resource, F: FnOnce(R::Instance, bool) + Send + 'static> {
    pub(crate) guard: Guard<R::Instance, F>,
}

impl<R: Resource, F: FnOnce(R::Instance, bool) + Send + 'static> AnyGuardTrait for TypedGuard<R, F>
where
    R::Instance: Any,
{
    fn as_any(&self) -> &dyn Any {
        &*self.guard
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut *self.guard
    }

    fn taint(&mut self) {
        self.guard.taint();
    }

    fn is_tainted(&self) -> bool {
        self.guard.is_tainted()
    }
}

// ---------------------------------------------------------------------------
// Type-erased pool wrapper
// ---------------------------------------------------------------------------

/// Type-erased pool interface so the manager can store pools of different
/// resource types in a single map.
#[allow(clippy::type_complexity)]
pub(crate) trait AnyPool: Send + Sync {
    /// Acquire a type-erased instance. Returns the guard and the wait duration.
    fn acquire_any<'a>(
        &'a self,
        ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = Result<(AnyGuard, Duration)>> + Send + 'a>>;

    /// Shut down the pool.
    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Current pool utilization: (active, idle, max_size).
    fn utilization_snapshot(&self) -> (usize, usize, usize);

    /// Pre-create up to `count` idle instances, respecting `max_size`.
    fn scale_up(&self, count: usize) -> Pin<Box<dyn Future<Output = usize> + Send + '_>>;

    /// Remove up to `count` idle instances, respecting `min_size`.
    fn scale_down(&self, count: usize) -> Pin<Box<dyn Future<Output = usize> + Send + '_>>;
}

/// Concrete adapter from `Pool<R>` to `AnyPool`.
///
/// Returned by [`Manager::get_pool`](crate::manager::Manager::get_pool) for typed pool access.
pub struct TypedPool<R: Resource> {
    /// The underlying pool.
    pub pool: Pool<R>,
}

impl<R: Resource> AnyPool for TypedPool<R>
where
    R::Instance: Any,
{
    fn acquire_any<'a>(
        &'a self,
        ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = Result<(AnyGuard, Duration)>> + Send + 'a>> {
        Box::pin(async move {
            let (guard, wait_duration) = self.pool.acquire(ctx).await?;
            Ok((
                Box::new(TypedGuard::<R, _> { guard }) as AnyGuard,
                wait_duration,
            ))
        })
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { self.pool.shutdown().await })
    }

    fn utilization_snapshot(&self) -> (usize, usize, usize) {
        self.pool.utilization_snapshot()
    }

    fn scale_up(&self, count: usize) -> Pin<Box<dyn Future<Output = usize> + Send + '_>> {
        Box::pin(async move { self.pool.scale_up(count).await })
    }

    fn scale_down(&self, count: usize) -> Pin<Box<dyn Future<Output = usize> + Send + '_>> {
        Box::pin(async move { self.pool.scale_down(count).await })
    }
}

// ---------------------------------------------------------------------------
// PoolEntry
// ---------------------------------------------------------------------------

/// A pool together with the scope it was registered under.
#[derive(Clone)]
pub(crate) struct PoolEntry {
    pub(crate) pool: Arc<dyn AnyPool>,
    pub(crate) scope: Scope,
    /// Canonical resource key (scope-independent). Used by `find_most_specific`
    /// to locate the most scope-compatible entry for a given dependency key.
    pub(crate) resource_key: ResourceKey,
    /// Typed handle for `get_pool::<R>()` downcast; same Arc as pool.
    pub(crate) typed_handle: Option<Arc<dyn std::any::Any + Send + Sync>>,
}
