# Webhook subsystem — implementation plan

**Status:** ready to execute
**Spec:** `docs/plans/2026-04-13-webhook-subsystem-spec.md`
**Deferred items:** `docs/plans/2026-04-13-webhook-api-v2.md` (skeleton created in session 3)
**Predecessor:** `docs/plans/2026-04-13-poll-quick-hardening.md` (poll work landed)

## Session order (non-negotiable)

1. **Session 1 — Hardening bundle.** Non-breaking fixes on the live
   adapter, so transport has a trustworthy plug-in target.
2. **Session 2 — HTTP transport.** New module in `nebula-api` that
   makes webhooks actually fireable. Adds capability to
   `TriggerContext`.
3. **Session 3 — Delete orphan crate.** Remove `crates/webhook/`
   after transport is green and we've salvaged `WebhookRateLimiter`.

Rationale: tech-lead's call. Hardening first gives us a solid target.
Deletion last means if transport work surfaces a salvage need, we
cannibalize before we delete (once gone, nobody re-reads it).

---

## Session 1 — Webhook hardening bundle

**Target files:**
- `crates/action/src/webhook.rs` — all logic changes
- `crates/action/tests/dx_webhook.rs` — new tests
- `crates/action/tests/webhook_signature.rs` — signature tests
- `.project/context/crates/action.md` — principle lines

**Non-breaking constraint:** zero changes to public trait shapes.
New functions are additive. `MAX_HEADER_COUNT` constant change is
source-compatible because callers who referenced it get a larger
cap, not a smaller one. `TriggerContext.webhook` field addition
happens in Session 2, not here.

### Steps

**H1. Fix `handle_request` error path — send 500 via oneshot before Err.**

Current: `handle_event` calls `self.action.handle_request(&request, &state, ctx).await?;` — the `?` drops the oneshot sender, transport gets `RecvError`, nobody knows what to return.

Target: on `Err(e)`, take the oneshot sender and send
`WebhookHttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new())`
before propagating Err. Transport sees the 500 and writes it; runtime
still sees the `ActionError` for logging/metrics.

```rust
// approximate shape
let response = match self.action.handle_request(&request, &state, ctx).await {
    Ok(r) => r,
    Err(e) => {
        if let Some(tx) = response_tx {
            let _ = tx.send(WebhookHttpResponse::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                Bytes::new(),
            ));
        }
        return Err(e);
    }
};
```

Test: `handle_request_error_sends_500_via_oneshot`.

**H2. `verify_hmac_sha256_with_timestamp` helper.**

New public function in `webhook.rs`:

```rust
pub fn verify_hmac_sha256_with_timestamp(
    request: &WebhookRequest,
    secret: &[u8],
    sig_header: &str,
    ts_header: &str,
    tolerance: Duration,
    canonicalize: impl Fn(&str, &[u8]) -> Vec<u8>,
) -> Result<SignatureOutcome, ActionError>
```

Implementation:
1. Reject empty secret (existing fail-closed rule).
2. Look up `ts_header` via `headers.get_all()` — reject multi-valued.
3. Parse timestamp as `i64` (epoch seconds). Stripe uses seconds;
   Slack uses seconds. If non-numeric → `Invalid`.
4. Compute `request.received_at()` as epoch seconds.
5. Compute `skew = |received - ts|`. If `skew > tolerance` → `Invalid`.
   Also reject future: `ts - received > 60s` → `Invalid`.
6. Build canonical bytes via `canonicalize(ts_str, request.body())`.
7. Look up `sig_header` via `get_all()` — reject multi-valued.
8. Hex-decode with `hex::decode`. On decode error → `Invalid`.
9. Compute HMAC over canonical bytes; compare with
   `mac.verify_slice(&expected)` (constant-time).

Two tests:
- `verify_hmac_sha256_with_timestamp_stripe_scheme` using
  `canonicalize = |ts, body| format!("{ts}.{}", std::str::from_utf8(body).unwrap()).into_bytes()`
- `verify_hmac_sha256_with_timestamp_slack_scheme` using
  `canonicalize = |ts, body| format!("v0:{ts}:{}", std::str::from_utf8(body).unwrap()).into_bytes()`
- Plus `verify_hmac_sha256_with_timestamp_rejects_future_timestamp`
  and `verify_hmac_sha256_with_timestamp_rejects_old_timestamp`.

