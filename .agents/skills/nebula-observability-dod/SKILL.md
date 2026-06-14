---
name: nebula-observability-dod
description: Use when adding a new state, error path, or hot path, emitting metrics or tracing spans, or before claiming a feature done — observability is Definition of Done.
---

# Nebula observability is Definition of Done

In Nebula, observability is not polish you add later — it is a binding completion
gate. Root `AGENTS.md` ("Agent Rules") states it directly: *"New state / error /
hot path must ship with a typed error variant + tracing span + invariant check."*
Canon `docs/PRODUCT_CANON.md` §4.6 makes structured observability a product
invariant, and the operational test (`docs/OBSERVABILITY.md` §5) is: *you can
explain what happened in a run without reading Rust source.* If your change is not
observable to that bar, it is not done.

This skill is a checklist, not an essay. Follow it.

## When to use

- Implementing a new execution state, error variant, or hot path.
- Adding or naming a metric, or adding a tracing span / event.
- Propagating trace context across a boundary (HTTP → queue → engine → action).
- Doing the final DoD pass before calling work complete or opening a PR.

## 1. The DoD triple — none is optional

For any new **state**, **error path**, or **hot path**, all three must ship in the
same change:

1. **Typed error variant** — a `thiserror`/`NebulaError` variant, never a bare
   string or `unwrap()/expect()/panic!()` in library code (root `AGENTS.md`;
   `edit-guard.sh` enforces it). The variant carries enough structure to classify
   (see `nebula-error::Classify`, used by the analysis loop in
   `docs/OBSERVABILITY.md` §5).
2. **Tracing span / event** — a `tracing` span or event at the new path, with
   structured fields (see §3). The subscriber is set up once by `nebula-log`; you
   just emit at the call site.
3. **Invariant check** — assert the state-machine / value invariant the new path
   assumes, and emit on violation. For execution-state work, transition legality
   lives in `crates/execution/src/transition.rs`; record durable transitions to the
   journal (§4) rather than mutating silently.

Do not split these across PRs and do not defer one as "follow-up" — that is the
exact "observability as completion" failure the rule exists to prevent.

## 2. Metrics — `nebula_*`, bounded cardinality, defined in `nebula-metrics`

- **Define names in `nebula-metrics`, not at the call site.** Name constants live
  in `crates/metrics/src/naming.rs` (the `naming` module — e.g.
  `NEBULA_EXECUTIONS_STARTED_TOTAL`, `NEBULA_ACTION_DURATION_SECONDS`). Adding a
  `NEBULA_*` const to a primitive file (`counter.rs`/`gauge.rs`/`histogram.rs`/
  `registry.rs`/`labels.rs`) is a layering violation — it belongs in the **policy**
  section (`naming.rs` / `filter.rs`). See `crates/metrics/AGENTS.md`.
- **`nebula_*` prefix** on every metric series (and the canon `nebula.<area>.`
  dotted form for credential metrics, per `docs/OBSERVABILITY.md` §6).
- **Bounded label cardinality.** Enumerate every label's value set; never put a
  high-cardinality identifier (`execution_id`, `node_id`, `correlation_id`,
  `trace_id`) on a metric label. `LabelAllowlist` (`crates/metrics/src/filter.rs`)
  strips high-cardinality keys before they reach the registry — do not work around
  it. Good labels are closed sets like `outcome={success,failure}` or
  `tier={l1,l2}` (see the refresh-coordinator metrics in `docs/OBSERVABILITY.md`
  §7.1 for the cardinality-by-construction pattern).
- **Single observability crate (ADR-0046).** `nebula-telemetry` was absorbed into
  `nebula-metrics`; primitives + naming policy + Prometheus **and** OTLP export are
  unified there. The OTel SDK appears only in `crates/metrics/src/otlp.rs` — never
  import `opentelemetry*` from primitives/export modules. (The README still labels
  OTLP "planned"; that is stale — OTLP push export is implemented per
  `crates/metrics/AGENTS.md`.)
- The `/metrics` HTTP endpoint serving `snapshot()` lives in `nebula-api`, not in
  `nebula-metrics`.

## 3. Tracing — one pipeline, propagated context, structured fields

- **One subscriber-init pipeline.** All binaries (and integration tests) initialize
  `tracing` through `nebula-log` (`auto_init` / `init_with`). Do not stand up a
  second subscriber. `nebula-log` does *not* redact secrets — pass redacted forms
  to `tracing::*!` for any credential/token field (canon §12.5; `crates/log/`).
- **Structured fields, not interpolated strings.** Emit
  `tracing::info!(execution_id = %id, node_id = %node, ...)`, not
  `info!("started {id}/{node}")`. Operators query fields; they cannot query prose.
- **Propagate W3C trace context across boundaries (ADR-0050).** The carrier is
  `nebula_core::obs::W3cTraceContext` (no HTTP types in core). The path is:
  `nebula-api` middleware extracts inbound `traceparent` →
  `ControlQueueEntry::w3c_trace_context` persists it across the async handoff →
  the engine `ControlConsumer` re-attaches via `attach_control_queue_w3c_parent`
  before dispatch. `nebula_api::init_api_telemetry` must install **both** the
  global `TraceContextPropagator` **and** the `tracing_opentelemetry` layer — the
  propagator alone is a silent no-op. Outbound resource HTTP is named under
  `nebula.action.resource_http.request` with host/scheme only (no path, no secrets).
