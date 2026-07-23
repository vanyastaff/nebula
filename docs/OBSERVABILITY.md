---
name: Nebula observability contract
description: SLI / SLO / error budget, structured event schema for execution_journal, core analysis loop for operators.
status: accepted
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, MATURITY.md]
---

# Nebula observability contract

## 1. Service level indicators (SLIs)

Nebula's SLIs describe observable, measurable engine behavior that matters to operators. Each SLI has a measurement method (where the number comes from), a rolling window (typically 28 days), and a canonical name used in dashboards and alerts.

| SLI | Measurement | Window |
|---|---|---|
| `execution_terminal_rate` | `SELECT count(*) FILTER (WHERE status IN ('succeeded','failed','cancelled')) / count(*) FROM executions WHERE started_at >= now() - interval '28 days'` | 28d |
| `cancel_honor_latency_p95` | Histogram of `cancelled_at - cancel_requested_at` over the same window | 28d, p95 |
| `checkpoint_write_success_rate` | Ratio of successful checkpoint writes to attempted checkpoint writes (emitted from `nebula-execution` metrics) | 28d |
| `dispatch_lag_p95` | Histogram of `control_queue_drained_at - control_queue_inserted_at` | 28d, p95 |

## 2. Service level objectives (SLOs)

SLOs are operator commitments. Numbers below are targets; actuals live in the maturity dashboard per crate (`docs/MATURITY.md` `SLI ready` column) and in the runtime dashboard outside this repo.

| SLI | SLO target | Rationale |
|---|---|---|
| `execution_terminal_rate` | ≥ 99.0% | 1% budget absorbs legitimate long-running / externally-blocked runs and genuine engine failures. |
| `cancel_honor_latency_p95` | ≤ 5 seconds under default dispatch interval | Operators expect "cancel" to mean "stop within a few seconds"; slower violates §10 knife step 5. |
| `checkpoint_write_success_rate` | ≥ 99.9% | Checkpoint loss degrades recovery fidelity; §11.5 best-effort framing assumes rare failure. |
| `dispatch_lag_p95` | ≤ 1 second | Control-plane signals (cancel, trigger) must feel immediate to operators. |

## 3. Error budgets

Error budget = `1 - SLO`. Budgeting policy:

- Budget burn > 10% in a rolling 7-day window triggers an investigation (not paging).
- Budget burn > 50% in 24 hours pages the on-call.
- Budget reset is rolling, not calendar — no "fresh budget on the 1st" effect.

## 4. Structured event schema (execution_journal)

Every durable event appended to `execution_journal` follows this shape:

```jsonc
{
  "execution_id": "exec_...",
  "node_id": "node_...",
  "attempt": 1,
  "correlation_id": "trace_...",
  "trace_id": "...",       // OpenTelemetry
  "span_id": "...",        // OpenTelemetry
  "event_type": "started" | "checkpoint" | "retry" | "cancel_requested" | "cancelled" | "failed" | "succeeded" | ...,
  "payload": { ... },      // event-type specific
  "timestamp": "2026-04-17T..."
}
```

High-cardinality fields (`execution_id`, `node_id`, `correlation_id`, `trace_id`) are required; enums (`event_type`) are documented with a closed set in `crates/execution/src/journal.rs`.

Principle (from Observability Engineering): append rich structured events first, aggregate to metrics second. Never add a metric without the underlying event being available for drill-down.

## 5. Core analysis loop

Operator procedure for any failed or stuck run:

1. **What failed?** Query `execution_journal` by `execution_id` for the last event before the failure. `event_type` + `payload.error` pins the failing step.
2. **When?** Compare `event_type='started'` timestamp to the failure event timestamp; cross-reference with `trace_id` in the observability stack.
3. **What changed?** Check recent deploys, config changes, dependency upgrades — `MATURITY.md` `frontier` crates are likely culprits if the run touched them.
4. **What to try?** For transient classifications (per `nebula-error::Classify`): wait and retry. For permanent: open an issue with the journal excerpt. For "unknown": ask in #observability with the trace_id; do not retry blindly.

