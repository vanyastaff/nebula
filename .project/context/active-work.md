# Active Work
Updated: 2026-04-13

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
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
