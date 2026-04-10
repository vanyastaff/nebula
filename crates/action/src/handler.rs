//! Dynamic handler traits and typed action adapters.
//!
//! The engine dispatches actions via [`ActionHandler`], a top-level enum whose
//! variants wrap `Arc<dyn XxxHandler>` trait objects. Typed action authors write
//! `impl StatelessAction<Input=T, Output=U>` and register via the registry's
//! helper methods (e.g., `nebula_runtime::ActionRegistry::register_stateless`),
//! which wraps the typed action in the corresponding adapter automatically.
//!
//! ## Handler traits
//!
//! Five handler traits model the JSON-level contract for each action kind:
//!
//! - [`StatelessHandler`] — one-shot JSON in, JSON out
//! - [`StatefulHandler`] — iterative with mutable JSON state
//! - [`TriggerHandler`] — start/stop lifecycle (uses [`TriggerContext`])
//! - [`ResourceHandler`] — configure/cleanup lifecycle
//! - [`AgentHandler`] — autonomous agent (stub for Phase 9)
//!
//! [`ActionHandler`] is the top-level enum the engine dispatches on.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::{ActionContext, TriggerContext};
use crate::error::ActionError;
use crate::metadata::ActionMetadata;
use crate::resource::ResourceAction;
use crate::result::ActionResult;
use crate::stateful::StatefulAction;
use crate::stateless::StatelessAction;
use crate::trigger::{PollAction, TriggerAction, WebhookAction};

/// Wraps a [`StatelessAction`] as a [`dyn StatelessHandler`].
///
/// Handles JSON deserialization of input and serialization of output so the
/// runtime can work with untyped JSON throughout, while action authors write
/// strongly-typed Rust.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{StatelessActionAdapter, StatelessAction, Action, ActionResult, ActionError};
/// use nebula_action::handler::StatelessHandler;
///
/// struct EchoAction { meta: ActionMetadata }
/// impl Action for EchoAction { ... }
/// impl StatelessAction for EchoAction {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///     async fn execute(&self, input: Self::Input, _ctx: &impl Context)
///         -> Result<ActionResult<Self::Output>, ActionError>
///     {
///         Ok(ActionResult::success(input))
///     }
/// }
///
/// let handler: Arc<dyn StatelessHandler> = Arc::new(StatelessActionAdapter::new(EchoAction { ... }));
/// ```
pub struct StatelessActionAdapter<A> {
    action: A,
}

impl<A> StatelessActionAdapter<A> {
    /// Wrap a typed stateless action.
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
impl<A> StatelessHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

        let result = self.action.execute(typed_input, ctx).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── StatefulActionAdapter ──────────────────────────────────────────────────

/// Wraps a [`StatefulAction`] as a [`dyn StatefulHandler`].
///
/// Handles JSON (de)serialization of input, output, and state so the runtime
/// works with untyped JSON while action authors write strongly-typed Rust.
///
/// State is serialized to/from `serde_json::Value` between iterations for
/// engine checkpointing.
///
/// # Example
///
/// ```rust,ignore
/// let handler: Arc<dyn StatefulHandler> = Arc::new(StatefulActionAdapter::new(my_action));
/// ```
pub struct StatefulActionAdapter<A> {
    action: A,
}

impl<A> StatefulActionAdapter<A> {
    /// Wrap a typed stateful action.
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
impl<A> StatefulHandler for StatefulActionAdapter<A>
where
    A: StatefulAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
    A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    fn init_state(&self) -> Result<Value, ActionError> {
        serde_json::to_value(self.action.init_state())
            .map_err(|e| ActionError::fatal(format!("init_state serialization failed: {e}")))
    }

    fn migrate_state(&self, old: Value) -> Option<Value> {
        // If the migrated state can't be serialized back to JSON, treat as migration failure.
        // This is acceptable because the alternative (a different error) would be more confusing.
        self.action
            .migrate_state(old)
            .and_then(|state| serde_json::to_value(state).ok())
    }

