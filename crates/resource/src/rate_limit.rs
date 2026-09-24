//! Per-resource rate limiting, declared by the resource author.
//!
//! A provider's limit is known to whoever wrote the integration, so the
//! [`Provider`](crate::Provider) declares a [`ResiliencePolicy`]; a stored
//! row may only *override* it within the bounds the policy allows
//! ([`Override`]). The limit itself is a
//! [GCRA](nebula_resilience::rate_limiter::gcra) enforced through a
//! [`LimitStore`](nebula_resilience::rate_limiter::gcra::LimitStore) keyed by
//! a [`LimitKey`]: two rows with the same key share one quota, which is how
//! several resources on one provider account stay within that account's
//! limit.
//!
//! A limiter is consumed once per acquire and, through
//! [`ResourceGuard::limits`](crate::ResourceGuard::limits), once per
//! outbound call. When the limit is exhausted the caller waits for its slot,
//! never past its deadline: beyond it the caller gets
//! [`ErrorKind::Exhausted`](crate::ErrorKind::Exhausted) with a
//! `retry_after` at once and nothing is consumed. A limit is local policy,
//! not backend health, so a denial never trips a
//! [`RecoveryGate`](crate::RecoveryGate); an unreachable shared store fails
//! closed as [`ErrorKind::Backpressure`](crate::ErrorKind::Backpressure).
//!
//! Lifecycle events are published on *transitions* only —
//! [`RateLimitEngaged`](crate::ResourceEvent::RateLimitEngaged),
//! [`RateLimitCleared`](crate::ResourceEvent::RateLimitCleared),
//! [`RateLimitPenalized`](crate::ResourceEvent::RateLimitPenalized),
//! [`RateLimitStoreUnavailable`](crate::ResourceEvent::RateLimitStoreUnavailable)
//! — never per call, so a saturated limiter cannot flood the bounded event
//! bus. They carry the resource key only, never limit keys.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_core::ResourceKey;
use nebula_eventbus::EventBus;
pub use nebula_resilience::rate_limiter::gcra::{
    Denied, ErasedLimitStore, Grant, LimitKey, LimitStoreError, MemoryLimitStore, Rate, RateConfig,
    ReserveRequest,
};
use nebula_schema::Schema;
use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, ErrorKind},
    events::ResourceEvent,
};

/// Default cap on how long one provider `Retry-After` may block a key.
pub const DEFAULT_MAX_PENALTY: Duration = Duration::from_mins(5);

/// Where a limit's state lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LimitScope {
    /// Shared by every worker process: a provider's quota is per account, not
    /// per process. Falls back to [`Process`](Self::Process), with a warning,
    /// until the manager is given a shared store.
    #[default]
    Cluster,
    /// This process only: for protecting local capacity.
    Process,
}

/// What a stored row may change about the declared limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Override {
    /// Nothing: the declared limit is enforced as is.
    Fixed,
    /// Only slower: a longer interval and no larger burst. With no declared
    /// limit, any limit is a tightening.
    #[default]
    TightenOnly,
    /// Anything up to this ceiling, e.g. a provider's higher paid tier.
    UpTo(Rate),
}

/// The resilience behaviour a resource author declares for their resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResiliencePolicy {
    rate: Option<Rate>,
    scope: LimitScope,
    overrides: Override,
    max_penalty: Duration,
}

