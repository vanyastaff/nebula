//! Base trigger trait, handler contract, and transport-agnostic event type.
//!
//! A trigger action is a workflow starter — it lives outside the execution
//! graph and emits new workflow executions in response to external events
//! (webhooks, cron, polling, etc.). The runtime drives the `start`/`stop`
//! lifecycle and receives workflow inputs through the trigger context.
//!
//! ## Shape peers
//!
//! Specialized trigger families live in their own files:
//!
//! - [`crate::webhook`] — push-based webhooks with HMAC signature verification
//! - [`crate::poll`] — pull-based periodic polling with cursor state
//!
//! Both register their adapters against the dyn-trait contract defined
//! here ([`TriggerHandler`]).
//!
//! ## Transport-agnostic event
//!
//! [`IncomingEvent`] is the type boundary between a transport (HTTP
//! webhook server, message-queue consumer, etc.) and an action. Its
//! fallible constructor enforces size and header-count limits at
//! construction — see [`DEFAULT_MAX_BODY_BYTES`] and
//! [`MAX_HEADER_COUNT`] for the defaults.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::action::Action;
use crate::context::TriggerContext;
use crate::error::{ActionError, ValidationReason};
use crate::metadata::ActionMetadata;

// ── Core trait ──────────────────────────────────────────────────────────────

/// Trigger action: workflow starter, lives outside the execution graph.
///
/// The runtime calls `start` to begin listening (e.g. webhook, poll); `stop`
/// to tear down. Triggers emit new workflow executions; they do not run
/// inside one.
///
/// Uses [`TriggerContext`] (workflow_id, trigger_id, cancellation), not
/// [`Context`](crate::context::Context).
pub trait TriggerAction: Action {
    /// Start the trigger (register listener, schedule poll, etc.).
    fn start(&self, ctx: &TriggerContext) -> impl Future<Output = Result<(), ActionError>> + Send;

    /// Stop the trigger (unregister, cancel schedule).
    fn stop(&self, ctx: &TriggerContext) -> impl Future<Output = Result<(), ActionError>> + Send;
}

// ── Incoming event ──────────────────────────────────────────────────────────

/// Default maximum webhook body size accepted by [`IncomingEvent::try_new`]: 1 MiB.
///
/// Rationale: covers 99% of real-world webhook payloads. Reference
/// providers:
/// - Stripe: 256 KB per event
/// - Slack: ~3 MB
/// - GitHub push events: typically < 100 KB (release artifacts can hit 25 MB)
/// - Twilio: 64 KB
///
/// For providers that need more, use
/// [`IncomingEvent::try_new_with_limits`] and pass an explicit cap —
/// we want oversize intake to be a deliberate decision visible in
/// code review, not an accident.
pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Maximum header count accepted by [`IncomingEvent::try_new`].
///
/// RFC 9110 does not mandate a limit; most HTTP servers cap at 100.
/// We allow 128 to leave headroom for providers that include many
/// tracing/forwarding headers, while preventing an O(n²) CPU DoS if
/// an action called `header()` in a loop against a huge attacker-
/// supplied header set.
pub const MAX_HEADER_COUNT: usize = 128;

/// External event delivered to a trigger (HTTP request, message, etc.).
///
/// Transport-agnostic: webhook layer constructs this from HTTP requests,
/// message queue layer constructs it from queue messages. The event is
/// a **type boundary** between transport and action — size and header
/// limits are enforced here at construction via
/// [`try_new`](Self::try_new) and [`try_new_with_limits`](Self::try_new_with_limits).
///
/// Header keys are normalized to ASCII lowercase at construction, so
/// [`header`](Self::header) is O(1). Duplicate keys collapse to the
/// last value (acceptable for webhook signature verification — nobody
/// signs `Set-Cookie`; if you need multi-valued headers, parse them
/// from the underlying transport before constructing the event).
///
/// `body` holds canonical raw bytes — use directly for HMAC signature
/// verification (do NOT round-trip through `body_json()` for signing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingEvent {
    /// Raw payload body. Canonical bytes for signature verification.
    pub body: Vec<u8>,
    /// Headers / metadata (HTTP headers, message attributes, etc.).
    /// Keys are stored lowercased for O(1) case-insensitive lookup.
    pub headers: HashMap<String, String>,
    /// Source identifier (URL path, topic name, queue name).
    #[serde(default)]
    pub source: String,
}