    /// Execute one iteration, deserializing input and state from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Validation`] if input or state deserialization fails,
    /// or propagates errors from the underlying action.
    ///
    /// # State checkpointing invariant
    ///
    /// State mutations performed by the typed action are flushed back to
    /// `state` **before** any error from `action.execute()` is propagated.
    /// If the typed action increments a counter or advances a cursor and
    /// then returns [`ActionError::Retryable`], the engine checkpoints the
    /// new state — retries resume from the mutated position instead of
    /// replaying completed work (which would duplicate API calls, double
    /// charges, and double emits).
    ///
    /// The only path that does NOT checkpoint is `Validation` raised while
    /// deserializing input or state — in that case `typed_state` was never
    /// created and cannot have been mutated.
    async fn execute(
        &self,
        input: &Value,
        state: &mut Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        // Adapter clones input ONCE per iteration to deserialize into typed A::Input.
        let typed_input: A::Input = serde_json::from_value(input.clone())
            .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

        let mut typed_state: A::State = serde_json::from_value(state.clone()).or_else(|e| {
            self.action.migrate_state(state.clone()).ok_or_else(|| {
                ActionError::validation(format!("state deserialization failed: {e}"))
            })
        })?;

        // Run the typed action. typed_state may be mutated regardless of
        // Ok/Err — flush it back to JSON before propagating so that a
        // Retryable does not replay completed work on retry.
        let action_result = self
            .action
            .execute(typed_input, &mut typed_state, ctx)
            .await;

        match serde_json::to_value(&typed_state) {
            Ok(new_state) => {
                *state = new_state;
            }
            Err(ser_err) => match &action_result {
                Ok(_) => {
                    // Success path: surface the serialization failure as fatal.
                    return Err(ActionError::fatal(format!(
                        "state serialization failed: {ser_err}"
                    )));
                }
                Err(action_err) => {
                    // Error path: the action error is the actionable signal.
                    // Log the serde failure forensically and let the original
                    // error propagate — masking it would break retry classification.
                    tracing::error!(
                        action = %self.action.metadata().key,
                        serialization_error = %ser_err,
                        action_error = %action_err,
                        "stateful adapter: state serialization failed on error path; \
                         checkpoint lost, propagating original action error"
                    );
                }
            },
        }

        let result = action_result?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── TriggerActionAdapter ───────────────────────────────────────────────────

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

// ── WebhookTriggerAdapter ─────────────────────────────────────────────────

/// Wraps a [`WebhookAction`] as a [`dyn TriggerHandler`] with state management.
///
/// Stores state from `on_activate` in a `RwLock<Option<Arc<State>>>`. `handle_event`
/// clones the `Arc` under the read lock and releases the lock BEFORE awaiting
/// `handle_request` — prevents deadlock with concurrent `start`/`stop` taking a
/// write lock (parking_lot RwLock is not reentrant and not async-aware).
///
/// `handle_event` before `start()` returns `ActionError::Fatal` (no silent default state).
///
/// Created automatically by `nebula_runtime::ActionRegistry::register_webhook`.
pub struct WebhookTriggerAdapter<A: WebhookAction> {
    action: A,
    state: RwLock<Option<Arc<A::State>>>,
}

impl<A: WebhookAction> WebhookTriggerAdapter<A> {
    /// Wrap a typed webhook action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self {
            action,
            state: RwLock::new(None),
        }
    }
}

#[async_trait]
impl<A> TriggerHandler for WebhookTriggerAdapter<A>
where
    A: WebhookAction + Send + Sync + 'static,
    A::State: Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        let new_state = self.action.on_activate(ctx).await?;
        *self.state.write() = Some(Arc::new(new_state));
        Ok(())
    }

    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        let stored = self.state.write().take();
        match stored {
            Some(arc_state) => {
                // Consume Arc — in normal flow no concurrent handle_event holds it.
                let owned = Arc::try_unwrap(arc_state).unwrap_or_else(|arc| (*arc).clone());
                self.action.on_deactivate(owned, ctx).await
            }
            // stop() without prior start() — no-op, nothing to deactivate.
            None => Ok(()),
        }
    }

    fn accepts_events(&self) -> bool {
        true
    }

    async fn handle_event(
        &self,
        event: IncomingEvent,
        ctx: &TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        // Clone Arc under read lock; the guard drops at end of statement BEFORE
        // the await on handle_request. Holding a parking_lot guard across .await
        // would be unsound (non-Send) and risk re-entry panic with start/stop.
        let state = self.state.read().as_ref().cloned().ok_or_else(|| {
            ActionError::fatal("handle_event called before start — no state available")
        })?;

        self.action.handle_request(&event, &state, ctx).await
    }
}

impl<A: WebhookAction> fmt::Debug for WebhookTriggerAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookTriggerAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}

// ── PollTriggerAdapter ────────────────────────────────────────────────────

/// Wraps a [`PollAction`] as a [`dyn TriggerHandler`].
///
/// `start()` runs a blocking loop: sleep → poll → emit events.
/// Cancellation via `TriggerContext::cancellation`.
/// `stop()` is a no-op (cancellation token handles shutdown).
///
/// Created automatically by `nebula_runtime::ActionRegistry::register_poll`.
///
/// # Error handling
///
/// - `ActionError::Fatal` from `poll()` stops the loop immediately
/// - `ActionError::Retryable` skips the current cycle, continues at next interval
/// - Emit failures are silently dropped (transient emitter issues don't kill the trigger)
pub struct PollTriggerAdapter<A: PollAction> {
    action: A,
}

