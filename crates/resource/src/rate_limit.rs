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
    fmt::{self, Write as _},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
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

use sha2::{Digest as _, Sha256};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResiliencePolicy {
    rate: Option<Rate>,
    keyed: Vec<(&'static str, Rate)>,
    scope: LimitScope,
    overrides: Override,
    max_penalty: Duration,
    account_slots: Vec<&'static str>,
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
            keyed: Vec::new(),
            scope: LimitScope::Cluster,
            overrides: Override::TightenOnly,
            max_penalty: DEFAULT_MAX_PENALTY,
            account_slots: Vec::new(),
        }
    }

    /// Declares the provider's limit.
    #[must_use]
    pub const fn rate(mut self, rate: Rate) -> Self {
        self.rate = Some(rate);
        self
    }

    /// Declares a limit per value of `dimension` — a provider's "one message
    /// per second per chat" — on top of the account limit.
    ///
    /// Calls opt in by naming the value
    /// ([`Limited::run_for`], [`ResourceLimiter::ready_for`]); only the call
    /// knows which chat or recipient it addresses. Values are hashed before
    /// they reach a limit store. Declaring a dimension again replaces it.
    #[must_use]
    pub fn keyed(mut self, dimension: &'static str, rate: Rate) -> Self {
        self.keyed.retain(|(declared, _)| *declared != dimension);
        self.keyed.push((dimension, rate));
        self
    }

    /// Names the credential slot that identifies the provider account the
    /// limit belongs to; may be called for several slots.
    ///
    /// Rows bound to the same credentials share one account quota. By
    /// default every bound credential counts, which splits the quota when
    /// rows share the account credential but differ in an auxiliary one (a
    /// TLS or signing credential). Declaring the account slots keys the
    /// quota by those alone.
    #[must_use]
    pub fn account_credential(mut self, slot: &'static str) -> Self {
        if !self.account_slots.contains(&slot) {
            self.account_slots.push(slot);
        }
        self
    }

    /// The credential slots that identify the provider account; empty when
    /// every bound credential does.
    #[must_use]
    pub fn account_slots(&self) -> &[&'static str] {
        &self.account_slots
    }

    /// The declared per-key limits, by dimension.
    #[must_use]
    pub fn keyed_rates(&self) -> &[(&'static str, Rate)] {
        &self.keyed
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
        self.check_override(self.rate, requested)
            .map_err(|rule| Error::permanent(format!("resilience_override.rate: {rule}")))?;
        Ok(Some(requested))
    }

    /// The per-key limits to enforce once `requested` overrides are applied,
    /// by dimension.
    ///
    /// Only declared dimensions exist: an override cannot invent one, since
    /// no call would name it. Each override obeys the same [`Override`] rule
    /// as the account rate, against its own declared rate; a dimension's
    /// [`UpTo`](Override::UpTo) ceiling is its declared rate.
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] for an undeclared dimension or an override the
    /// policy does not allow. Messages never restate the dimension name, which
    /// is operator input.
    pub fn effective_keyed(
        &self,
        requested: &[(String, Rate)],
    ) -> Result<Vec<(&'static str, Rate)>, Error> {
        let undeclared = requested
            .iter()
            .any(|(name, _)| !self.keyed.iter().any(|(declared, _)| declared == name));
        if undeclared {
            return Err(Error::permanent(
                "resilience_override.keyed: names a dimension this resource does not declare",
            ));
        }
        self.keyed
            .iter()
            .map(|(dimension, declared)| {
                let Some((_, rate)) = requested.iter().find(|(name, _)| name == dimension) else {
                    return Ok((*dimension, *declared));
                };
                let ceiling = match self.overrides {
                    Override::UpTo(_) => Override::UpTo(*declared),
                    other => other,
                };
                Self::check_rule(ceiling, Some(*declared), *rate)
                    .map(|()| (*dimension, *rate))
                    .map_err(|rule| Error::permanent(format!("resilience_override.keyed: {rule}")))
            })
            .collect()
    }

    fn check_override(&self, declared: Option<Rate>, requested: Rate) -> Result<(), &'static str> {
        Self::check_rule(self.overrides, declared, requested)
    }

    /// Why `requested` may not replace `declared` under `rule`. The reasons
    /// name the rule, never the values: they reach API clients verbatim.
    fn check_rule(
        rule: Override,
        declared: Option<Rate>,
        requested: Rate,
    ) -> Result<(), &'static str> {
        let refusal = match (rule, declared) {
            (Override::Fixed, _) => Some("this resource does not allow overriding it"),
            (Override::TightenOnly, None) => None,
            (Override::TightenOnly, Some(declared)) => (!requested.is_no_looser_than(&declared))
                .then_some(
                    "may only be slower than the resource's declared rate, with no larger burst",
                ),
            (Override::UpTo(ceiling), _) => {
                (!requested.is_no_looser_than(&ceiling)).then_some("exceeds the resource's ceiling")
            },
        };
        refusal.map_or(Ok(()), Err)
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
}

/// What an operator may change about a resource's resilience, stored on the
/// resource row (`resilience_override`) and bounded by the resource's
/// [`ResiliencePolicy`].
///
/// A document rather than a bare rate so later knobs (window quotas) extend
/// it without another column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ResilienceOverride {
    /// Replaces the resource's declared rate, within its policy.
    #[serde(default)]
    pub rate: Option<RateLimitSettings>,
    /// Replaces declared per-key limits, one entry per dimension.
    #[serde(default)]
    pub keyed: Vec<KeyedOverride>,
}

