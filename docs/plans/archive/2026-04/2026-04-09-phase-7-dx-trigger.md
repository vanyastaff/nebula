# Phase 7: DX Types — TriggerAction Family

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `TriggerEventOutcome` + `handle_event`/`accepts_events` to `TriggerHandler`, create `trigger.rs` with DX traits (`WebhookAction`, `PollAction`) and typed adapters (`WebhookTriggerAdapter`, `PollTriggerAdapter`), plus registry convenience methods and test harnesses.

**Architecture:** `IncomingEvent` and `TriggerEventOutcome` live in `handler.rs` (engine-facing types alongside `TriggerHandler`). New `crates/action/src/trigger.rs` — `WebhookAction` + `PollAction` DX traits, re-exports `IncomingEvent`. Each DX trait gets a typed adapter that implements `TriggerHandler` directly. `TriggerHandler::handle_event` takes `IncomingEvent` (typed), not `Value` — no serialization round-trip. `WebhookTriggerAdapter` stores state in `RwLock<Option<Arc<State>>>` for proper activate/deactivate lifecycle. `PollTriggerAdapter` runs blocking poll loop in `start()`. Registry gets `register_webhook()` / `register_poll()` convenience methods.

**Tech Stack:** Rust 1.94, `serde`/`serde_json`, `async-trait`, `tokio` (time, select!), `tokio-util` (CancellationToken), `parking_lot` (RwLock for webhook state)

**Prerequisites:** Phase 6 done. `TriggerAction`, `TriggerHandler`, `TriggerActionAdapter`, `TriggerTestHarness` exist.

**Why adapters, not macros:** Review of Phase 6 macros revealed that `impl_webhook_action!` cannot override `TriggerHandler::handle_event` (macro generates `impl TriggerAction`, but `handle_event` lives on `TriggerHandler`). Adapters solve this naturally — `WebhookTriggerAdapter` impl `TriggerHandler` directly with state storage, event routing, and proper lifecycle. Same pattern as existing `StatefulActionAdapter`.

---

### Task 1: Add `IncomingEvent` and `TriggerEventOutcome` to handler.rs

**Files:**
- Modify: `crates/action/src/handler.rs`

**Step 1: Add `IncomingEvent` struct**

Add before the `TriggerHandler` trait definition (around line 470). Add imports if missing:

```rust
use std::collections::HashMap;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
```

```rust
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
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let event = IncomingEvent::new(b"payload", &[("Content-Type", "application/json")]);
    /// ```
    #[must_use]
    pub fn new(body: &[u8], headers: &[(&str, &str)]) -> Self {
        Self {
            body: body.to_vec(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
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

    /// Get a header value by key (case-insensitive).
    #[must_use]
    pub fn header(&self, key: &str) -> Option<&str> {
        let key_lower = key.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == key_lower)
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
    pub fn body_json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}
```

**Step 2: Add `TriggerEventOutcome` enum**

After `IncomingEvent`:

```rust
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
    /// Create a skip outcome (event filtered, no execution).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(TriggerEventOutcome::skip())
    /// ```
    #[must_use]
    pub fn skip() -> Self {
        Self::Skip
    }

    /// Create a single-emit outcome.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(TriggerEventOutcome::emit(payload))
    /// ```
    #[must_use]
    pub fn emit(payload: Value) -> Self {
        Self::Emit(payload)
    }

    /// Create a batch-emit outcome. Empty vec becomes `Skip`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(TriggerEventOutcome::emit_many(vec![p1, p2]))
    /// ```
    #[must_use]
    pub fn emit_many(payloads: Vec<Value>) -> Self {
        if payloads.is_empty() {
            Self::Skip
        } else {
            Self::EmitMany(payloads)
        }
    }

    /// Whether this outcome will emit any executions.
    #[must_use]
    pub fn will_emit(&self) -> bool {
        !matches!(self, Self::Skip)
    }
}
```

**Step 3: Add `accepts_events` and `handle_event` to `TriggerHandler`**

In the `TriggerHandler` trait, after `stop()`. Note: `handle_event` takes typed `IncomingEvent`, NOT `Value` — no JSON round-trip:

```rust
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
        Err(ActionError::fatal("trigger does not accept external events"))
    }