impl Default for ResiliencePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ResiliencePolicy {
    /// No declared limit; rows may add one ([`Override::TightenOnly`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rate: None,
            scope: LimitScope::Cluster,
            overrides: Override::TightenOnly,
            max_penalty: DEFAULT_MAX_PENALTY,
        }
    }

    /// Declares the provider's limit.
    #[must_use]
    pub const fn rate(mut self, rate: Rate) -> Self {
        self.rate = Some(rate);
        self
    }

    /// Chooses where the limit's state lives (default: cluster-wide).
    #[must_use]
    pub const fn scope(mut self, scope: LimitScope) -> Self {
        self.scope = scope;
        self
    }

    /// Chooses what stored rows may override (default: tighten only).
    #[must_use]
    pub const fn overrides(mut self, overrides: Override) -> Self {
        self.overrides = overrides;
        self
    }

    /// Caps how long one provider `Retry-After` may block the limit.
    #[must_use]
    pub const fn max_penalty(mut self, max_penalty: Duration) -> Self {
        self.max_penalty = max_penalty;
        self
    }

    /// The declared limit, if any.
    #[must_use]
    pub const fn declared_rate(&self) -> Option<Rate> {
        self.rate
    }

    /// Where the limit's state lives.
    #[must_use]
    pub const fn limit_scope(&self) -> LimitScope {
        self.scope
    }

    /// The longest one provider `Retry-After` may block the limit.
    #[must_use]
    pub const fn penalty_cap(&self) -> Duration {
        self.max_penalty
    }

    /// The limit to enforce once `requested` is applied.
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] when the override is not allowed: any override
    /// under [`Override::Fixed`], a looser one under
    /// [`Override::TightenOnly`], one above the ceiling under
    /// [`Override::UpTo`].
    pub fn effective_rate(&self, requested: Option<Rate>) -> Result<Option<Rate>, Error> {
        let Some(requested) = requested else {
            return Ok(self.rate);
        };
        let allowed = match (self.overrides, self.rate) {
            (Override::Fixed, _) => false,
            (Override::TightenOnly, None) => true,
            (Override::TightenOnly, Some(declared)) => requested.is_no_looser_than(&declared),
            (Override::UpTo(ceiling), _) => requested.is_no_looser_than(&ceiling),
        };
        if allowed {
            Ok(Some(requested))
        } else {
            Err(Error::permanent(
                "rate limit override exceeds what the resource's policy allows",
            ))
        }
    }
}

/// Operator form of a rate-limit override (the UI schema of
/// [`RateConfig`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RateLimitSettings {
    /// Requests permitted per `period_ms`; at least 1.
    #[field(label = "Requests", description = "Permitted per period; at least 1")]
    pub requests: u32,
    /// Window the requests are spread over, in milliseconds; positive.
    #[field(
        label = "Period (ms)",
        description = "Window the requests are spread over"
    )]
    pub period_ms: u64,
    /// Requests allowed back to back after an idle spell; defaults to 1.
    #[field(
        label = "Burst",
        description = "Back-to-back requests after idling; default 1"
    )]
    #[serde(default)]
    pub burst: Option<u32>,
}

impl RateLimitSettings {
    /// `requests` per `period_ms`, no burst.
    #[must_use]
    pub const fn new(requests: u32, period_ms: u64) -> Self {
        Self {
            requests,
            period_ms,
            burst: None,
        }
    }

    /// Allows `burst` requests back to back.
    #[must_use]
    pub const fn with_burst(mut self, burst: u32) -> Self {
        self.burst = Some(burst);
        self
    }

    /// The validated rate.
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] for zero requests, period or burst, or an
    /// unrepresentable rate.
    pub fn to_rate(self) -> Result<Rate, Error> {
        let mut config = RateConfig::new(self.requests, self.period_ms);
        if let Some(burst) = self.burst {
            config = config.with_burst(burst);
        }
        Rate::try_from(config)
            .map_err(|error| Error::permanent(format!("invalid rate limit: {error}")))
    }

    /// Parses operator JSON (`None` or `null` = no override).
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] for malformed JSON, unknown fields, or an
    /// invalid rate.
    pub fn rate_from_value(value: Option<&serde_json::Value>) -> Result<Option<Rate>, Error> {
        match value {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => Self::deserialize(value)
                .map_err(|error| Error::permanent(format!("invalid rate limit settings: {error}")))?
                .to_rate()
                .map(Some),
        }
    }
}

