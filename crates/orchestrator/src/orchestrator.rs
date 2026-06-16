//! Capability-routed job-dispatch pull loop.
//!
//! [`Orchestrator`] mirrors the shape of `ControlConsumer` in `nebula-engine`:
//! biased `tokio::select!`, `MissedTickBehavior::Delay` with first-tick skip,
//! claim-deadline held across the select so reclaim ticks don't reset backoff,
//! per-row failure isolation, and optional [`MetricsRegistry`] injection via
//! the builder.
//!
//! ## Shutdown contract
//!
//! When [`CancellationToken`] is cancelled the orchestrator flushes the
//! in-flight batch already being processed, then returns. It does **not** begin
//! a fresh [`JobDispatchQueue::claim_pending`] once shutdown is requested.
//! Rows claimed but not yet marked remain in `Processing` and are recovered
//! by the next runner's reclaim sweep.
//!
//! Worst-case shutdown observability latency is bounded by
//! `max(one reclaim_stuck() sweep, batch_size × one sink.dispatch() latency)`,
//! because `tick()` / `sweep_reclaim()` run in `select!` arm bodies and
//! shutdown is only observed on the next loop iteration.
//!
//! [`CancellationToken`]: tokio_util::sync::CancellationToken
//! [`MetricsRegistry`]: nebula_metrics::MetricsRegistry
//! [`JobDispatchQueue::claim_pending`]: nebula_storage_port::store::JobDispatchQueue::claim_pending

use std::{sync::Arc, time::Duration};

use nebula_core::PluginKey;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, NEBULA_ORCHESTRATOR_RECLAIM_TOTAL,
        orchestrator_dispatch_outcome, orchestrator_reclaim_outcome,
    },
};
use nebula_storage_port::dto::JobDispatchMsg;
use nebula_storage_port::store::{JobDispatchQueue, ReclaimOutcome};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::sink::ExecutionSink;

/// Default claim batch size.
///
/// Mirrors `ControlConsumer::DEFAULT_BATCH_SIZE` in `nebula-engine`: small
/// enough that a slow sink does not block many rows from operator visibility;
/// large enough to avoid per-row round-trips on a busy queue.
pub const DEFAULT_BATCH_SIZE: u32 = 32;

/// Default idle poll interval (queue empty).
///
/// Matches `ControlConsumer` — short enough for interactive latency on the
/// local path; the Postgres path may shorten further once `LISTEN/NOTIFY` is
/// wired as an optimisation.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum backoff between `claim_pending` retries after repeated storage
/// errors. Prevents a high-frequency error-log flood when the backend is down.
pub const MAX_CLAIM_ERROR_BACKOFF: Duration = Duration::from_secs(30);

/// Default staleness window before a `Processing` row is reclaimable.
///
/// Matches `ControlConsumer` — 5× a 30-second lease TTL so a runner that
/// has missed 15 heartbeats is presumed dead.
pub const DEFAULT_RECLAIM_AFTER: Duration = Duration::from_secs(150);

/// Default cadence of the reclaim sweep.
pub const DEFAULT_RECLAIM_INTERVAL: Duration = Duration::from_secs(30);

/// Default retry budget before a reclaim-eligible row moves to `Failed`.
pub const DEFAULT_MAX_RECLAIM_COUNT: u32 = 3;

/// Capability-routed job-dispatch pull loop (ADR-0095).
///
/// Claims [`JobDispatchQueue`] rows whose `required_plugins ⊆ available_plugins`,
/// hands each to an [`ExecutionSink`], and fences the row dispatched or failed.
/// A periodic sweep reclaims rows stuck in `Processing` after a crashed runner.
///
/// Construct with [`Orchestrator::new`] and optional builder methods, then
/// call [`Orchestrator::run`] (or [`Orchestrator::spawn`]).
///
/// [`JobDispatchQueue`]: nebula_storage_port::store::JobDispatchQueue
#[must_use = "call .spawn() or .run() to start the pull loop"]
pub struct Orchestrator {
    queue: Arc<dyn JobDispatchQueue>,
    sink: Arc<dyn ExecutionSink>,
    /// Fixed 16-byte fence token recorded in `processed_by` and matched on
    /// `mark_dispatched` / `mark_failed`. Typed `[u8; 16]` end-to-end — no
    /// truncate/pad of an arbitrary-length id, which would let two distinct
    /// workers collapse to the same token and ack each other's rows.
    processor_id: [u8; 16],
    available_plugins: Vec<PluginKey>,
    batch_size: u32,
    poll_interval: Duration,
    reclaim_after: Duration,
    reclaim_interval: Duration,
    max_reclaim_count: u32,
    /// Shared metrics registry. Defaults to a private fresh registry so the
    /// orchestrator is always emit-safe without injection; production
    /// composition roots inject the shared registry via [`with_metrics`] so
    /// counters reach the Prometheus scrape endpoint.
    ///
    /// [`with_metrics`]: Self::with_metrics
    metrics: MetricsRegistry,
}

