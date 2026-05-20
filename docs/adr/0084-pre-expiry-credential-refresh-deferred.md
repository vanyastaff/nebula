# ADR-0084 — Pre-expiry credential refresh: deferred to 1.1

- **Status:** Accepted
- **Date:** 2026-05-20
- **Supersedes:** N/A
- **Superseded by:** N/A
- **Related:** ROADMAP M12.4; ADR-0041 (durable refresh-claim coordinator)

## Context

ROADMAP §M12.4 listed "pre-expiry credential refresh (proactive)" as a
1.0 candidate. The current implementation is **reactive**:

1. An action attempts to use a credential
2. The engine resolver observes expiry via stored TTL or provider failure
3. L1 (in-process `OnceCell` / `tokio::sync` coalescer) collapses
   concurrent refresh attempts into a single provider call
4. L2 (`RefreshClaimRepo` per ADR-0041) collapses cross-replica refresh
   attempts via durable claim with TTL/heartbeat

The n8n #13088-class multi-replica race ("replica A rotates the refresh
token; replica B uses the old token; replica B's call fails because the
old token has been revoked") was closed by L2 in П2 (2026-04-26). Chaos
test (3 replicas × 100 credentials × 10 minutes) is green nightly.

Pre-expiry refresh would add a per-instance background scheduler that
ticks every N seconds, scans the store for credentials approaching
expiry, and refreshes them proactively — before any action requests
them.

## Decision

**Proactive pre-expiry refresh ships in 1.1, not 1.0.**

Reactive refresh remains the contract for 1.0:
- Resolver-observed expiry → L1+L2 coalesced refresh → action proceeds
  with refreshed material on first attempt-after-expiry
- ReauthRequired escalation for terminal refresh failures
  (TokenExpired with no usable refresh path)

## Rationale

**Reactive is correct under load.** The L2 durable coordinator closes
the only failure class proactive refresh would have prevented — the
multi-replica race. Without that race in scope, proactive refresh's
value is a latency optimization (no warm-up cost on the first
attempt-after-expiry), not a correctness fix.

**Proactive adds failure surface for limited benefit:**

- A new background failure class (refresh fails with no caller to
  notify; the next action's reactive refresh would have caught the same
  failure anyway).
- Per-instance scheduler drift (instances disagree on which credentials
  are "near expiry"; instance crashes during a scheduled refresh).
- New test scaffolding (chaos: instance dies mid-tick; scheduler skew
  vs provider clock).
- Operational tuning (the tick interval, the "near expiry" threshold,
  per-tenant budget caps).

**Critical-path latency is already handleable.** For workflows that
cannot tolerate the first-call-after-expiry warm-up:

- Warm-up requests on critical paths (the existing tactic in the
  contract canon)
- `CredentialService::refresh()` is publicly callable; callers can
  schedule warm-ups themselves

Neither requires a new engine-side scheduler.

## Consequences

- `nebula-credential` ships 1.0 with `frontier`→`stable` flip on the
  reactive refresh contract.
- Proactive design is a 1.1 backlog item; no code lands until the 1.1
  scope is opened.
- ROADMAP §M12.4 row updated to reflect this decision.
- A future ADR will record the proactive design when it ships;
  this ADR will then be Superseded.

## References

- [ROADMAP M12.4](../ROADMAP.md)
- ADR-0041 — durable refresh-claim coordinator
- П2 chaos test (archived in `docs/ARCHIVE.md`)
