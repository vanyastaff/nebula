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
//! Every row has a [`ResourceLimiter`]; it is consumed once per acquire. The
//! resource author wraps the client built in `Provider::create` once, with
//! [`ResourceContext::limits`](crate::ResourceContext::limits) and
//! [`ResourceLimiter::wrap`], so action code calls the [`Limited`] client and
//! never sees the limit:
//!
//! ```ignore
//! async fn create(&self, config: &Config, ctx: &ResourceContext) -> Result<Self::Instance, Error> {
//!     let bot = teloxide::Bot::new(&config.token);
//!     Ok(ctx.limits().wrap(bot, TelegramThrottle))
//! }
//! // In an action:
//! guard.run(async |bot| bot.send_message(chat, "hi").await).await?;
//! ```
//!
//! The [`Throttle`] tells the provider's "slow down" apart from other
//! outcomes; on it every caller of the quota pauses for the provider's
//! `retry_after` (capped at the policy's `max_penalty`), or for an
//! exponential backoff when it named no time. A resource that declares no
//! rate pays nothing for this: its limiter paces nothing and only honours
//! pauses.
//!
//! When the limit is exhausted the caller waits for its slot, never past its
//! deadline: beyond it the caller gets
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
        Arc, Mutex, PoisonError,
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

use crate::{error::Error, events::ResourceEvent};

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

/// The GCRA quota a limiter draws on.
pub(crate) struct Quota {
    store: Arc<dyn ErasedLimitStore>,
    key: LimitKey,
    rate: Rate,
}

impl Quota {
    pub(crate) fn new(store: Arc<dyn ErasedLimitStore>, key: LimitKey, rate: Rate) -> Self {
        Self { store, key, rate }
    }
}

/// Where a limiter reports: the row's resource key and the manager's bus.
struct Reporter {
    resource_key: ResourceKey,
    events: Arc<EventBus<ResourceEvent>>,
}

/// The limit of one registry row.
///
/// Every row has one. With a rate (declared by the resource or set on the
/// row) it paces calls on the row's quota and records a provider's "slow
/// down" in the limit store, where every caller of the quota sees it. Without
/// a rate it paces nothing and only honours pauses, kept in this process: a
/// resource with no limit pays one uncontended lock per acquire.
pub struct ResourceLimiter {
    quota: Option<Quota>,
    max_penalty: Duration,
    reporter: Option<Reporter>,
    /// Pause of a limiter without a quota (a quota keeps its pause in the
    /// store).
    paused_until: Mutex<Option<tokio::time::Instant>>,
    engaged: AtomicBool,
    store_down: AtomicBool,
    refusals: AtomicU32,
}

impl fmt::Debug for ResourceLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceLimiter")
            .field(
                "resource_key",
                &self
                    .reporter
                    .as_ref()
                    .map(|reporter| &reporter.resource_key),
            )
            .field("rate", &self.rate())
            .finish_non_exhaustive()
    }
}

impl ResourceLimiter {
    pub(crate) fn new(
        quota: Option<Quota>,
        max_penalty: Duration,
        resource_key: ResourceKey,
        events: Arc<EventBus<ResourceEvent>>,
    ) -> Self {
        Self::build(
            quota,
            max_penalty,
            Some(Reporter {
                resource_key,
                events,
            }),
        )
    }

    /// A limiter of no registry row: no rate, no events, pauses kept in this
    /// value alone.
    pub(crate) fn detached() -> Arc<Self> {
        Arc::new(Self::build(None, DEFAULT_MAX_PENALTY, None))
    }

    fn build(quota: Option<Quota>, max_penalty: Duration, reporter: Option<Reporter>) -> Self {
        Self {
            quota,
            max_penalty,
            reporter,
            paused_until: Mutex::new(None),
            engaged: AtomicBool::new(false),
            store_down: AtomicBool::new(false),
            refusals: AtomicU32::new(0),
        }
    }

    /// The enforced rate; `None` when the row only honours pauses.
    #[must_use]
    pub fn rate(&self) -> Option<Rate> {
        self.quota.as_ref().map(|quota| quota.rate)
    }

