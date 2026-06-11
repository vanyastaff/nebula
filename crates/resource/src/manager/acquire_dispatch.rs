//! Type-erased acquire dispatch stored at registration time.
//!
//! [`Manager::register`] captures a topology-specific
//! `acquire_*_at_scope` closure so callers that only know a [`ResourceKey`]
//! (engine / action accessor) can still run the full lease pipeline.
//!
//! The hook receives the `Arc<dyn AnyManagedResource>` already resolved by
//! [`Registry::get_acquire_for`](crate::registry::Registry::get_acquire_for)'s
//! single scope walk and downcasts it — it does **not** perform a second
//! `DashMap` walk at the matched scope. The registry resolves the row
//! exactly once per erased acquire.

use std::{any::Any, sync::Arc};

use crate::registry::ErasedAcquireFn;

#[cfg(test)]
use crate::error::Error;

/// No-op acquire used by registry unit tests that only exercise lookup.
#[cfg(test)]
#[must_use]
pub(crate) fn noop_erased_acquire() -> ErasedAcquireFn {
    Arc::new(|_mgr, _ctx, _opts, _resolved| {
        Box::pin(async {
            Err(Error::permanent(
                "acquire dispatch not configured for this registry test entry",
            ))
        })
    })
}

fn arc_acquire_resident<R>() -> ErasedAcquireFn
where
    R: crate::topology::resident::Resident
        + crate::resource::HasCredentialSlots
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    Arc::new(move |mgr, ctx, opts, resolved| {
        Box::pin(async move {
            let guard = mgr
                .acquire_resident_at_scope::<R>(&ctx, &opts, resolved)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

fn arc_acquire_pooled<R>() -> ErasedAcquireFn
where
    R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    Arc::new(move |mgr, ctx, opts, resolved| {
        Box::pin(async move {
            let guard = mgr
                .acquire_pooled_at_scope::<R>(&ctx, &opts, resolved)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

/// Erased acquire hook for a resident topology row.
///
/// The hook downcasts the row resolved by the single
/// [`get_acquire_for`](crate::registry::Registry::get_acquire_for) scope
/// walk — there is no second registry walk, so the row is no longer
/// re-pinned by a captured slot identity (walk-1 already matched it).
pub(crate) fn erased_acquire_resident<R>() -> ErasedAcquireFn
where
    R: crate::topology::resident::Resident
        + crate::resource::HasCredentialSlots
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    arc_acquire_resident::<R>()
}

/// Erased acquire hook for a pooled topology row.
///
/// See [`erased_acquire_resident`] — downcasts the single-walk-resolved
/// row, no second registry walk.
pub(crate) fn erased_acquire_pooled<R>() -> ErasedAcquireFn
where
    R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    arc_acquire_pooled::<R>()
}
