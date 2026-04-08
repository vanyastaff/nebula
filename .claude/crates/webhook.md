# nebula-webhook
Inbound webhook server — UUID-isolated endpoints per trigger, single server per runtime.

## Invariants
- One `WebhookServer` per runtime. All triggers share the same HTTP port — routing is by UUID path segment.
- `TriggerHandle` is RAII — dropping it automatically deregisters the webhook endpoint.

## Key Decisions
- UUID isolation: each trigger gets a unique UUID path (`/webhook/{uuid}`) for security and routing. External services register this URL.
- `Environment` separates test vs production traffic — test webhooks never cross into production routing.
- `StateStore` / `MemoryStateStore` for per-trigger state persistence across webhook calls.
- `WebhookAction` trait: implement `on_subscribe` (register with external service), `on_webhook` (handle incoming), `on_unsubscribe` (cleanup), `test` (verify connection).

## Traps
- One `WebhookServer` per process — all triggers share one port.
- `on_webhook` returning `None` acks the request but emits no event (used for filtering).
- **Layer rule**: `webhook` is API layer — cannot import `nebula-runtime` (Exec). `InboundQueue` lives in `webhook::queue`; adapt to `nebula_runtime::TaskQueue` in the embedding app.
- `with_inbound_queue` builds a new `Arc` — call before cloning the server Arc.
- Enqueue failure → HTTP 500 so the sender retries (at-least-once).
- `WebhookDeliverer`: 4xx = permanent failure; 5xx/conn errors retry. `max_retries=0` clamped to 1.
- `metrics` constants are strings only — registry wiring is a TODO for nebula-telemetry.
- `reqwest` is a regular dep (not dev-only) since 2026-04-07.

## Relations
- Depends on nebula-resource. Used by nebula-runtime for trigger management.

<\!-- reviewed: 2026-03-25 -->

<!-- reviewed: 2026-03-30 -->
<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 — dep cleanup only: removed unused Cargo.toml deps via cargo shear --fix, no code changes -->