    /// Wraps a client so every call through it runs under this limit, with
    /// `throttle` telling a provider's "slow down" apart from other outcomes.
    ///
    /// Build the wrapper once, in [`Provider::create`](crate::Provider::create),
    /// from [`ResourceContext::limits`](crate::ResourceContext::limits).
    #[must_use]
    pub fn wrap<C, T>(self: &Arc<Self>, client: C, throttle: T) -> Limited<C, T> {
        Limited {
            client,
            throttle,
            limits: Arc::clone(self),
        }
    }

    /// Waits for one permit, never past `deadline`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Exhausted`](crate::ErrorKind::Exhausted) with a
    ///   `retry_after` when the slot (or the end of a pause) lands after
    ///   `deadline`; nothing is consumed.
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
        let wait = if let Some(quota) = &self.quota {
            self.reserve(quota, max_wait).await?
        } else {
            let wait = self.pause_remaining();
            if wait > max_wait {
                self.engage();
                return Err(self.tagged(Error::exhausted(
                    "resource paused past the deadline",
                    Some(wait),
                )));
            }
            wait
        };
        if wait.is_zero() {
            self.clear();
        } else {
            self.engage();
            tokio::time::sleep(wait).await;
        }
        Ok(())
    }

    /// Books one slot on `quota`; the wait until it.
    async fn reserve(&self, quota: &Quota, max_wait: Duration) -> Result<Duration, Error> {
        let decision = quota
            .store
            .reserve_boxed(&quota.key, &quota.rate, ReserveRequest::new(1, max_wait))
            .await;
        match decision {
            Ok(Ok(grant)) => {
                self.store_recovered();
                Ok(grant.wait)
            },
            Ok(Err(Denied::Later { retry_after })) => {
                self.store_recovered();
                self.engage();
                Err(self.tagged(Error::exhausted(
                    "rate limit exhausted before the deadline",
                    Some(retry_after),
                )))
            },
            Ok(Err(Denied::Never { burst })) => Err(self.tagged(Error::permanent(format!(
                "rate limit: one permit exceeds the burst of {burst}"
            )))),
            Ok(Err(_)) => {
                Err(self.tagged(Error::exhausted("rate limit refused the request", None)))
            },
            Err(error) => Err(self.store_unavailable(&error)),
        }
    }

    /// Time left of a local pause; clears one that has ended.
    fn pause_remaining(&self) -> Duration {
        let mut paused_until = self
            .paused_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let remaining = paused_until.map_or(Duration::ZERO, |until| {
            until.saturating_duration_since(tokio::time::Instant::now())
        });
        if remaining.is_zero() {
            *paused_until = None;
        }
        remaining
    }

    /// Pauses every caller of this limit for `retry_after`, capped at the
    /// policy's `max_penalty`.
    ///
    /// [`Limited`] calls this for you when its [`Throttle`] recognises a
    /// provider's "slow down"; call it directly only for a signal that does
    /// not come back from a call.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Backpressure`](crate::ErrorKind::Backpressure) when the
    /// limit store cannot be reached.
    pub async fn penalize(&self, retry_after: Duration) -> Result<(), Error> {
        let block = retry_after.min(self.max_penalty);
        if let Some(quota) = &self.quota {
            quota
                .store
                .penalize_boxed(&quota.key, &quota.rate, retry_after, self.max_penalty)
                .await
                .map_err(|error| self.store_unavailable(&error))?;
        } else {
            let until = tokio::time::Instant::now() + block;
            let mut paused_until = self
                .paused_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *paused_until = Some(paused_until.map_or(until, |current| current.max(until)));
        }
        self.emit(|key| ResourceEvent::RateLimitPenalized {
            key,
            retry_after: block,
        });
        self.engaged.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Records the verdict on one call: a pause on a provider's "slow down",
    /// a backoff reset otherwise.
    ///
    /// Without a `retry_after` the pause backs off exponentially over
    /// consecutive refusals, from one second up to the cap. A failure to
    /// record the pause is logged; it never masks the call's own outcome.
    async fn report(&self, verdict: Verdict) {
        match verdict {
            Verdict::Pass => self.refusals.store(0, Ordering::Relaxed),
            Verdict::Throttled { retry_after } => {
                let refusals = self.refusals.fetch_add(1, Ordering::Relaxed);
                let block = retry_after.unwrap_or_else(|| backoff(refusals));
                if let Err(error) = self.penalize(block).await {
                    tracing::debug!(
                        target: "nebula_resource::rate_limit",
                        %error,
                        "could not record a provider refusal"
                    );
                }
            },
        }
    }

    fn tagged(&self, error: Error) -> Error {
        match &self.reporter {
            Some(reporter) => error.with_resource_key(reporter.resource_key.clone()),
            None => error,
        }
    }

    fn emit(&self, event: impl FnOnce(ResourceKey) -> ResourceEvent) {
        if let Some(reporter) = &self.reporter {
            let _ = reporter.events.emit(event(reporter.resource_key.clone()));
        }
    }

    fn engage(&self) {
        if !self.engaged.swap(true, Ordering::Relaxed) {
            self.emit(|key| ResourceEvent::RateLimitEngaged { key });
        }
    }

    fn clear(&self) {
        if self.engaged.swap(false, Ordering::Relaxed) {
            self.emit(|key| ResourceEvent::RateLimitCleared { key });
        }
    }

    fn store_recovered(&self) {
        if self.store_down.swap(false, Ordering::Relaxed) {
            self.emit(|key| ResourceEvent::RateLimitStoreRecovered { key });
        }
    }

    fn store_unavailable(&self, error: &LimitStoreError) -> Error {
        if !self.store_down.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                target: "nebula_resource::rate_limit",
                resource_key = ?self.reporter.as_ref().map(|reporter| &reporter.resource_key),
                %error,
                "rate limit store unavailable; failing closed"
            );
            self.emit(|key| ResourceEvent::RateLimitStoreUnavailable { key });
        }
        self.tagged(Error::backpressure("rate limit store unavailable"))
    }
}

