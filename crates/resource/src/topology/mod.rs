//! Topology traits for resource management.
//!
//! Each topology describes a different access pattern for resources:
//!
//! | Topology | Pattern |
//! |----------|---------|
//! | [`Pooled`] | N interchangeable instances with checkout/recycle |
//! | [`Resident`] | One shared instance, clone on acquire |
//! | [`Bounded`] | One runtime, capped short-lived leases — the [`Cap`](bounded::CapMarker) typestate (`Unbounded` / `Capped<N>` / `Exclusive`) selects the concurrency bound and release shape |
//!
//! [`Bounded`] is the single parameterized capped-lease topology: its cap
//! typestate covers the long-lived-runtime, multiplexed-session, and
//! one-caller-at-a-time access patterns behind one trait + one runtime.
//!
//! `Daemon` and `EventSource` live in `nebula_engine::daemon` per engine daemon topology —
//! integration model boundary reserves "Resource" for pool/SDK clients.

pub mod bounded;
pub mod pooled;
pub mod resident;

pub use bounded::{
    Bounded, BoundedRelease, CapMarker, Capped, Exclusive as ExclusiveCap, Unbounded,
};
pub use pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};
pub use resident::Resident;
