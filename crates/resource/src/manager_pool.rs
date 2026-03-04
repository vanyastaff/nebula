//! Type-erased pool adapters used internally by the [`Manager`](crate::manager::Manager).
//!
//! These types bridge the generic `Pool<R>` to a `dyn AnyPool` trait object
//! so the manager can store pools of different resource types in a single map.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use nebula_core::CredentialKey;
use nebula_credential::traits::RotationStrategy;

use crate::context::Context;
use crate::error::Result;
use crate::guard::Guard;
use crate::manager_guard::{AnyGuard, AnyGuardTrait};
use crate::pool::Pool;
use crate::resource::Resource;
use crate::scope::Scope;

// ---------------------------------------------------------------------------
// Type-erased guard (typed inner)
// ---------------------------------------------------------------------------

/// Concrete guard wrapping a typed `Guard`.
pub(crate) struct TypedGuard<R: Resource> {
    pub(crate) guard: Guard<R::Instance>,
}

impl<R: Resource> AnyGuardTrait for TypedGuard<R>
where
    R::Instance: Any,
{
    fn as_any(&self) -> &dyn Any {
        &*self.guard
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut *self.guard
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

/// Pool that can react to credential rotation (has a credential handler).
pub(crate) trait RotatablePool: Send + Sync {
    /// Apply credential rotation to the pool.
    fn handle_rotation(
        &self,
        new_state: &serde_json::Value,
        strategy: RotationStrategy,
        credential_key: CredentialKey,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
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
                Box::new(TypedGuard::<R> { guard }) as AnyGuard,
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

impl<R: Resource> RotatablePool for TypedPool<R>
where
    R::Instance: Any,
{
    fn handle_rotation(
        &self,
        new_state: &serde_json::Value,
        strategy: RotationStrategy,
        credential_key: CredentialKey,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let pool = self.pool.clone();
        let new_state = new_state.clone();
        Box::pin(async move {
            pool.handle_rotation(&new_state, strategy, credential_key)
                .await
        })
    }
}

// ---------------------------------------------------------------------------
// PoolEntry
// ---------------------------------------------------------------------------

/// A pool together with the scope it was registered under.
pub(crate) struct PoolEntry {
    pub(crate) pool: Arc<dyn AnyPool>,
    pub(crate) scope: Scope,
    /// Typed handle for `get_pool::<R>()` downcast; same Arc as pool.
    pub(crate) typed_handle: Option<Arc<dyn std::any::Any + Send + Sync>>,
}
