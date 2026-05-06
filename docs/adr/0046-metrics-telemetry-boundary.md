# ADR-0046: Merge `nebula-telemetry` into `nebula-metrics` — single observability crate

**Status:** Accepted
**Date:** 2026-05-06
**Supersedes:** L1 canon invariant `[L1-§3.10]` (in `crates/telemetry/README.md`) — see "Supersession" below.
**Superseded by:** —
**ROADMAP:** §M9 — Observability + DoD audit pass
**Issues:** #595 (metrics OTLP label allocation), #591 (related cardinality work), #598 (telemetry: verify OpenTelemetry setup against bridge-pattern guide). Referenced for context — this ADR does **not** claim to close them.

> **Note on prior audits.** Prior audits in `docs/audits/` (May 2026) covering `nebula-metrics`, `nebula-telemetry`, and the joint stack are superseded for the boundary-decision question by this ADR. A comprehensive observability re-audit (covering implementation invariants, cardinality safety, exporter correctness) is deferred to a follow-up `/aif-plan` iteration after the merge implementation lands. Findings from those prior audits are not referenced here; ADR-0046 engages with the boundary question on first principles.

## Context

The Nebula workspace ships two cross-cutting observability crates:

- `nebula-telemetry` (~1.86 kLOC): primitive metric storage. `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `HistogramSnapshot`, `LabelInterner`, `LabelSet`, `MetricKey`, `MetricKind`, `TelemetryError`, `TelemetryResult`. Five direct deps (`nebula-error`, `thiserror`, `tracing`, `dashmap`, `lasso`).
- `nebula-metrics` (~2.17 kLOC): naming policy, label safety, and Prometheus text-format export. ~50 `NEBULA_*` constants, `LabelAllowlist`, `PrometheusExporter`, `TelemetryAdapter`, `record_eventbus_stats(EventBusStats)`. Two direct deps (`nebula-eventbus`, `nebula-telemetry`).

Both are members of the cross-cutting layer per `ARCHITECTURE.md`. Three workspace consumers (`nebula-api`, `nebula-engine`, `nebula-resource`) declare both as Cargo dependencies. The split is documented as load-bearing in `crates/telemetry/README.md` via canon `[L1-§3.10]`: *"This crate is the primitive layer. Naming conventions (`nebula_*` prefix), adapters, and export formats belong in `nebula-metrics` — not here. If naming helpers appear in this crate, that is a layering violation."*

A first-principles audit (`docs/audits/metrics-telemetry-merge.md`) revisited this split in light of accumulated workspace evidence:

1. **No load-bearing telemetry-only consumer exists.** Six workspace files import `nebula_telemetry::*` directly without `nebula_metrics::*` — `crates/api/src/state.rs` plus five `crates/engine/tests/*.rs`. All six use `MetricsRegistry` as a primitive type (not in a context that excludes naming policy); each could equally use `nebula_metrics::MetricsRegistry` (the re-export). The split has no consumer for whom primitives-without-policy is a semantic requirement.

2. **Daily friction is real and quantified.** The split costs:
   - 8 dual-import call-sites in workspace consumers (audit §2.2 Category A);
   - The 431-LOC `TelemetryAdapter` bridge type, whose only purpose is to mediate between naming constants in `-metrics` and the registry in `-telemetry`;
   - ~8 type re-exports + ~50 constant re-exports in `nebula-metrics::lib.rs`, plus a dedicated `prelude` module that re-exports primitives from `-telemetry`;
   - 5+ deep-path internal imports from `-metrics` reaching into `-telemetry`'s submodule shape (`telemetry::metrics::*`, `telemetry::labels::*`) — coupling on inner module layout, not just public API;
   - Recurring sync overhead when `-telemetry` adds a public type (the recent breaking `feat(telemetry)!: fallible registry and histogram snapshots (#645)` propagated through `-metrics::lib.rs` re-exports).

3. **The `[L1-§3.10]` invariant is doc-enforced only.** `deny.toml` contains no `[[bans.wrappers]]` rule for either crate. A file in `crates/telemetry/src/` defining `pub const NEBULA_FOO: &str = "..."` would compile, lint clean, and pass CI. The "layering violation" exists only in code review.

4. **Public contract is unaffected.** `nebula-sdk` and `nebula-plugin-sdk` (the third-party integrator surface per `ARCHITECTURE.md`) declare no dependency on either crate. `nebula-api` depends on both internally but does not re-export them in `lib.rs`. The `/metrics` HTTP endpoint structure is unchanged regardless of the boundary decision.

5. **Ecosystem fit is unique.** The Rust observability ecosystem splits along facade ↔ exporter (`metrics-rs`), API ↔ SDK ↔ exporter (`opentelemetry-rust`), or not at all (`prometheus`-the-crate is a single-crate monolith). Nebula's primitives-vs-policy axis matches none of these. The closest analog to a merged outcome is `prometheus`-the-crate.

The merger question therefore reduces to: *does the split protect a property that cannot be expressed inside one crate?* Per the audit, the answer is no — `pub`/`pub(crate)` discipline plus comment-separated `mod` declarations in `lib.rs` express the same constraint without crate-level overhead, and CI does not enforce the constraint at the crate level today regardless.

## Decision

**Merge `nebula-telemetry` into `nebula-metrics` as a single crate with a flat module layout.**

### Final crate name

`nebula-metrics`. Rationale: preserves consumer-facing identity (most call-sites already import from there); matches Rust ecosystem naming (`prometheus`, `metrics-rs`, `opentelemetry-metrics` all use "metrics" for this concept); honest scope — content is metrics-only (no traces, no spans, no logs), so "telemetry" or "observability" would over-promise.

### Flat module layout

```
crates/metrics/src/
├── lib.rs              # crate root: `mod` declarations + re-exports + crate docs
├── counter.rs          # Counter
├── gauge.rs            # Gauge
├── histogram.rs        # Histogram, HistogramSnapshot
├── registry.rs         # MetricsRegistry
├── labels.rs           # LabelInterner, LabelSet, MetricKey
├── naming.rs           # NEBULA_* constants + label helpers
├── filter.rs           # LabelAllowlist
├── prometheus.rs       # PrometheusExporter (renames export/prometheus.rs)
├── eventbus.rs         # record_eventbus_stats(EventBusStats)
├── error.rs            # MetricsError, MetricsResult
└── prelude.rs
```

`lib.rs` groups `mod` declarations by concern via comment separators (primitives / policy / export / instrumentation / error). External consumers see a flat `pub use` surface — `nebula_metrics::Counter`, not `nebula_metrics::primitives::Counter`. Submodule wrapper directories are not introduced preventively; a single file may later be split into its own folder if it outgrows ~700 LOC, but that is reactive, not anticipated.

### Renames

- `TelemetryError` → `MetricsError`
- `TelemetryResult` → `MetricsResult`
- `TelemetryAdapter` is **deleted**. Its methods become inherent `impl MetricsRegistry { ... }` blocks or free functions in the appropriate flat module. The bridge type has no role inside one crate.

### Boundary mechanism (replaces `[L1-§3.10]`)

The constraint *"primitive types must not co-locate with naming policy in the same module"* is preserved as a Rust-level discipline:
- `counter.rs`, `gauge.rs`, `histogram.rs`, `registry.rs`, `labels.rs` define primitive types only.
- `naming.rs`, `filter.rs`, `prometheus.rs`, `eventbus.rs` define policy/export.
- `lib.rs` re-exports each at the top level for external consumers.

Anyone introducing a `NEBULA_*` constant in `counter.rs` (etc.) is rejected at code review or via a future grep-CI lint, exactly as `[L1-§3.10]` was enforced before. The mechanism is the same; the *unit of separation* shrinks from "two crates" to "two module groups in one crate".

## Supersession of `[L1-§3.10]`

Canon `[L1-§3.10]` in `crates/telemetry/README.md` claimed: *"This crate is the primitive layer. Naming conventions (`nebula_*` prefix), adapters, and export formats belong in `nebula-metrics` — not here."*

That invariant is **superseded by this ADR**. After implementation:
- `crates/telemetry/` is deleted; the L1 invariant has no anchor.
- The new constraint is documented in `crates/metrics/README.md` and enforced at the module level via `lib.rs`'s comment-separated `mod` blocks.

This is a deliberate canon revision per memory `feedback_adr_revisable.md`: when an ADR / invariant forces workarounds (here: 431-LOC `TelemetryAdapter`, 8 dual-import sites, deep-path internal coupling), it should be superseded — not patched around.

## Alternatives considered

### Option 1 — Keep split as-is

Rejected. The split protects a doc-only invariant (`[L1-§3.10]`) that `cargo deny check bans` does not enforce. Inside one merged crate the same constraint is expressible with `pub`/`pub(crate)` discipline and `mod` boundaries — no weaker. The split's daily cost (audit §5: ~500 LOC boundary maintenance + 8 dual-import sites) is paid for an unrealized benefit (no consumer needs primitives-without-policy per audit §2.3). Embeddability of `nebula-telemetry` as a standalone primitives library is an unexercised README claim (root `examples/` does not import either crate); ecosystem-grade primitive embeddability follows the *facade* pattern (`metrics-rs`), which the current split does not provide either.

### Option 3 — Hybrid (re-export module or feature gate)

Rejected. Two variants were considered:

- **3a) `nebula_metrics::primitives` re-export module.** Adds a re-export layer on top of the existing split. Per memory `feedback_no_shims`: bridges that solve symptoms of the wrong split are a smell. Keeps the 431-LOC `TelemetryAdapter` and all daily friction; reduces only the import-path inconsistency. Worst of both worlds.

- **3b) Feature-gated policy.** Adds a `policy` Cargo feature on `nebula-metrics` so consumers wanting primitives-only can opt out. Adds compile-time matrix complexity for an unrealized use case. Per memory `feedback_idiom_currency`: feature flags must justify their existence. Rejected.

## Consequences

### Breakage classification

**Internal-only breaking, public NON-breaking.**

- **Public contract** (sdk / plugin-sdk / api HTTP surface): unchanged. No semver bump for plugin authors or HTTP API consumers.
- **Workspace-internal**: ~25 files require mechanical rename `nebula_telemetry::*` → `nebula_metrics::*`. Distribution per audit §2.2:
  - 8 dual-import sites collapse to single-import (Category A).
  - 6 telemetry-only sites get a path rename (Category B).
  - 5+ deep-path internal `-metrics` imports become `use crate::*`.
  - ~12 doctest blocks rewrite import lines.
  - `telemetry/examples/basic_metrics.rs` is deleted (content rehomed into `crates/metrics/examples/` if still useful).
  - Three consumer `Cargo.toml`s (`api`, `engine`, `resource`) drop the `nebula-telemetry` line.
  - Root `Cargo.toml` `workspace.members` removes `crates/telemetry`.
  - `nebula-metrics` `Cargo.toml` drops the `nebula-telemetry` dep; keeps `nebula-eventbus`.

### What goes away

- 431-LOC `TelemetryAdapter` bridge type (audit §5.3).
- ~8 type re-exports + ~50 constant re-exports in `nebula-metrics::lib.rs`.
- Recurring re-export sync work when `-telemetry` adds public types.
- 8 dual-import call-sites' second import line.
- The `[L1-§3.10]` doc invariant (replaced by module-level discipline in `crates/metrics/lib.rs`).

### What is preserved

- `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `HistogramSnapshot`, `LabelInterner`, `LabelSet`, `MetricKey`, `MetricKind` — same types, top-level path under `nebula_metrics`.
- All `NEBULA_*` constants and label helpers (`crates/metrics/src/naming.rs` content unchanged).
- `LabelAllowlist`, `PrometheusExporter`, `record_eventbus_stats` — same APIs, top-level path.
- `nebula-eventbus` dep (carried by the merged crate; the four `nebula_eventbus_*` gauges remain).
- Public HTTP `/metrics` endpoint behavior (unchanged — `nebula-api` calls `nebula_metrics::snapshot()` either way).

### What is foreclosed

- Publishing `nebula-telemetry` as a standalone crates.io primitives library. This option was reserved by README framing but is unrealized today; the workspace `Cargo.toml` does not configure it for independent publication. If future requirements drive this, a separate ADR can revisit (the merged crate could be re-split along the *facade* axis per audit §4.6 Path B, which is the ecosystem-idiomatic embeddability route — different from today's primitive-vs-policy split).

## Next steps

This ADR records the boundary decision. **The implementation lands as a follow-up `/aif-plan` iteration** after this branch (`docs/metrics-telemetry-merge-audit`) is merged. No code changes ship on this branch.

Suggested follow-up plan slug: `metrics-telemetry-merge-implementation`. Scope:

- Move all `crates/telemetry/src/` content into `crates/metrics/src/` per the flat layout above.
- Apply the renames (`Telemetry{Error,Result}` → `Metrics{Error,Result}`, drop `TelemetryAdapter`).
- Update workspace consumers (`api`, `engine`, `resource`) — mechanical rename + Cargo.toml edits.
- Update doctests (~12 blocks).
- Delete `crates/telemetry/`.
- Update `crates/metrics/README.md` to absorb the boundary explanation that lived in `crates/telemetry/README.md`.
- Update `ARCHITECTURE.md` and `AGENTS.md` Layered Dependency Map (cross-cutting layer drops `telemetry` entry).
- Update `.ai-factory/DESCRIPTION.md` Tech-Stack section if `nebula-telemetry` is named.

The implementation plan should anchor breakage scope and acceptance criteria to this ADR rather than re-deriving them. The Working Hypothesis structure quoted above is the implementation target.

Until that follow-up plan lands, **no code changes ship**: this branch is documentation-only.

## References

- [docs/audits/metrics-telemetry-merge.md](../audits/metrics-telemetry-merge.md) — the audit doc this ADR adopts as evidence (§1 Inventory, §2 Call-site map, §3 Why split today, §4 Ecosystem reference, §5 Friction observed, §6 Options, §7 Decision criteria).
- [crates/metrics/README.md](../../crates/metrics/README.md) — current `nebula-metrics` role description (will be rewritten by the implementation plan).
- [crates/telemetry/README.md](../../crates/telemetry/README.md) — current `nebula-telemetry` role description, including `[L1-§3.10]` (file deleted by the implementation plan).
- [crates/metrics/src/lib.rs](../../crates/metrics/src/lib.rs) — the re-export layer that becomes the merged crate's natural surface.
- [crates/metrics/src/adapter.rs](../../crates/metrics/src/adapter.rs) — `TelemetryAdapter` (deleted by the implementation plan).
- [.ai-factory/ROADMAP.md](../../.ai-factory/ROADMAP.md) §M9 — Observability + DoD audit pass.
- ADR-0042 (`docs/adr/0042-layered-retry.md`) — format and rigor reference for this ADR.