```

**Step 4: Run check + tests**

Run: `cargo check -p nebula-action && cargo nextest run -p nebula-action`
Expected: compiles, all tests pass

**Step 5: Commit**

```
feat(action): IncomingEvent, TriggerEventOutcome, handle_event on TriggerHandler
```

---

### Task 2: Tests for `TriggerEventOutcome`

**Files:**
- Modify: `crates/action/src/handler.rs` (add tests in existing `#[cfg(test)]` module)

**Step 1: Add tests**

```rust
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
        let o = TriggerEventOutcome::emit_many(vec![
            serde_json::json!(1),
            serde_json::json!(2),
        ]);
        assert!(o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::EmitMany(v) if v.len() == 2));
    }

    #[test]
    fn trigger_event_outcome_empty_emit_many_is_skip() {
        let o = TriggerEventOutcome::emit_many(vec![]);
        assert!(!o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Skip));
    }
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action trigger_event_outcome`
Expected: 4 PASS

**Step 3: Commit**

```
test(action): TriggerEventOutcome unit tests
```

---

### Task 3: Create `trigger.rs` shell with IncomingEvent re-export

**Files:**
- Create: `crates/action/src/trigger.rs`
- Modify: `crates/action/src/lib.rs`

**Step 1: Create `trigger.rs` shell (DX traits will be added in Tasks 4+6)**

```rust
//! DX convenience traits for common [`TriggerAction`](crate::execution::TriggerAction) patterns.
//!
//! Each trait has a corresponding typed adapter that implements
//! [`TriggerHandler`](crate::handler::TriggerHandler) directly.
//! Register via convenience methods on [`ActionRegistry`](crate::registry::ActionRegistry).
//!
//! - [`WebhookAction`] — webhook lifecycle (register/handle/unregister)
//!   → `registry.register_webhook(action)`
//! - [`PollAction`] — periodic polling with in-memory cursor
//!   → `registry.register_poll(action)`

// Re-export engine-facing types for DX trait convenience.
pub use crate::handler::{IncomingEvent, TriggerEventOutcome};
```

**Step 2: Add `trigger` module to `lib.rs`**

Add after `pub mod testing;`:

```rust
/// DX convenience types for common TriggerAction patterns (webhook, poll).
pub mod trigger;
```

**Step 3: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles

**Step 4: Commit**

```
feat(action): trigger.rs module shell with IncomingEvent re-export
```

---

### Task 4: Add `WebhookAction` trait + `WebhookTriggerAdapter`

**Files:**
- Modify: `crates/action/src/trigger.rs`
- Modify: `crates/action/src/handler.rs`

**Step 1: Append `WebhookAction` trait to `trigger.rs`**

```rust
// ── WebhookAction ───────────────────────────────────────────────────────────

/// Webhook trigger — register/handle/unregister lifecycle.
///
/// Implement `handle_request` (required), and optionally `on_activate`/`on_deactivate`.
/// Register via `registry.register_webhook(action)`.
///
/// The [`WebhookTriggerAdapter`](crate::handler::WebhookTriggerAdapter) stores
/// state from `on_activate` and passes it to `handle_request`/`on_deactivate`.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::trigger::{WebhookAction, IncomingEvent};
/// use nebula_action::handler::TriggerEventOutcome;
///
/// struct GitHubWebhook { secret: String }
///
/// impl WebhookAction for GitHubWebhook {
///     type State = WebhookReg;
///
///     async fn on_activate(&self, ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
///         Ok(WebhookReg { hook_id: register(ctx).await? })
///     }
///
///     async fn handle_request(&self, event: &IncomingEvent, state: &Self::State, ctx: &TriggerContext)
///         -> Result<TriggerEventOutcome, ActionError> {
///         if !verify(&state.secret, &event.body, event.header("X-Hub-Signature-256")) {
///             return Ok(TriggerEventOutcome::skip());
///         }
///         Ok(TriggerEventOutcome::emit(event.body_json()?))
///     }
///
///     async fn on_deactivate(&self, state: WebhookReg, ctx: &TriggerContext) -> Result<(), ActionError> {
///         delete_hook(&state.hook_id).await
///     }
/// }
///
/// registry.register_webhook(GitHubWebhook { secret: "...".into() });
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
    async fn on_activate(
        &self,
        _ctx: &TriggerContext,
    ) -> Result<Self::State, ActionError> {
        Ok(Self::State::default())
    }

    /// Handle an incoming event. Return `Emit` to start a workflow, `Skip` to filter.
    ///
    /// State from `on_activate` is passed by reference via `Arc`.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if event processing fails.
    async fn handle_request(
        &self,
        event: &IncomingEvent,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError>;

    /// Unregister webhook on deactivation.
    ///
    /// Receives the state stored from `on_activate`. Default: no-op.
    /// Not called if `on_activate` was never called (stop without start).
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if unregistration fails.
    async fn on_deactivate(
        &self,
        _state: Self::State,
        _ctx: &TriggerContext,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}
```

