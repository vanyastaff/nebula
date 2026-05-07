---
id: 0049
title: webhook-handler-convergence
status: accepted
date: 2026-05-07
supersedes: []
superseded_by: []
tags: [api, webhook, transport, m3]
related:
  - crates/api/src/services/webhook/transport.rs
  - crates/api/src/services/webhook/bootstrap.rs
  - crates/api/src/services/webhook/events.rs
  - crates/storage/src/repos/webhook_activation.rs
  - crates/action/src/webhook/factory.rs
  - .ai-factory/ROADMAP.md
  - .ai-factory/plans/webhook-dispatch.md
  - docs/adr/0022-webhook-signature-policy.md
  - docs/adr/0046-metrics-telemetry-boundary.md
  - docs/adr/0048-idempotency-store-backend.md
---

# 0049. Webhook Handler Convergence — One Pipe, Two URL Shapes

## Context

`nebula-api` shipped two parallel webhook ingress pipelines:

1. **`WebhookTransport`** (`services::webhook`) — programmatic, keyed
   on `(uuid, nonce)` runs `WebhookAction::handle_request` against
   the typed `nebula-action` surface. Mature: signature policy,
   replay-window, per-key rate limiter, metrics, tracing spans.
2. **`WebhookDispatcher`** (`crate::webhook`) — slug-routed
   `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}`, configured via
   `triggers.config` JSONB, dispatched via `WebhookTriggerSink`.
   The default sink in production was `NoopSink` — events were
   silently dropped on the floor.

Two surfaces meant duplicated auth (`WebhookAuthConfig` vs
`SignaturePolicy`), duplicated rate limiting
(`middleware::webhook_ratelimit` vs the transport's per-key
limiter), and divergent metrics namespaces. Worse, the
slug pipeline was provably broken: no production composition root
attached a non-noop `WebhookTriggerSink`, so every operator-configured
trigger HTTP request returned 202 then dropped its event.

ROADMAP §M3.3 exit criteria:

- single dispatch pipe — both URL shapes funnel into one routing
  map and one signature/replay/rate-limit pipeline;
- storage bootstrap — `build_app` loads active activation rows and
  registers them in the transport;
- replay window default 5 min, `RFC 9457 problem+json` on rejection;
- per-key rate-limit isolation;
- provider verification (Slack `url_verification`, Stripe
  `pending_webhook` ping, Generic `?challenge=`);
- single `NEBULA_WEBHOOK_*` metric namespace with documented
  cardinality budget;
- ADR (this document);
- `task dev:check` and `task deny` green.

## Decision

Converge on **one pipe**: `WebhookTransport`. Both URL shapes funnel
through a single `dispatch_inner` function that runs:

```
routing-map lookup → 404
rate-limit         → 429 + Retry-After
signature verify   → 401 (HMAC + timestamp/replay-window)
pre_handle         → optional RespondNow short-circuit
handle_request     → engine
```

### Routing key

```rust
pub enum WebhookKey {
    Programmatic { uuid: Uuid, nonce: String },
    Slug(TriggerCoordinates),
}
```