impl IncomingEvent {
    /// Construct an event with the [`DEFAULT_MAX_BODY_BYTES`] body cap
    /// and [`MAX_HEADER_COUNT`] header count cap.
    ///
    /// Header keys are normalized to ASCII lowercase at construction.
    ///
    /// For providers whose payloads exceed the defaults (e.g., GitHub
    /// release events up to 25 MB), use
    /// [`try_new_with_limits`](Self::try_new_with_limits).
    ///
    /// # Errors
    ///
    /// - [`ActionError::DataLimitExceeded`] if `body.len() > DEFAULT_MAX_BODY_BYTES`
    /// - [`ActionError::Validation`] if `headers.len() > MAX_HEADER_COUNT`
    pub fn try_new(body: &[u8], headers: &[(&str, &str)]) -> Result<Self, ActionError> {
        Self::try_new_with_limits(body, headers, DEFAULT_MAX_BODY_BYTES, MAX_HEADER_COUNT)
    }

    /// Construct with explicit limits. Prefer [`try_new`](Self::try_new)
    /// unless you know your provider exceeds the defaults.
    ///
    /// Header keys are normalized to ASCII lowercase at construction.
    ///
    /// # Errors
    ///
    /// - [`ActionError::DataLimitExceeded`] if `body.len() > max_body`
    /// - [`ActionError::Validation`] if `headers.len() > max_headers`
    pub fn try_new_with_limits(
        body: &[u8],
        headers: &[(&str, &str)],
        max_body: usize,
        max_headers: usize,
    ) -> Result<Self, ActionError> {
        if body.len() > max_body {
            return Err(ActionError::DataLimitExceeded {
                limit_bytes: max_body as u64,
                actual_bytes: body.len() as u64,
            });
        }
        if headers.len() > max_headers {
            return Err(ActionError::validation(
                "headers",
                ValidationReason::OutOfRange,
                Some(format!(
                    "too many headers on incoming event: {} > {max_headers}",
                    headers.len()
                )),
            ));
        }

        // Normalize keys to lowercase once so `header()` is O(1) and
        // case-insensitive lookup does not allocate per call.
        let mut map = HashMap::with_capacity(headers.len());
        for (k, v) in headers {
            map.insert(k.to_ascii_lowercase(), (*v).to_string());
        }

        Ok(Self {
            body: body.to_vec(),
            headers: map,
            source: String::new(),
        })
    }

    /// Set the source identifier.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Get a header value by key (ASCII case-insensitive, O(1)).
    ///
    /// Keys are normalized to lowercase at construction — HTTP headers
    /// are ASCII per RFC 9110. Fast path: when the caller already
    /// passes a lowercase key, lookup is allocation-free. Slow path:
    /// a single fold to lowercase for mixed-case callers, still O(1)
    /// vs the previous O(n) linear scan with per-entry
    /// `eq_ignore_ascii_case`.
    ///
    /// Duplicate keys collapse to the last value stored during
    /// construction — callers needing multi-valued header semantics
    /// must parse them from the underlying transport.
    #[must_use]
    pub fn header(&self, key: &str) -> Option<&str> {
        // Fast path: key is already all-lowercase (or has no ASCII
        // letters). Avoid the allocation from `to_ascii_lowercase`.
        if !key.bytes().any(|b| b.is_ascii_uppercase()) {
            self.headers.get(key).map(String::as_str)
        } else {
            self.headers
                .get(&key.to_ascii_lowercase())
                .map(String::as_str)
        }
    }

    /// Get the body as a UTF-8 string slice.
    #[must_use]
    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Parse the body as JSON.
    ///
    /// The body is already bounded by the construction-time limit (see
    /// [`try_new`](Self::try_new) / [`try_new_with_limits`](Self::try_new_with_limits)),
    /// so this call inherits that DoS protection without an additional
    /// byte-size guard.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the body is not valid JSON.
    pub fn body_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}

// ── Event outcome ───────────────────────────────────────────────────────────

/// Outcome of processing an external event pushed to a trigger.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TriggerEventOutcome {
    /// Event filtered out — no workflow execution.
    Skip,
    /// Emit a single workflow execution with this input payload.
    Emit(Value),
    /// Emit multiple workflow executions (batch webhook, fan-out).
    EmitMany(Vec<Value>),
}

