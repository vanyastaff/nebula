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
- Don't start multiple `WebhookServer` instances in one process — only one port is expected.
- `TriggerCtx::webhook_url()` returns the full URL including the UUID. Use this when registering with the external provider in `on_subscribe`.
- `on_webhook` returns `Option<Event>` — returning `None` acknowledges the webhook but emits no event (useful for filtering).

## Relations
- Depends on nebula-resource (for `Context`). Used by nebula-runtime for webhook trigger management.

<\!-- reviewed: 2026-03-25 -->

<!-- reviewed: 2026-03-30 -->
<!-- reviewed: 2026-04-02 -->