impl<A: PollAction> PollTriggerAdapter<A> {
    /// Wrap a typed poll action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self { action }
    }
}

#[async_trait]
impl<A> TriggerHandler for PollTriggerAdapter<A>
where
    A: PollAction + Send + Sync + 'static,
    A::Cursor: Send + Sync,
    A::Event: Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        let mut cursor = A::Cursor::default();
        let interval = self.action.poll_interval();

        loop {
            tokio::select! {
                () = ctx.cancellation.cancelled() => {
                    return Ok(());
                }
                () = tokio::time::sleep(interval) => {
                    match self.action.poll(&mut cursor, ctx).await {
                        Ok(events) => {
                            for event in events {
                                // Bad payload from a single event must not kill the trigger.
                                let Ok(payload) = serde_json::to_value(&event) else {
                                    continue;
                                };
                                // Transient emit failure — skip this event, continue polling.
                                let _ = ctx.emitter.emit(payload).await;
                            }
                        }
                        Err(e) if e.is_fatal() => return Err(e),
                        Err(_) => {
                            // Retryable poll error — skip this cycle, try next interval.
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
        // Cancellation token handles shutdown — no-op.
        Ok(())
    }
}

impl<A: PollAction> fmt::Debug for PollTriggerAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollTriggerAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}

// ── ResourceActionAdapter ──────────────────────────────────────────────────

/// Wraps a [`ResourceAction`] as a [`dyn ResourceHandler`].
///
/// Bridges the typed `configure`/`cleanup` lifecycle to the JSON-erased handler
/// trait. The `configure` result is boxed as `Box<dyn Any + Send + Sync>`;
/// `cleanup` downcasts it back to the typed `Instance`.
///
/// # Example
///
/// ```rust,ignore
/// let handler: Arc<dyn ResourceHandler> = Arc::new(ResourceActionAdapter::new(my_resource));
/// ```
pub struct ResourceActionAdapter<A> {
    action: A,
}

impl<A> ResourceActionAdapter<A> {
    /// Wrap a typed resource action.
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

/// # Note on Config vs Instance
///
/// `ResourceAction` has separate `Config` and `Instance` types. This adapter
/// boxes `Config` in `configure()` and downcasts `Config` in `cleanup()`,
/// so it requires `Instance == Config`. For resource actions where these
/// types differ, implement `ResourceHandler` directly.
#[async_trait]
impl<A> ResourceHandler for ResourceActionAdapter<A>
where
    A: ResourceAction + Send + Sync + 'static,
    A::Config: Send + Sync + 'static,
    A::Instance: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    /// Configure the resource by delegating to the typed action.
    ///
    /// The `_config` parameter is reserved for future use; the typed
    /// [`ResourceAction::configure`] obtains its configuration from context.
    /// The typed `Config` result is boxed as `dyn Any`.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the resource cannot be configured.
    async fn configure(
        &self,
        _config: Value,
        ctx: &ActionContext,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError> {
        let instance = self.action.configure(ctx).await?;
        // We box Config here. cleanup() downcasts to Instance.
        // When Config == Instance (the common case), this works.
        // When they differ, the downcast in cleanup() returns ActionError::Fatal.
        Ok(Box::new(instance) as Box<dyn std::any::Any + Send + Sync>)
    }

    /// Clean up the resource by downcasting the instance and delegating.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] if the instance cannot be downcast to
    /// the expected type, or propagates errors from the underlying action.
    async fn cleanup(
        &self,
        instance: Box<dyn std::any::Any + Send + Sync>,
        ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        let typed_instance = instance.downcast::<A::Instance>().map_err(|_| {
            ActionError::fatal(format!(
                "resource instance downcast failed: expected {}",
                std::any::type_name::<A::Instance>()
            ))
        })?;
        self.action.cleanup(*typed_instance, ctx).await
    }
}

// ── Handler traits ─────────────────────────────────────────────────────────

/// Stateless action handler — JSON in, JSON out.
///
/// This is the JSON-level contract for one-shot actions. The engine sends
/// a `serde_json::Value` input and receives a `serde_json::Value` output
/// wrapped in [`ActionResult`].
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failures.
#[async_trait]
pub trait StatelessHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute with JSON input and context.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    async fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

/// Stateful action handler — JSON in, mutable JSON state, JSON out.
///
/// The engine calls `execute` repeatedly. State is persisted as JSON between
/// iterations for checkpointing. Return [`ActionResult::Continue`] for another
/// iteration or [`ActionResult::Break`] when done.
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failures.
#[async_trait]
pub trait StatefulHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Create initial state as JSON for the first iteration.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] if the initial state cannot be produced
    /// (e.g., serialization failure in an adapter).
    fn init_state(&self) -> Result<Value, ActionError>;

    /// Attempt to migrate state from a previous version.
    ///
    /// Called when state deserialization fails during `execute`. Returns
    /// migrated state as JSON, or `None` to propagate the error.
    fn migrate_state(&self, old: Value) -> Option<Value> {
        let _ = old;
        None
    }

    /// Execute one iteration with mutable JSON state.
    ///
    /// # State checkpointing
    ///
    /// Implementations MUST flush any state mutations back to `state` before
    /// returning, regardless of whether the iteration succeeded. Returning
    /// `Err(Retryable)` after mutating internal typed state without
    /// checkpointing causes the engine to re-run the iteration against a
    /// stale snapshot — partial work is replayed and external side effects
    /// (API calls, DB writes, emits) are duplicated.
    ///
    /// The only exception is when state deserialization fails at the start
    /// of the iteration — in that case no mutations could have occurred and
    /// there is nothing to flush.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    async fn execute(
        &self,
        input: &Value,
        state: &mut Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

/// External event delivered to a trigger (HTTP request, message, etc.).
///
/// Transport-agnostic: webhook layer constructs this from HTTP requests,
/// message queue layer constructs it from queue messages.
///
/// `body` holds canonical raw bytes — use directly for HMAC signature
/// verification (do NOT round-trip through `body_json()` for signing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingEvent {
    /// Raw payload body. Canonical bytes for signature verification.
    pub body: Vec<u8>,
    /// Headers / metadata (HTTP headers, message attributes, etc.).
    pub headers: HashMap<String, String>,
    /// Source identifier (URL path, topic name, queue name).
    #[serde(default)]
    pub source: String,
}

impl IncomingEvent {
    /// Create a new incoming event.
    #[must_use]
    pub fn new(body: &[u8], headers: &[(&str, &str)]) -> Self {
        Self {
            body: body.to_vec(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            source: String::new(),
        }
    }

    /// Set the source identifier.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Get a header value by key (ASCII case-insensitive).
    ///
    /// HTTP headers are ASCII per RFC 7230, so ASCII case folding is sufficient
    /// and avoids allocations on lookup.
    #[must_use]
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// Get the body as a UTF-8 string slice.
    #[must_use]
    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Parse the body as JSON.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the body is not valid JSON.
    pub fn body_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}

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

/// Trigger handler — start/stop lifecycle for workflow triggers.
///
/// Uses [`TriggerContext`] (workflow_id, trigger_id, cancellation) instead
/// of [`ActionContext`]. Triggers live outside the execution graph and emit
/// new workflow executions.
///
/// # Errors
///
/// Returns [`ActionError`] if start or stop fails.
#[async_trait]
pub trait TriggerHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Start the trigger (register listener, schedule poll, etc.).
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be started.
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError>;

    /// Stop the trigger (unregister, cancel schedule).
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

/// Resource handler — configure/cleanup lifecycle for graph-scoped resources.
///
/// The engine runs `configure` before downstream nodes; the resulting instance
/// is scoped to the branch. When the scope ends, `cleanup` is called.
///
/// # Errors
///
/// Returns [`ActionError`] on configuration or cleanup failure.
#[async_trait]
pub trait ResourceHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Build the resource for this scope.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the resource cannot be configured.
    async fn configure(
        &self,
        config: Value,
        ctx: &ActionContext,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError>;

    /// Clean up the resource instance when the scope ends.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if cleanup fails.
    async fn cleanup(
        &self,
        instance: Box<dyn std::any::Any + Send + Sync>,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
}

/// Agent handler — autonomous agent execution (stub for Phase 9).
///
/// Agents combine tool use, planning, and iterative execution. This trait
/// is a placeholder with the same signature as [`StatelessHandler`]; the
/// full agent protocol will be defined in Phase 9.
///
/// # Errors
///
/// Returns [`ActionError`] on execution failure.
#[async_trait]
pub trait AgentHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute the agent with JSON input and context.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if agent execution fails.
    async fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

// ── ActionHandler enum ─────────────────────────────────────────────────────

/// Top-level handler enum — the engine dispatches based on variant.
///
/// Each variant wraps an `Arc<dyn XxxHandler>` so handlers can be shared
/// across nodes in the workflow graph.
#[derive(Clone)]
#[non_exhaustive]
pub enum ActionHandler {
    /// One-shot stateless execution.
    Stateless(Arc<dyn StatelessHandler>),
    /// Iterative execution with persistent JSON state.
    Stateful(Arc<dyn StatefulHandler>),
    /// Workflow trigger (start/stop lifecycle).
    Trigger(Arc<dyn TriggerHandler>),
    /// Graph-scoped resource (configure/cleanup).
    Resource(Arc<dyn ResourceHandler>),
    /// Autonomous agent (stub for Phase 9).
    Agent(Arc<dyn AgentHandler>),
}

impl ActionHandler {
    /// Get metadata regardless of variant.
    #[must_use]
    pub fn metadata(&self) -> &ActionMetadata {
        match self {
            Self::Stateless(h) => h.metadata(),
            Self::Stateful(h) => h.metadata(),
            Self::Trigger(h) => h.metadata(),
            Self::Resource(h) => h.metadata(),
            Self::Agent(h) => h.metadata(),
        }
    }

    /// Check if this is a stateless handler.
    #[must_use]
    pub fn is_stateless(&self) -> bool {
        matches!(self, Self::Stateless(_))
    }

    /// Check if this is a stateful handler.
    #[must_use]
    pub fn is_stateful(&self) -> bool {
        matches!(self, Self::Stateful(_))
    }

    /// Check if this is a trigger handler.
    #[must_use]
    pub fn is_trigger(&self) -> bool {
        matches!(self, Self::Trigger(_))
    }

    /// Check if this is a resource handler.
    #[must_use]
    pub fn is_resource(&self) -> bool {
        matches!(self, Self::Resource(_))
    }

    /// Check if this is an agent handler.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent(_))
    }
}

impl fmt::Debug for ActionHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stateless(h) => f.debug_tuple("Stateless").field(&h.metadata().key).finish(),
            Self::Stateful(h) => f.debug_tuple("Stateful").field(&h.metadata().key).finish(),
            Self::Trigger(h) => f.debug_tuple("Trigger").field(&h.metadata().key).finish(),
            Self::Resource(h) => f.debug_tuple("Resource").field(&h.metadata().key).finish(),
            Self::Agent(h) => f.debug_tuple("Agent").field(&h.metadata().key).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};
    use tokio_util::sync::CancellationToken;

    use crate::action::Action;
    use crate::context::Context;
    use crate::dependency::ActionDependencies;
    use crate::metadata::ActionMetadata;
    use crate::stateless::StatelessAction;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};

    use super::*;

    // ── Test action ────────────────────────────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct AddInput {
        a: i64,
        b: i64,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct AddOutput {
        sum: i64,
    }

    struct AddAction {
        meta: ActionMetadata,
    }

    impl AddAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("math.add"),
                    "Add",
                    "Adds two numbers",
                ),
            }
        }
    }

    impl ActionDependencies for AddAction {}

    impl Action for AddAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for AddAction {
        type Input = AddInput;
        type Output = AddOutput;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(AddOutput {
                sum: input.a + input.b,
            }))
        }
    }

    fn make_ctx() -> ActionContext {
        ActionContext::new(
            ExecutionId::nil(),
            NodeId::nil(),
            WorkflowId::nil(),
            CancellationToken::new(),
        )
    }

    // ── Tests ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn adapter_executes_typed_action() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 3, "b": 7 });
        let result = StatelessHandler::execute(&adapter, input, &ctx)
            .await
            .unwrap();

        match result {
            ActionResult::Success { output } => {
                let v = output.into_value().unwrap();
                let out: AddOutput = serde_json::from_value(v).unwrap();
                assert_eq!(out.sum, 10);
            }
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn adapter_returns_validation_error_on_bad_input() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let bad_input = serde_json::json!({ "x": "not a number" });
        let err = StatelessHandler::execute(&adapter, bad_input, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn adapter_exposes_metadata() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        assert_eq!(
            StatelessHandler::metadata(&adapter).key,
            nebula_core::action_key!("math.add")
        );
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let _: Arc<dyn StatelessHandler> = Arc::new(adapter);
    }

    // ── Handler trait test helpers ─────────────────────────────────────────────

    fn test_meta(key: &str) -> ActionMetadata {
        ActionMetadata::new(
            nebula_core::ActionKey::new(key).expect("valid test key"),
            key,
            "test handler",
        )
    }

    struct TestStatelessHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl StatelessHandler for TestStatelessHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn execute(
            &self,
            input: Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct TestStatefulHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl StatefulHandler for TestStatefulHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        fn init_state(&self) -> Result<Value, ActionError> {
            Ok(serde_json::json!(0))
        }

        async fn execute(
            &self,
            input: &Value,
            state: &mut Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            let count = state.as_u64().unwrap_or(0);
            *state = serde_json::json!(count + 1);
            Ok(ActionResult::success(input.clone()))
        }
    }

    struct TestTriggerHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl TriggerHandler for TestTriggerHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn start(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }

        async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestResourceHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl ResourceHandler for TestResourceHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn configure(
            &self,
            _config: Value,
            _ctx: &ActionContext,
        ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError> {
            Ok(Box::new(42u32))
        }

        async fn cleanup(
            &self,
            _instance: Box<dyn std::any::Any + Send + Sync>,
            _ctx: &ActionContext,
        ) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestAgentHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl AgentHandler for TestAgentHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn execute(
            &self,
            input: Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    // ── Handler trait dyn-compatibility tests ──────────────────────────────────

    #[test]
    fn stateless_handler_is_dyn_compatible() {
        let h = TestStatelessHandler {
            meta: test_meta("test.stateless"),
        };
        let _: Arc<dyn StatelessHandler> = Arc::new(h);
    }

    #[test]
    fn stateful_handler_is_dyn_compatible() {
        let h = TestStatefulHandler {
            meta: test_meta("test.stateful"),
        };
        let _: Arc<dyn StatefulHandler> = Arc::new(h);
    }

    #[test]
    fn trigger_handler_is_dyn_compatible() {
        let h = TestTriggerHandler {
            meta: test_meta("test.trigger"),
        };
        let _: Arc<dyn TriggerHandler> = Arc::new(h);
    }

    #[test]
    fn resource_handler_is_dyn_compatible() {
        let h = TestResourceHandler {
            meta: test_meta("test.resource"),
        };
        let _: Arc<dyn ResourceHandler> = Arc::new(h);
    }

    #[test]
    fn agent_handler_is_dyn_compatible() {
        let h = TestAgentHandler {
            meta: test_meta("test.agent"),
        };
        let _: Arc<dyn AgentHandler> = Arc::new(h);
    }

    // ── ActionHandler metadata delegation ──────────────────────────────────────

    #[test]
    fn action_handler_metadata_delegates_to_inner() {
        let cases: Vec<(&str, ActionHandler)> = vec![
            (
                "test.stateless",
                ActionHandler::Stateless(Arc::new(TestStatelessHandler {
                    meta: test_meta("test.stateless"),
                })),
            ),
            (
                "test.stateful",
                ActionHandler::Stateful(Arc::new(TestStatefulHandler {
                    meta: test_meta("test.stateful"),
                })),
            ),
            (
                "test.trigger",
                ActionHandler::Trigger(Arc::new(TestTriggerHandler {
                    meta: test_meta("test.trigger"),
                })),
            ),
            (
                "test.resource",
                ActionHandler::Resource(Arc::new(TestResourceHandler {
                    meta: test_meta("test.resource"),
                })),
            ),
            (
                "test.agent",
                ActionHandler::Agent(Arc::new(TestAgentHandler {
                    meta: test_meta("test.agent"),
                })),
            ),
        ];

        for (expected_key, handler) in &cases {
            assert_eq!(
                handler.metadata().key,
                nebula_core::ActionKey::new(expected_key).expect("valid test key")
            );
        }
    }

    // ── ActionHandler variant checks ───────────────────────────────────────────

    #[test]
    fn action_handler_variant_checks() {
        let stateless = ActionHandler::Stateless(Arc::new(TestStatelessHandler {
            meta: test_meta("test.stateless"),
        }));
        assert!(stateless.is_stateless());
        assert!(!stateless.is_stateful());
        assert!(!stateless.is_trigger());
        assert!(!stateless.is_resource());
        assert!(!stateless.is_agent());

        let stateful = ActionHandler::Stateful(Arc::new(TestStatefulHandler {
            meta: test_meta("test.stateful"),
        }));
        assert!(!stateful.is_stateless());
        assert!(stateful.is_stateful());

        let trigger = ActionHandler::Trigger(Arc::new(TestTriggerHandler {
            meta: test_meta("test.trigger"),
        }));
        assert!(!trigger.is_stateless());
        assert!(trigger.is_trigger());

        let resource = ActionHandler::Resource(Arc::new(TestResourceHandler {
            meta: test_meta("test.resource"),
        }));
        assert!(!resource.is_stateless());
        assert!(resource.is_resource());

        let agent = ActionHandler::Agent(Arc::new(TestAgentHandler {
            meta: test_meta("test.agent"),
        }));
        assert!(!agent.is_stateless());
        assert!(agent.is_agent());
    }

    // ── ActionHandler Debug ────────────────────────────────────────────────────

    #[test]
    fn action_handler_debug_shows_variant_and_key() {
        let handler = ActionHandler::Stateless(Arc::new(TestStatelessHandler {
            meta: test_meta("test.stateless"),
        }));
        let debug = format!("{handler:?}");
        assert!(debug.contains("Stateless"));
        assert!(debug.contains("test.stateless"));
    }

    // ── StatelessActionAdapter as StatelessHandler ────────────────────────────

    #[test]
    fn stateless_adapter_is_dyn_stateless_handler() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let _: Arc<dyn StatelessHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn stateless_adapter_implements_stateless_handler() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let handler: Arc<dyn StatelessHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 5, "b": 3 });
        let result = handler.execute(input, &ctx).await.unwrap();

        match result {
            ActionResult::Success { output } => {
                let v = output.into_value().unwrap();
                let out: AddOutput = serde_json::from_value(v).unwrap();
                assert_eq!(out.sum, 8);
            }
            _ => panic!("expected Success"),
        }
    }

    // ── StatefulActionAdapter tests ───────────────────────────────────────────

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct CounterState {
        count: u32,
    }

    struct CounterAction {
        meta: ActionMetadata,
    }

    impl CounterAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.counter"),
                    "Counter",
                    "Counts up to 3",
                ),
            }
        }
    }

    impl ActionDependencies for CounterAction {}

    impl Action for CounterAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl crate::stateful::StatefulAction for CounterAction {
        type Input = Value;
        type Output = Value;
        type State = CounterState;

        fn init_state(&self) -> CounterState {
            CounterState { count: 0 }
        }

        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &impl crate::context::Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            state.count += 1;
            if state.count >= 3 {
                Ok(ActionResult::Break {
                    output: crate::output::ActionOutput::Value(
                        serde_json::json!({"final": state.count}),
                    ),
                    reason: crate::result::BreakReason::Completed,
                })
            } else {
                Ok(ActionResult::Continue {
                    output: crate::output::ActionOutput::Value(
                        serde_json::json!({"current": state.count}),
                    ),
                    progress: Some(state.count as f64 / 3.0),
                    delay: None,
                })
            }
        }
    }

    #[test]
    fn stateful_adapter_is_dyn_compatible() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let _: Arc<dyn StatefulHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn stateful_adapter_init_state_serializes() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let state = adapter.init_state().unwrap();
        let cs: CounterState = serde_json::from_value(state).unwrap();
        assert_eq!(cs.count, 0);
    }

    #[tokio::test]
    async fn stateful_adapter_iterates_with_state() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let handler: Arc<dyn StatefulHandler> = Arc::new(adapter);
        let ctx = make_ctx();
        let mut state = handler.init_state().unwrap();

        // Iteration 1: count goes 0 → 1, Continue
        let result = handler
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Continue { .. }));
        let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(cs.count, 1);

        // Iteration 2: count goes 1 → 2, Continue
        let result = handler
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Continue { .. }));
        let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(cs.count, 2);

        // Iteration 3: count goes 2 → 3, Break
        let result = handler
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap();
        assert!(matches!(result, ActionResult::Break { .. }));
        let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(cs.count, 3);
    }

    #[tokio::test]
    async fn stateful_adapter_returns_validation_error_on_bad_state() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let ctx = make_ctx();
        let mut bad_state = serde_json::json!("not a counter state");

        let err = adapter
            .execute(&serde_json::json!({}), &mut bad_state, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    /// Action that mutates state to `mark` then fails with the configured error.
    ///
    /// Used to prove that `StatefulActionAdapter::execute` flushes state
    /// back to JSON before propagating errors — critical for avoiding
    /// duplicated side effects on retry.
    struct MutateThenFailAction {
        meta: ActionMetadata,
        fail_with: ActionError,
        mark: u32,
    }

    impl ActionDependencies for MutateThenFailAction {}

    impl Action for MutateThenFailAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl crate::stateful::StatefulAction for MutateThenFailAction {
        type Input = Value;
        type Output = Value;
        type State = CounterState;

        fn init_state(&self) -> CounterState {
            CounterState { count: 0 }
        }

        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &impl crate::context::Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            state.count = self.mark;
            Err(self.fail_with.clone())
        }
    }

    fn mutate_fail(fail_with: ActionError, mark: u32) -> MutateThenFailAction {
        MutateThenFailAction {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.mutate_fail"),
                "MutateFail",
                "Mutates state then fails",
            ),
            fail_with,
            mark,
        }
    }

    #[tokio::test]
    async fn stateful_adapter_checkpoints_state_on_retryable_error() {
        // Prove: an action that advances cursor/counter state and then returns
        // Retryable must have its mutations flushed to the JSON state so the
        // engine checkpoints the new position. Otherwise retry replays
        // completed work.
        let adapter = StatefulActionAdapter::new(mutate_fail(
            ActionError::retryable("transient upstream error"),
            42,
        ));
        let ctx = make_ctx();
        let mut state = serde_json::json!({ "count": 0 });

        let err = adapter
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap_err();

        assert!(err.is_retryable(), "error must still be retryable");
        assert_eq!(
            state,
            serde_json::json!({ "count": 42 }),
            "state mutations before Err must be checkpointed"
        );
    }

    #[tokio::test]
    async fn stateful_adapter_checkpoints_state_on_fatal_error() {
        // Symmetric invariant: even fatal errors must checkpoint state, so
        // an operator debugging the failure can see the position at which
        // the action gave up.
        let adapter =
            StatefulActionAdapter::new(mutate_fail(ActionError::fatal("schema mismatch"), 7));
        let ctx = make_ctx();
        let mut state = serde_json::json!({ "count": 0 });

        let err = adapter
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap_err();

        assert!(err.is_fatal());
        assert_eq!(state, serde_json::json!({ "count": 7 }));
    }

    #[tokio::test]
    async fn stateful_adapter_preserves_state_on_validation_error() {
        // Deserialization failure happens BEFORE the typed action runs, so
        // typed_state never existed and nothing should be written back. The
        // input JSON must remain verbatim so the engine can decide how to
        // recover (e.g., schema migration path outside the adapter).
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let ctx = make_ctx();
        let bad = serde_json::json!("not a counter state");
        let mut state = bad.clone();

        let err = adapter
            .execute(&serde_json::json!({}), &mut state, &ctx)
            .await
            .unwrap_err();

        assert!(matches!(err, ActionError::Validation(_)));
        assert_eq!(state, bad, "state must be untouched on deser failure");
    }

    // ── TriggerActionAdapter tests ────────────────────────────────────────────

    struct MockTriggerAction {
        meta: ActionMetadata,
        started: std::sync::atomic::AtomicBool,
    }

    impl MockTriggerAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.trigger_action"),
                    "MockTrigger",
                    "Tracks start/stop",
                ),
                started: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl ActionDependencies for MockTriggerAction {}

    impl Action for MockTriggerAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl crate::trigger::TriggerAction for MockTriggerAction {
        async fn start(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            self.started
                .store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        }

        async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            self.started
                .store(false, std::sync::atomic::Ordering::Release);
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
        assert!(
            adapter
                .action
                .started
                .load(std::sync::atomic::Ordering::Acquire)
        );

        adapter.stop(&ctx).await.unwrap();
        assert!(
            !adapter
                .action
                .started
                .load(std::sync::atomic::Ordering::Acquire)
        );
    }

    // ── ResourceActionAdapter tests ───────────────────────────────────────────

    struct MockResourceAction {
        meta: ActionMetadata,
    }

    impl MockResourceAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.resource_action"),
                    "MockResource",
                    "Creates a string pool",
                ),
            }
        }
    }

    impl ActionDependencies for MockResourceAction {}

    impl Action for MockResourceAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl crate::resource::ResourceAction for MockResourceAction {
        type Config = String;
        type Instance = String;

        async fn configure(
            &self,
            _ctx: &impl crate::context::Context,
        ) -> Result<String, ActionError> {
            Ok("pool-default".to_owned())
        }

        async fn cleanup(
            &self,
            _instance: String,
            _ctx: &impl crate::context::Context,
        ) -> Result<(), ActionError> {
            Ok(())
        }
    }

    #[test]
    fn resource_adapter_is_dyn_compatible() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let _: Arc<dyn ResourceHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn resource_adapter_configure_returns_boxed_instance() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let instance = handler
            .configure(serde_json::json!({}), &ctx)
            .await
            .unwrap();
        let typed = instance.downcast::<String>().unwrap();
        assert_eq!(*typed, "pool-default");
    }

    #[tokio::test]
    async fn resource_adapter_cleanup_receives_typed_instance() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let instance: Box<dyn std::any::Any + Send + Sync> = Box::new("pool-default".to_owned());
        handler.cleanup(instance, &ctx).await.unwrap();
    }

    #[tokio::test]
    async fn resource_adapter_cleanup_fails_on_wrong_type() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let wrong_instance: Box<dyn std::any::Any + Send + Sync> = Box::new(42u32);
        let err = handler.cleanup(wrong_instance, &ctx).await.unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    // ── Adapter into_inner tests ──────────────────────────────────────────────

    #[test]
    fn stateless_adapter_into_inner_returns_action() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let action = adapter.into_inner();
        assert_eq!(action.metadata().key, nebula_core::action_key!("math.add"));
    }

    #[test]
    fn stateful_adapter_into_inner_returns_action() {
        let adapter = StatefulActionAdapter::new(CounterAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().key,
            nebula_core::action_key!("test.counter")
        );
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

    #[test]
    fn resource_adapter_into_inner_returns_action() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().key,
            nebula_core::action_key!("test.resource_action")
        );
    }

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
