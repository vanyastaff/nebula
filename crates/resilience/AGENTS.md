# nebula-resilience — Agent orientation
> Agent quick-map for `crates/resilience/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** In-process stability-patterns pipeline (retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, load-shed) that action authors compose at outbound call sites; retry filtering is driven by `nebula-error::Classify`.
**Layer:** Cross-cutting — depends only downward (root AGENTS.md -> Layered Dependency Map); only Nebula dep is `nebula-error`.

## Common Tasks

| Task | Steps |
|------|-------|
| Add resilience to an outbound call | Compose patterns via `ResiliencePipeline<E>` / `PipelineBuilder` in `src/pipeline.rs`. See `docs/composition.md`. |
| Understand retry semantics | ADR-0068 defines two layers: this crate retries transient outbound calls inside one action attempt; the engine separately owns operator-declared node re-execution. `nebula-error::Classify::retry_hint()` classifies failures, but does not authorize retry across an ambiguous remote-effect boundary (canon §11.2–§11.3). |
| Add a new resilience pattern | Add standalone module, integrate into `PipelineBuilder`, add to `src/lib.rs` re-exports. Add criterion bench in `benches/`. |
| Run loom model checks | `RUSTFLAGS="--cfg loom" cargo test -p nebula-resilience --features loom --lib loom` |
| Run benchmarks | `cargo bench -p nebula-resilience` (14 criterion benches) |

## Commands
- `cargo check -p nebula-resilience`  ·  all features: `cargo check -p nebula-resilience --all-features`
- loom model-check: `RUSTFLAGS="--cfg loom" cargo test -p nebula-resilience --features loom --lib loom`
- benches: `cargo bench -p nebula-resilience` (14 criterion benches, e.g. `compose`, `retry`, `hedge`)
- features: `serde` (default), `full` (= serde), `loom`

## Key files
- `src/lib.rs` — crate docs + re-export surface (the public API map)
- `src/pipeline.rs` — `ResiliencePipeline<E>` / `PipelineBuilder`; composes the patterns
- `src/error.rs` — `CallError<E>` (`#[non_exhaustive]`, no type erasure); per-pattern variants
- `src/classifier.rs` + `src/context.rs` — `ErrorClassifier` (Classify seam) and `PolicyContext` (cancel/deadline/scope)
- `src/circuit_breaker.rs` · `src/retry.rs` · `src/bulkhead.rs` · `src/rate_limiter.rs` · `src/hedge.rs` — the standalone patterns
- `src/gate.rs` — cooperative-shutdown barrier; `src/sink.rs` — `MetricsSink` observability hooks

## Conventions & never-do
- **ADR-0068 / canon §11.2 define two retry layers.** This crate owns
  in-action outbound-call retry; the engine owns operator-declared node retry
  with persisted attempt accounting. Keep the trigger boundary explicit and
  obey canon §11.3 after an ambiguous remote effect.
- Retry/transient-vs-permanent is decided by `nebula-error::Classify::retry_hint()`, never by per-call folklore in action bodies.
- NOT a durable control plane (in-process only — durable cancel/dispatch lives in `execution_control_queue`) and NOT a metrics exporter (events feed `nebula-metrics` via sinks, not the reverse).
- `CallError<E>` keeps the caller's `E` — no forced mapping, no `Box<dyn Error>` erasure; keep variants additive (`#[non_exhaustive]`).
- `#![deny(unsafe_code)]`; loom-gated atomics behind `cfg(loom)` for model checks only.

## See also
- `README.md` — full design · crate-local guides in `docs/` (`composition.md`, `observability.md`, `gate.md`, `api-reference.md`, `architecture.md`)
- Canon `docs/PRODUCT_CANON.md` §4.2/§4.3/§11.2 (Circuit Breaker + Timeout + Retry-with-Backoff)