**Step 2: Add `WebhookTriggerAdapter` to `handler.rs`**

Add after `TriggerActionAdapter` impl block (around line 306). Will need to add imports:

```rust
use std::sync::Arc;
use crate::trigger::WebhookAction;
use parking_lot::RwLock;
// IncomingEvent is already in handler.rs (same module) — no import needed.
```

Then the adapter:

```rust
// ── WebhookTriggerAdapter ─────────────────────────────────────────────────

/// Wraps a [`WebhookAction`] as a [`dyn TriggerHandler`] with state management.
///
/// Stores state from `on_activate` in a `RwLock<Option<Arc<State>>>`, passes
/// it to `handle_request` (read lock, no clone) and `on_deactivate`.
/// Overrides `accepts_events()` and `handle_event()`.
///
/// Created automatically by [`ActionRegistry::register_webhook`].
///
/// # Example
///
/// ```rust,ignore
/// let handler: Arc<dyn TriggerHandler> = Arc::new(WebhookTriggerAdapter::new(my_webhook));
/// ```
pub struct WebhookTriggerAdapter<A: WebhookAction> {
    action: A,
    // Arc so handle_event can clone cheaply and release the lock before await.
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
                // Consume the Arc — in normal flow no concurrent handle_event holds it.
                // If a race occurs (rare), fall back to cloning inner state.
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
        // Clone Arc under lock, release before await — prevents deadlock with start/stop.
        let state = self
            .state
            .read()
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                ActionError::fatal("handle_event called before start — no state available")
            })?;

        self.action.handle_request(&event, &state, ctx).await
    }
}

impl<A: WebhookAction> std::fmt::Debug for WebhookTriggerAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookTriggerAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}
```

**Step 3: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles

**Step 4: Commit**

```
feat(action): WebhookAction trait and WebhookTriggerAdapter
```

---

### Task 5: Tests for WebhookAction

**Files:**
- Create: `crates/action/tests/dx_webhook.rs`

**Step 1: Write the tests**

```rust
//! Integration tests for WebhookAction DX trait + WebhookTriggerAdapter.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nebula_action::action::Action;
use nebula_action::context::TriggerContext;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::handler::{
    IncomingEvent, TriggerEventOutcome, TriggerHandler, WebhookTriggerAdapter,
};
use nebula_action::metadata::ActionMetadata;
use nebula_action::testing::TestContextBuilder;
use nebula_action::trigger::WebhookAction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WebhookReg {
    hook_id: String,
}

struct TestWebhook {
    meta: ActionMetadata,
    secret: String,
    activated: Arc<AtomicBool>,
    deactivated: Arc<AtomicBool>,
}

impl ActionDependencies for TestWebhook {}
impl Action for TestWebhook {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl WebhookAction for TestWebhook {
    type State = WebhookReg;

    async fn on_activate(&self, _ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
        self.activated.store(true, Ordering::Relaxed);
        Ok(WebhookReg { hook_id: "hook_123".into() })
    }

    async fn handle_request(
        &self,
        event: &IncomingEvent,
        _state: &WebhookReg,
        _ctx: &TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        let sig = event.header("X-Secret").unwrap_or_default();
        if sig != self.secret {
            return Ok(TriggerEventOutcome::skip());
        }
        let payload = event.body_json::<serde_json::Value>()
            .map_err(|e| ActionError::validation(format!("bad json: {e}")))?;
        Ok(TriggerEventOutcome::emit(payload))
    }