- Shipping spans to an external OTLP collector is gated on M9.2 and only activates
  when an endpoint is configured (`nebula-log` `telemetry` feature); the in-process
  propagation above works without it.

## 4. Durable events — append to `execution_journal` with the fixed schema

Significant, replayable events are appended to the journal, not just logged. The
Rust WAL type is `JournalEntry` (`crates/execution/src/journal.rs`) — an
`event`-tagged, **closed** enum (`ExecutionStarted`, `NodeScheduled`, `NodeStarted`,
`NodeCompleted`, `NodeFailed`, `NodeSkipped`, `ExecutionCompleted`,
`ExecutionFailed`, `CancellationRequested`). The durable `execution_journal` row
shape is fixed (`docs/OBSERVABILITY.md` §4):

```
execution_id, node_id, attempt, correlation_id, trace_id, span_id,
event_type (closed enum), payload (event-type specific), timestamp
```

Principle (canon / Observability Engineering): **append rich structured events
first, aggregate to metrics second.** Never add a metric without the underlying
journal event being available for drill-down. A new event kind extends the closed
enum in `journal.rs` — it is not a free-form string.

## 5. Which surface does each field belong to?

High-cardinality identifiers go on **spans and journal events**, never on **metric
labels**:

| Field | Span attr | Journal event | Metric label |
|---|---|---|---|
| `execution_id`, `node_id`, `correlation_id`, `trace_id`, `span_id` | yes | yes (required, §4) | **no** |
| `outcome`, `tier`, `event_type`, closed-set reasons | yes | yes (as `event_type`/payload) | **yes** (bounded) |

Before adding a field, ask which surface it belongs to and put it only there. The
`LabelAllowlist` guard exists because this rule was violated before.

## 6. SLI/SLO awareness — make new hot paths feedable

The engine has four canonical SLIs (`docs/OBSERVABILITY.md` §1–2). A new hot path
should emit enough to keep these measurable:

| SLI | Measured from | SLO target |
|---|---|---|
| `execution_terminal_rate` | `executions` table status counts (28d) | ≥ 99.0% |
| `cancel_honor_latency_p95` | `cancelled_at − cancel_requested_at` histogram | ≤ 5s |
| `checkpoint_write_success_rate` | `nebula-execution` checkpoint-write metrics | ≥ 99.9% |
| `dispatch_lag_p95` | `control_queue_drained_at − control_queue_inserted_at` | ≤ 1s |

If your change touches execution termination, cancel handling, checkpoint writes,
or control-queue dispatch, confirm the timestamps/counters those SLIs read from are
still emitted. Error budget = `1 − SLO`; budgets are rolling, not calendar
(`docs/OBSERVABILITY.md` §3).

## 7. Comments — behavior-first, no pinned section numbers

Document *what the code does and why*, in behavior terms. Do **not** pin ADR or
canon section numbers inside function bodies — references rot, and per project
memory comments must read fine after a plan/spec is deleted (no plan-id /
"Phase X" / `TODO(A-5)` either; `edit-guard.sh` enforces the TODO/FIXME side).
Keep ADR/spec citations in the crate README or this skill, not in hot-path code.

## Final DoD pass — checklist before claiming done

- [ ] Typed error variant added (no `unwrap()/expect()/panic!()` in lib code).
- [ ] Tracing span/event at the new path with structured fields (no string interp).
- [ ] Invariant asserted; durable transition appended to `execution_journal` if it
      changes execution state.
- [ ] Any new metric: `nebula_*` named, constant in `crates/metrics/src/naming.rs`,
      labels are bounded closed sets, no high-cardinality id as a label.
- [ ] Trace context propagated if the change crosses HTTP/queue/engine boundaries.
- [ ] No new subscriber init; secrets passed only in redacted form.
- [ ] SLI feeds intact if the path touches terminal/cancel/checkpoint/dispatch.
- [ ] Comments are behavior-first; no ADR/canon section numbers or plan-ids in code.
- [ ] `cargo nextest run -p <crate>` and `task clippy` green (Stop-gate / lefthook).

## References

- `docs/OBSERVABILITY.md` — SLIs/SLOs, error budgets, journal event schema, cardinality rules, analysis loop.
- `AGENTS.md` (root, "Agent Rules") — observability is Definition of Done.
- ADR-0046 — single `nebula-metrics` crate (telemetry merged in).
- ADR-0050 — W3C trace-context across HTTP → queue → engine.
- `crates/metrics/README.md` + `crates/metrics/AGENTS.md` — naming, cardinality, OTLP seam (`src/otlp.rs`).
- `crates/log/README.md` + `crates/log/AGENTS.md` — single subscriber pipeline, secret-redaction boundary.
- `crates/execution/src/journal.rs` — `JournalEntry` closed-enum WAL.
