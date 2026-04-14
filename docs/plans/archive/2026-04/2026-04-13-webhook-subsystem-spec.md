# Webhook subsystem — target state spec

**Status:** spec (target state, informs implementation plans)
**Date:** 2026-04-13
**Scope:** `crates/action/src/webhook.rs`, `crates/action/src/context.rs`,
`crates/api/src/webhook/` (new), deletion of `crates/webhook/`
**Informed by:** poll-trigger audit methodology (2026-04-13-poll-*.md),
4 parallel agent reviews (architect, tech-lead, sdk-user, security-lead)

## 1. Motivation

Nebula has two incompatible `WebhookAction` traits in the workspace:

- **`nebula-action::webhook`** (1008 LOC) — active, wired via
  `ActionRegistry::register_webhook`, 342 existing tests. Has solid
  HMAC primitives, in-flight tracking, double-start rejection. **Missing
  the HTTP transport layer** — `handle_event` is called by nobody.
- **`crates/webhook/`** (4532 LOC) — orphan. Duplicate `WebhookAction`
  trait with incompatible shape (`on_subscribe/on_webhook/...`). Own
  `TriggerCtx` on top of the **deprecated** `nebula_resource::Context`
  compat re-export. Own axum server, own route map, own rate limiter.
  **Zero callers anywhere in the workspace**; only pre-implementation
  `crates/api/design/*.md` references it.

On top of this, the live adapter has 15 audit findings (W1–W15)
spanning critical (oneshot leak, replay footgun, multi-valued header
trap), major (cancellation ignored, JSON bomb, busy spin) and minor
(header cap, secret type).

30 real-world webhook integrations were walked end-to-end (GitHub,
Stripe, Slack, Discord, Telegram, Shopify, Twilio, Mailgun, Zoom, MS
Graph, AWS SNS, Salesforce, etc.). Fit distribution against the live
trait: **12 Good / 12 Medium / 6 Poor**. The Medium tier is dominated
by replay-window and base64-HMAC gaps; the Poor tier is dominated by
missing crypto primitives (Ed25519, RSA) and infrastructure gaps
(subscription renewal, mTLS).

## 2. Goals

1. **One source of truth** for `WebhookAction` — `nebula-action` wins,
   the orphan crate is deleted.
2. **End-to-end fireable webhooks** — HTTP transport wired so that a
   `WebhookAction` plugin actually runs when a real provider POSTs.
3. **Security-hardened adapter** — replay protection, constant-time
   everywhere, multi-header rejection, JSON bomb cap, real health
   reporting.
4. **DX for action authors** — they can register a webhook with a
   real provider without guessing URLs, write Stripe/Slack signature
   verification in ≤5 lines, and never touch a wall clock.
5. **Single axum server for Nebula** — webhooks share the API server,
   no second ingress port, no duplicated middleware stack.

## 3. Non-goals (explicitly deferred)

- **Typestate `WebhookAction<Inactive>/<Active>`** — architect's
  proposal. Collapses W1+W2+W5 into one `type` change but is a
  breaking trait redesign. `webhook-api-v2.md`.
- **Cursor/state persistence across restarts** — blocked on runtime
  storage (same blocker as poll F8).
- **Event ID deduplication at adapter level** — blocked on state
  persistence. Action authors implement their own bounded LRU until
  runtime gains `TriggerStateStore`.
- **Ed25519 / RSA-SHA1 / HMAC-SHA1 primitives** — wait for first real
  provider user. Not enough signal to justify pulling `ed25519-dalek`
  or RSA into `nebula-action` core.
- **Stripe/Slack-specific parser helpers** (`parse_stripe_signature`,
  `parse_slack_v0`) — the generic `verify_hmac_sha256_with_timestamp`
  helper covers both via a `canonicalize_fn` parameter. Dedicated
  per-provider parsers can land later if they ship in a provider
  crate.
- **Subscription renewal scheduler** (MS Graph 3-day expiry, Google
  Calendar push renewal). Requires runtime-scheduled background job
  for trigger-owned work — same class as cursor persistence.
- **Outbound webhook delivery** (`WebhookDeliverer` from orphan
  crate). Belongs in a future `nebula-action::http_sender` action
  type, not a webhook crate. Rewrite when needed; do not salvage.
- **TriggerState test/prod UUID separation** from orphan crate.
  Environment split belongs in `ActionRegistry` metadata when the
  registry grows environment awareness, not in transport.
- **Reconcile loop for orphan external hooks** (W2). Operational
  tooling, separate design doc.