/// Registration-time limit input for one row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RowLimit {
    /// Operator override of the declared rate, checked against the policy.
    pub rate: Option<Rate>,
    /// Quota the row draws on. Rows with equal keys share one limit (the
    /// engine keys stored rows by provider account). `None` keys the limit
    /// to this registry row alone.
    pub key: Option<LimitKey>,
}

impl RowLimit {
    /// An override of the declared rate.
    #[must_use]
    pub const fn rate(rate: Rate) -> Self {
        Self {
            rate: Some(rate),
            key: None,
        }
    }

    /// Shares the quota under `key`.
    #[must_use]
    pub fn with_key(mut self, key: LimitKey) -> Self {
        self.key = Some(key);
        self
    }
}

/// A limit store shared by every worker process, for
/// [`LimitScope::Cluster`] limits.
#[derive(Clone)]
pub struct SharedLimitStore(pub Arc<dyn ErasedLimitStore>);

impl fmt::Debug for SharedLimitStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SharedLimitStore")
    }
}

/// The enforced limit of one registry row.
pub struct ResourceLimiter {
    store: Arc<dyn ErasedLimitStore>,
    key: LimitKey,
    rate: Rate,
    max_penalty: Duration,
    resource_key: ResourceKey,
    events: Arc<EventBus<ResourceEvent>>,
    engaged: AtomicBool,
    store_down: AtomicBool,
    refusals: AtomicU32,
}

impl fmt::Debug for ResourceLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceLimiter")
            .field("resource_key", &self.resource_key)
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

impl ResourceLimiter {
    pub(crate) fn new(
        store: Arc<dyn ErasedLimitStore>,
        key: LimitKey,
        rate: Rate,
        max_penalty: Duration,
        resource_key: ResourceKey,
        events: Arc<EventBus<ResourceEvent>>,
    ) -> Self {
        Self {
            store,
            key,
            rate,
            max_penalty,
            resource_key,
            events,
            engaged: AtomicBool::new(false),
            store_down: AtomicBool::new(false),
            refusals: AtomicU32::new(0),
        }
    }

    /// The enforced rate.
    #[must_use]
    pub const fn rate(&self) -> Rate {
        self.rate
    }