impl Orchestrator {
    /// Construct an orchestrator.
    ///
    /// `processor_id` is the fixed 16-byte fence token recorded in the row's
    /// `processed_by`. Supply the full id bytes — no truncation or padding is
    /// done, which would let two distinct workers collapse to the same token.
    pub fn new(
        queue: Arc<dyn JobDispatchQueue>,
        sink: Arc<dyn ExecutionSink>,
        processor_id: [u8; 16],
        available_plugins: Vec<PluginKey>,
    ) -> Self {
        Self {
            queue,
            sink,
            processor_id,
            available_plugins,
            batch_size: DEFAULT_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
            reclaim_after: DEFAULT_RECLAIM_AFTER,
            reclaim_interval: DEFAULT_RECLAIM_INTERVAL,
            max_reclaim_count: DEFAULT_MAX_RECLAIM_COUNT,
            metrics: MetricsRegistry::new(),
        }
    }

    /// Override the claim batch size. Default: [`DEFAULT_BATCH_SIZE`].
    pub fn with_batch_size(mut self, n: u32) -> Self {
        self.batch_size = n;
        self
    }

    /// Override the idle poll interval. Default: [`DEFAULT_POLL_INTERVAL`].
    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    /// Override the staleness window before a row is reclaimable.
    /// Default: [`DEFAULT_RECLAIM_AFTER`].
    pub fn with_reclaim_after(mut self, d: Duration) -> Self {
        self.reclaim_after = d;
        self
    }

    /// Override the cadence of the reclaim sweep tick.
    /// Default: [`DEFAULT_RECLAIM_INTERVAL`].
    pub fn with_reclaim_interval(mut self, d: Duration) -> Self {
        self.reclaim_interval = d;
        self
    }

    /// Override the max retry budget before a reclaim-eligible row moves to
    /// `Failed`. Default: [`DEFAULT_MAX_RECLAIM_COUNT`].
    pub fn with_max_reclaim_count(mut self, n: u32) -> Self {
        self.max_reclaim_count = n;
        self
    }

    /// Inject the shared [`MetricsRegistry`] the orchestrator emits counters
    /// against. Without this the counters increment against a private registry
    /// no scraper sees.
    pub fn with_metrics(mut self, m: MetricsRegistry) -> Self {
        self.metrics = m;
        self
    }