impl TriggerEventOutcome {
    /// Create a skip outcome.
    #[must_use]
    pub fn skip() -> Self {
        Self::Skip
    }

    /// Create a single-emit outcome.
    #[must_use]
    pub fn emit(payload: Value) -> Self {
        Self::Emit(payload)
    }

    /// Create a batch-emit outcome. Empty vec becomes `Skip`.
    #[must_use]
    pub fn emit_many(payloads: Vec<Value>) -> Self {
        if payloads.is_empty() {
            Self::Skip
        } else {
            Self::EmitMany(payloads)
        }
    }

    /// Whether this outcome will emit any executions.
    ///
    /// Returns `false` for `Skip` and for `EmitMany(vec![])` constructed
    /// directly (the `emit_many` constructor normalizes empty vecs to `Skip`).
    #[must_use]
    pub fn will_emit(&self) -> bool {
        match self {
            Self::Skip => false,
            Self::Emit(_) => true,
            Self::EmitMany(v) => !v.is_empty(),
        }
    }
}

// ── Handler trait ───────────────────────────────────────────────────────────

/// Trigger handler — start/stop lifecycle for workflow triggers.
///
/// Uses [`TriggerContext`] (workflow_id, trigger_id, cancellation) instead
/// of [`ActionContext`](crate::context::ActionContext). Triggers live
/// outside the execution graph and emit new workflow executions.
///
/// # Errors
///
/// Returns [`ActionError`] if start or stop fails.
#[async_trait]
pub trait TriggerHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Start the trigger.
    ///
    /// # Semantics — two valid shapes
    ///
    /// Implementations fall into one of two categories:
    ///
    /// 1. **Setup-and-return** — register an external listener (webhook,
    ///    message queue consumer), then return immediately. The listener
    ///    runs asynchronously outside this call. Example:
    ///    [`crate::webhook::WebhookTriggerAdapter`].
    ///
    /// 2. **Run-until-cancelled** — run the entire trigger loop inline,
    ///    returning only when `ctx.cancellation` fires or a fatal error
    ///    occurs. Example: [`crate::poll::PollTriggerAdapter`].
    ///
    /// **Callers MUST spawn `start()` in a dedicated task** and must not
    /// assume it returns promptly. Calling sites that drive multiple
    /// triggers sequentially will deadlock on shape (2).
    ///
    /// # Lifecycle
    ///
    /// `start` must be paired with [`stop`](Self::stop). Calling `start`
    /// twice without an intervening `stop` returns [`ActionError::Fatal`] —
    /// implementations MUST NOT silently re-register, as this would leak
    /// external resources (webhook registrations, poll loops, schedules).
    ///
    /// # Cancel safety
    ///
    /// Implementations MUST be cancel-safe — dropping the returned
    /// future at any `.await` point MUST NOT leak external resources,
    /// corrupt internal state, or lose in-flight work beyond what the
    /// trigger's contract already tolerates (e.g., a poll trigger may
    /// lose one cycle of cursor progress, but must not leak its
    /// listener registration).
    ///
    /// Shape-1 (setup-and-return) implementations register the
    /// listener, then return. Cancel safety is trivially satisfied
    /// because no `.await` sits on user state after registration.
    /// Shape-2 (run-until-cancelled) implementations must structure
    /// their `tokio::select!` so that each branch future is itself
    /// cancel-safe — see [`crate::poll::PollTriggerAdapter::start`]
    /// for a worked example using cancel-safe
    /// `CancellationToken::cancelled()` and `tokio::time::sleep`.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be started, if `start`
    /// is called twice without an intervening `stop`, or (for shape 2) if
    /// the trigger loop encounters a fatal error while running.
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError>;

    /// Stop the trigger (unregister, cancel schedule).
    ///
    /// Clears any state set by [`start`](Self::start) so a subsequent
    /// `start` call is accepted. For shape-2 triggers (run-until-cancelled),
    /// prefer cancelling `ctx.cancellation` to let `start()` exit cleanly.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be stopped cleanly.
    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError>;

    /// Whether this trigger accepts externally pushed events.
    ///
    /// Engine/webhook layer checks this before calling `handle_event`.
    /// Default: `false`.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Handle an external event pushed to this trigger.
    ///
    /// Only called when [`accepts_events`](Self::accepts_events) returns `true`.
    /// Takes typed [`IncomingEvent`] directly — no serialization round-trip.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] by default — triggers that don't accept
    /// external events should never have this called.
    async fn handle_event(
        &self,
        event: IncomingEvent,
        ctx: &TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        let _ = (event, ctx);
        Err(ActionError::fatal(
            "trigger does not accept external events",
        ))
    }
}