- **IP whitelist / mTLS / JWT path authentication** — wait for first
  provider that needs it (Salesforce, some enterprise webhooks).

## 4. Target architecture

### 4.1 Layer placement

| Component | Crate | Layer |
|-----------|-------|-------|
| `WebhookAction` trait | `nebula-action` | Business |
| `WebhookRequest` / `WebhookResponse` | `nebula-action` | Business |
| HMAC primitives | `nebula-action` | Business |
| `WebhookTriggerAdapter` | `nebula-action` | Business |
| `WebhookEndpointProvider` trait | `nebula-action` | Business |
| `WebhookTransport` (axum module) | `nebula-api` | API |
| `WebhookRateLimiter` (salvaged) | `nebula-api` | API |
| `EndpointProviderImpl` | `nebula-api` | API |

Layer invariant: API → Business, no upward dependency. `nebula-action`
declares the provider trait; `nebula-api` implements it. Action code
never imports `nebula-api`.

### 4.2 Contract between action and transport

**TriggerContext gains an optional capability:**

```rust
// crates/action/src/context.rs
pub struct TriggerContext {
    // ... existing fields ...
    pub webhook: Option<Arc<dyn WebhookEndpointProvider>>,
}
```

**New trait in `nebula-action::webhook`:**

```rust
pub trait WebhookEndpointProvider: Send + Sync {
    /// Full public URL to register with the external provider.
    /// Example: "https://nebula.example.com/webhooks/{trigger_uuid}/{nonce}".
    fn endpoint_url(&self) -> &url::Url;

    /// Path component only, without scheme/host. Useful for embedding
    /// in signed payloads or logs.
    fn endpoint_path(&self) -> &str;
}
```

**Action usage pattern:**

```rust
async fn on_activate(&self, ctx: &TriggerContext) -> Result<Self::State, ActionError> {
    let url = ctx.webhook
        .as_ref()
        .ok_or_else(|| ActionError::fatal("webhook trigger activated without endpoint provider"))?
        .endpoint_url();
    let hook_id = github_api::create_hook(&self.repo, url.as_str(), &self.secret).await?;
    Ok(GitHubState { hook_id })
}
```

**Transport responsibility (in `nebula-api`):**
1. At trigger activation time, generate `(trigger_uuid, nonce)` —
   `Uuid::new_v4()` + 16 random bytes hex-encoded.
2. Build `EndpointProviderImpl { url: base_url / "webhooks" /
   trigger_uuid / nonce }`.
3. Inject `Arc<EndpointProviderImpl>` into `TriggerContext.webhook`
   before calling `adapter.start(ctx)`.
4. Register `(trigger_uuid, nonce)` → `Arc<WebhookTriggerAdapter>` in
   a routing map.
5. On HTTP POST to `/webhooks/{trigger_uuid}/{nonce}`, look up the
   routing map; construct `WebhookRequest`; dispatch.
6. At trigger deactivation, remove from routing map.

The nonce prevents H4 (orphan external hook pointing at stale UUID):
a re-registration of the same `trigger_uuid` generates a fresh nonce,
so old URLs stop routing.

### 4.3 Component responsibilities

#### `nebula-action::webhook::WebhookAction` trait

**Unchanged shape.** Same `on_activate → handle_request → on_deactivate`.
Same `type State: Clone + Send + Sync`. Same `WebhookResponse` return
type. Preserving the DX that sdk-user rated "production-grade".

#### `nebula-action::webhook::WebhookTriggerAdapter`

**Responsibilities (some new):**
1. Double-start rejection via `RwLock<Option<Arc<State>>>` (unchanged)
2. In-flight counter (unchanged, but W6: replace `yield_now` spin
   with `tokio::sync::Notify` wakeup from `InFlightGuard::drop`)
3. Downcast `TriggerEvent` → `WebhookRequest` (unchanged)
4. **W1 — `handle_request` error path sends `500`** via oneshot
   before propagating Err. No more RecvError leaking to transport.
5. **W5 — `handle_request` wrapped in `tokio::select!`** with
   `ctx.cancellation`. Cancellation mid-request returns
   `ActionError::retryable("cancelled")` instead of blocking `stop()`.
6. **W12 — TriggerHealth**: `record_success(1)` on emit,
   `record_error()` on handle_request failure or signature mismatch
   counted as error (configurable via enum if needed). Parity with
   poll.

#### `nebula-action::webhook::verify_hmac_sha256` (hardened)

Signature unchanged: `(request, secret, header) -> Result<SignatureOutcome, _>`.

