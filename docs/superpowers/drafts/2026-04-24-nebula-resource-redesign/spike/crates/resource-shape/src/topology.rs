//! Spike topology sub-traits — minimum surface for the 5 retained shapes.
//!
//! Per ADR-0037, Daemon and EventSource are out of scope (extracted to
//! engine fold). Spike only validates the topologies that remain on
//! `nebula-resource`: `Pooled`, `Resident`, `Service`, `Transport`,
//! `Exclusive`.
//!
//! Each topology compiles ON TOP of `Resource` and inherits its
//! `type Credential`. The point of the spike is to confirm that a
//! topology-bound resource (e.g. `MockPostgresPool: Pooled` with
//! `type Credential = SecretToken`) does NOT need any extra dance to
//! plug into the §3.6 hook chain — `Manager` dispatches via the
//! `Resource` parent surface.
//!
//! The associated methods on each sub-trait below are pruned to the
//! minimum needed to demonstrate "this topology composes". Phase 6
//! Tech Spec §4 elaborates the full set.

use std::future::Future;

use crate::resource::{Resource, ResourceContext};

// ── Pooled ─────────────────────────────────────────────────────────────

/// Pool topology — N interchangeable instances with checkout/recycle.
pub trait Pooled: Resource {
    /// Async recycle check. Default keeps the instance.
    fn recycle(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        async { Ok(true) }
    }
}

// ── Resident ───────────────────────────────────────────────────────────

/// Resident topology — one shared instance, clone on acquire.
pub trait Resident: Resource
where
    Self::Lease: Clone,
{
    /// Sync O(1) liveness check. Default = always alive.
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool {
        true
    }
}

// ── Service ────────────────────────────────────────────────────────────

/// Service topology — long-lived runtime, short-lived tokens.
pub trait Service: Resource {
    /// Acquire a token from the running service.
    fn acquire_token(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
}

// ── Transport ──────────────────────────────────────────────────────────

/// Transport topology — shared connection, multiplexed sessions.
pub trait Transport: Resource {
    /// Open a new session on the transport.
    fn open_session(
        &self,
        transport: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
}

// ── Exclusive ──────────────────────────────────────────────────────────

/// Exclusive topology — one caller at a time via semaphore.
pub trait Exclusive: Resource {
    /// Reset state after each exclusive use. Default no-op.
    fn reset(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}
