use std::time::Duration;

// ──────────────────────────────────────────────────────────────────────────
// Configuration
// ──────────────────────────────────────────────────────────────────────────

/// Configuration knobs for the two-tier coordinator.
///
/// Per sub-spec the four time-related parameters carry interlocking
/// invariants verified by [`RefreshCoordConfig::validate`]:
///
/// - `heartbeat_interval × 3 <= claim_ttl` -- three heartbeat ticks must fit inside one claim TTL
///   so two consecutive missed heartbeats still leave the claim valid until the next tick.
/// - `refresh_timeout + 2 × heartbeat_interval <= claim_ttl` -- the caller-wait budget expires
///   while at least two heartbeat opportunities remain inside the original TTL. The owned
///   provider/persistence task is not cancelled at this point.
/// - `reclaim_sweep_interval <= claim_ttl` -- sweeps must run at least as often as a claim's TTL
///   so a crashed holder is accounted within one TTL window. Expired normal claims are reclaimed;
///   expired provider-side-effect claims are retained as poison.
///
/// The boundary case `heartbeat_interval × 3 == claim_ttl` is allowed
/// (mirrors the execution-lease shape: `ttl / 3 ==
/// heartbeat_interval`).
///
/// CI test asserts `RefreshCoordConfig::default().validate().is_ok()`.
#[derive(Clone, Debug)]
pub struct RefreshCoordConfig {
    /// Claim TTL applied to every L2 acquire/heartbeat call.
    pub claim_ttl: Duration,
    /// Cadence of background heartbeat ticks while a claim is held.
    pub heartbeat_interval: Duration,
    /// Per-phase wait budget for an L1 waiter, L2 contention, and the owned
    /// refresh task.
    ///
    /// Each phase consumes at most one such budget; this is not a single
    /// end-to-end deadline. Expiry never cancels provider/persistence work
    /// after the sentinel boundary.
    pub refresh_timeout: Duration,
    /// Cadence of the background reclaim sweep (Stage 3.3).
    pub reclaim_sweep_interval: Duration,
    /// Distinct accounted incidents inside `sentinel_window` required before
    /// atomically installing the `ReauthRequired` aggregate transition.
    pub sentinel_threshold: u32,
    /// Database-clock rolling window for atomic sentinel escalation.
    pub sentinel_window: Duration,
}

impl Default for RefreshCoordConfig {
    fn default() -> Self {
        Self {
            claim_ttl: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(10),
            refresh_timeout: Duration::from_secs(8),
            reclaim_sweep_interval: Duration::from_secs(30),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        }
    }
}

/// Validation errors for [`RefreshCoordConfig`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A duration used as a lease or Tokio interval was zero.
    #[error("config field {field} must be greater than zero")]
    ZeroDuration {
        /// Property whose zero value would make lease semantics invalid or panic
        /// `tokio::time::interval`.
        field: &'static str,
    },
    /// A zero sentinel threshold would escalate every accounted event and is
    /// almost certainly a deployment mistake.
    #[error("sentinel_threshold must be greater than zero")]
    ZeroSentinelThreshold,
    /// `heartbeat_interval × 3` exceeds `claim_ttl` -- three heartbeat
    /// ticks would not fit inside one TTL window.
    #[error("heartbeat_interval \u{d7} 3 must be \u{2264} claim_ttl")]
    HeartbeatTooSlow,
    /// `refresh_timeout + 2 × heartbeat_interval` exceeds `claim_ttl` --
    /// a caller could stop waiting without two heartbeat opportunities left
    /// inside the original claim TTL.
    #[error("refresh_timeout + 2 \u{d7} heartbeat_interval must be \u{2264} claim_ttl")]
    RefreshTimeoutTooLong,
    /// `reclaim_sweep_interval` exceeds `claim_ttl`.
    #[error("reclaim_sweep_interval must be \u{2264} claim_ttl")]
    ReclaimTooSlow,
    /// A computed Duration overflowed during invariant validation. The
    /// surfaced field name lets operators spot which knob to bound (e.g.
    /// `heartbeat_interval × 3` or `refresh_timeout + 2 × heartbeat_interval`).
    /// `validate()` MUST surface bad config as a typed error rather than
    /// panicking inside the very fn meant to detect bad config.
    #[error("config field {field} overflowed Duration during invariant check (value: {value:?})")]
    Overflow {
        /// Logical name of the operand that overflowed.
        field: &'static str,
        /// The pre-overflow operand; useful in operator messages.
        value: Duration,
    },

    /// Metric primitive registration failed for the coordinator's series.
    #[error("telemetry metrics error: {0}")]
    Telemetry(#[from] nebula_metrics::MetricsError),
}

impl RefreshCoordConfig {
    /// Verify the per- interlocking invariants.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::*` whose variant names which invariant the
    /// configuration violates. Returns `ConfigError::Overflow` if any of
    /// the intermediate `Duration` arithmetic (e.g. `heartbeat_interval × 3`,
    /// `refresh_timeout + 2 × heartbeat_interval`) overflows `Duration::MAX`
    /// -- the canonical fix is to lower the offending knob.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.claim_ttl.is_zero() {
            return Err(ConfigError::ZeroDuration { field: "claim_ttl" });
        }
        if self.heartbeat_interval.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "heartbeat_interval",
            });
        }
        if self.refresh_timeout.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "refresh_timeout",
            });
        }
        if self.reclaim_sweep_interval.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "reclaim_sweep_interval",
            });
        }
        if self.sentinel_window.is_zero() {
            return Err(ConfigError::ZeroDuration {
                field: "sentinel_window",
            });
        }
        if self.sentinel_threshold == 0 {
            return Err(ConfigError::ZeroSentinelThreshold);
        }

        // `Duration::checked_mul` and `checked_add` return `None` on
        // overflow rather than panicking -- surface that as a typed
        // `ConfigError::Overflow` so a user-supplied
        // `Duration::MAX / 2`-ish value doesn't blow up the config gate.
        let hb_x3 = self
            .heartbeat_interval
            .checked_mul(3)
            .ok_or(ConfigError::Overflow {
                field: "heartbeat_interval * 3",
                value: self.heartbeat_interval,
            })?;
        if hb_x3 > self.claim_ttl {
            return Err(ConfigError::HeartbeatTooSlow);
        }
        let hb_x2 = self
            .heartbeat_interval
            .checked_mul(2)
            .ok_or(ConfigError::Overflow {
                field: "heartbeat_interval * 2",
                value: self.heartbeat_interval,
            })?;
        let hold_budget = self
            .refresh_timeout
            .checked_add(hb_x2)
            .ok_or(ConfigError::Overflow {
                field: "refresh_timeout + heartbeat_interval * 2",
                value: self.refresh_timeout,
            })?;
        if hold_budget > self.claim_ttl {
            return Err(ConfigError::RefreshTimeoutTooLong);
        }
        if self.reclaim_sweep_interval > self.claim_ttl {
            return Err(ConfigError::ReclaimTooSlow);
        }
        Ok(())
    }
}