**Changes:**
- **W10/H3** — reject multi-valued signature headers:
  `headers.get_all(name).iter().count() != 1` → return `Invalid`.
  Prevents proxy-chain duplication attacks.
- **M3** — timing reorder: always run hex decode, always run MAC,
  AND the results. No data-dependent early return on malformed hex.

#### `nebula-action::webhook::verify_hmac_sha256_with_timestamp` (NEW)

```rust
pub fn verify_hmac_sha256_with_timestamp(
    request: &WebhookRequest,
    secret: &[u8],
    sig_header: &str,
    ts_header: &str,
    tolerance: Duration,
    canonicalize: impl Fn(&str, &[u8]) -> Vec<u8>,
) -> Result<SignatureOutcome, ActionError>;
```

**Semantics:**
- Clock source: `request.received_at()` (the arrival timestamp
  recorded by the transport). **Never** reads wall clock.
- Verifies `ts_header` is within `tolerance` of `received_at()`,
  both past AND future (future > 60s → `Invalid`).
- Builds canonical signed bytes via `canonicalize(ts_str, body)`.
  Stripe: `format!("{ts}.{body_utf8}")`. Slack:
  `format!("v0:{ts}:{body_utf8}")`.
- Strict single-valued headers for both `sig_header` and `ts_header`.
- Rejects empty secret (fail-closed, same as `verify_hmac_sha256`).

**Covers:** Stripe, Slack, Mailgun (any HMAC-over-timestamp-prefix
scheme). Action author supplies the canonicalizer, adapter supplies
the constant time.

#### `nebula-action::webhook::verify_hmac_sha256_base64` (NEW)

```rust
pub fn verify_hmac_sha256_base64(
    request: &WebhookRequest,
    secret: &[u8],
    header: &str,
) -> Result<SignatureOutcome, ActionError>;
```

**Covers:** Shopify, Square — HMAC-SHA256 where the header is
base64-encoded instead of hex. Internal implementation shares
everything with `verify_hmac_sha256` except the decode step.

#### `nebula-action::webhook::body_json_bounded<T>` (NEW)

```rust
impl WebhookRequest {
    pub fn body_json_bounded<T: DeserializeOwned>(
        &self,
        max_depth: usize,
    ) -> Result<T, serde_json::Error>;
}
```

**Semantics:**
- Uses a depth-capped `serde_json::Deserializer` wrapper (via
  `serde_stacker` or manual implementation).
- Default recommendation: `max_depth = 64` (enough for any real
  webhook payload, well short of stack overflow at 2 MiB worker
  stacks).
- Existing `body_json` keeps its shape but gets a `# Security` note
  in the docstring pointing at `body_json_bounded` for hostile
  payloads.

**Covers:** H2 (JSON bomb).

#### `nebula-action::webhook::WebhookRequest`

- `MAX_HEADER_COUNT = 256` (was 128) — M4, accommodates
  Cloudflare+NGINX+service-mesh stacks.
- No other shape changes.

#### `nebula-action::context::TriggerContext`

- **New field** `pub webhook: Option<Arc<dyn WebhookEndpointProvider>>`.
- Default `None` when created via `TriggerContext::new` — transports
  set it via a new `with_webhook_endpoint` builder.
- Non-webhook triggers (poll, future Queue/Stream) leave it `None`.

### 4.4 `nebula-api::webhook` module (NEW)

**Files:**
- `crates/api/src/webhook/mod.rs` — module root
- `crates/api/src/webhook/transport.rs` — axum router + dispatch
- `crates/api/src/webhook/provider.rs` — `EndpointProviderImpl`
- `crates/api/src/webhook/routing.rs` — `DashMap<(Uuid, String), Arc<dyn TriggerHandler>>`
- `crates/api/src/webhook/ratelimit.rs` — salvaged `WebhookRateLimiter`

**`WebhookTransport` contract:**

```rust
pub struct WebhookTransport {
    config: WebhookTransportConfig,
    routing: Arc<RoutingMap>,
    registry: Arc<ActionRegistry>,
    rate_limiter: Option<Arc<WebhookRateLimiter>>,
}

pub struct WebhookTransportConfig {
    pub base_url: url::Url,           // "https://nebula.example.com"
    pub path_prefix: String,          // "/webhooks"
    pub body_limit_bytes: usize,      // default 1 MiB (matches WebhookRequest default)
    pub response_timeout: Duration,   // default 10s (time to wait on oneshot)
    pub rate_limit: Option<u64>,      // requests per minute per path, None = disabled
}
```