// ── TriggerActionAdapter ────────────────────────────────────────────────────

/// Wraps a [`TriggerAction`] as a [`dyn TriggerHandler`].
///
/// Simple delegation: `start` and `stop` call through to the typed trait
/// with [`TriggerContext`].
///
/// # Example
///
/// ```rust,ignore
/// let handler: Arc<dyn TriggerHandler> = Arc::new(TriggerActionAdapter::new(my_trigger));
/// ```
pub struct TriggerActionAdapter<A> {
    action: A,
}

impl<A> TriggerActionAdapter<A> {
    /// Wrap a typed trigger action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait]
impl<A> TriggerHandler for TriggerActionAdapter<A>
where
    A: TriggerAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    /// Start the trigger by delegating to the typed action.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be started.
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        self.action.start(ctx).await
    }

    /// Stop the trigger by delegating to the typed action.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be stopped cleanly.
    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        self.action.stop(ctx).await
    }
}

impl<A: Action> fmt::Debug for TriggerActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerActionAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use nebula_core::id::{NodeId, WorkflowId};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::action::Action;
    use crate::dependency::ActionDependencies;

    // ── TriggerActionAdapter tests ────────────────────────────────────────────

    struct MockTriggerAction {
        meta: ActionMetadata,
        started: AtomicBool,
    }

    impl MockTriggerAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.trigger_action"),
                    "MockTrigger",
                    "Tracks start/stop",
                ),
                started: AtomicBool::new(false),
            }
        }
    }

    impl ActionDependencies for MockTriggerAction {}

    impl Action for MockTriggerAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl TriggerAction for MockTriggerAction {
        async fn start(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            self.started.store(true, Ordering::Release);
            Ok(())
        }

        async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            self.started.store(false, Ordering::Release);
            Ok(())
        }
    }

    fn make_trigger_ctx() -> TriggerContext {
        TriggerContext::new(WorkflowId::nil(), NodeId::nil(), CancellationToken::new())
    }

    #[test]
    fn trigger_adapter_is_dyn_compatible() {
        let adapter = TriggerActionAdapter::new(MockTriggerAction::new());
        let _: Arc<dyn TriggerHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn trigger_adapter_delegates_start_stop() {
        let action = MockTriggerAction::new();
        let adapter = TriggerActionAdapter::new(action);
        let ctx = make_trigger_ctx();

        adapter.start(&ctx).await.unwrap();
        assert!(adapter.action.started.load(Ordering::Acquire));

        adapter.stop(&ctx).await.unwrap();
        assert!(!adapter.action.started.load(Ordering::Acquire));
    }

    #[test]
    fn trigger_adapter_into_inner_returns_action() {
        let adapter = TriggerActionAdapter::new(MockTriggerAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().key,
            nebula_core::action_key!("test.trigger_action")
        );
    }

    // ── TriggerEventOutcome tests ─────────────────────────────────────────────

    #[test]
    fn trigger_event_outcome_skip() {
        let o = TriggerEventOutcome::skip();
        assert!(!o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Skip));
    }

    #[test]
    fn trigger_event_outcome_emit() {
        let o = TriggerEventOutcome::emit(serde_json::json!({"key": "val"}));
        assert!(o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Emit(_)));
    }

    #[test]
    fn trigger_event_outcome_emit_many() {
        let o = TriggerEventOutcome::emit_many(vec![serde_json::json!(1), serde_json::json!(2)]);
        assert!(o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::EmitMany(v) if v.len() == 2));
    }

    #[test]
    fn trigger_event_outcome_empty_emit_many_is_skip() {
        let o = TriggerEventOutcome::emit_many(vec![]);
        assert!(!o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Skip));
    }

    #[test]
    fn trigger_event_outcome_direct_empty_emit_many_will_emit_false() {
        // Bypass the constructor — direct enum construction with empty vec.
        let o: TriggerEventOutcome = TriggerEventOutcome::EmitMany(vec![]);
        assert!(!o.will_emit());
    }
}
