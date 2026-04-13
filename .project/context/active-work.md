# Active Work
Updated: 2026-04-13

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **Webhook subsystem Sessions 1+2** (2026-04-13, two commits): (S1) nebula-action::webhook hardening bundle H1-H10 — oneshot-500 on handle_request Err, cancel-safe handle_request (503 on cancel), `verify_hmac_sha256_with_timestamp` helper for Stripe/Slack replay windows with injectable canonicalizer and `received_at()` clock source, `verify_hmac_sha256_base64` for Shopify/Square, `body_json_bounded(max_depth)` pre-scanner for JSON bomb defence, multi-valued header rejection, timing-invariant hex decode, TriggerHealth wiring, MAX_HEADER_COUNT 128→256, `Notify` replacing `yield_now` spin in stop(). 22 new tests (370 green). (S2) nebula-api::webhook HTTP transport — `WebhookTransport` with axum Router mounted at `POST /webhooks/{trigger_uuid}/{nonce}`, `WebhookEndpointProvider` capability on `TriggerContext` (declared in nebula-action, implemented in nebula-api), `RoutingMap` DashMap, salvaged `WebhookRateLimiter` wrapping `nebula_resilience::SlidingWindow`, `EndpointProviderImpl` building per-activation URLs with 128-bit nonce (defence against stale external hook routing). 20 new tests (9 integration round-trip + 11 unit). 3246 workspace tests green. Session 3 pending: delete orphan `crates/webhook/` (4532 LOC). See `docs/plans/2026-04-13-webhook-subsystem-spec.md`, `webhook-implementation-plan.md`.
- **Poll trigger hardening** (2026-04-13, two plans): critical correctness fixes (B1-B6: RetryBatch backoff + errored flag, DedupCursor deserialize clamp, override_next clamping, total-loss observability, Partial empty-events handling, batch-size debug_assert) AND quick-hardening bundle (H1-H7: loop flip poll→sleep, stop() cancels token, jitter seed per-trigger-identity, #[non_exhaustive] on PollResult, PollConfig validate_and_clamp, persistence warning in trait doc, PollConfig=constraint-not-policy principle). 348 tests green. See `docs/plans/2026-04-13-poll-critical-fixes.md` and `2026-04-13-poll-quick-hardening.md`. Deferred breaking changes tracked in `2026-04-13-poll-api-v2.md`.
- **Trigger subsystem refactor** (commits 1-5 of 5, 04-09–12):
  - Commit 1 ✅ `TriggerEvent` envelope + `WebhookRequest`. `IncomingEvent` deleted.
  - Commit 2 ✅ `WebhookResponse` enum (Accept/Respond) + `WebhookHttpResponse` + oneshot response channel.
  - Commit 3 ✅ `PollCycle` + `EmitFailurePolicy`. `poll(&cursor) -> PollCycle`, cursor checkpoint on success.
  - Commit 4 ✅ `poll_timeout()` + `tokio::time::timeout` wrapper.
  - Commit 5 ✅ Dropped `Default+Serialize+DeserializeOwned` from `WebhookAction::State`. M1 fix (in-flight counter). `PollTriggerAdapter::stop` documented.
- **nebula-action audit + refactor** (04-11–12): handler re-export purge, derives, DeferredRetryConfig validation, POLL_INTERVAL_FLOOR on ctx.logger, workspace clippy cleanup, nightly rustfmt pass, .claude→.project split, context file trimming.

## Blocked
- **engine**: needs credential DI + Postgres storage
- **auth**: RFC phase
- **poll cursor persistence (F8 / V5)**: blocked on runtime storage — only `MemoryStorage` exists today. Tracked in `docs/plans/2026-04-13-poll-api-v2.md`. High-value (payment/audit) integrations must not ship against current `PollAction` without downstream idempotency.

## Next Up
- Credential bugs B1-B9 (B6 CRITICAL)
- CredentialPhase + OwnerId
- Wire CredentialResolver into ActionContext