**Router:**

```rust
impl WebhookTransport {
    pub fn router(&self) -> axum::Router {
        axum::Router::new()
            .route(
                &format!("{}/:trigger_uuid/:nonce", self.config.path_prefix),
                axum::routing::any(webhook_handler),
            )
            .with_state(self.clone())
    }
}
```

**Dispatch flow (`webhook_handler`):**
1. Extract `(trigger_uuid, nonce)` from path.
2. Rate-limit check (if enabled) — return `429` on exceed.
3. Look up routing map. Miss → `404`.
4. Read body with `axum::body::to_bytes(body, config.body_limit)`.
   Size exceed → `413 Payload Too Large`.
5. Construct `WebhookRequest::try_new(method, path, query, headers, body)`.
   Construction error (header count cap) → `400`.
6. Create `oneshot::channel::<WebhookHttpResponse>()`.
7. Attach sender via `WebhookRequest::with_response_channel`.
8. Wrap in `TriggerEvent::new` + `TriggerEventMeta`.
9. Call `handler.handle_event(event, ctx).await`.
    - On `Err(e)` → log, return generic `500` with no body (don't
      leak error text to external caller).
10. Race `rx.recv_timeout(config.response_timeout)` against shutdown.
    - Timeout → `504 Gateway Timeout`.
    - RecvError (handler dropped sender) → `500` (should not happen
      after W1 fix; flag with tracing warn if it does).
11. Return `WebhookHttpResponse { status, headers, body }` as the HTTP
    response.

**Error-to-status mapping (public contract):**

| Situation | Status |
|---|---|
| Path doesn't match `/webhooks/:uuid/:nonce` | 404 |
| Unknown `(uuid, nonce)` | 404 |
| Body exceeds `body_limit_bytes` | 413 |
| Header count exceeds 256 | 400 |
| Rate limit exceeded | 429 |
| Handler returns `ActionError` (any kind) | 500 with generic body |
| Oneshot timeout | 504 |
| Oneshot RecvError (unexpected after W1 fix) | 500 |

**Activation / deactivation API:**

```rust
impl WebhookTransport {
    /// Called by runtime when a webhook trigger starts.
    pub async fn activate(
        &self,
        adapter: Arc<dyn TriggerHandler>,
    ) -> Result<ActivationHandle, ActionError>;
}

pub struct ActivationHandle {
    trigger_uuid: Uuid,
    nonce: String,
    provider: Arc<EndpointProviderImpl>,
}
```

Runtime calls `transport.activate(adapter)`, receives handle, sets
`ctx.webhook = Some(handle.provider.clone())`, then calls
`adapter.start(ctx)`. On stop, runtime calls
`transport.deactivate(handle)` which removes the routing entry.

### 4.5 Deletion scope

Delete in one commit after transport is working:

- `crates/webhook/` entire directory — 4532 LOC
- `"crates/webhook"` line in workspace `Cargo.toml`
- References in `crates/api/design/*.md`:
  - `API.md` line 38 (table entry), line 151, 178
  - `PLAN.md` lines 13, 35, 126
  - `README.md` line 12
  - `ROADMAP.md` line 8
  - `TASKS.md` line 18
  - `VISION.md` line 135
  
  All to be replaced with references to the new `nebula-api::webhook`
  module, or removed if no longer relevant.
- `deny.toml` if it has any `webhook` crate reference.

No salvage except `WebhookRateLimiter` (copy before delete, not link).

## 5. Security invariants

Every implementation choice in section 4 must preserve:

1. **Signature verification is constant-time** — all comparisons
   via `subtle::ConstantTimeEq` or `hmac::Mac::verify_slice`. No
   early returns on data-dependent branches.
2. **Clock is transport-attached, not wall-read** — replay checks
   read `request.received_at()`, set once by transport at arrival.
3. **Multi-valued signature/timestamp headers are rejected** — defense
   against proxy-chain injection.
4. **Body size and header count capped at construction time** — no
   way to build a `WebhookRequest` that bypasses limits.
5. **JSON parsing is depth-capped** — `body_json_bounded` for hostile
   payloads; `body_json` docstring warns.
6. **Empty secret fails closed** — `Err(Validation)`, never silent
   success.
7. **Error paths never hang the transport** — `handle_request` error
   sends a `500` via oneshot before returning; oneshot timeout in
   transport is a hard guarantee.
8. **Nonce per registration** — stale external hooks can't route to
   new triggers that happen to reuse a UUID.

## 6. DX invariants