**H3. `verify_hmac_sha256` rejects multi-valued signature header.**

Change implementation to use `headers.get_all(name).iter()` and
return `SignatureOutcome::Invalid` if count != 1.

Test: `verify_hmac_sha256_rejects_multi_valued_header` — build a
`WebhookRequest` with two `X-Hub-Signature-256` headers, assert
`Invalid` regardless of whether either one is valid.

**H4. `verify_hmac_sha256_base64` helper.**

Copy-paste of `verify_hmac_sha256` with one change: decode via
`base64::engine::general_purpose::STANDARD.decode(sig_hex)` instead
of `hex::decode`. Strip common prefix `"sha256="` before decode.

Test: `verify_hmac_sha256_base64_shopify_scheme` with a hand-computed
Shopify-style HMAC.

**H5. `body_json_bounded<T>(max_depth)` on `WebhookRequest`.**

Implementation: use `serde_stacker` if available, or a manual
`RecursionLimiter` wrapper around `&mut serde_json::Deserializer`.
Decision: use `serde_stacker` (workspace dep check first; if not
present, add to `nebula-action` Cargo.toml).

Existing `body_json` gets a `# Security` docstring block pointing
at `body_json_bounded` for untrusted input with recommended
`max_depth = 64`.

Test: `body_json_bounded_rejects_deep_nesting` — build a body of
`"{".repeat(100) + "}".repeat(100)` and assert Err with max_depth=64.
Also: `body_json_bounded_accepts_reasonable_nesting` with depth 10.

**H6. `handle_request` wrapped in cancellation select.**

In `handle_event`, wrap the call:

```rust
let response = tokio::select! {
    _ = ctx.cancellation.cancelled() => {
        if let Some(tx) = response_tx {
            let _ = tx.send(WebhookHttpResponse::new(
                StatusCode::SERVICE_UNAVAILABLE,
                Bytes::from_static(b"shutting down"),
            ));
        }
        return Err(ActionError::retryable("webhook trigger cancelled mid-request"));
    }
    result = self.action.handle_request(&request, &state, ctx) => {
        // ... existing match arm including H1 error path ...
    }
};
```

Test: `handle_request_cancelled_mid_flight_returns_cleanly` — start
a handler that awaits forever, call handle_event in a spawn, cancel
the token, assert the spawn returns with Err(retryable) and the
in-flight counter dropped to 0.

**H7. `TriggerHealth` wired into webhook adapter.**

Add `ctx.health.record_success(1)` when `WebhookResponse::Accept` or
`Respond` with `TriggerEventOutcome::emit`. Add `ctx.health.record_error()`
on `handle_request` Err. No `record_idle` equivalent for webhooks
(push model, not pull — idle is the default state between pushes).

Tests:
- `webhook_adapter_records_health_success_on_emit`
- `webhook_adapter_records_health_error_on_failure`

**H8. `MAX_HEADER_COUNT` → 256.**

One-line constant change. Update docstring rationale. No test change
(existing tests use smaller header counts).

**H9. Timing-invariant hex decode in `verify_hmac_sha256` (M3).**

Reorder: always run `hex::decode`, always run
`HmacSha256::new_from_slice` + `mac.update(body)`, then compare
results. If hex decode failed, substitute a zero vector for the
expected tag; compare always runs. The final branch on decode
success is outside the timing-critical path.

Audit this carefully — it's easy to reintroduce a data-dependent
branch. Review checklist:
- No `?` on hex decode result
- No early return before `mac.update` + `mac.finalize`
- Final result is `SignatureOutcome::Invalid` if either decode failed
  or compare returned err, computed via boolean AND

Existing tests should cover this; add one specifically:
`verify_hmac_sha256_invalid_hex_still_runs_mac` — assert elapsed
time is similar (within some factor) between invalid-hex and
valid-hex cases. Timing assertions are flaky, so alternative:
assert internal instrumentation counter. Simpler: just assert the
code path is taken via a test-only hook.

*Plan note: if timing-invariant proves hard to test cleanly, fall
back to code review + documentation. Don't block the session on
flaky timing tests.*

**H10. In-flight counter via `Notify` instead of `yield_now` spin.**

Replace:
```rust
while self.in_flight.load(Ordering::Acquire) > 0 {
    tokio::task::yield_now().await;
}
```