    async fn on_deactivate(&self, _state: WebhookReg, _ctx: &TriggerContext) -> Result<(), ActionError> {
        self.deactivated.store(true, Ordering::Relaxed);
        Ok(())
    }
}

fn make_webhook() -> (TestWebhook, Arc<AtomicBool>, Arc<AtomicBool>) {
    let activated = Arc::new(AtomicBool::new(false));
    let deactivated = Arc::new(AtomicBool::new(false));
    (TestWebhook {
        meta: ActionMetadata::builder("test.webhook", "Test Webhook").build(),
        secret: "mysecret".into(),
        activated: activated.clone(),
        deactivated: deactivated.clone(),
    }, activated, deactivated)
}

#[tokio::test]
async fn webhook_adapter_start_stores_state() {
    let (webhook, activated, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    assert!(activated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn webhook_adapter_stop_passes_stored_state() {
    let (webhook, _, deactivated) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    adapter.stop(&ctx).await.unwrap();
    assert!(deactivated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn webhook_adapter_handle_event_emits_on_valid_secret() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();

    let event = IncomingEvent::new(
        br#"{"action":"push"}"#,
        &[("X-Secret", "mysecret")],
    );

    let outcome = adapter.handle_event(event, &ctx).await.unwrap();
    assert!(outcome.will_emit());
}

#[tokio::test]
async fn webhook_adapter_handle_event_skips_on_bad_secret() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();

    let event = IncomingEvent::new(
        br#"{"action":"push"}"#,
        &[("X-Secret", "wrong")],
    );

    let outcome = adapter.handle_event(event, &ctx).await.unwrap();
    assert!(!outcome.will_emit());
}

#[tokio::test]
async fn webhook_adapter_accepts_events() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    assert!(adapter.accepts_events());
}

#[tokio::test]
async fn webhook_adapter_handle_event_before_start_fails() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    // handle_event without start() → Fatal error
    let event = IncomingEvent::new(
        br#"{"action":"push"}"#,
        &[("X-Secret", "mysecret")],
    );

    let result = adapter.handle_event(event, &ctx).await;
    assert!(result.is_err());
    nebula_action::assert_fatal!(result);
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action dx_webhook`
Expected: all 5 PASS

**Step 3: Commit**

```
test(action): WebhookAction + WebhookTriggerAdapter integration tests
```

---

### Task 6: Add `PollAction` trait + `PollTriggerAdapter`

**Files:**
- Modify: `crates/action/src/trigger.rs`
- Modify: `crates/action/src/handler.rs`

**Step 1: Append `PollAction` to `trigger.rs`**

```rust
// ── PollAction ──────────────────────────────────────────────────────────────

/// Periodic polling trigger with cursor persistence.
///
/// Implement `poll_interval` and `poll`, then register via
/// `registry.register_poll(action)`.
///
/// The [`PollTriggerAdapter`](crate::handler::PollTriggerAdapter) runs a
/// blocking loop in `start()`: sleep → poll → emit events. Cancelled via
/// `TriggerContext::cancellation`.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::trigger::PollAction;
///
/// struct RssPoll { feed_url: String }
/// impl PollAction for RssPoll {
///     type Cursor = String;
///     type Event = Value;
///     fn poll_interval(&self) -> Duration { Duration::from_secs(300) }
///     async fn poll(&self, cursor: &mut String, ctx: &TriggerContext)
///         -> Result<Vec<Value>, ActionError> {
///         let items = fetch_rss(&self.feed_url, cursor).await?;
///         Ok(items)
///     }
/// }
/// registry.register_poll(RssPoll { feed_url: "...".into() });
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
    async fn poll(
        &self,
        cursor: &mut Self::Cursor,
        ctx: &TriggerContext,
    ) -> Result<Vec<Self::Event>, ActionError>;
}
```

**Step 2: Add `PollTriggerAdapter` to `handler.rs`**

Add after `WebhookTriggerAdapter`:

```rust
use crate::trigger::PollAction;

// ── PollTriggerAdapter ────────────────────────────────────────────────────

/// Wraps a [`PollAction`] as a [`dyn TriggerHandler`].
///
/// `start()` runs a blocking loop: sleep → poll → emit events.
/// Cancellation via `TriggerContext::cancellation`.
/// `stop()` is a no-op (cancellation token handles shutdown).
///
/// Created automatically by [`ActionRegistry::register_poll`].
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
                                let payload = serde_json::to_value(&event)
                                    .map_err(|e| ActionError::fatal(
                                        format!("poll event serialization failed: {e}")
                                    ))?;
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
        Ok(())
    }
}