/// An override of one declared per-key limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct KeyedOverride {
    /// The dimension the resource declares, e.g. `chat_id`.
    #[field(
        label = "Dimension",
        description = "Declared by the resource, e.g. chat_id"
    )]
    pub dimension: String,
    /// The limit per value of that dimension.
    pub rate: RateLimitSettings,
}

impl ResilienceOverride {
    /// An override of the rate.
    #[must_use]
    pub const fn rate(rate: RateLimitSettings) -> Self {
        Self {
            rate: Some(rate),
            keyed: Vec::new(),
        }
    }

    /// Adds an override of the per-key limit of `dimension`.
    #[must_use]
    pub fn with_keyed(mut self, dimension: impl Into<String>, rate: RateLimitSettings) -> Self {
        self.keyed.push(KeyedOverride {
            dimension: dimension.into(),
            rate,
        });
        self
    }

    /// The per-key rates this override asks for, validated, by dimension.
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] for a dimension named twice or a zero or
    /// unrepresentable rate.
    pub fn requested_keyed(&self) -> Result<Vec<(String, Rate)>, Error> {
        let mut rates: Vec<(String, Rate)> = Vec::with_capacity(self.keyed.len());
        for entry in &self.keyed {
            if rates.iter().any(|(name, _)| *name == entry.dimension) {
                return Err(Error::permanent(
                    "resilience_override.keyed: names a dimension more than once",
                ));
            }
            let rate = entry.rate.to_rate().map_err(|error| {
                Error::permanent(
                    "resilience_override.keyed.rate: requests, period_ms and burst must be \
                     positive and representable",
                )
                .with_source(error)
            })?;
            rates.push((entry.dimension.clone(), rate));
        }
        Ok(rates)
    }

    /// Parses the stored document (`None` or `null` = no override).
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] naming the offending field for a malformed
    /// document or an invalid rate. Messages carry field paths, never the
    /// submitted values; the parser's own report is kept as the source.
    pub fn from_value(value: Option<&serde_json::Value>) -> Result<Self, Error> {
        match value {
            None | Some(serde_json::Value::Null) => Ok(Self::default()),
            Some(value) => Self::deserialize(value).map_err(|error| {
                Error::permanent(
                    "resilience_override: expected an object with an optional `rate` \
                     ({requests, period_ms, burst?}) and optional `keyed` \
                     ([{dimension, rate}])",
                )
                .with_source(error)
            }),
        }
    }

    /// The rate this override asks for, validated.
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] for a zero or unrepresentable rate.
    pub fn requested_rate(&self) -> Result<Option<Rate>, Error> {
        self.rate
            .map(|settings| {
                settings.to_rate().map_err(|error| {
                    Error::permanent(
                        "resilience_override.rate: requests, period_ms and burst must be \
                         positive and representable",
                    )
                    .with_source(error)
                })
            })
            .transpose()
    }

    /// The rate to enforce under `policy` once this override is applied.
    ///
    /// # Errors
    ///
    /// As [`requested_rate`](Self::requested_rate), and a permanent
    /// [`Error`] when `policy` does not allow the override.
    pub fn apply(&self, policy: &ResiliencePolicy) -> Result<Option<Rate>, Error> {
        policy.effective_rate(self.requested_rate()?)
    }

    /// The per-key limits to enforce under `policy` once this override is
    /// applied, by dimension.
    ///
    /// # Errors
    ///
    /// As [`requested_keyed`](Self::requested_keyed), and a permanent
    /// [`Error`] for an undeclared dimension or an override `policy` does not
    /// allow.
    pub fn apply_keyed(
        &self,
        policy: &ResiliencePolicy,
    ) -> Result<Vec<(&'static str, Rate)>, Error> {
        policy.effective_keyed(&self.requested_keyed()?)
    }
}