    /// Spawn the orchestrator as a Tokio task. Returns a [`JoinHandle`] that
    /// completes when `shutdown` is cancelled.
    ///
    /// ## Shutdown contract
    ///
    /// The orchestrator flushes the in-flight batch already being processed,
    /// then returns; it does not begin a fresh claim once shutdown is
    /// requested. Rows claimed but not yet marked remain in `Processing` and
    /// are recovered by the next runner's reclaim sweep.
    pub fn spawn(self, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move { self.run(shutdown).await })
    }

    /// Run the pull loop on the current task. Exits when `shutdown` is
    /// cancelled. Prefer [`spawn`](Self::spawn) unless integrating into a
    /// custom task structure.
    ///
    /// ## Shutdown contract
    ///
    /// The orchestrator flushes the in-flight batch already being processed,
    /// then returns; it does not begin a fresh claim once shutdown is
    /// requested. Rows claimed but not yet marked remain in `Processing` and
    /// are recovered by the next runner's reclaim sweep.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            processor = %hex_display(&self.processor_id),
            batch_size = self.batch_size,
            poll_ms = self.poll_interval.as_millis() as u64,
            reclaim_after_ms = self.reclaim_after.as_millis() as u64,
            reclaim_interval_ms = self.reclaim_interval.as_millis() as u64,
            max_reclaim_count = self.max_reclaim_count,
            available_plugins = ?self.available_plugins.iter().map(PluginKey::as_str).collect::<Vec<_>>(),
            "orchestrator started (ADR-0095)"
        );

        let mut consecutive_errors: u32 = 0;
        let mut reclaim_ticker = tokio::time::interval(self.reclaim_interval);
        // Skip the immediate first tick — nothing is stuck yet and the first
        // `claim_pending` call has priority. Mirrors ControlConsumer.
        reclaim_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let _ = reclaim_ticker.tick().await;

        // Hold `claim_deadline` across the select so a reclaim tick does not
        // reset the backoff / idle-poll clock.
        let mut claim_deadline = tokio::time::Instant::now();

        loop {
            let claim_sleep = tokio::time::sleep_until(claim_deadline);
            tokio::pin!(claim_sleep);

            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!(
                        processor = %hex_display(&self.processor_id),
                        "orchestrator shutting down"
                    );
                    return;
                }
                _ = reclaim_ticker.tick() => {
                    self.sweep_reclaim().await;
                    // `claim_deadline` is preserved — reclaim does not reset
                    // the backoff or idle poll delay.
                }
                () = &mut claim_sleep => {
                    let next_delay = self.tick(&mut consecutive_errors).await;
                    claim_deadline = tokio::time::Instant::now()
                        + next_delay.unwrap_or(Duration::ZERO);
                }
            }
        }
    }

    /// Run a single reclaim sweep.
    ///
    /// Storage errors are logged and swallowed — a transient failure on one
    /// sweep should not abort the loop; the next tick will retry.
    async fn sweep_reclaim(&self) {
        let swept: Result<ReclaimOutcome, String> = self
            .queue
            .reclaim_stuck(self.reclaim_after, self.max_reclaim_count)
            .await
            .map_err(|e| e.to_string());

        match swept {
            Ok(outcome) => {
                if outcome.reclaimed > 0 {
                    let labels = self
                        .metrics
                        .interner()
                        .single("outcome", orchestrator_reclaim_outcome::RECLAIMED);
                    if let Ok(c) = self
                        .metrics
                        .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &labels)
                    {
                        c.inc_by(outcome.reclaimed);
                    }
                }
                if outcome.exhausted > 0 {
                    let labels = self
                        .metrics
                        .interner()
                        .single("outcome", orchestrator_reclaim_outcome::EXHAUSTED);
                    if let Ok(c) = self
                        .metrics
                        .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &labels)
                    {
                        c.inc_by(outcome.exhausted);
                    }
                }

                if outcome.reclaimed > 0 || outcome.exhausted > 0 {
                    tracing::warn!(
                        processor = %hex_display(&self.processor_id),
                        reclaimed = outcome.reclaimed,
                        exhausted = outcome.exhausted,
                        reclaim_after_ms = self.reclaim_after.as_millis() as u64,
                        "orchestrator reclaim sweep recovered stuck rows (ADR-0095)"
                    );
                } else {
                    tracing::debug!(
                        processor = %hex_display(&self.processor_id),
                        "orchestrator reclaim sweep: no stuck rows"
                    );
                }
            },
            Err(e) => {
                tracing::error!(
                    processor = %hex_display(&self.processor_id),
                    error = %e,
                    "orchestrator reclaim sweep failed; will retry next tick"
                );
            },
        }
    }

    /// Drain a single batch. Returns the duration to wait before the next
    /// claim attempt, or `None` for immediate re-claim.
    async fn tick(&self, consecutive_errors: &mut u32) -> Option<Duration> {
        let claimed: Result<Vec<JobDispatchMsg>, String> = self
            .queue
            .claim_pending(&self.processor_id, self.batch_size, &self.available_plugins)
            .await
            .map_err(|e| e.to_string());

        let claimed = match claimed {
            Ok(rows) => {
                *consecutive_errors = 0;
                rows
            },
            Err(e) => {
                *consecutive_errors = consecutive_errors.saturating_add(1);
                let backoff = claim_error_backoff(self.poll_interval, *consecutive_errors);
                tracing::error!(
                    error = %e,
                    consecutive_errors = *consecutive_errors,
                    backoff_ms = backoff.as_millis() as u64,
                    "orchestrator claim_pending failed; backing off"
                );
                return Some(backoff);
            },
        };

        if claimed.is_empty() {
            return Some(self.poll_interval);
        }

        for msg in claimed {
            self.handle_entry(msg).await;
        }
        None
    }

    async fn handle_entry(&self, msg: JobDispatchMsg) {
        // The queue routing predicate (`required_plugins ⊆ available_plugins`)
        // is enforced at claim time. This assert checks the implied single-key
        // condition (`required_plugin_key ∈ available_plugins`, since
        // `required_plugin_key ∈ required_plugins` by the DTO invariant) in
        // debug builds only — it is not a release guard.
        debug_assert!(
            self.available_plugins.contains(&msg.required_plugin_key),
            "claim routing invariant violated: required_plugin_key {:?} not in available_plugins {:?}",
            msg.required_plugin_key,
            self.available_plugins
        );

        let row_id = msg.id;
        let has_w3c = msg.w3c_traceparent.is_some();

        let span = tracing::info_span!(
            "orchestrator.dispatch",
            execution_id = %msg.execution_id,
            required_plugin_key = %msg.required_plugin_key,
            command = msg.command.as_str(),
            reclaim_count = msg.reclaim_count,
            row_has_w3c = has_w3c,
        );

        // Attach W3C parent if present — same non-fatal policy as
        // ControlConsumer: malformed carrier → warn and dispatch as root span.
        if let Some(ref tp) = msg.w3c_traceparent {
            match nebula_core::W3cTraceContext::from_traceparent_str(tp) {
                Ok(w3c) => attach_w3c_parent(&span, &w3c),
                Err(e) => {
                    tracing::warn!(
                        target: "nebula_orchestrator",
                        row_id = %hex_display(&row_id),
                        error = %e,
                        "orchestrator row has a malformed w3c traceparent; \
                         dispatching without trace linkage"
                    );
                },
            }
        }

        let sink = Arc::clone(&self.sink);
        let dispatch_result = sink.dispatch(&msg).instrument(span).await;

        match dispatch_result {
            Ok(()) => {
                self.mark_dispatched(&row_id).await;
                let labels = self
                    .metrics
                    .interner()
                    .single("outcome", orchestrator_dispatch_outcome::DISPATCHED);
                if let Ok(c) = self
                    .metrics
                    .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &labels)
                {
                    c.inc();
                }
            },
            Err(ref e) => {
                tracing::error!(
                    row_id = %hex_display(&row_id),
                    execution_id = %msg.execution_id,
                    command = msg.command.as_str(),
                    error = %e,
                    "orchestrator dispatch failed; marking row failed (ADR-0095)"
                );
                self.mark_failed(&row_id, &e.to_string()).await;
                let labels = self
                    .metrics
                    .interner()
                    .single("outcome", orchestrator_dispatch_outcome::FAILED);
                if let Ok(c) = self
                    .metrics
                    .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &labels)
                {
                    c.inc();
                }
            },
        }
    }

    async fn mark_dispatched(&self, id: &[u8; 16]) {
        // If `mark_dispatched` fails, the row stays in `Processing` and the
        // reclaim sweep redelivers. Correctness under redelivery requires the
        // `ExecutionSink` to be idempotent per `(execution_id, command)`.
        if let Err(e) = self
            .queue
            .mark_dispatched(id, &self.processor_id)
            .await
            .map_err(|e| e.to_string())
        {
            tracing::error!(
                row_id = %hex_display(id),
                error = %e,
                "orchestrator mark_dispatched failed; row left in Processing for reclaim"
            );
        }
    }

    async fn mark_failed(&self, id: &[u8; 16], reason: &str) {
        if let Err(e) = self
            .queue
            .mark_failed(id, &self.processor_id, reason)
            .await
            .map_err(|e| e.to_string())
        {
            tracing::error!(
                row_id = %hex_display(id),
                error = %e,
                "orchestrator mark_failed failed; row left in Processing for reclaim"
            );
        }
    }
}