impl<A: PollAction> std::fmt::Debug for PollTriggerAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollTriggerAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}
```

**Step 3: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles

**Step 4: Commit**

```
feat(action): PollAction trait and PollTriggerAdapter
```

---

### Task 7: Tests for PollAction

**Files:**
- Create: `crates/action/tests/dx_poll.rs`

**Step 1: Write the tests**

```rust
//! Integration tests for PollAction DX trait + PollTriggerAdapter.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nebula_action::action::Action;
use nebula_action::context::TriggerContext;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::handler::{PollTriggerAdapter, TriggerHandler};
use nebula_action::metadata::ActionMetadata;
use nebula_action::testing::TestContextBuilder;
use nebula_action::trigger::PollAction;

struct TickPoller {
    meta: ActionMetadata,
    poll_count: Arc<AtomicU32>,
}

impl ActionDependencies for TickPoller {}
impl Action for TickPoller {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl PollAction for TickPoller {
    type Cursor = u32;
    type Event = serde_json::Value;

    fn poll_interval(&self) -> Duration { Duration::from_millis(10) }

    async fn poll(
        &self,
        cursor: &mut u32,
        _ctx: &TriggerContext,
    ) -> Result<Vec<serde_json::Value>, ActionError> {
        *cursor += 1;
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![serde_json::json!({"tick": *cursor})])
    }
}