    /// Waits for one permit, never past `deadline`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Exhausted`](crate::ErrorKind::Exhausted) with a
    ///   `retry_after` when the slot lands after `deadline`; nothing is
    ///   consumed.
    /// - [`ErrorKind::Backpressure`](crate::ErrorKind::Backpressure) when the
    ///   limit store cannot be reached (fail closed).
    ///
    /// # Cancel safety
    ///
    /// Cancelling during the wait forfeits the booked slot: the limiter errs
    /// on sending less, never more.
    pub async fn ready(&self, deadline: Option<std::time::Instant>) -> Result<(), Error> {
        let max_wait = deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(std::time::Instant::now())
        });
        let decision = self
            .store
            .reserve_boxed(&self.key, &self.rate, ReserveRequest::new(1, max_wait))
            .await;
        let grant = match decision {
            Ok(Ok(grant)) => {
                self.store_recovered();
                grant
            },
            Ok(Err(Denied::Later { retry_after })) => {
                self.store_recovered();
                self.engage();
                return Err(Error::exhausted(
                    "rate limit exhausted before the deadline",
                    Some(retry_after),
                )
                .with_resource_key(self.resource_key.clone()));
            },
            Ok(Err(Denied::Never { burst })) => {
                return Err(Error::permanent(format!(
                    "rate limit: one permit exceeds the burst of {burst}"
                ))
                .with_resource_key(self.resource_key.clone()));
            },
            Ok(Err(_)) => {
                return Err(Error::exhausted("rate limit refused the request", None)
                    .with_resource_key(self.resource_key.clone()));
            },
            Err(error) => return Err(self.store_unavailable(&error)),
        };
        if grant.wait.is_zero() {
            self.clear();
        } else {
            self.engage();
            tokio::time::sleep(grant.wait).await;
        }
        Ok(())
    }

    /// Blocks every caller of this limit for the provider's `retry_after`,
    /// capped at the policy's maximum. Call it on a provider's 429.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Backpressure`](crate::ErrorKind::Backpressure) when the
    /// limit store cannot be reached.
    pub async fn penalize(&self, retry_after: Duration) -> Result<(), Error> {
        self.store
            .penalize_boxed(&self.key, &self.rate, retry_after, self.max_penalty)
            .await
            .map_err(|error| self.store_unavailable(&error))?;
        let _ = self.events.emit(ResourceEvent::RateLimitPenalized {
            key: self.resource_key.clone(),
            retry_after: retry_after.min(self.max_penalty),
        });
        self.engaged.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Runs one outbound call under the limit.
    ///
    /// Waits for a permit (see [`ready`](Self::ready)), runs `call`, and when
    /// the call fails with [`ErrorKind::Exhausted`](crate::ErrorKind::Exhausted)
    /// — the provider said "too many requests" — blocks the whole limit key
    /// for the provider's `retry_after` (capped at the policy's
    /// `max_penalty`). Without a `retry_after` the block backs off
    /// exponentially over consecutive refusals, from one second up to the
    /// cap; the first call that gets through resets it.
    ///
    /// # Errors
    ///
    /// Whatever [`ready`](Self::ready) or `call` returns. A failure to record
    /// the penalty is logged, never masks the call's own error.
    pub async fn call<T, F, Fut>(
        &self,
        deadline: Option<std::time::Instant>,
        call: F,
    ) -> Result<T, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, Error>>,
    {
        self.ready(deadline).await?;
        let result = call().await;
        match &result {
            Ok(_) => self.refusals.store(0, Ordering::Relaxed),
            Err(error) => {
                if let ErrorKind::Exhausted { retry_after } = error.kind() {
                    let refusals = self.refusals.fetch_add(1, Ordering::Relaxed);
                    let block = retry_after.unwrap_or_else(|| backoff(refusals));
                    if let Err(penalty_error) = self.penalize(block).await {
                        tracing::debug!(
                            target: "nebula_resource::rate_limit",
                            resource_key = %self.resource_key,
                            %penalty_error,
                            "could not record a provider refusal"
                        );
                    }
                }
            },
        }
        result
    }

    fn engage(&self) {
        if !self.engaged.swap(true, Ordering::Relaxed) {
            let _ = self.events.emit(ResourceEvent::RateLimitEngaged {
                key: self.resource_key.clone(),
            });
        }
    }

    fn clear(&self) {
        if self.engaged.swap(false, Ordering::Relaxed) {
            let _ = self.events.emit(ResourceEvent::RateLimitCleared {
                key: self.resource_key.clone(),
            });
        }
    }

    fn store_recovered(&self) {
        if self.store_down.swap(false, Ordering::Relaxed) {
            let _ = self.events.emit(ResourceEvent::RateLimitStoreRecovered {
                key: self.resource_key.clone(),
            });
        }
    }

    fn store_unavailable(&self, error: &LimitStoreError) -> Error {
        if !self.store_down.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                target: "nebula_resource::rate_limit",
                resource_key = %self.resource_key,
                %error,
                "rate limit store unavailable; failing closed"
            );
            let _ = self.events.emit(ResourceEvent::RateLimitStoreUnavailable {
                key: self.resource_key.clone(),
            });
        }
        Error::backpressure("rate limit store unavailable")
            .with_resource_key(self.resource_key.clone())
    }
}

/// Block after the `refusals`-th consecutive provider refusal that carried no
/// `retry_after`: 1 s, 2 s, 4 s, … (the policy cap applies on top).
fn backoff(refusals: u32) -> Duration {
    Duration::from_secs(1u64 << refusals.min(20))
}

/// Parses an HTTP `Retry-After` header value given in seconds.
///
/// The HTTP-date form is not parsed and yields `None`, as does anything
/// malformed; callers then fall back to the limiter's own backoff.
#[must_use]
pub fn retry_after_from_header(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
#[path = "rate_limit_tests.rs"]
mod tests;
