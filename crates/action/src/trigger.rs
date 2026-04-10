//! Core [`TriggerAction`] trait and DX convenience patterns.
//!
//! A trigger action is a workflow starter — it lives outside the execution
//! graph and emits new workflow executions in response to external events
//! (webhooks, cron, polling, etc.). The runtime drives the `start`/`stop`
//! lifecycle and receives workflow inputs through the trigger context.
//!
//! ## DX convenience traits
//!
//! Each convenience trait has a corresponding typed adapter that implements
//! `TriggerHandler` directly. Register via convenience methods on
//! `ActionRegistry`.
//!
//! - [`WebhookAction`] — webhook lifecycle (register/handle/unregister)
//!   → `registry.register_webhook(action)`
//! - [`PollAction`] — periodic polling with in-memory cursor
//!   → `registry.register_poll(action)`

// Re-export engine-facing types for DX trait convenience.
pub use crate::handler::{IncomingEvent, TriggerEventOutcome};

use std::future::Future;

use serde::{Serialize, de::DeserializeOwned};

use crate::action::Action;
use crate::context::TriggerContext;
use crate::error::ActionError;

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

/// Webhook trigger — register/handle/unregister lifecycle.
///
/// Implement `handle_request` (required), and optionally `on_activate`/`on_deactivate`.
/// Register via `registry.register_webhook(action)`.
///
/// State from `on_activate` is stored by the adapter and passed to `handle_request`
/// (by reference) and `on_deactivate` (by value). For mutable per-event state, wrap
/// fields in `Mutex` or atomic types inside `Self::State`.
///
/// # Example
///
/// Use [`verify_hmac_sha256`](crate::webhook::verify_hmac_sha256) from
/// the `webhook` module for constant-time signature verification —
/// naive `==` comparison on HMAC digests leaks the secret via a
/// prefix-length timing side-channel.
///
/// ```rust,ignore
/// use nebula_action::trigger::{WebhookAction, IncomingEvent, TriggerEventOutcome};
/// use nebula_action::webhook::verify_hmac_sha256;
///
/// struct GitHubWebhook { secret: Vec<u8> }
///
/// impl WebhookAction for GitHubWebhook {
///     type State = WebhookReg;
///
///     async fn on_activate(&self, ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
///         Ok(WebhookReg { hook_id: register(ctx).await? })
///     }
///
///     async fn handle_request(&self, event: &IncomingEvent, _state: &Self::State, _ctx: &TriggerContext)
///         -> Result<TriggerEventOutcome, ActionError> {
///         let outcome = verify_hmac_sha256(event, &self.secret, "X-Hub-Signature-256")?;
///         if !outcome.is_valid() {
///             return Ok(TriggerEventOutcome::skip());
///         }
///         Ok(TriggerEventOutcome::emit(event.body_json()?))
///     }
///
///     async fn on_deactivate(&self, state: WebhookReg, _ctx: &TriggerContext) -> Result<(), ActionError> {
///         delete_hook(&state.hook_id).await
///     }
/// }
/// ```
pub trait WebhookAction: Action + Send + Sync + 'static {
    /// Persisted state between activate/deactivate (e.g., webhook registration ID).
    type State: Serialize + DeserializeOwned + Default + Clone + Send + Sync;

    /// Register webhook with external service. Returns state to persist.
    ///
    /// Default: returns `State::default()` (no-op activation).
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if registration fails.
    fn on_activate(
        &self,
        _ctx: &TriggerContext,
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send {
        async { Ok(Self::State::default()) }
    }

    /// Handle an incoming event. Return `Emit` to start a workflow, `Skip` to filter.
    ///
    /// State from `on_activate` is passed by reference. The adapter clones an
    /// internal Arc cheaply before this call — no contention with start/stop.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if event processing fails.
    fn handle_request(
        &self,
        event: &IncomingEvent,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<TriggerEventOutcome, ActionError>> + Send;

    /// Unregister webhook on deactivation.
    ///
    /// Receives the state stored from `on_activate`. Default: no-op.
    /// Not called if `on_activate` was never called (stop without start).
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if unregistration fails.
    fn on_deactivate(
        &self,
        _state: Self::State,
        _ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }
}

/// Periodic polling trigger with in-memory cursor.
///
/// Implement `poll_interval` and `poll`, then register via
/// `registry.register_poll(action)`.
///
/// The `PollTriggerAdapter` runs a blocking loop in `start()`:
/// sleep → poll → emit events. Cancellation via `TriggerContext::cancellation`.
///
/// **Note:** The cursor is in-memory only. Across process restarts the cursor
/// resets to `Default::default()` — full persistence requires runtime storage
/// integration (post-v1).
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::trigger::PollAction;
///
/// struct RssPoll { feed_url: String }
/// impl PollAction for RssPoll {
///     type Cursor = String;
///     type Event = serde_json::Value;
///     fn poll_interval(&self) -> std::time::Duration { std::time::Duration::from_secs(300) }
///     async fn poll(&self, cursor: &mut String, ctx: &TriggerContext)
///         -> Result<Vec<serde_json::Value>, ActionError> {
///         let items = fetch_rss(&self.feed_url, cursor).await?;
///         Ok(items)
///     }
/// }
/// ```
pub trait PollAction: Action + Send + Sync + 'static {
    /// Cursor type for tracking poll position.
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    /// Event type emitted per poll cycle (each becomes a workflow execution).
    type Event: Serialize + Send + Sync;

    /// Interval between poll cycles.
    fn poll_interval(&self) -> std::time::Duration;

    /// Execute one poll cycle. Mutate cursor to track position.
    ///
    /// Return events to emit. Empty vec = nothing new.
    ///
    /// # Errors
    ///
    /// [`ActionError::Retryable`] for transient failures (skip this cycle),
    /// [`ActionError::Fatal`] to stop the trigger permanently.
    fn poll(
        &self,
        cursor: &mut Self::Cursor,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<Vec<Self::Event>, ActionError>> + Send;
}
