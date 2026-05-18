---
id: 0005
title: trigger-health-trait
status: accepted
date: 2026-04-12
supersedes: []
superseded_by: []
tags: [action, trigger, observability, health]
related: [crates/action/src/capability.rs, crates/action/src/poll.rs, crates/action/src/context.rs]
---

# 0005. TriggerHealth — atomic lock-free health state on TriggerContext

## Context

Poll-based triggers run in long-lived loops. Without a standard way to surface
health metrics (idle streaks, error counts, last-success time), operators had
no visibility into whether a trigger was cycling normally or silently degraded.
Each trigger adapter would have needed to invent its own monitoring scheme,
leading to inconsistent observability.

The injection pattern already established for other capabilities —
`ExecutionEmitter`, `ActionLogger`, `CredentialAccessor` on `ActionContext` —
provided the model: inject a capability object on construction; adapters write
to it; the runtime reads it. The question was shape: trait or concrete struct?

Memory note (`memory/project_health_trait.md`): future generalisation of
`TriggerHealth` into a shared `Health<T>` trait when `Resource` and `Agent`
have comparable lifecycle requirements is deferred; premature generalisation
would complicate the API before the second use-case existed.

## Decision

Add `TriggerHealth` as a concrete struct (not a trait) on `TriggerContext`,
accessed as `Arc<TriggerHealth>`. The struct holds universal metrics via
atomics — `last_active_at`, `last_success_at`, `idle_streak`, `error_streak`,
`total_emitted`, `total_cycles` — with `Relaxed` ordering (eventual consistency
is sufficient for monitoring).

Family-specific data goes in `details: Option<Value>` (arbitrary JSON).
`PollTriggerAdapter` serialises a `PollHealth` struct (current interval,
consecutive_empty, dedup_keys count) into `details` after every cycle.

No trait because health state shape is universal across all trigger families;
trait dispatch would add indirection without enabling new behaviour. If
`Resource` or `Agent` need health reporting, promote to a trait then.

## Consequences

Positive:

- Zero-allocation per cycle: all counters are atomics; no `Mutex`, no
  `HashMap`, no heap allocation on the hot path.
- Uniform health surface: every trigger adapter gets the same metrics without
  custom code; the runtime reads one struct shape for all trigger types.
- `details: Option<Value>` allows family-specific metrics (poll interval,
  dedup keys) without changing the core struct.

Negative:

- Concrete struct, not a trait: extending for non-trigger use-cases requires
  either a future generalisation or duplication if Resource/Agent need a
  similar capability before the generalisation lands.
- `Relaxed` ordering means cross-field consistency is not guaranteed under
  concurrent writes; counts may be transiently inconsistent in multi-threaded
  trigger runtimes. Acceptable for monitoring; not suitable for security
  invariants.

Follow-up:

- Generalise into a `Health<Details>` trait when `Resource` or `Agent` adopts
  health reporting (tracked in `memory/project_health_trait.md`).
- Wire `TriggerHealth` snapshots into the telemetry/metrics pipeline (currently
  available via `snapshot()` but not automatically emitted to OTLP).

## Alternatives considered

- **`TriggerHealthReporter` trait (write side) + concrete snapshot (read side).**
  Considered but rejected: the extra trait layer adds indirection for no gain
  since the shape is universal. The commit message itself notes "trait dispatch
  adds nothing" at this stage.
- **`Arc<Mutex<TriggerHealthState>>`**: simpler type but allocates on every
  write; not appropriate for a per-cycle hot path.

## Seam / verification

Seam: `crates/action/src/capability.rs` — `TriggerHealth` struct with atomic
fields; `record_success`, `record_idle`, `record_error`, `snapshot` methods.
`crates/action/src/poll.rs` — `PollTriggerAdapter` calls `record_*` after every
cycle.
`crates/action/src/context.rs` — `TriggerContext` carries `Arc<TriggerHealth>`.

Landed in commit `be6b62c2` (feat(action): TriggerHealth reporting capability).
