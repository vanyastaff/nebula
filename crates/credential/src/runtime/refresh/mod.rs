//! Two-tier refresh coordinator (L1 in-process + L2 cross-replica claim).
//!
//! See `docs/INTEGRATION_MODEL.md` (credential refresh) for integration context.
//!
//! `RefreshCoordinator` is the curated public outer surface:
//! `refresh_coalesced(...)` acquires L1 first, then a durable L2 claim via
//! `RefreshClaimStore`. The process-local coalescer and claim authority stay
//! private to this module.
//!
//! # Layering
//!
//! - **L1 (in-process):** `L1RefreshCoalescer` (`l1.rs`, private). Coalesces concurrent refreshes
//!   inside the same replica process via per-credential `oneshot` waiters and a global concurrency
//!   semaphore. Includes a per-credential `nebula_resilience::CircuitBreaker`.
//! - **L2 (cross-replica):** `RefreshClaimStore` (in `nebula-storage-port`). CAS-based claim with
//!   TTL + heartbeat.

mod audit;
mod coordinator;
mod l1;
mod metrics;
mod reclaim;
mod sentinel;
pub mod token_refresh;
pub mod transport;

pub use coordinator::{
    ConfigError, RefreshCoordConfig, RefreshCoordinator, RefreshDisposition, RefreshError,
    RefreshRecheckError,
};
pub use metrics::RefreshCoordMetrics;
pub use reclaim::ReclaimSweepHandle;
pub use sentinel::{SentinelThresholdConfig, SentinelTrigger};
pub use token_refresh::{
    OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES, OAuthProviderErrorCode, TokenRefreshError,
    refresh_oauth2_state,
};
pub use transport::{
    RefreshTransport, RefreshTransportError, TokenPostRequest, TokenPostResponse,
    TokenPostResponseError,
};