Every implementation choice in section 4 must preserve:

1. **Action authors never construct HTTP responses directly.** Use
   `WebhookResponse::accept` / `::respond`.
2. **Action authors never read wall clock.** Use `req.received_at()`
   via the replay-window helper.
3. **Action authors never write custom HMAC loops** for Stripe/Slack
   /Mailgun. Use `verify_hmac_sha256_with_timestamp` with a
   canonicalizer closure.
4. **Action authors never guess webhook URLs.** Use
   `ctx.webhook.endpoint_url()`.
5. **Failed `handle_request` gets a real `ActionError`** propagated
   to runtime; transport sends generic 500 to external caller.
6. **`WebhookAction::State` pattern unchanged** — `&State` in
   `handle_request`, owned in `on_deactivate`. sdk-user explicitly
   rated this as the best part of the current API; don't regress.

## 7. Test coverage requirements

New tests (section numbers refer to the implementation plan):

**Hardening (Session 1):**
- `handle_request_error_sends_500_via_oneshot`
- `verify_hmac_sha256_with_timestamp_stripe_scheme`
- `verify_hmac_sha256_with_timestamp_slack_scheme`
- `verify_hmac_sha256_with_timestamp_rejects_future_timestamp`
- `verify_hmac_sha256_with_timestamp_rejects_old_timestamp`
- `verify_hmac_sha256_rejects_multi_valued_header`
- `verify_hmac_sha256_base64_shopify_scheme`
- `body_json_bounded_rejects_deep_nesting`
- `handle_request_cancelled_mid_flight_returns_cleanly`
- `webhook_adapter_records_health_success`
- `webhook_adapter_records_health_error_on_failure`
- `in_flight_notify_wakes_stop` (replaces yield_now spin)

**Transport (Session 2):**
- `transport_round_trip_accepts_valid_signed_request`
- `transport_routes_by_uuid_and_nonce`
- `transport_returns_404_for_unknown_trigger`
- `transport_returns_413_for_oversized_body`
- `transport_returns_504_on_handler_timeout`
- `transport_rate_limit_returns_429`
- `ctx_webhook_endpoint_url_is_populated_at_activate`
- `endpoint_url_includes_nonce_uuid_format`

**Deletion (Session 3):**
- Workspace still builds.
- All 342+ existing tests still pass.
- `cargo clippy --workspace` clean.

## 8. Acceptance criteria

The webhook subsystem is spec-complete when:

1. A `WebhookAction` plugin can:
   - Call `ctx.webhook.endpoint_url()` inside `on_activate` to get a
     real HTTPS URL to register with GitHub/Stripe/Slack.
   - Implement Stripe signature verification in ≤5 lines using
     `verify_hmac_sha256_with_timestamp`.
   - Return `WebhookResponse::respond` for Slack URL verification and
     see the challenge echoed in the HTTP response.
2. A real HTTP `POST /webhooks/{uuid}/{nonce}` reaches the plugin's
   `handle_request` and the plugin's emission actually creates a
   workflow execution.
3. All 15 audit findings W1–W15 have either: a fix in hardening, a
   line in `webhook-api-v2.md` spec, or an explicit "won't fix, here's
   why" note in this spec.
4. `crates/webhook/` is gone from `git ls-files`.
5. Workspace test count ≥ 360 (342 existing + 18+ new).
6. `.project/context/crates/action.md` records the webhook principles
   alongside the poll principles (constraint-not-policy, orthogonal
   health, persistence-read-before-shipping analogues).

## 9. Open questions for the plan phase

Things the spec intentionally leaves to the implementation plan:

- **Where the routing map lives**: global mutex-map, per-transport
  `DashMap`, or something else. Plan-time detail.
- **How transport discovers `base_url`**: config field, environment
  variable, or both. Plan-time.
- **`with_webhook_endpoint` builder naming** on `TriggerContext`:
  `with_webhook`, `with_webhook_binding`, or
  `with_webhook_endpoint_provider`. Plan-time bikeshed.
- **Whether the routing map holds `Arc<dyn TriggerHandler>` directly
  or goes through `ActionRegistry`**. ArchitectRally said "go through
  registry"; plan must decide the exact shape.
- **`WebhookRateLimiter` salvage mechanism**: copy + keep original
  git history as context, or full rewrite. Plan-time.
- **Test fixture for integration test in Session 2**: hyper client
  against `TestServer`, or a mocked `Arc<ActionRegistry>`. Plan-time.

These belong in `2026-04-13-webhook-implementation-plan.md` (next
step), not here.
