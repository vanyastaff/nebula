//! Type-erased acquire dispatch stored at registration time.
//!
//! [`Manager::register_with_identity`] captures a topology-specific
//! `acquire_*_for` closure so callers that only know a [`ResourceKey`]
//! (engine / action accessor) can still run the full lease pipeline.

use std::{any::Any, sync::Arc};

use crate::registry::ErasedAcquireFn;

#[cfg(test)]
use crate::error::Error;

/// No-op acquire used by registry unit tests that only exercise lookup.
#[cfg(test)]
#[must_use]
pub(crate) fn noop_erased_acquire() -> ErasedAcquireFn {
    Arc::new(|_mgr, _ctx, _opts, _scope| {
        Box::pin(async {
            Err(Error::permanent(
                "acquire dispatch not configured for this registry test entry",
            ))
        })
    })
}

fn arc_acquire_resident<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::resident::Resident + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Clone + Send + 'static,
{
    Arc::new(move |mgr, ctx, opts, matched_scope| {
        Box::pin(async move {
            let guard = mgr
                .acquire_resident_at_scope::<R>(&ctx, &opts, slot_identity, matched_scope)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

fn arc_acquire_pooled<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Into<R::Runtime> + Send + 'static,
{
    Arc::new(move |mgr, ctx, opts, matched_scope| {
        Box::pin(async move {
            let guard = mgr
                .acquire_pooled_at_scope::<R>(&ctx, &opts, slot_identity, matched_scope)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

fn arc_acquire_service<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::service::Service + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    Arc::new(move |mgr, ctx, opts, matched_scope| {
        Box::pin(async move {
            let guard = mgr
                .acquire_service_at_scope::<R>(&ctx, &opts, slot_identity, matched_scope)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

fn arc_acquire_transport<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    Arc::new(move |mgr, ctx, opts, matched_scope| {
        Box::pin(async move {
            let guard = mgr
                .acquire_transport_at_scope::<R>(&ctx, &opts, slot_identity, matched_scope)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

fn arc_acquire_exclusive<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    Arc::new(move |mgr, ctx, opts, matched_scope| {
        Box::pin(async move {
            let guard = mgr
                .acquire_exclusive_at_scope::<R>(&ctx, &opts, slot_identity, matched_scope)
                .await?;
            Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
        })
    })
}

/// Erased acquire hook for a resident topology row.
pub(crate) fn erased_acquire_resident<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::resident::Resident + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Clone + Send + 'static,
{
    arc_acquire_resident::<R>(slot_identity)
}

/// Erased acquire hook for a pooled topology row.
pub(crate) fn erased_acquire_pooled<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Into<R::Runtime> + Send + 'static,
{
    arc_acquire_pooled::<R>(slot_identity)
}

/// Erased acquire hook for a service topology row.
pub(crate) fn erased_acquire_service<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::service::Service + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    arc_acquire_service::<R>(slot_identity)
}

/// Erased acquire hook for a transport topology row.
pub(crate) fn erased_acquire_transport<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    arc_acquire_transport::<R>(slot_identity)
}

/// Erased acquire hook for an exclusive topology row.
pub(crate) fn erased_acquire_exclusive<R>(slot_identity: u64) -> ErasedAcquireFn
where
    R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    arc_acquire_exclusive::<R>(slot_identity)
}
