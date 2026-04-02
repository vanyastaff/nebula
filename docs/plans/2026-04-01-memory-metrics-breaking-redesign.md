# Memory Metrics Breaking Redesign (v2)

## Objective

Replace the current `MetricsExtension` metric storage and registration workflow with a layout that avoids full payload movement, reduces allocator churn, and keeps the hot path predictable under load.

## Why this is breaking

1. `MetricsExtension` internals and registration flow will change from sorted map semantics to slot/index semantics.
2. Metric insertion ordering and duplicate handling guarantees may change.
3. `global_metrics()` cloning behavior will be replaced by shared-state access semantics.
4. Some helper structures will become private implementation details with different invariants.

## Current bottlenecks (from assembly)

1. Heavy stack frame and high call complexity in `MetricsExtension::register_metric`.
2. Frequent `memmove`/`memcpy` and repeated allocator calls in registration path.
3. Dynamic dispatch in hot path.
4. Panic and bounds-check paths still embedded in large function body.

## Target design

### 1. Storage model

Use split storage:

- `metrics_by_name: BTreeMap<String, MetricId>` (index)
- `metrics_slots: Vec<Option<MemoryMetric>>` (stable slots)
- `free_list: Vec<MetricId>` (slot reuse)

`MetricId` is a dense newtype around `usize`.

### 2. Mutation semantics

- Register: O(log n) index + O(1) slot insert/reuse.
- Update existing metric name: in-place slot replacement.
- Remove (future-safe API): tombstone slot and push id to free list.

No bulk metric payload movement on registration.

### 3. Global access

`global_metrics()` should stop rebuilding extension copies.
It should return a shared handle (Arc) to extension state.

### 4. Safety and invariants

- Every id in index points to a non-empty slot.
- Every id in free list points to an empty slot.
- Slot transitions are only `None -> Some` (allocate) and `Some -> None` (free).

## Implementation phases

### Phase 1 (this change set)

1. Introduce `MetricId`, split storage, and rewritten `register_metric`.
2. Keep public API mostly source-compatible where possible.
3. Remove clone-heavy global reconstruction path.
4. Update tests.

### Phase 2

1. Add explicit unregister API and deterministic compaction policy.
2. Reduce dynamic dispatch in hot path using reporter adapter enum.

Status: Implemented on 2026-04-01 (reporter adapter + unregister API with slot reuse).

### Phase 3

1. Add benchmark suite specific to metric registration throughput/latency.
2. Tune cache locality and lock strategy from profile data.

## Acceptance criteria

1. `cargo check -p nebula-memory` passes.
2. `cargo nextest run -p nebula-memory` passes.
3. No `memmove`-dominated hot path in `MetricsExtension::register_metric` assembly.
4. `global_metrics()` no longer rebuilds an ad-hoc extension copy.

## Risk

1. Behavior changes around metric replacement order.
2. Potential hidden assumptions in consumers about sorted storage iteration.
3. Global extension access semantics change may expose lifetime assumptions.

## Rollback

Revert to map-based storage while retaining tests introduced in this redesign; tests become a safety harness for subsequent redesign attempts.
