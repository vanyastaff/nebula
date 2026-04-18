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