/// Attach the remote OTel parent from `w3c` to `span`.
///
/// Mirrors `control_trace::attach_control_queue_w3c_parent` from `nebula-engine`
/// without importing that crate (layer boundary). Invalid carriers leave the
/// span as a root — same non-fatal policy as the HTTP edge.
fn attach_w3c_parent(span: &tracing::Span, w3c: &nebula_core::W3cTraceContext) {
    use opentelemetry::global;
    use opentelemetry::propagation::Extractor;
    use opentelemetry::trace::TraceContextExt;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    struct W3cExtractor<'a> {
        traceparent: &'a str,
        tracestate: Option<&'a str>,
    }

    impl Extractor for W3cExtractor<'_> {
        fn get(&self, key: &str) -> Option<&str> {
            if key.eq_ignore_ascii_case("traceparent") {
                return Some(self.traceparent);
            }
            if key.eq_ignore_ascii_case("tracestate") {
                return self.tracestate;
            }
            None
        }

        fn keys(&self) -> Vec<&str> {
            if self.tracestate.is_some() {
                vec!["traceparent", "tracestate"]
            } else {
                vec!["traceparent"]
            }
        }
    }

    let parent_ctx = global::get_text_map_propagator(|prop| {
        prop.extract(&W3cExtractor {
            traceparent: w3c.traceparent(),
            tracestate: w3c.tracestate(),
        })
    });

    // Borrow parent_ctx via .span()/.span_context(), then clone the SpanContext
    // (a Clone-cheap struct) to get an owned value. The borrow ends at the
    // semicolon, so parent_ctx can be moved into set_parent below.
    let span_ctx = parent_ctx.span().span_context().clone();
    if span_ctx.is_valid() {
        match span.set_parent(parent_ctx) {
            Ok(()) => tracing::debug!(
                trace_id = %span_ctx.trace_id(),
                "orchestrator: linked dispatch span to W3C parent from job-dispatch row"
            ),
            Err(err) => tracing::warn!(
                trace_id = %span_ctx.trace_id(),
                error = ?err,
                "orchestrator: span.set_parent failed after carrier validation; dispatch span stays root"
            ),
        }
    } else {
        tracing::debug!(
            "orchestrator: W3C carrier on row did not yield valid OTel parent; dispatch span stays root"
        );
    }
}