With a `tokio::sync::Notify` that `InFlightGuard::drop` calls
`notify_waiters` on when the counter hits 0. `stop()` awaits
`notify.notified()` in a loop.

Shape:
```rust
pub struct WebhookTriggerAdapter<A: WebhookAction> {
    action: A,
    state: RwLock<Option<Arc<A::State>>>,
    in_flight: AtomicU32,
    idle_notify: Arc<Notify>,
}

struct InFlightGuard { counter: Arc<AtomicU32>, notify: Arc<Notify> }
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.counter.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.notify.notify_waiters();
        }
    }
}

// In stop():
while self.in_flight.load(Ordering::Acquire) > 0 {
    self.idle_notify.notified().await;
}
```

Test: `in_flight_notify_wakes_stop` — spawn a handler that takes
100ms, call stop in parallel, assert stop returns within 200ms and
doesn't spin.

### Session 1 commit

One commit, title along the lines of:

```
fix(action): webhook trigger hardening bundle

Non-breaking audit fixes on WebhookTriggerAdapter and HMAC helpers:

- H1: handle_request error path sends 500 via oneshot before Err,
  fixing transport-hang on bug in user handler
- H2: new verify_hmac_sha256_with_timestamp helper for Stripe/Slack
  replay-window schemes, injectable canonicalizer, uses
  request.received_at() as clock source
- H3: verify_hmac_sha256 rejects multi-valued sig headers via
  get_all().count() != 1 — defense against proxy-chain injection
- H4: verify_hmac_sha256_base64 helper for Shopify/Square
- H5: body_json_bounded<T>(max_depth) replaces unguarded body_json
  for hostile JSON (serde_stacker)
- H6: handle_request wrapped in cancellation select, returns
  retryable with 503 via oneshot on shutdown mid-request
- H7: TriggerHealth wired — record_success on emit, record_error
  on handler failure (parity with poll)
- H8: MAX_HEADER_COUNT 128 → 256 for CF+NGINX+mesh stacks
- H9: verify_hmac_sha256 timing-invariant — hex decode and MAC
  always run, results AND'd
- H10: in-flight counter wakes via Notify, not yield_now spin

12 new tests across dx_webhook.rs and webhook_signature.rs.
No API breaks; existing 342 tests stay green.

Spec: docs/plans/2026-04-13-webhook-subsystem-spec.md
Plan: docs/plans/2026-04-13-webhook-implementation-plan.md

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

**Verification before commit:**
- `cargo +nightly fmt -p nebula-action`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run -p nebula-action` (expect 342 + 12 = 354+)
- `cargo test -p nebula-action --doc`

---

## Session 2 — HTTP transport in `nebula-api`

**Target files:**
- `crates/action/src/context.rs` — add `webhook` field
- `crates/action/src/webhook.rs` — add `WebhookEndpointProvider` trait
- `crates/api/src/webhook/mod.rs` — new module
- `crates/api/src/webhook/transport.rs` — `WebhookTransport` + handler
- `crates/api/src/webhook/provider.rs` — `EndpointProviderImpl`
- `crates/api/src/webhook/routing.rs` — `RoutingMap`
- `crates/api/src/webhook/ratelimit.rs` — salvaged `WebhookRateLimiter`
- `crates/api/src/lib.rs` — export `webhook` module
- `crates/api/src/app.rs` or `routes/` — mount the webhook router
- `crates/api/tests/webhook_transport_integration.rs` — new test file
- `.project/context/crates/action.md` — note new capability
- `.project/context/crates/api.md` — create or extend

### Step 2A — Add `WebhookEndpointProvider` trait to `nebula-action`

```rust
// crates/action/src/webhook.rs

/// Capability that gives webhook actions the public URL at which
/// external providers should POST. Implemented by the HTTP transport
/// layer (`nebula-api::webhook::EndpointProviderImpl`) and injected
/// into `TriggerContext` at trigger activation time.
pub trait WebhookEndpointProvider: Send + Sync + std::fmt::Debug {
    /// Full public URL to register with the external provider.
    /// Example: `https://nebula.example.com/webhooks/{uuid}/{nonce}`.
    fn endpoint_url(&self) -> &url::Url;

    /// Path component only (without scheme/host).
    fn endpoint_path(&self) -> &str;
}
```

Add `url = { workspace = true }` to `nebula-action` if not already
present.

### Step 2B — Add `webhook` field to `TriggerContext`

```rust
// crates/action/src/context.rs
use crate::webhook::WebhookEndpointProvider;