This loop is the operational half of PRODUCT_CANON §2 success sentence: *you can explain what happened in a run without reading Rust source.*

## 6. Credential-level observability

`CredentialMetrics` (`nebula-credential::metrics`) defines well-known counter names for credential lifecycle operations. All counters use the `nebula.credential.` prefix and are emitted through a `MetricsEmitter` injected via context (not a global/static registry).

| Metric name | Meaning |
|---|---|
| `nebula.credential.resolve_total` | Total credential resolutions attempted |
| `nebula.credential.refresh_total` | Total credential refreshes attempted |
| `nebula.credential.refresh_failed_total` | Total credential refresh failures |
| `nebula.credential.test_total` | Total credential connectivity tests |
| `nebula.credential.rotations_total` | Total credential rotations completed |
| `nebula.credential.dynamic_lease_issued_total` | Total dynamic credential leases issued |
| `nebula.credential.dynamic_lease_released_total` | Total dynamic credential leases expired or released |
| `nebula.credential.tamper_detection_total` | Total tamper detection events |
| `nebula.credential.refresh.err_uri_rejected_total` | Total IdP `error_uri` values rejected by `sanitize_error_uri` (SEC-02 hardening 2026-04-27) — labels: `reason ∈ {scheme, controlchars, parse_failed}` |
| `nebula.credential.refresh.body_truncated_total` | Total non-2xx token responses rejected by the bounded reader (SEC-01 hardening 2026-04-27) — labels: `reason ∈ {content_length_too_large, body_too_large, read_chunk}` |

Standard labels: `credential_key` (e.g. `"github_token"`), `outcome` (`"success"` / `"failure"`), `dynamic` (`"true"` / `"false"`), `reason` (refresh failure reason).

