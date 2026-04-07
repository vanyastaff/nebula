# nebula-webhook
Inbound webhook server — UUID-isolated endpoints per trigger, single server per runtime.

## Invariants
- One `WebhookServer` per runtime. All triggers share the same HTTP port — routing is by UUID path segment.
- `TriggerHandle` is RAII — dropping it automatically deregisters the webhook endpoint.

## Key Decisions
- UUID isolation: each trigger gets `/webhook/{uuid}` for security and routing.
- `Environment` separates test vs production traffic.
- `StateStore` / `MemoryStateStore` for per-trigger state persistence.
- `WebhookAction` trait: `on_subscribe`, `on_webhook`, `on_unsubscribe`, `test`.
- Rate limiting, signature verification (`HmacSha256Verifier`), and webhook metrics removed — will be reimplemented at API/middleware layer when webhook v2 lands.

## Traps
- Don't start multiple `WebhookServer` instances in one process.
- `TriggerCtx::webhook_url()` returns full URL with UUID — use in `on_subscribe`.
- `on_webhook` returns `Option<Event>` — `None` acknowledges without emitting.
- `Error::RateLimited` variant removed — don't match on it.

## Relations
- Depends on nebula-resource (for `Context`). Used by nebula-runtime.

<!-- reviewed: 2026-04-07 — removed rate_limit, verifier, metrics modules; RateLimited error variant removed -->