/// What one call's outcome says about the provider's limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// Not a limit signal; resets the backoff.
    Pass,
    /// The provider asked to slow down: every caller of the quota pauses for
    /// `retry_after` (capped at the policy's `max_penalty`), or for an
    /// exponential backoff when the provider named no time.
    Throttled {
        /// How long the provider asked to wait, if it said.
        retry_after: Option<Duration>,
    },
}

/// Tells a provider's "slow down" apart from other call outcomes.
///
/// Written once per resource type, next to the client it classifies. A
/// closure over the whole outcome works directly; [`on_error`] adapts one that
/// needs to look at the error only. The throttle sees only its own client's
/// outcomes, so a limit hit on some other resource inside the call is never
/// mistaken for this provider's.
pub trait Throttle<T, E>: Send + Sync {
    /// Classifies one outcome.
    fn check(&self, outcome: &Result<T, E>) -> Verdict;
}

impl<T, E, F> Throttle<T, E> for F
where
    F: Fn(&Result<T, E>) -> Verdict + Send + Sync,
{
    fn check(&self, outcome: &Result<T, E>) -> Verdict {
        self(outcome)
    }
}

/// A throttle that never sees a limit signal: calls are paced, nothing
/// pauses them.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoThrottle;

impl<T, E> Throttle<T, E> for NoThrottle {
    fn check(&self, _outcome: &Result<T, E>) -> Verdict {
        Verdict::Pass
    }
}

/// A throttle that classifies errors only; see [`on_error`].
#[derive(Debug, Clone, Copy)]
pub struct OnError<F>(F);

/// Builds a [`Throttle`] from a classifier of errors, for clients that report
/// a limit as an error (`teloxide::RequestError::RetryAfter`, an SDK's
/// `ThrottlingException`, ...). Successful outcomes are [`Verdict::Pass`].
#[must_use]
pub const fn on_error<F>(classify: F) -> OnError<F> {
    OnError(classify)
}

