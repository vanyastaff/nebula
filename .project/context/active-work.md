# Active Work
Updated: 2026-04-12

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
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

## Next Up
- Credential bugs B1-B9 (B6 CRITICAL)
- CredentialPhase + OwnerId
- Wire CredentialResolver into ActionContext