pub struct TriggerContext {
    // ... existing fields ...
    /// Webhook endpoint capability — Some for webhook triggers,
    /// None for poll / other shapes.
    pub webhook: Option<Arc<dyn WebhookEndpointProvider>>,
}
```

Add `with_webhook_endpoint` builder:

```rust
impl TriggerContext {
    #[must_use]
    pub fn with_webhook_endpoint(
        mut self,
        provider: Arc<dyn WebhookEndpointProvider>,
    ) -> Self {
        self.webhook = Some(provider);
        self
    }
}
```

Update `TriggerContext::new` to default `webhook: None`.

Test: `trigger_context_webhook_default_is_none` +
`trigger_context_with_webhook_endpoint_stores_provider`.

### Step 2C — Salvage `WebhookRateLimiter` to `nebula-api`

Source: `crates/webhook/src/rate_limit.rs` (227 LOC).
Destination: `crates/api/src/webhook/ratelimit.rs`.

Copy verbatim (with the FNV comment block), update any `use
crate::error::Error` to a local error type. Do **not** link the old
crate — straight copy. Run existing unit tests that come with the
module.

### Step 2D — `RoutingMap` and `EndpointProviderImpl`

```rust
// crates/api/src/webhook/routing.rs
use dashmap::DashMap;
use nebula_action::TriggerHandler;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct RoutingMap {
    entries: DashMap<(Uuid, String), Arc<dyn TriggerHandler>>,
}

impl RoutingMap {
    pub fn new() -> Self { /* ... */ }
    pub fn insert(&self, uuid: Uuid, nonce: String, handler: Arc<dyn TriggerHandler>);
    pub fn lookup(&self, uuid: &Uuid, nonce: &str) -> Option<Arc<dyn TriggerHandler>>;
    pub fn remove(&self, uuid: &Uuid, nonce: &str);
}
```

```rust
// crates/api/src/webhook/provider.rs
use nebula_action::webhook::WebhookEndpointProvider;
use url::Url;

#[derive(Debug)]
pub(crate) struct EndpointProviderImpl {
    url: Url,
    path: String,
}

impl EndpointProviderImpl {
    pub fn new(base_url: &Url, path_prefix: &str, uuid: Uuid, nonce: &str) -> Self {
        let path = format!("{path_prefix}/{uuid}/{nonce}");
        let mut url = base_url.clone();
        url.set_path(&path);
        Self { url, path }
    }
}

impl WebhookEndpointProvider for EndpointProviderImpl {
    fn endpoint_url(&self) -> &Url { &self.url }
    fn endpoint_path(&self) -> &str { &self.path }
}
```

### Step 2E — `WebhookTransport` struct

```rust
// crates/api/src/webhook/transport.rs

pub struct WebhookTransport {
    config: Arc<WebhookTransportConfig>,
    routing: Arc<RoutingMap>,
    rate_limiter: Option<Arc<WebhookRateLimiter>>,
}

pub struct WebhookTransportConfig {
    pub base_url: Url,
    pub path_prefix: String,
    pub body_limit_bytes: usize,
    pub response_timeout: Duration,
    pub rate_limit_per_minute: Option<u64>,
}

impl WebhookTransport {
    pub fn new(config: WebhookTransportConfig) -> Self;

    pub async fn activate(
        &self,
        handler: Arc<dyn TriggerHandler>,
    ) -> ActivationHandle;

    pub async fn deactivate(&self, handle: ActivationHandle);

    pub fn router(&self) -> axum::Router;
}