/// Registration-time limit input for one row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RowLimit {
    /// Operator override of the declared rate, checked against the policy.
    pub rate: Option<Rate>,
    /// Operator overrides of declared per-key limits, by dimension, checked
    /// against the policy.
    pub keyed: Vec<(String, Rate)>,
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
            keyed: Vec::new(),
            key: None,
        }
    }

    /// Overrides the per-key limit of `dimension`.
    #[must_use]
    pub fn with_keyed(mut self, dimension: impl Into<String>, rate: Rate) -> Self {
        self.keyed.push((dimension.into(), rate));
        self
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

/// Per-key limits of a row ([`ResiliencePolicy::keyed`]), drawn from the
/// same store as its quota under keys derived from the row's limit key, so
/// they are shared exactly as widely as the quota and never across tenants.
pub(crate) struct KeyedLimits {
    store: Arc<dyn ErasedLimitStore>,
    base: LimitKey,
    rates: Vec<(&'static str, Rate)>,
}

impl KeyedLimits {
    pub(crate) fn new(
        store: Arc<dyn ErasedLimitStore>,
        base: LimitKey,
        rates: Vec<(&'static str, Rate)>,
    ) -> Self {
        Self { store, base, rates }
    }

    /// The limit key and rate of one value of `dimension`. The value is
    /// hashed: a chat id or an e-mail address never reaches a limit store.
    fn limit_for(&self, dimension: &str, value: &str) -> Result<(LimitKey, Rate), Error> {
        let Some((dimension, rate)) = self
            .rates
            .iter()
            .find(|(declared, _)| *declared == dimension)
        else {
            return Err(Error::permanent(format!(
                "rate limit: the resource declares no per-key limit named `{dimension}`"
            )));
        };
        let digest = Sha256::new()
            .chain_update(dimension.as_bytes())
            .chain_update([0])
            .chain_update(value.as_bytes())
            .finalize();
        let hash = digest[..16]
            .iter()
            .fold(String::with_capacity(32), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        let key = LimitKey::new(format!("{}:k:{dimension}:{hash}", self.base.as_str())).map_err(
            |_| {
                Error::permanent(format!(
                    "rate limit: per-key limit `{dimension}` does not form a valid limit key"
                ))
            },
        )?;
        Ok((key, *rate))
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
    keyed: Option<KeyedLimits>,
    max_penalty: Duration,
    reporter: Option<Reporter>,
    /// The latest account pause this process recorded. The only pause of a
    /// limiter without a quota; with a quota the store holds the pause for
    /// new bookings, and this lets callers already sleeping on an earlier
    /// booking honour it when they wake.
    paused_until: Mutex<Option<tokio::time::Instant>>,
    engaged: AtomicBool,
    /// Callers currently waiting for a slot; the limit clears when the last
    /// one's wait ends.
    waiters: AtomicUsize,
    store_down: AtomicBool,
    /// Consecutive refusals of the account quota.
    refusals: AtomicU32,
    /// Consecutive refusals of each key with an unbroken run of them, and
    /// when the last one came.
    key_refusals: Mutex<std::collections::HashMap<LimitKey, (u32, tokio::time::Instant)>>,
    /// Keys callers of this limiter are booking or waiting on right now,
    /// with the pause recorded for each meanwhile: a pause only has to be
    /// kept locally for callers that booked before it, and new bookings see
    /// it through the store. Bounded by the callers in flight.
    key_waits: Mutex<std::collections::HashMap<LimitKey, KeyWait>>,
    /// Refunds running in the background (see [`release`](Self::release)).
    refunds: Arc<AtomicUsize>,
    /// Set once a client is [`wrap`](Self::wrap)ped: its calls book the
    /// quota, so an acquire only honours pauses (see
    /// [`ready_to_acquire`](Self::ready_to_acquire)).
    per_call: AtomicBool,
    /// An acquire booked a permit before the first client was wrapped; the
    /// next call through a wrapped client uses it (see
    /// [`ready_to_call`](Self::ready_to_call)).
    prepaid: AtomicBool,
}

/// What a caller does once its wait is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// Its slot has come and nothing holds it back.
    Proceed,
    /// It waited out a pause past its slot: it books again.
    Rebook,
}

/// Bound on refunds one limiter runs in the background at once; past it a
/// slot given up lapses unused, which errs on sending less.
const MAX_PENDING_REFUNDS: usize = 32;

/// How long one background refund may take before it is abandoned.
const REFUND_BUDGET: Duration = Duration::from_secs(5);

/// Callers of one key in flight, and the pause recorded for it meanwhile.
#[derive(Debug, Default)]
struct KeyWait {
    callers: usize,
    paused_until: Option<tokio::time::Instant>,
    /// The pause did not reach the store, so it is kept here past its
    /// callers, until it ends.
    kept: bool,
}

impl KeyWait {
    fn needed(&self, now: tokio::time::Instant) -> bool {
        self.callers > 0 || (self.kept && self.paused_until.is_some_and(|until| until > now))
    }
}

/// Bound on key pauses kept only in this process (their store write
/// failed); past it such a pause pauses the whole account locally instead,
/// which errs on sending less.
const MAX_KEPT_KEY_PAUSES: usize = 4_096;

/// Bound on the keys whose refusal runs are remembered; past it the key
/// refused longest ago is forgotten, so its next refusal backs off from
/// the start again.
const MAX_KEY_REFUSAL_RUNS: usize = 4_096;

/// Registers one caller of a key for as long as it lives, including when
/// the call is cancelled, so a pause recorded meanwhile reaches it.
struct KeyInterest<'a> {
    limiter: &'a ResourceLimiter,
    key: LimitKey,
}

impl<'a> KeyInterest<'a> {
    fn start(limiter: &'a ResourceLimiter, key: &LimitKey) -> Self {
        limiter
            .key_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.clone())
            .or_default()
            .callers += 1;
        Self {
            limiter,
            key: key.clone(),
        }
    }
}

/// Keeps a key's pause in this process when dropped unconfirmed: the
/// store write may have failed or been cancelled, and then only this
/// process knows the pause.
struct KeepUnlessConfirmed<'a> {
    limiter: &'a ResourceLimiter,
    key: &'a LimitKey,
    block: Duration,
    confirmed: bool,
}

impl Drop for KeepUnlessConfirmed<'_> {
    fn drop(&mut self) {
        if !self.confirmed {
            self.limiter.keep_key_pause(self.key, self.block);
        }
    }
}

impl Drop for KeyInterest<'_> {
    fn drop(&mut self) {
        let mut waits = self
            .limiter
            .key_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(wait) = waits.get_mut(&self.key) {
            wait.callers = wait.callers.saturating_sub(1);
            if !wait.needed(tokio::time::Instant::now()) {
                waits.remove(&self.key);
            }
        }
    }
}

/// Counts one waiting caller for as long as it lives, including when the
/// wait is cancelled.
struct Waiting<'a>(&'a ResourceLimiter);