#[tokio::test(start_paused = true)]
async fn poll_adapter_emits_events() {
    let poll_count = Arc::new(AtomicU32::new(0));
    let adapter = PollTriggerAdapter::new(TickPoller {
        meta: ActionMetadata::builder("tick", "Tick").build(),
        poll_count: poll_count.clone(),
    });
    let (ctx, emitter, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let ctx_clone = ctx.clone();
    let handle = tokio::spawn(async move { adapter.start(&ctx_clone).await });

    // Deterministic: advance time past the first poll interval, yield repeatedly
    // to let the spawned task process the sleep wakeup.
    tokio::time::advance(Duration::from_millis(15)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();

    let result = handle.await.unwrap();
    assert!(result.is_ok());

    // Relaxed assertion: we care that SOME polling happened deterministically,
    // not the exact count (which depends on yield ordering).
    let count = poll_count.load(Ordering::Relaxed);
    assert!(count >= 1, "expected at least 1 poll, got {count}");
    assert!(emitter.count() >= 1, "expected at least 1 emit, got {}", emitter.count());
}

#[tokio::test]
async fn poll_adapter_stop_is_noop() {
    let adapter = PollTriggerAdapter::new(TickPoller {
        meta: ActionMetadata::builder("tick", "Tick").build(),
        poll_count: Arc::new(AtomicU32::new(0)),
    });
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    assert!(adapter.stop(&ctx).await.is_ok());
}

#[tokio::test]
async fn poll_adapter_does_not_accept_events() {
    let adapter = PollTriggerAdapter::new(TickPoller {
        meta: ActionMetadata::builder("tick", "Tick").build(),
        poll_count: Arc::new(AtomicU32::new(0)),
    });
    assert!(!adapter.accepts_events());
}

#[tokio::test]
async fn poll_action_cursor_advances() {
    let poll_count = Arc::new(AtomicU32::new(0));
    let action = TickPoller {
        meta: ActionMetadata::builder("tick", "Tick").build(),
        poll_count,
    };
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();
    let mut cursor = 0u32;

    let events = action.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(cursor, 1);
    assert_eq!(events.len(), 1);

    let events = action.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(cursor, 2);
    assert_eq!(events.len(), 1);
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-action dx_poll`
Expected: all 4 PASS

**Step 3: Commit**

```
test(action): PollAction + PollTriggerAdapter integration tests
```

---

### Task 8: Add `register_webhook` and `register_poll` to `ActionRegistry`

**Files:**
- Modify: `crates/action/src/registry.rs`

**Step 1: Read current `register_trigger` method**

It's around line 152. Follow the same pattern.

**Step 2: Add convenience methods**

After `register_trigger`, add:

```rust
    /// Register a webhook action — wraps in [`WebhookTriggerAdapter`] automatically.
    ///
    /// [`WebhookTriggerAdapter`]: crate::handler::WebhookTriggerAdapter
    pub fn register_webhook<A>(&mut self, action: A)
    where
        A: crate::trigger::WebhookAction + Send + Sync + 'static,
        <A as crate::trigger::WebhookAction>::State: Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(
            Arc::new(crate::handler::WebhookTriggerAdapter::new(action)),
        );
        self.register(metadata, handler);
    }

    /// Register a poll action — wraps in [`PollTriggerAdapter`] automatically.
    ///
    /// [`PollTriggerAdapter`]: crate::handler::PollTriggerAdapter
    pub fn register_poll<A>(&mut self, action: A)
    where
        A: crate::trigger::PollAction + Send + Sync + 'static,
        <A as crate::trigger::PollAction>::Cursor: Send + Sync,
        <A as crate::trigger::PollAction>::Event: Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(
            Arc::new(crate::handler::PollTriggerAdapter::new(action)),
        );
        self.register(metadata, handler);
    }
```

**Step 3: Run check + tests**

Run: `cargo check -p nebula-action && cargo nextest run -p nebula-action`
Expected: compiles, all pass

**Step 4: Commit**

```
feat(action): register_webhook and register_poll on ActionRegistry
```

---

### Task 9: Wire up exports and prelude

**Files:**
- Modify: `crates/action/src/lib.rs`
- Modify: `crates/action/src/prelude.rs`

**Step 1: Add re-exports to `lib.rs`**

In the public re-exports section:

```rust
pub use handler::{IncomingEvent, PollTriggerAdapter, TriggerEventOutcome, WebhookTriggerAdapter};
pub use trigger::{PollAction, WebhookAction};
```

**Step 2: Add to prelude**

```rust
pub use crate::handler::{IncomingEvent, TriggerEventOutcome};
pub use crate::trigger::{PollAction, WebhookAction};
```

**Step 3: Run full check + clippy + tests**

Run: `cargo fmt && cargo clippy -p nebula-action -- -D warnings && cargo nextest run -p nebula-action`
Expected: all pass, zero warnings

**Step 4: Commit**

```
feat(action): wire up trigger DX types in exports and prelude
```

---

### Task 10: Workspace validation + context docs

**Files:**
- Modify: `.claude/crates/action.md`

**Step 1: Run workspace check**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`
Expected: all pass (sandbox failures are pre-existing)

**Step 2: Update `.claude/crates/action.md`**

Add to "Key Decisions":

```
- `trigger.rs`: DX traits for TriggerAction — `WebhookAction` (register/handle/unregister lifecycle) + `PollAction` (blocking poll loop with cursor). No macros — typed adapters (`WebhookTriggerAdapter`, `PollTriggerAdapter`) implement `TriggerHandler` directly, same pattern as `StatefulActionAdapter`. Registry convenience methods: `register_webhook()`, `register_poll()`.
- `TriggerEventOutcome` enum (Skip/Emit/EmitMany) on `TriggerHandler` — universal event ingress. `accepts_events()` + `handle_event()` with default error. Webhook layer calls `handle_event()` on registered triggers where `accepts_events() == true`.
- `IncomingEvent` — transport-agnostic event struct (body bytes + headers map + source). Not HTTP-specific. Lives in `handler.rs` (not `trigger.rs`) to avoid circular imports — re-exported from `trigger.rs`.
- `TriggerHandler::handle_event` takes typed `IncomingEvent` directly (NOT `Value`) — no JSON round-trip, no 4x body bloat. `handle_event` is a transport primitive, not user-input erasure.
- `WebhookTriggerAdapter` stores state from `on_activate` in `Mutex<Option<State>>`, passes to `handle_request`/`on_deactivate`. Proper lifecycle management.
```

Add to "Traps":

```
- `PollTriggerAdapter::start()` blocks until cancellation — engine MUST spawn it in a task. In tests use `tokio::spawn` + `tokio::time::pause()`/`advance()` for deterministic timing.
- `WebhookTriggerAdapter::handle_event` before `start()` returns `ActionError::Fatal` — webhook layer must ensure trigger is started before routing events.
- `IncomingEvent` lives in `handler.rs` (not `trigger.rs`) to avoid circular imports — `trigger.rs` re-exports it. Both are valid public paths: `nebula_action::handler::IncomingEvent` and `nebula_action::trigger::IncomingEvent`.
- `handle_event()` on TriggerHandler defaults to Fatal error — only call on triggers where `accepts_events() == true`.
- `ctx.cancellation` is accessed as a pub field in `PollTriggerAdapter` — tech debt, should be a method. Tracked for TriggerContext refactor.
- `PollTriggerAdapter` emit failures are silently skipped (`let _ = ...`). Fatal poll errors stop the loop, retryable errors skip the cycle.
```

**Step 3: Commit**

```
docs(action): update context docs for Phase 7 trigger DX types
```

---

## Exit Criteria Verification

| Criterion                                           | Test                                                     |
|-----------------------------------------------------|----------------------------------------------------------|
| WebhookAdapter start stores state                   | `dx_webhook::webhook_adapter_start_stores_state`         |
| WebhookAdapter stop passes stored state             | `dx_webhook::webhook_adapter_stop_passes_stored`         |
| WebhookAdapter handle_event emits on valid secret   | `dx_webhook::webhook_adapter_handle_event_emits`         |
| WebhookAdapter handle_event skips on bad secret     | `dx_webhook::webhook_adapter_handle_event_skips`         |
| WebhookAdapter accepts_events = true                | `dx_webhook::webhook_adapter_accepts_events`             |
| WebhookAdapter handle_event before start = Fatal    | `dx_webhook::webhook_adapter_handle_event_before_start`  |
| PollAdapter emits events in loop (deterministic)    | `dx_poll::poll_adapter_emits_events`                     |
| PollAdapter stop is no-op                           | `dx_poll::poll_adapter_stop_is_noop`                     |
| PollAdapter accepts_events = false                  | `dx_poll::poll_adapter_does_not_accept_events`           |
| PollAction cursor advances                          | `dx_poll::poll_action_cursor_advances`                   |
| TriggerEventOutcome constructors                    | `handler::trigger_event_outcome_*` (4 tests)             |

## Design Notes

**Why adapters instead of macros:** Macros generate `impl TriggerAction`, but `handle_event` lives on `TriggerHandler`. Adapters implement `TriggerHandler` directly — can override `accepts_events` and `handle_event` naturally. Also solves state persistence (RwLock inside adapter).

**State management:** `WebhookTriggerAdapter` stores state as `RwLock<Option<Arc<State>>>`. `handle_event` clones the `Arc` under the read lock (cheap pointer copy), then releases the lock BEFORE calling `handle_request().await` — prevents deadlock with concurrent `start`/`stop` taking a write lock (parking_lot RwLock is not reentrant and not async-aware). `stop()` uses `Arc::try_unwrap` with fallback clone for safety. `handle_event` before `start()` returns Fatal error (no silent default state).

**Emit failure resilience:** `PollTriggerAdapter` distinguishes `Fatal` (stop loop) from `Retryable` (skip cycle, continue). Emit failures are logged and skipped — transient emitter issues don't kill the trigger.

**async fn in DX traits:** DX traits use `async fn` (Rust 1.94 native RPITIT), not `impl Future<...> + Send`. Traits are used via static dispatch (`Adapter<A: WebhookAction>`), not `dyn` — object safety is not a concern.

**Why `handle_event(IncomingEvent)` typed, not `Value`:** `handle_event` is a transport primitive with a fixed structure (body + headers + source) — false symmetry with `StatelessHandler` which takes `Value` for user-defined input shapes. Typed parameter eliminates double serialization (raw `Vec<u8>` body would balloon 4x as JSON array of numbers), removes deserialization error path, and lets webhook layer pass `IncomingEvent` directly without knowing internal structure. `IncomingEvent` lives in `handler.rs` alongside `TriggerHandler` to avoid circular import with `trigger.rs`. Re-exported from `trigger.rs` for public DX API.

**EventTrigger (deferred):** Same adapter pattern: `EventTriggerAdapter<A>` with `start()` = connect + event loop + reconnect. Additive, no changes to existing code.
