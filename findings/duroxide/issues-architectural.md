# Issues — Architectural Analysis — duroxide

Source: `gh issue list --repo microsoft/duroxide --state all --limit 100`

Total issues found: 10 (7 open, 3 closed). Repository is very young (created Nov 2025, v0.1.28 as of Apr 2026). Issue count is well below the 100 threshold for mandatory citation, but all issues are documented below.

## Open Issues (7)

### #10 — duroxide-pg: concurrent worker startup crashes on migration race condition
- **Labels:** bug
- **Architectural relevance:** Multi-node deployment / migration safety. Provider startup is not idempotent under concurrent initialization — a race on PostgreSQL schema migrations causes crashes. Affects production deployments where multiple nodes start simultaneously.
- **URL:** https://github.com/microsoft/duroxide/issues/10

### #9 — Expose event queue depth as orchestration-visible state
- **Labels:** none (enhancement)
- **Architectural relevance:** Observability gap. Orchestrations cannot currently introspect the depth of their own event queue (`dequeue_event` inbox), limiting reactive/adaptive control-flow.
- **URL:** https://github.com/microsoft/duroxide/issues/9

### #8 — Add tryDequeueEvent(queueName) to orchestration context
- **Labels:** none (enhancement)
- **Architectural relevance:** Event model extension. Requests a non-blocking `tryDequeueEvent()` to allow orchestrations to poll the queue without committing to wait. Currently only blocking `dequeue_event()` exists.
- **URL:** https://github.com/microsoft/duroxide/issues/8

### #7 — PostgresProvider.connectWithSchema races on concurrent first-time startup
- **Labels:** none (bug — duplicate of #10 theme)
- **Architectural relevance:** PostgreSQL provider race condition on schema namespace creation (`pg_namespace_nspname_index` duplicate key error).
- **URL:** https://github.com/microsoft/duroxide/issues/7

### #5 — Runtime should explicitly handle orphan orchestrator queue messages
- **Labels:** none
- **Architectural relevance:** Orphan queue handling. Duplicate of theme in #4 — orphan messages (for instances that no longer exist) should be detected and discarded rather than repeatedly processed. Affects robustness.
- **URL:** https://github.com/microsoft/duroxide/issues/5

### #4 — Runtime should explicitly handle orphan orchestrator queue messages
- **Labels:** none
- **Architectural relevance:** Same as #5 — orphan queue messages currently fall through to unregistered-handler backoff rather than clean termination.
- **URL:** https://github.com/microsoft/duroxide/issues/4

### #3 — Stale activity cleanup: TTL or background sweep for undeliverable worker queue items
- **Labels:** enhancement
- **Architectural relevance:** Storage hygiene. Activities scheduled for instances that later get deleted or canceled remain in the worker queue indefinitely. No TTL or sweep mechanism exists.
- **URL:** https://github.com/microsoft/duroxide/issues/3

## Closed Issues (3)

### #13 — "duroxide-windows-x64: Requires rebuild with updated internal references"
- Internal build/packaging issue for Windows npm package.
- **URL:** https://github.com/microsoft/duroxide/issues/13

### #11 — Rename Windows x64 npm package
- Build artifact naming.
- **URL:** https://github.com/microsoft/duroxide/issues/11

### #2 — This repo is missing important files
- Early repo setup issue — resolved.
- **URL:** https://github.com/microsoft/duroxide/issues/2

## Architectural Observations from Issues

1. **Orphan queue message handling** (issues #4 and #5) reveals a gap: the runtime's dispatch loop does not distinguish "instance doesn't exist" from "handler not registered". Both fall into `unregistered_backoff`, meaning orphan messages use exponential backoff slots unnecessarily.

2. **PostgreSQL provider races** (issues #7 and #10) indicate the ecosystem `duroxide-pg` provider has not yet achieved production-grade startup idempotency — a known gap for multi-node deployment.

3. **Stale activity cleanup** (issue #3) reveals that duroxide does not implement any TTL-based or background sweep mechanism for stranded worker queue items. This is a long-lived storage leak risk in production.

Note: With only 10 total issues, the Tier 1 "≥3 cited issues" requirement is satisfied by the 3 substantive open issues (#3, #4/#5, #10/#7).