pub struct ActivationHandle {
    uuid: Uuid,
    nonce: String,
    pub provider: Arc<EndpointProviderImpl>,
}
```

### Step 2F — The axum handler

```rust
async fn webhook_handler(
    State(transport): State<Arc<WebhookTransport>>,
    Path((trigger_uuid, nonce)): Path<(Uuid, String)>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> axum::response::Response {
    // 1. Rate limit
    if let Some(limiter) = &transport.rate_limiter {
        let key = format!("{trigger_uuid}/{nonce}");
        if !limiter.check(&key).await {
            return (StatusCode::TOO_MANY_REQUESTS, "").into_response();
        }
    }

    // 2. Route lookup
    let handler = match transport.routing.lookup(&trigger_uuid, &nonce) {
        Some(h) => h,
        None => return (StatusCode::NOT_FOUND, "").into_response(),
    };

    // 3. Construct WebhookRequest
    let path = uri.path().to_string();
    let query = uri.query().map(String::from);
    let request = match WebhookRequest::try_new(method, path, query, headers, body) {
        Ok(r) => r,
        Err(ActionError::DataLimitExceeded { .. }) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "").into_response();
        }
        Err(_) => return (StatusCode::BAD_REQUEST, "").into_response(),
    };

    // 4. Oneshot + TriggerEvent
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = request.with_response_channel(tx);
    let event = TriggerEvent::new(request);

    // 5. Dispatch
    let dispatch_result = tokio::time::timeout(
        transport.config.response_timeout,
        async {
            // Build a synthetic TriggerContext for this call.
            // In production, this comes from the handler's stored ctx.
            // For now, transport holds a ctx template and clones per call.
            let ctx = transport.make_ctx_for_handler(&handler);
            handler.handle_event(event, &ctx).await
        },
    ).await;

    match dispatch_result {
        Ok(Ok(outcome)) => {
            // Outcome used by runtime for workflow emission elsewhere;
            // HTTP response comes from oneshot.
            match rx.await {
                Ok(http) => (http.status, http.headers, http.body).into_response(),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(?e, "webhook handler returned error");
            // Handler already sent 500 via oneshot per H1. If it
            // didn't, fall back to generic 500.
            match rx.try_recv() {
                Ok(http) => (http.status, http.headers, http.body).into_response(),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
            }
        }
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "").into_response(),
    }
}
```

**Open design question resolved here:** `make_ctx_for_handler` —
where does the `TriggerContext` come from on each dispatch?

Decision: transport stores a `TriggerContext` template in
`ActivationHandle` at activation time, with all capabilities wired
(emitter, scheduler, credentials, logger, health, webhook). Each
dispatch clones it and shares the `ctx` with the handler. The
adapter is the same `Arc<dyn TriggerHandler>` for the life of the
trigger, so we don't need a handler-to-context map — one context
per ActivationHandle.

Adjust `ActivationHandle` to carry the full ctx:

```rust
pub struct ActivationHandle {
    uuid: Uuid,
    nonce: String,
    pub provider: Arc<EndpointProviderImpl>,
    pub ctx: TriggerContext,  // template, cloned per dispatch
}
```

### Step 2G — Mount the router

In `crates/api/src/app.rs` (or wherever the API router is assembled),
merge the webhook router:

```rust
let app = axum::Router::new()
    .merge(existing_api_routes)
    .merge(webhook_transport.router());