`Programmatic` is minted by `WebhookTransport::activate(...)` (the
runtime's typed-action path). `Slug(TriggerCoordinates)` is loaded
from storage at startup by `bootstrap_webhook_activations` (E1) and
mutated by `TriggerLifecycleEvent` consumers (E2) and the admin
reload endpoint (E3).

### Provider catalog

Slack / Stripe / Generic verification lives in `WebhookAction`
subtypes (`crates/action/src/webhook/providers/`), each implementing
the same trait used by programmatic activations. Operator-configured
rows carry `action_kind: "slack" | "stripe" | "generic"` in
`triggers.config.webhook_activation`; the engine's
`ActionRegistry::register_webhook_provider` keeps a string-keyed
factory map; bootstrap looks up the factory by `action_kind` and
calls `factory.build(spec)` to produce a `BuiltWebhookHandler`.

The string key (vs the typed `ActionKey` used by regular actions) is
intentional: provider kinds come from operator-supplied storage data,
not Rust types. The factory registry is a sibling of the typed
`ActionRegistry`, not an extension.

### Signature policy + replay window

`SignaturePolicy::Required(RequiredPolicy { secret, header,
scheme, timestamp_header, timestamp_format, replay_window })` —
one source of truth shared across both URL shapes. When
`timestamp_header` is `None` the replay check is skipped
(preserves behaviour for legacy actions); otherwise the policy
runs `validate_timestamp` before the HMAC math so cheap rejections
do not pay the constant-time signature cost. Default window 300 s
matches Slack and Stripe; future-skew capped at 60 s independently
of the configured window. Rejection emits `RFC 9457 problem+json`
under `https://nebula.dev/problems/webhook-signature` /
`https://nebula.dev/problems/webhook-replay-window`.

### Per-key rate limit

`WebhookRateLimiter` lives at `services::webhook::ratelimit` (moved
from `middleware/webhook_ratelimit` in F1; the latter was a misnomer
— no axum `Layer` ever existed). Sliding window per `WebhookKey`,
LRU-capped by tracked-path count to prevent attacker churn from
permanently exhausting the limiter (#271). Flooding one slug does
not affect another; the programmatic surface buckets independently
from slug surfaces.

## Consequences

### Positive

- One mental model. Every webhook hits the same five-step pipeline,
  observed by the same metrics, traced by the same span fields.
- Storage-loaded slug activations get rate-limit + signature +
  metrics for free, not "added later in a separate refactor".
- Future providers ship as new `WebhookAction` subtypes (a single
  module, ~150 lines) instead of new modules in the API layer.
- The dispatch path no longer drops events: `NoopSink` is gone.
- ADR-0046 cardinality budget honored: `tenant_id` is bounded per
  deployment, `webhook_key_kind` is a 2-element enum, no
  `User-Agent`-derived label leakage.

### Breaking

- Removed: `WebhookDispatcher`, `WebhookAuthConfig`,
  `WebhookTriggerSink`, `MpscSink`, `NoopSink` (relocated where
  test-only), `crate::middleware::webhook_ratelimit`,
  `crate::webhook::*`, `crate::handlers::webhook`,
  `crate::routes::webhook`. Any external consumer of these types
  gets a hard compile break and must migrate to
  `crate::services::webhook` types.
- `triggers.config` JSONB is now kind-namespaced
  (`webhook_activation`, `cron`, `event` keys); existing webhook
  rows that put fields at the top level must be migrated. Migration
  0025 attaches `COMMENT ON COLUMN triggers.config` documenting the
  contract.

### Out of scope (1.0 follow-ups)

- **Producer-side `TriggerLifecycleEvent` wiring.** E2 ships the
  consumer (transport-side subscriber). The producer side — storage
  CRUD callsites that `emit()` `Created`/`Updated`/`Deleted` events
  — is deferred. Without producers the bus is exercised only by the
  admin reload endpoint and tests.
- **Distributed rate-limit / replay-cache stores** for multi-instance
  API deployments. Same deferral as ADR-0048 §"Hybrid backend pattern":
  per-instance is correct semantics; horizontal scaling needs a
  shared store for tight enforcement.
- **Dynamic webhook path parameters** (e.g. `/{slug}/{user_id}`).
  n8n implements this via `WebhookEntity.webhookId + pathLength` +
  `:paramName` segments and resolves params at lookup time; our
  `WebhookKey::Slug(TriggerCoordinates)` is fixed-arity. Deferred to
  M3.6 (validation milestone) — same place we'd add per-trigger
  JSON-schema validation.
- **Test-mode `WebhookTransport`** for the "execute workflow once"
  UX. n8n ships `TestWebhooks` as a second `IWebhookManager` impl
  (in-memory only, no DB persist). When the UI needs it, a second
  impl behind the same trait will land — not a parallel pipeline.
- **Full per-branch outcome instrumentation
  (`NEBULA_WEBHOOK_REQUESTS_TOTAL` + `NEBULA_WEBHOOK_LATENCY_SECONDS`)**.
  G1 ships the metric names + Prometheus HELP catalog; G2 wires the
  replay/rate-limit counters and bootstrap-failure counter. The full
  per-outcome request counter and latency histogram (one increment
  on every terminal branch in `dispatch_inner`) lands as a
  follow-up alongside the cardinality regression test.

## Prior art (n8n)

The convergence direction matches n8n's webhook architecture,
validated by reading
`packages/cli/src/webhooks/{live-webhooks,webhook.service,webhook-request-handler}.ts`
and `packages/@n8n/db/src/entities/webhook-entity.ts`:

- **Single `WebhookService.findWebhook(method, path)` (DB + cache)**
  serves both operator-configured and programmatic activations.
- **Provider verification lives inside the trigger node**
  (`SlackTrigger.node.ts`, `GithubTriggerHelpers.ts`), not in the
  router. Mirrors our `WebhookAction::pre_handle` hook.
- **Test-mode is a second impl of the same `IWebhookManager`**
  interface (`test-webhooks.ts`), not a parallel architecture.

Differences worth noting:

- n8n's webhook DB row is composite-keyed `(webhookPath, method)`
  and supports dynamic path params; we keep `WebhookKey::Slug` fixed
  for M3.3 and defer dynamic params to M3.6.
- n8n hardcodes signature math per node, while our `RequiredPolicy`
  carries replay-window config in the policy itself. The latter is
  cleaner: deduplication metadata lives next to the secret.

## Cross-links

- ADR-0022 — webhook signature policy origin and `Required` /
  `OptionalAcceptUnsigned` / `Custom` semantics.
- ADR-0046 — metrics / telemetry boundary; this ADR honors the
  cardinality budget rules established there.
- ADR-0048 — idempotency hybrid backend pattern. Same deferral
  shape for distributed coordination (multi-instance dedup).
- ROADMAP §M3.3 — milestone closure tracker.