impl<T, E, F> Throttle<T, E> for OnError<F>
where
    F: Fn(&E) -> Verdict + Send + Sync,
{
    fn check(&self, outcome: &Result<T, E>) -> Verdict {
        outcome.as_ref().err().map_or(Verdict::Pass, &self.0)
    }
}

/// A client whose every call runs under a resource's rate limit.
///
/// Built by [`ResourceLimiter::wrap`] in `Provider::create`. Calls go through
/// [`run`](Self::run); there is deliberately no `Deref` to the client, so a
/// call cannot skip the limit by accident. [`unlimited`](Self::unlimited) is
/// the explicit, reviewable way to do so.
pub struct Limited<C, T = NoThrottle> {
    client: C,
    throttle: T,
    limits: Arc<ResourceLimiter>,
}

impl<C: Clone, T: Clone> Clone for Limited<C, T> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            throttle: self.throttle.clone(),
            limits: Arc::clone(&self.limits),
        }
    }
}

impl<C, T> fmt::Debug for Limited<C, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Limited")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl<C, T> Limited<C, T> {
    /// Runs one call under the limit, waiting for a permit as long as needed.
    ///
    /// # Errors
    ///
    /// [`LimitedError::Limit`] when the limit refused the call (see
    /// [`ResourceLimiter::ready`]), [`LimitedError::Call`] with the client's
    /// own error otherwise.
    pub async fn run<R, E>(
        &self,
        call: impl AsyncFnOnce(&C) -> Result<R, E>,
    ) -> Result<R, LimitedError<E>>
    where
        T: Throttle<R, E>,
    {
        self.run_until(None, call).await
    }

    /// Runs one call under the limit, waiting for a permit never past
    /// `deadline`.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run); a permit past `deadline` is
    /// [`LimitedError::Limit`] with `Exhausted` and a `retry_after`.
    pub async fn run_until<R, E>(
        &self,
        deadline: Option<std::time::Instant>,
        call: impl AsyncFnOnce(&C) -> Result<R, E>,
    ) -> Result<R, LimitedError<E>>
    where
        T: Throttle<R, E>,
    {
        self.limits
            .ready(deadline)
            .await
            .map_err(LimitedError::Limit)?;
        let outcome = call(&self.client).await;
        self.limits.report(self.throttle.check(&outcome)).await;
        outcome.map_err(LimitedError::Call)
    }

    /// The client, bypassing the limit. Use only for calls the provider does
    /// not count (a local builder, a cached lookup).
    #[must_use]
    pub const fn unlimited(&self) -> &C {
        &self.client
    }

    /// The limit calls run under.
    #[must_use]
    pub const fn limits(&self) -> &Arc<ResourceLimiter> {
        &self.limits
    }
}

/// Error of a call through [`Limited`].
#[derive(Debug)]
pub enum LimitedError<E> {
    /// The limit refused the call; it never reached the provider.
    Limit(Error),
    /// The call ran and failed with the client's own error.
    Call(E),
}

impl<E: fmt::Display> fmt::Display for LimitedError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(error) => write!(formatter, "rate limit: {error}"),
            Self::Call(error) => error.fmt(formatter),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for LimitedError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Limit(error) => Some(error),
            Self::Call(error) => Some(error),
        }
    }
}

/// Block after the `refusals`-th consecutive provider refusal that carried no
/// `retry_after`: 1 s, 2 s, 4 s, … (the policy cap applies on top).
fn backoff(refusals: u32) -> Duration {
    Duration::from_secs(1u64 << refusals.min(20))
}

/// Parses an HTTP `Retry-After` header value: delay seconds or an HTTP date
/// (a date in the past is zero).
///
/// Anything malformed yields `None`; a [`Verdict::Throttled`] without a
/// `retry_after` then falls back to the limiter's own backoff.
#[must_use]
pub fn retry_after_from_header(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(
        at.duration_since(std::time::SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

#[cfg(test)]
#[path = "rate_limit_tests.rs"]
mod tests;