> **SEC-01/02 metric emission status (2026-04-27).** Per credential security hardening (archived sub-spec; see the maintainers' private design vault) §6, the metric *names* are reserved here as part of the doc-sync stage (`docs/PRODUCT_CANON.md` §3.5 and §4.5 operational honesty: a new error path must register its observability surface alongside the code that emits it). Emission wiring is deferred to the metric-bus integration cascade — the security-hardening fix surfaces the rejection paths via typed `TokenHttpError` (bounded reader) and the `[*_redacted]` placeholder (sanitizer). When the credential-metrics emitter is wired through `parse_token_response`, both counters get bumped at the existing `Err(...)` returns; no new error semantics are introduced in this stage.

**Analysis loop integration:** when investigating credential-related failures, include credential metrics alongside `execution_journal` events. A spike in `refresh_failed_total` or `tamper_detection_total` is an early signal before execution failures surface.

## 7. Credential refresh coordinator (two-tier L1+L2)

The two-tier refresh coordinator (design archived; see the maintainers' private design vault — refresh coordination sub-spec) coordinates rotation across replicas via an in-process L1 coalescer plus a durable L2 claim repo. Three observability surfaces let operators audit cross-replica behavior without reading Rust source:

### 7.1 Metrics

All five series live in `nebula_metrics::naming::NEBULA_CREDENTIAL_REFRESH_COORD_*` and are emitted through pre-bound `RefreshCoordMetrics` handles attached to the `RefreshCoordinator` at composition time. Cardinality is closed by construction — every label set is enumerated in the `refresh_coord_*` submodules.

| Metric | Type | Labels | Cardinality | Meaning |
|---|---|---|---|---|
| `nebula_credential_refresh_coord_claims_total` | counter | `outcome={acquired,contended,outcome_unknown,exhausted}` | 4 series | L2 claim acquisition outcomes. `exhausted` means ordinary contention consumed the retry budget; `outcome_unknown` means an expired `RefreshInFlight` row denied provider replay and remains durable poison until explicit reconciliation. |
| `nebula_credential_refresh_coord_coalesced_total` | counter | `tier={l1,l2}` | 2 series | Refresh calls coalesced rather than running an IdP POST. `l1` = same-process oneshot waiter; `l2` = post-backoff state recheck found another replica refreshed first (`CoalescedByOtherReplica`). |
| `nebula_credential_refresh_coord_sentinel_events_total` | counter | `action={recorded,reauth_triggered}` | 2 series | Sentinel accounting by the reclaim sweep. `recorded` ticks once per distinct poisoned claim UUID; repeated requests/sweeps do not spend the threshold budget. `reauth_triggered` ticks when the database-clock rolling-window count crosses `sentinel_threshold` and publishes a lossy `CredentialEvent::ReauthRequired` observation. It does not prove a durable aggregate transition. |
| `nebula_credential_refresh_coord_reclaim_sweeps_total` | counter | `outcome={reclaimed,outcome_unknown_accounted,no_work}` | 3 series | Reclaim-sweep outcomes. `reclaimed` means only pre-provider normal rows were deleted. `outcome_unknown_accounted` takes precedence for a mixed sweep: evidence was recorded while an expired `RefreshInFlight` row remained fail-closed poison. |
| `nebula_credential_refresh_coord_hold_duration_seconds` | histogram | (none) | 1 series | Time from L2 acquisition until coordinator finalization/drop. The cancel-safe critical section may outlive `refresh_timeout`, and heartbeat renews `claim_ttl`; a rising tail is a slow/stuck-integration signal, not proof of lease expiry. |

### 7.2 Tracing spans

Three spans wrap the coordinator and reclaim sweep paths.

| Span | Site | Attributes |
|---|---|---|
| `credential.refresh.coordinate` | `RefreshCoordinator::refresh_coalesced` | `credential_id`, `replica_id`, `tier` (closed set `{l1, l1_no_progress, l1_outcome_unknown, l2_acquired, l2_coalesced, l2_outcome_unknown}` — see note below) |
| `credential.refresh.claim.acquire` | per L2 `try_claim` attempt inside the backoff loop | `credential_id`, `replica_id`, `attempt` (0-indexed) |
| `credential.refresh.sentinel.detected` | per stuck `RefreshInFlight` row in `run_one_sweep` | `credential_id`, `crashed_holder`, `generation` |

**`tier` value semantics (closed set of six).** The span field is recorded at the actual outcome site, never speculatively, so an operator reading the span knows precisely which path the call took:

- `l1` — same-process L1 oneshot resolved; the caller was a waiter and was woken without running the closure. Pairs with `coalesced_total{tier="l1"}`.
- `l1_no_progress` — the L1 winner reached an exact, replay-safe outcome without advancing authoritative state. The waiter surfaces `PriorAttemptNoProgress`; it does not automatically replay the operation and does not increment the coalesced metric.
- `l1_outcome_unknown` — same-process waiter reached `refresh_timeout` or its sender closed abnormally before exact completion. It surfaces `CriticalOutcomePending`, never increments the coalesced metric, and emits `credential.refresh.l1.wait.outcome_unknown`.
- `l2_acquired` — won L1, acquired the L2 claim, and ran the user's refresh closure. Pairs with `claims_total{outcome="acquired"}`.
- `l2_coalesced` — won L1, contended L2, slept on backoff, then the post-backoff state recheck found another replica had already refreshed. Surfaces as `RefreshError::CoalescedByOtherReplica`. Pairs with `coalesced_total{tier="l2"}`.
- `l2_outcome_unknown` — won L1 but found an expired `RefreshInFlight` row. Provider dispatch was denied, the call surfaced `CriticalOutcomePending`, and the row remains durable poison. Pairs with `claims_total{outcome="outcome_unknown"}` and the structured `credential.refresh.claim.outcome_unknown` event.

The distinction between `l2_acquired` and `l2_coalesced` is load-bearing: a span tagged `l2_acquired` in production logs is a true claim acquisition, whereas `l2_coalesced` is a near-miss that fired the n8n #13088 protection path. Treating them as a single `l2` value (as an earlier draft did) would conflate "acquired the row" with "another replica beat us to it" — operationally distinct outcomes.

The metric labels `coalesced_total{tier=l1|l2}` keep the original two-value vocabulary because the metric only counts exact coalesce events. `l1_no_progress` and `l1_outcome_unknown` are deliberately excluded. The L2 non-coalesce span values map to `claims_total`: `l2_acquired` to `outcome="acquired"` and `l2_outcome_unknown` to `outcome="outcome_unknown"`.

### 7.3 Audit events

The coordinator emits three events through the same `AuditSink` used by `AuditLayer` for
owner-bound `CredentialPersistence` operations. Audit value types live in `nebula-credential` and
the storage decorator lives in `nebula-storage`; the operation enum is `#[non_exhaustive]`.

| Variant | Fields | When |
|---|---|---|
| `RefreshCoordClaimAcquired` | `credential_id`, `holder`, `ttl_secs` | once per L2 claim acquired |
| `RefreshCoordSentinelTriggered` | `credential_id`, `recent_count` | once per newly-accounted poisoned claim UUID |
| `RefreshCoordReauthThresholdReached` | `credential_id`, `reason` (`"sentinel_repeated"` for sub-spec §3.4 escalations) | once per newly-accounted incident whose rolling-window count is at or above the threshold; observational, not proof of a durable credential mutation |

Sink failures are logged at `warn!` but do not propagate to callers. The
store-side `AuditLayer` emits mutation observations only after an acknowledged
commit and is likewise best-effort: it never turns an audit-sink outage into a
false mutation failure or retries an ambiguous write. The decorator is not a
transaction boundary; durable mutation-plus-audit delivery belongs to the K3
transactional outbox.

### 7.4 Sample PromQL queries

```promql
# L2 claim contention rate (per minute) by replica.
rate(nebula_credential_refresh_coord_claims_total{outcome="contended"}[1m])

# L1 coalesce hit rate — fraction of refresh calls saved by the
# in-process L1 oneshot. High values are the *normal* case for hot
# credentials inside one replica (many concurrent callers, one IdP POST).
sum(rate(nebula_credential_refresh_coord_coalesced_total{tier="l1"}[5m]))
  /
(sum(rate(nebula_credential_refresh_coord_coalesced_total{tier="l1"}[5m]))
  + sum(rate(nebula_credential_refresh_coord_claims_total{outcome="acquired"}[5m])))

# L2 coalesce hit rate — fraction of refresh calls saved by the
# cross-replica post-backoff recheck. This is the n8n #13088 "near miss"
# signal: low/zero is normal; SUSTAINED elevation means two or more
# replicas are racing on the same credential and the recheck is the only
# thing preventing a double-POST. Alert when this trends up.
sum(rate(nebula_credential_refresh_coord_coalesced_total{tier="l2"}[5m]))
  /
(sum(rate(nebula_credential_refresh_coord_coalesced_total{tier="l2"}[5m]))
  + sum(rate(nebula_credential_refresh_coord_claims_total{outcome="acquired"}[5m])))

# Crashed-runner storm signal — pair these to triage.
sum(rate(nebula_credential_refresh_coord_reclaim_sweeps_total{outcome="reclaimed"}[5m]))
sum(increase(nebula_credential_refresh_coord_reclaim_sweeps_total{outcome="outcome_unknown_accounted"}[5m])) > 0
sum(rate(nebula_credential_refresh_coord_sentinel_events_total{action="recorded"}[5m]))

# Reauth-required observations crossing zero in the last hour — alert level.
increase(nebula_credential_refresh_coord_sentinel_events_total{action="reauth_triggered"}[1h]) > 0

# Hold duration P99 — compare with refresh_timeout to detect detached slow work.
# Values above the original claim_ttl are allowed while heartbeat renews the lease.
histogram_quantile(0.99, sum(rate(nebula_credential_refresh_coord_hold_duration_seconds_bucket[5m])) by (le))
```

**Analysis loop integration:** an `outcome="exhausted"` crossing zero or a `reauth_triggered` increment is a paging-class event. Cross-reference the `credential.refresh.coordinate` span's `trace_id` with the `execution_journal` to find which actions were waiting on the failed refresh. The sentinel event bus is deliberately non-authoritative; until the K3 owner-qualified command lands, operators must not interpret `reauth_triggered` as evidence that `reauth_required = true` was persisted.