impl<'a> Waiting<'a> {
    fn start(limiter: &'a ResourceLimiter) -> Self {
        limiter.waiters.fetch_add(1, Ordering::AcqRel);
        limiter.engage();
        Self(limiter)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        // The limit is not cleared when the last waiter leaves: a lone caller
        // at saturation would then report engaged and cleared on every call.
        // It is cleared when a caller passes without waiting and nobody else
        // waits (see `wait_out`), which is when it has stopped holding back.
        self.0.waiters.fetch_sub(1, Ordering::AcqRel);
    }
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
        keyed: Option<KeyedLimits>,
        max_penalty: Duration,
        resource_key: ResourceKey,
        events: Arc<EventBus<ResourceEvent>>,
    ) -> Self {
        Self::build(
            quota,
            keyed,
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
        Arc::new(Self::build(None, None, DEFAULT_MAX_PENALTY, None))
    }

    fn build(
        quota: Option<Quota>,
        keyed: Option<KeyedLimits>,
        max_penalty: Duration,
        reporter: Option<Reporter>,
    ) -> Self {
        Self {
            quota,
            keyed,
            max_penalty,
            reporter,
            paused_until: Mutex::new(None),
            engaged: AtomicBool::new(false),
            per_call: AtomicBool::new(false),
            prepaid: AtomicBool::new(false),
            waiters: AtomicUsize::new(0),
            store_down: AtomicBool::new(false),
            refusals: AtomicU32::new(0),
            key_refusals: Mutex::new(std::collections::HashMap::new()),
            key_waits: Mutex::new(std::collections::HashMap::new()),
            refunds: Arc::new(AtomicUsize::new(0)),
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
        // Calls through the client now book their own slots; an acquire must
        // not book a second one for the same provider call.
        self.per_call.store(true, Ordering::Release);
        Limited {
            client,
            throttle,
            limits: Arc::clone(self),
        }
    }

    /// What an acquire of the row waits for: a permit, as [`ready`](Self::ready),
    /// or, once a client has been [`wrap`](Self::wrap)ped, only the end of a
    /// pause. The wrapped client's calls book the quota one per provider
    /// call; booking at acquire as well would count every call twice.
    ///
    /// # Errors
    ///
    /// As [`ready`](Self::ready).
    pub(crate) async fn ready_to_acquire(
        &self,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), Error> {
        if self.per_call.load(Ordering::Acquire) {
            return self.wait_pause_only(deadline).await;
        }
        self.ready(deadline).await?;
        // The resource may wrap its client while creating for this very
        // acquire: its first call then uses the permit booked here rather
        // than booking a second one. One such credit at most, so a late
        // first wrap can never release a burst of unbooked calls.
        self.prepaid.store(true, Ordering::Release);
        Ok(())
    }

    /// What a call through a [`Limited`] client waits for: a permit, unless
    /// the acquire that preceded the first wrap already booked it, in which
    /// case only a pause.
    async fn ready_to_call(&self, deadline: Option<std::time::Instant>) -> Result<(), Error> {
        if self.prepaid.swap(false, Ordering::AcqRel) {
            return self.wait_pause_only(deadline).await;
        }
        self.ready(deadline).await
    }

    /// Waits out a local pause only, never past `deadline`, and refuses a
    /// deadline that has passed (before or after the wait).
    async fn wait_pause_only(&self, deadline: Option<std::time::Instant>) -> Result<(), Error> {
        let overran = || {
            deadline
                .is_some_and(|deadline| std::time::Instant::now() > deadline)
                .then(|| {
                    self.tagged(Error::exhausted(
                        "rate limit wait overran the deadline",
                        None,
                    ))
                })
        };
        if let Some(error) = overran() {
            return Err(error);
        }
        let pause = self.pause_remaining();
        if pause.is_zero() {
            return Ok(());
        }
        if pause > max_wait_until(deadline) {
            return Err(self.paused_past_deadline(pause));
        }
        let _waiting = Waiting::start(self);
        tokio::time::sleep(pause).await;
        overran().map_or(Ok(()), Err)
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
    /// The deadline is absolute: time spent reaching a shared store counts
    /// against it, and a slot that no longer fits once the store has
    /// answered is returned rather than waited for. A pause recorded while
    /// the caller waits still holds when it wakes.
    ///
    /// # Cancel safety
    ///
    /// Cancelling during the wait forfeits the booked slot: the limiter errs
    /// on sending less, never more.
    pub async fn ready(&self, deadline: Option<std::time::Instant>) -> Result<(), Error> {
        loop {
            let wait = self.account_slot(deadline).await?;
            if self.wait_out(wait, None, deadline).await? == Admission::Proceed {
                return Ok(());
            }
        }
    }

    /// Waits for one permit of the per-key limit `dimension` for `value` —
    /// one chat, one recipient — and one of the account limit, never past
    /// `deadline`.
    ///
    /// # Errors
    ///
    /// As [`ready`](Self::ready), and a permanent error when the resource
    /// declares no per-key limit named `dimension`.
    ///
    /// # Cancel safety
    ///
    /// As [`ready`](Self::ready).
    pub async fn ready_for(
        &self,
        dimension: &str,
        value: impl fmt::Display,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), Error> {
        let keyed = self.keyed_limits()?;
        let (key, rate) = keyed
            .limit_for(dimension, &value.to_string())
            .map_err(|error| self.tagged(error))?;
        // A keyed call books the account itself (aligned with the key), so a
        // cold acquire's credit is dropped rather than left for a later
        // call: using it there too would count one account permit twice.
        // Dropping it errs on sending less.
        self.prepaid.store(false, Ordering::Release);
        // Registered before booking, so a pause recorded after this caller's
        // slot was booked still reaches it when it wakes.
        let _interest = KeyInterest::start(self, &key);
        loop {
            let wait = if let Some(quota) = self.quota.as_ref().filter(|quota| {
                std::ptr::addr_eq(Arc::as_ptr(&quota.store), Arc::as_ptr(&keyed.store))
            }) {
                self.aligned_slots(quota, keyed, &key, &rate, deadline)
                    .await?
            } else if self.quota.is_none() {
                // No account rate, only its pauses: a pause is waited out
                // before the key's slot is booked, so the key's calls keep
                // their spacing instead of all running as the pause ends.
                let pause = self.account_slot(deadline).await?;
                if !pause.is_zero() {
                    let _waiting = Waiting::start(self);
                    tokio::time::sleep(pause).await;
                    continue;
                }
                self.book(&keyed.store, &key, &rate, deadline, 0)
                    .await?
                    .wait
            } else {
                let key_slot = self.book(&keyed.store, &key, &rate, deadline, 0).await?;
                match self.account_slot(deadline).await {
                    Ok(wait) => wait.max(key_slot.wait),
                    Err(error) => {
                        // The key's slot goes back while it is still the last
                        // one booked; otherwise it lapses unused, which errs on
                        // sending less.
                        self.release(&keyed.store, &key, &rate, key_slot);
                        return Err(error);
                    },
                }
            };
            if self.wait_out(wait, Some(&key), deadline).await? == Admission::Proceed {
                return Ok(());
            }
        }
    }

    /// Books one account slot and one slot of `key` at the same instant and
    /// returns the wait until it.
    ///
    /// Each limit only holds if the call runs at its own slot: running at
    /// the later of two independently booked slots would let calls whose
    /// earlier slots were spaced out run bunched together. So the later
    /// slot is taken as the time for both, and the earlier one is rebooked
    /// no earlier than it, until both land on one instant. A slot given up
    /// goes back while it is still the last one booked; otherwise it lapses
    /// unused, which errs on sending less.
    async fn aligned_slots(
        &self,
        quota: &Quota,
        keyed: &KeyedLimits,
        key: &LimitKey,
        rate: &Rate,
        deadline: Option<std::time::Instant>,
    ) -> Result<Duration, Error> {
        let mut key_slot = self.book(&keyed.store, key, rate, deadline, 0).await?;
        for _ in 0..MAX_ALIGN_ROUNDS {
            let account = match self
                .book(
                    &quota.store,
                    &quota.key,
                    &quota.rate,
                    deadline,
                    key_slot.allow_at,
                )
                .await
            {
                Ok(account) => account,
                Err(error) => {
                    self.release(&keyed.store, key, rate, key_slot);
                    return Err(error);
                },
            };
            if account.allow_at <= key_slot.allow_at {
                return Ok(account.wait);
            }
            // Given back before rebooking, while it is still the key's last
            // slot: the rebook then lands at the account's slot rather than
            // behind the slot it replaces.
            let key_refunded = matches!(
                within(deadline, keyed.store.cancel_boxed(key, rate, &key_slot)).await,
                Some(Ok(true))
            );
            let replaced = key_slot;
            key_slot = match self
                .book(&keyed.store, key, rate, deadline, account.allow_at)
                .await
            {
                Ok(key_slot) => key_slot,
                Err(error) => {
                    self.release(&quota.store, &quota.key, &quota.rate, account);
                    return Err(error);
                },
            };
            if key_slot.allow_at <= account.allow_at {
                return Ok(key_slot.wait);
            }
            // Neither slot could be given back, and the key moved past the
            // account again: both keys run on one schedule (a full in-process
            // store puts new keys on its shared overflow limit), so they
            // never meet. Run at the later slot, holding both: two slots for
            // one call errs on sending less, and each limit still holds.
            if !key_refunded && key_slot.allow_at > replaced.allow_at {
                let account_refunded = matches!(
                    within(
                        deadline,
                        quota.store.cancel_boxed(&quota.key, &quota.rate, &account),
                    )
                    .await,
                    Some(Ok(true))
                );
                if !account_refunded {
                    return Ok(key_slot.wait.max(account.wait));
                }
                continue;
            }
            let _ = within(
                deadline,
                quota.store.cancel_boxed(&quota.key, &quota.rate, &account),
            )
            .await;
        }
        self.release(&keyed.store, key, rate, key_slot);
        self.engage();
        Err(self.tagged(Error::exhausted(
            "rate limit: no instant both the account and the key allow",
            None,
        )))
    }

    fn keyed_limits(&self) -> Result<&KeyedLimits, Error> {
        self.keyed.as_ref().ok_or_else(|| {
            self.tagged(Error::permanent(
                "rate limit: the resource declares no per-key limits",
            ))
        })
    }

    /// The wait for one account permit: a slot of the quota, or the end of
    /// a local pause.
    async fn account_slot(&self, deadline: Option<std::time::Instant>) -> Result<Duration, Error> {
        if let Some(quota) = &self.quota {
            return self
                .book(&quota.store, &quota.key, &quota.rate, deadline, 0)
                .await
                .map(|grant| grant.wait);
        }
        let wait = self.pause_remaining();
        if wait > max_wait_until(deadline) {
            self.engage();
            return Err(self.paused_past_deadline(wait));
        }
        Ok(wait)
    }

    /// Waits out `wait`, then any pause recorded meanwhile for the account
    /// or for `key`: a caller that booked before a provider's "slow down"
    /// must not wake into it.
    ///
    /// After sleeping, pauses are read both from this process and from the
    /// limit store, so a penalty another worker recorded after this caller
    /// booked holds it back too. A caller that did not wait needs no such
    /// read: its slot was booked after any penalty already in the store.
    async fn wait_out(
        &self,
        wait: Duration,
        key: Option<&LimitKey>,
        deadline: Option<std::time::Instant>,
    ) -> Result<Admission, Error> {
        let pause_now = || {
            let key_pause = key.map_or(Duration::ZERO, |key| self.key_pause_remaining(key));
            self.pause_remaining().max(key_pause)
        };
        // The deadline may already have passed, before the store answered or
        // before a late wake-up: the call then does not run.
        let overran = || {
            deadline
                .is_some_and(|deadline| std::time::Instant::now() > deadline)
                .then(|| {
                    self.tagged(Error::exhausted(
                        "rate limit wait overran the deadline",
                        None,
                    ))
                })
        };
        if wait.is_zero() && pause_now().is_zero() {
            if let Some(error) = overran() {
                return Err(error);
            }
            if self.waiters.load(Ordering::Acquire) == 0 {
                self.clear();
            }
            return Ok(Admission::Proceed);
        }
        let _waiting = Waiting::start(self);
        tokio::time::sleep(wait).await;
        if let Some(error) = overran() {
            return Err(error);
        }
        // A penalty recorded after this caller booked, by this process or
        // any other sharing the store, holds it back as well.
        let Some(shared) = within(deadline, self.shared_penalty(key)).await else {
            return Err(self.tagged(Error::exhausted(
                "rate limit store did not answer before the deadline",
                None,
            )));
        };
        let pause = pause_now().max(shared?);
        if pause.is_zero() {
            return Ok(Admission::Proceed);
        }
        // The booked slot lapses unused, which errs on sending less.
        if pause > max_wait_until(deadline) {
            return Err(self.paused_past_deadline(pause));
        }
        // Waited out, then booked again: every caller that booked before the
        // pause would otherwise run the moment it ends, all together.
        tokio::time::sleep(pause).await;
        Ok(Admission::Rebook)
    }

    /// Time left of a penalty the limit store holds on the account quota or
    /// on `key`; zero for a limiter without a store.
    async fn shared_penalty(&self, key: Option<&LimitKey>) -> Result<Duration, Error> {
        let mut left = Duration::ZERO;
        if let Some(quota) = &self.quota {
            let penalty = quota
                .store
                .penalty_boxed(&quota.key)
                .await
                .map_err(|error| self.store_unavailable(&error))?;
            left = left.max(penalty);
        }
        if let (Some(key), Some(keyed)) = (key, &self.keyed) {
            let penalty = keyed
                .store
                .penalty_boxed(key)
                .await
                .map_err(|error| self.store_unavailable(&error))?;
            left = left.max(penalty);
        }
        Ok(left)
    }

    fn paused_past_deadline(&self, pause: Duration) -> Error {
        self.tagged(Error::exhausted(
            "resource paused past the deadline",
            Some(pause),
        ))
    }

    /// Books one slot of `key`, no earlier than `not_before` (a slot this
    /// store returned; `0` for none), that `deadline` still covers once the
    /// store has answered.
    async fn book(
        &self,
        store: &Arc<dyn ErasedLimitStore>,
        key: &LimitKey,
        rate: &Rate,
        deadline: Option<std::time::Instant>,
        not_before: u64,
    ) -> Result<Grant, Error> {
        let request = ReserveRequest::new(1, max_wait_until(deadline)).not_before(not_before);
        // Bounded by the deadline: a store slow to answer (a pool wait, a
        // row lock) does not keep the caller past it. A grant that lands
        // after the caller gave up lapses unused, which errs on sending less.
        let Some(decision) = within(deadline, store.reserve_boxed(key, rate, request)).await else {
            self.engage();
            return Err(self.tagged(Error::exhausted(
                "rate limit store did not answer before the deadline",
                None,
            )));
        };
        match decision {
            Ok(Ok(grant)) => {
                self.store_recovered();
                // The store call itself took time: a slot booked within the
                // budget measured before it can lie past the deadline now.
                // Return it rather than wait past the deadline.
                if grant.wait > max_wait_until(deadline) {
                    self.release(store, key, rate, grant);
                    self.engage();
                    return Err(self.tagged(Error::exhausted(
                        "rate limit exhausted before the deadline",
                        Some(grant.wait),
                    )));
                }
                Ok(grant)
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
        // Recorded locally first, before the store round trip, so callers of
        // this process already sleeping on an earlier booking wake no sooner
        // than the pause ends, even if the store is slow, fails, or this
        // call is cancelled.
        {
            let until = pause_deadline(tokio::time::Instant::now(), block);
            let mut paused_until = self
                .paused_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *paused_until = Some(paused_until.map_or(until, |current| current.max(until)));
        }
        self.engaged.store(true, Ordering::Relaxed);
        if let Some(quota) = &self.quota {
            quota
                .store
                .penalize_boxed(&quota.key, &quota.rate, retry_after, self.max_penalty)
                .await
                .map_err(|error| self.store_unavailable(&error))?;
        }
        self.emit(|key| ResourceEvent::RateLimitPenalized {
            key,
            retry_after: block,
        });
        Ok(())
    }

    /// Pauses every caller of the per-key limit `dimension` for `value` —
    /// one chat, not the whole account — for `retry_after`, capped at the
    /// policy's `max_penalty`.
    ///
    /// # Errors
    ///
    /// A permanent error when the resource declares no per-key limit named
    /// `dimension`;
    /// [`ErrorKind::Backpressure`](crate::ErrorKind::Backpressure) when the
    /// limit store cannot be reached.
    pub async fn penalize_for(
        &self,
        dimension: &str,
        value: impl fmt::Display,
        retry_after: Duration,
    ) -> Result<(), Error> {
        let keyed = self.keyed_limits()?;
        let (key, rate) = keyed
            .limit_for(dimension, &value.to_string())
            .map_err(|error| self.tagged(error))?;
        // Local first, as in `penalize`, and kept while the store records it:
        // this call counts as a caller of the key meanwhile, so one arriving
        // before the store has the penalty still sees it here.
        let block = retry_after.min(self.max_penalty);
        let _recording = KeyInterest::start(self, &key);
        self.pause_key(&key, block);
        // Unless the store confirms it, the pause is kept here until it ends:
        // on a store error, and equally when this call is cancelled while the
        // write is pending (the write may then be rolled back).
        let mut unconfirmed = KeepUnlessConfirmed {
            limiter: self,
            key: &key,
            block,
            confirmed: false,
        };
        if let Err(error) = keyed
            .store
            .penalize_boxed(&key, &rate, retry_after, self.max_penalty)
            .await
        {
            return Err(self.store_unavailable(&error));
        }
        unconfirmed.confirmed = true;
        self.emit(|key| ResourceEvent::RateLimitPenalized {
            key,
            retry_after: retry_after.min(self.max_penalty),
        });
        Ok(())
    }

    /// Records the verdict on one call made under `key` (a per-key limit's
    /// dimension and value, if the call named one): a pause on a provider's
    /// "slow down", a backoff reset otherwise.
    ///
    /// Without a `retry_after` the pause backs off exponentially over
    /// consecutive refusals, from one second up to the cap. A failure to
    /// record the pause is logged; it never masks the call's own outcome.
    async fn report(&self, verdict: Verdict, key: Option<(&str, &str)>) {
        // Consecutive refusals are counted per quota: the account's, and each
        // key's apart, so one chat's refusals never lengthen another's pause.
        let limit_key = key.and_then(|(dimension, value)| {
            let (limit_key, _) = self.keyed.as_ref()?.limit_for(dimension, value).ok()?;
            Some(limit_key)
        });
        let (retry_after, on_key) = match verdict {
            Verdict::Pass => {
                self.refusals.store(0, Ordering::Relaxed);
                if let Some(limit_key) = &limit_key {
                    self.key_refusals
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(limit_key);
                }
                return;
            },
            Verdict::Throttled { retry_after } => (retry_after, false),
            Verdict::KeyThrottled { retry_after } => (retry_after, true),
        };
        let penalized_key = key.filter(|_| on_key);
        let refusals = match (penalized_key, &limit_key) {
            (Some(_), Some(limit_key)) => self.key_refused(limit_key),
            _ => self.refusals.fetch_add(1, Ordering::Relaxed),
        };
        let block = retry_after.unwrap_or_else(|| backoff(refusals));
        let recorded = match penalized_key {
            Some((dimension, value)) => self.penalize_for(dimension, value, block).await,
            None => self.penalize(block).await,
        };
        if let Err(error) = recorded {
            tracing::debug!(
                target: "nebula_resource::rate_limit",
                %error,
                "could not record a provider refusal"
            );
        }
    }

    /// Gives `grant`'s slot back in the background, best effort: it returns
    /// while it is still the last one booked and otherwise lapses unused,
    /// and the caller never waits on the store for it.
    ///
    /// Bounded, so a stalled store cannot pile refunds up behind it: at most
    /// [`MAX_PENDING_REFUNDS`] run at once, each for at most
    /// [`REFUND_BUDGET`]; past either, the slot lapses instead.
    fn release(
        &self,
        store: &Arc<dyn ErasedLimitStore>,
        key: &LimitKey,
        rate: &Rate,
        grant: Grant,
    ) {
        let reserved = self
            .refunds
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |running| {
                (running < MAX_PENDING_REFUNDS).then_some(running + 1)
            });
        if reserved.is_err() {
            return;
        }
        let (store, key, rate, refunds) = (
            Arc::clone(store),
            key.clone(),
            *rate,
            Arc::clone(&self.refunds),
        );
        tokio::spawn(async move {
            let _ =
                tokio::time::timeout(REFUND_BUDGET, store.cancel_boxed(&key, &rate, &grant)).await;
            refunds.fetch_sub(1, Ordering::AcqRel);
        });
    }

    /// Counts one more refusal of `key` and returns the count before it.
    ///
    /// Every key keeps its own run; a pass ends it. At most
    /// [`MAX_KEY_REFUSAL_RUNS`] runs are kept: a new key past that replaces
    /// the one refused longest ago.
    fn key_refused(&self, key: &LimitKey) -> u32 {
        let now = tokio::time::Instant::now();
        let mut runs = self
            .key_refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !runs.contains_key(key)
            && runs.len() >= MAX_KEY_REFUSAL_RUNS
            && let Some(oldest) = runs
                .iter()
                .min_by_key(|(_, (_, last))| *last)
                .map(|(oldest, _)| oldest.clone())
        {
            runs.remove(&oldest);
        }
        let (count, last) = runs.entry(key.clone()).or_insert((0, now));
        let before = *count;
        *count = count.saturating_add(1);
        *last = now;
        before
    }

    /// Records a key's pause for the callers of this limiter booking or
    /// waiting on it right now, so none wakes before it ends. With no such
    /// caller there is nothing to keep: later bookings see the pause in the
    /// store.
    fn pause_key(&self, key: &LimitKey, block: Duration) {
        let until = pause_deadline(tokio::time::Instant::now(), block);
        let mut waits = self
            .key_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(wait) = waits.get_mut(key) {
            wait.paused_until = Some(
                wait.paused_until
                    .map_or(until, |current| current.max(until)),
            );
        }
    }

    /// Keeps a key's pause in this process until it ends, for a pause the
    /// store could not record: callers arriving later would otherwise book
    /// through the store unaware of it.
    fn keep_key_pause(&self, key: &LimitKey, block: Duration) {
        let now = tokio::time::Instant::now();
        let until = pause_deadline(now, block);
        let mut waits = self
            .key_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        waits.retain(|_, wait| wait.needed(now));
        let kept = waits.values().filter(|wait| wait.kept).count();
        if kept >= MAX_KEPT_KEY_PAUSES && !waits.get(key).is_some_and(|wait| wait.kept) {
            drop(waits);
            let mut paused_until = self
                .paused_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *paused_until = Some(paused_until.map_or(until, |current| current.max(until)));
            return;
        }
        let wait = waits.entry(key.clone()).or_default();
        wait.kept = true;
        wait.paused_until = Some(
            wait.paused_until
                .map_or(until, |current| current.max(until)),
        );
    }

    /// Takes over the pauses `previous` kept in this process: the account
    /// pause and every key pause kept past its callers. For a limiter that
    /// replaces another on the same row (a re-registration), so the
    /// replacement does not forget a provider's "slow down" that only this
    /// process knows. Pauses held in a limit store need nothing: the new
    /// limiter reads the same store.
    pub(crate) fn inherit_pauses(&self, previous: &Self) {
        let now = tokio::time::Instant::now();
        let account = *previous
            .paused_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(until) = account.filter(|until| *until > now) {
            let mut paused_until = self
                .paused_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *paused_until = Some(paused_until.map_or(until, |current| current.max(until)));
        }
        let kept: Vec<(LimitKey, tokio::time::Instant)> = previous
            .key_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, wait)| wait.kept)
            .filter_map(|(key, wait)| {
                wait.paused_until
                    .filter(|until| *until > now)
                    .map(|until| (key.clone(), until))
            })
            .collect();
        for (key, until) in kept {
            self.keep_key_pause(&key, until.saturating_duration_since(now));
        }
    }

    fn key_pause_remaining(&self, key: &LimitKey) -> Duration {
        let waits = self
            .key_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        waits
            .get(key)
            .and_then(|wait| wait.paused_until)
            .map_or(Duration::ZERO, |until| {
                until.saturating_duration_since(tokio::time::Instant::now())
            })
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
    /// The provider asked to slow down for the one key the call named — a
    /// chat's own flood limit — so only that key pauses
    /// ([`Limited::run_for`]). A call that named no key pauses the quota, as
    /// for [`Throttled`](Self::Throttled).
    KeyThrottled {
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
            .ready_to_call(deadline)
            .await
            .map_err(LimitedError::Limit)?;
        let outcome = call(&self.client).await;
        self.limits
            .report(self.throttle.check(&outcome), None)
            .await;
        outcome.map_err(LimitedError::Call)
    }

    /// Runs one call addressed to `value` of a per-key limit — `run_for(
    /// "chat_id", chat_id, …)` — under both that key's limit and the account
    /// limit, waiting for both as long as needed. A
    /// [`Verdict::KeyThrottled`] pauses only this key.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run); a permanent [`LimitedError::Limit`] when the
    /// resource declares no per-key limit named `dimension`.
    pub async fn run_for<R, E>(
        &self,
        dimension: &str,
        value: impl fmt::Display,
        call: impl AsyncFnOnce(&C) -> Result<R, E>,
    ) -> Result<R, LimitedError<E>>
    where
        T: Throttle<R, E>,
    {
        self.run_for_until(dimension, value, None, call).await
    }

    /// As [`run_for`](Self::run_for), never waiting past `deadline`.
    ///
    /// # Errors
    ///
    /// As [`run_for`](Self::run_for); a permit past `deadline` is
    /// [`LimitedError::Limit`] with `Exhausted` and a `retry_after`.
    pub async fn run_for_until<R, E>(
        &self,
        dimension: &str,
        value: impl fmt::Display,
        deadline: Option<std::time::Instant>,
        call: impl AsyncFnOnce(&C) -> Result<R, E>,
    ) -> Result<R, LimitedError<E>>
    where
        T: Throttle<R, E>,
    {
        let value = value.to_string();
        self.limits
            .ready_for(dimension, &value, deadline)
            .await
            .map_err(LimitedError::Limit)?;
        let outcome = call(&self.client).await;
        self.limits
            .report(self.throttle.check(&outcome), Some((dimension, &value)))
            .await;
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

/// `now + block`, saturating to a far-future instant: an author's
/// `max_penalty` may be as large as `Duration::MAX`.
fn pause_deadline(now: tokio::time::Instant, block: Duration) -> tokio::time::Instant {
    tokio::time::Instant::from_std(crate::deadline::deadline_after(
        now.into_std(),
        block,
        crate::deadline::UNBOUNDED_HORIZON,
    ))
}

/// Rebooking rounds before [`ResourceLimiter::aligned_slots`] gives up. Each
/// round moves both slots later, so under contention they meet within a
/// round or two; the bound only keeps a pathological store from spinning.
const MAX_ALIGN_ROUNDS: usize = 8;

/// `future`, abandoned at `deadline` (`None` when it did not finish by
/// then); unbounded without a deadline.
async fn within<T>(
    deadline: Option<std::time::Instant>,
    future: impl Future<Output = T>,
) -> Option<T> {
    match deadline {
        None => Some(future.await),
        Some(deadline) => tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
            .await
            .ok(),
    }
}

fn max_wait_until(deadline: Option<std::time::Instant>) -> Duration {
    deadline.map_or(Duration::MAX, |deadline| {
        deadline.saturating_duration_since(std::time::Instant::now())
    })
}

/// Block after the `refusals`-th consecutive provider refusal that carried no
/// `retry_after`: 1 s, 2 s, 4 s, … (the policy cap applies on top).
///
/// Jittered over the upper half of each step ("equal jitter"), so workers
/// and quotas refused at the same moment do not all resume on the same
/// power-of-two boundary and hit the provider together.
fn backoff(refusals: u32) -> Duration {
    let step = Duration::from_secs(1u64 << refusals.min(20));
    let half = step / 2;
    half + half.mul_f64(fastrand::f64())
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