/// Exponential backoff for repeated `claim_pending` storage errors.
///
/// Starts at `base` and doubles per consecutive error, capped at
/// [`MAX_CLAIM_ERROR_BACKOFF`]. `consecutive_errors` is 1-indexed.
///
/// Average case O(1); worst case O(1) (saturating arithmetic, fixed cap).
fn claim_error_backoff(base: Duration, consecutive_errors: u32) -> Duration {
    let multiplier = 1u64
        .checked_shl(consecutive_errors.saturating_sub(1).min(30))
        .unwrap_or(u64::MAX);
    let scaled = base
        .checked_mul(u32::try_from(multiplier.min(u64::from(u32::MAX))).unwrap_or(u32::MAX))
        .unwrap_or(MAX_CLAIM_ERROR_BACKOFF);
    scaled.min(MAX_CLAIM_ERROR_BACKOFF)
}

/// Hex-render opaque byte fields for structured logs.
fn hex_display(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_display_renders_bytes() {
        assert_eq!(hex_display(&[0x0a, 0xff, 0x00]), "0aff00");
    }

    #[test]
    fn claim_error_backoff_doubles_then_caps() {
        let base = Duration::from_millis(100);
        assert_eq!(claim_error_backoff(base, 1), Duration::from_millis(100));
        assert_eq!(claim_error_backoff(base, 2), Duration::from_millis(200));
        assert_eq!(claim_error_backoff(base, 3), Duration::from_millis(400));
        assert_eq!(claim_error_backoff(base, 4), Duration::from_millis(800));
        // Cap kicks in before overflow (100ms * 2^29 > 30s).
        assert_eq!(claim_error_backoff(base, 15), MAX_CLAIM_ERROR_BACKOFF);
        assert_eq!(claim_error_backoff(base, u32::MAX), MAX_CLAIM_ERROR_BACKOFF);
    }

    #[test]
    fn claim_error_backoff_zero_is_base() {
        // consecutive_errors == 0 never reached in practice (saturating_add
        // before call), but must be safe and return base.
        let base = Duration::from_millis(50);
        assert_eq!(claim_error_backoff(base, 0), base);
    }
}