```

The webhook routes live under `/webhooks/:uuid/:nonce` alongside the
existing `/workflows/*`, `/status` etc.

### Step 2H — Integration test

New file: `crates/api/tests/webhook_transport_integration.rs`.

Test flow:
1. Create a minimal `WebhookAction` that verifies HMAC and emits.
2. Build `WebhookTransport`, activate the action, get an
   `ActivationHandle`.
3. Spin up an axum `TestServer` with the transport's router.
4. POST to `/webhooks/{uuid}/{nonce}` with a valid HMAC-signed body.
5. Assert: 200 OK, `SpyEmitter` saw the emission, handler's
   `handle_request` was called.
6. Second test: bad HMAC, assert 200 OK (skip outcome), emitter saw
   nothing.
7. Third test: unknown uuid, 404.
8. Fourth test: body larger than `body_limit_bytes`, 413.
9. Fifth test: handler that hangs > `response_timeout`, 504.
10. Sixth test: `ctx.webhook.endpoint_url()` inside `on_activate`
    returns the URL we can then parse and assert matches.

### Session 2 commit

```
feat(api): webhook HTTP transport

New `nebula-api::webhook` module providing the HTTP ingress layer
for nebula-action::WebhookAction triggers:

- WebhookTransport struct with axum Router, routing map,
  optional WebhookRateLimiter (salvaged from crates/webhook/
  before deletion)
- POST /webhooks/:trigger_uuid/:nonce — path lookup, construct
  WebhookRequest, dispatch to handler.handle_event, await oneshot
  response
- ActivationHandle carries per-trigger TriggerContext with the
  new WebhookEndpointProvider capability set
- Error mapping: 404 unknown trigger, 413 oversized body, 429
  rate limit, 500 handler error, 504 handler timeout
- EndpointProviderImpl generates /webhooks/{uuid}/{nonce} URLs
  so action authors can register the URL with real providers
- WebhookEndpointProvider trait added to nebula-action::webhook
- TriggerContext.webhook: Option<Arc<dyn ...>> field added

New integration test suite exercises round-trip HMAC validation,
signature rejection, timeout, rate limit, 413, 404, and
endpoint URL presence in on_activate context.

WebhookRateLimiter copied from crates/webhook/ verbatim. Original
crate is deleted in the next session.

Spec: docs/plans/2026-04-13-webhook-subsystem-spec.md
Plan: docs/plans/2026-04-13-webhook-implementation-plan.md

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

**Verification:**
- `cargo clippy -p nebula-api -- -D warnings`
- `cargo clippy -p nebula-action -- -D warnings`
- `cargo nextest run -p nebula-api`
- `cargo nextest run -p nebula-action` (still green)

---

## Session 3 — Delete `crates/webhook/`

**Target files:**
- `Cargo.toml` (root) — remove workspace member
- `crates/webhook/` — directory deletion
- `crates/api/design/*.md` — rewrite or delete references
- `deny.toml` — check and update if webhook has entries
- `.project/context/active-work.md` — record deletion
- `docs/plans/2026-04-13-webhook-api-v2.md` — create skeleton

### Steps

**3A. Create `webhook-api-v2.md` skeleton FIRST.**

Before deletion, record everything we intentionally deferred so it
doesn't rot:

```markdown
# Webhook subsystem — deferred breaking changes

## V1 — Typestate WebhookAction<Inactive>/<Active>
Architect proposal. Collapses W1+W2+W5. Breaking trait redesign.

## V2 — Event ID dedup (Idempotency-Key / X-Event-ID)
Adapter-level bounded LRU. Blocked on runtime state storage.

## V3 — Orphan external hook reconcile loop
Operational tooling for failed rollback.

## V4 — Ed25519 / RSA-SHA1 / HMAC-SHA1 primitives
Wait for first real user. Per-provider crates preferred.

## V5 — Subscription renewal scheduler
MS Graph, Google Calendar push. Cross-cutting runtime job.

## V6 — TriggerState test/prod UUID split
Environment separation in ActionRegistry metadata.

## V7 — Outbound webhook delivery
nebula-action::http_sender action type, not a webhook crate.

## V8 — SecretString in HMAC helpers
Migration from &[u8] to &SecretString for zeroize discipline.

## V9 — WebhookAction::event_id(&request) -> Option<String>
Opt-in trait method for adapter-managed dedup.

## V10 — body_json<T> depth cap by default
Breaking: existing body_json callers may get Err on previously-
accepted deeply-nested input.
```

Commit skeleton before deletion so we don't forget anything under
the pressure of deletion work.

**3B. Verify nothing in workspace actually imports `nebula_webhook`.**

```bash
grep -rn "nebula_webhook\|nebula-webhook" crates/ apps/ examples/ \
  | grep -v "^crates/webhook/" \
  | grep -v "docs/\|design/"
```

Expected output: empty. If non-empty, investigate before deleting.

**3C. Rewrite `crates/api/design/*.md`.**

Files to update:
- `API.md` line 38 — remove webhook table row or replace with reference to new transport.
- `PLAN.md` lines 13, 35, 126 — drop `nebula-webhook` dependency mentions.
- `README.md` line 12 — drop reference.
- `ROADMAP.md` line 8 — drop reference.
- `TASKS.md` line 18 — mark task as superseded by the new transport.
- `VISION.md` line 135 — drop optional `nebula-webhook` mention.

Alternative: mark the whole design doc set as stale and write a
note pointing at the new spec + implementation plan. Plan-time
judgment call — if the docs are significantly wrong about the
architecture, stale them; if they're directionally right minus the
crate name, edit them.

**3D. Remove from workspace.**

`Cargo.toml` root: delete the `"crates/webhook",` line.

**3E. `rm -rf crates/webhook/`.**

Before running: verify git tree is clean except for deletions.
After running: `cargo check --workspace` to confirm nothing breaks.
Expected: zero compile errors, zero unresolved imports.

**3F. `deny.toml` audit.**

Grep `crates/webhook` or `nebula-webhook` in `deny.toml`. If present,
remove. Run `cargo deny check` to confirm layer rules still pass.

**3G. Update `active-work.md`.**

Add to Recently Completed:

```markdown
- **Webhook subsystem consolidation** (2026-04-13, three sessions):
  hardening bundle (H1-H10: oneshot error path, replay-window
  helper, multi-header rejection, base64 HMAC, body_json_bounded,
  cancellation safety, TriggerHealth, header cap, timing-invariant
  verify, Notify instead of spin), HTTP transport in nebula-api
  (axum router, routing map, WebhookEndpointProvider capability on
  TriggerContext, rate limiter salvaged), deleted orphan
  `crates/webhook/` (4532 LOC, zero callers, deprecated compat
  layer). Three plans: webhook-subsystem-spec.md,
  webhook-implementation-plan.md, webhook-api-v2.md.
```

Remove any `crates/webhook` mention from `Blocked` or `In Progress`
sections.

### Session 3 commit

```
chore: delete orphan crates/webhook, record deferred webhook v2

crates/webhook/ was a 4532 LOC standalone crate with its own
WebhookAction trait (incompatible shape), own TriggerCtx built on
the deprecated nebula_resource::Context compat layer, and own axum
server. Zero callers anywhere in the workspace; only pre-
implementation crates/api/design/*.md mentioned it.

With webhook hardening (commit X) and the nebula-api::webhook
transport (commit Y) in place, the orphan is redundant and its
continued presence actively harms the codebase:
- Duplicate WebhookAction trait creates import confusion
- Depends on deprecated types, extending their lifetime
- 4500 LOC that nobody reads or tests

The WebhookRateLimiter module was salvaged verbatim into
nebula-api::webhook::ratelimit before deletion.
WebhookDeliverer (outbound webhook sender) is gone; future
outbound work will be a nebula-action::http_sender action type,
not a revived crate.

Updated crates/api/design/*.md to remove references.
Updated active-work.md with the three-session summary.
Created docs/plans/2026-04-13-webhook-api-v2.md skeleton
tracking 10 deferred items: typestate redesign, event ID dedup,
reconcile loop, Ed25519/RSA/SHA1 primitives, subscription
renewal, test/prod env split, outbound delivery, SecretString
migration, opt-in dedup trait method, body_json depth cap.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

**Verification:**
- `cargo check --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`
- `cargo deny check`
- `git ls-files crates/webhook/` — expect empty

---

## Post-implementation acceptance

All three sessions are done when the spec's section 8 acceptance
criteria are met:

1. ✅ Action plugins can call `ctx.webhook.endpoint_url()` in
   `on_activate` to get a real URL.
2. ✅ Stripe signature verification fits in ≤5 lines via
   `verify_hmac_sha256_with_timestamp`.
3. ✅ A real `POST /webhooks/{uuid}/{nonce}` reaches
   `handle_request` and the action's emission creates a workflow
   execution.
4. ✅ All 15 audit findings W1–W15 have a fix or a v2 spec entry.
5. ✅ `crates/webhook/` is gone from `git ls-files`.
6. ✅ Workspace tests ≥ 360 (342 existing + hardening tests +
   transport integration tests).
7. ✅ `.project/context/crates/action.md` records webhook principles.
8. ✅ `deny.toml` still passes layer rules.

## Rollback notes

- **Session 1 rollback**: `git revert` the commit. No state leaks
  anywhere.
- **Session 2 rollback**: `git revert` the commit AND remove
  `webhook: None` from any `TriggerContext::new` test fixtures that
  already adopted the field. Low risk if done within the same
  session.
- **Session 3 rollback**: `git revert` — the deletion commit
  preserves the full crate in history. Restore is `git checkout
  HEAD^1 -- crates/webhook/` + re-add workspace member.

## Estimated effort

- Session 1: 3–4 hours (10 small fixes, 12 tests, one commit).
- Session 2: 6–8 hours (new module, transport wiring, integration
  test fixtures, handle the TriggerContext cloning design).
- Session 3: 30–60 minutes (deletion + docs + spec skeleton).

Total: ~1.5 focused days of work. Can be done in three separate
sessions or pipelined.
