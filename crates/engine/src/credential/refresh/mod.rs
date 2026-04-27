//! Two-tier refresh coordinator (L1 in-process + L2 cross-replica claim).
//!
//! Per ADR-0041 + sub-spec
//! `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` §3.
//!
//! `RefreshCoordinator` is the public outer surface. Stage 2.1 wires the
//! type as a delegating wrapper around the renamed-to-private
//! `L1RefreshCoalescer` so the rename is atomic and existing callers
//! (`CredentialResolver`) keep compiling. Stage 2.2 replaces the wrapper
//! with the real two-tier acquisition (`refresh_coalesced(...)` acquires
//! L1 mutex first, then a durable L2 claim via `RefreshClaimRepo`).
//!
//! # Layering
//!
//! - **L1 (in-process):** `L1RefreshCoalescer` (`l1.rs`, private). Coalesces concurrent refreshes
//!   inside the same replica process via per-credential `oneshot` waiters and a global concurrency
//!   semaphore. Includes a per-credential `nebula_resilience::CircuitBreaker`.
//! - **L2 (cross-replica):** `RefreshClaimRepo` (in `nebula-storage`). CAS-based claim with TTL +
//!   heartbeat. Lands in Stage 2.2.

mod audit;
mod coordinator;
mod l1;
mod metrics;
mod reclaim;
mod sentinel;

#[cfg(test)]
mod test_fixtures;

pub use coordinator::{
    ConfigError, RefreshAttempt, RefreshConfigError, RefreshCoordConfig, RefreshCoordinator,
    RefreshError,
};
pub use metrics::RefreshCoordMetrics;
pub use reclaim::ReclaimSweepHandle;
pub use sentinel::{SentinelDecision, SentinelThresholdConfig, SentinelTrigger};
