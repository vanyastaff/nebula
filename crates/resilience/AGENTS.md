# nebula-resilience — Agent orientation
> Agent quick-map for `crates/resilience/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** In-process stability-patterns pipeline (retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, load-shed) that action authors compose at outbound call sites; retry filtering is driven by `nebula-error::Classify`.
**Layer:** Cross-cutting — depends only downward (root AGENTS.md -> Layered Dependency Map); only Nebula dep is `nebula-error`.

## Common Tasks

| Task | Steps |
|------|-------|
| Add resilience to an outbound call | Compose patterns via `ResiliencePipeline<E>` / `PipelineBuilder` in `src/pipeline.rs`. See `docs/composition.md`. |
| Understand retry semantics | Retry/transient-vs-permanent decided by `nebula-error::Classify::retry_hint()`, never by per-call folklore. This is the ONLY retry surface in the workflow stack (canon §11.2). |
| Add a new resilience pattern | Add standalone module, integrate into `PipelineBuilder`, add to `src/lib.rs` re-exports. Add criterion bench in `benches/`. |
| Run loom model checks | `RUSTFLAGS="--cfg loom" cargo test -p nebula-resilience --features loom --lib loom` |
| Run benchmarks | `cargo bench -p nebula-resilience` (14 criterion benches) |

## Commands
- `cargo check -p nebula-resilience`  ·  all features: `cargo check -p nebula-resilience --all-features`
- `cargo nextest run -p nebula-resilience`  ·  doctests: `cargo test -p nebula-resilience --doc`
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
- **Canon §11.2: this is the ONLY retry surface in the workflow stack** — the engine does NOT re-execute nodes; never add engine-level retry expecting this crate to defer to it.
- Retry/transient-vs-permanent is decided by `nebula-error::Classify::retry_hint()`, never by per-call folklore in action bodies.
- NOT a durable control plane (in-process only — durable cancel/dispatch lives in `execution_control_queue`) and NOT a metrics exporter (events feed `nebula-metrics` via sinks, not the reverse).
- `CallError<E>` keeps the caller's `E` — no forced mapping, no `Box<dyn Error>` erasure; keep variants additive (`#[non_exhaustive]`).
- `#![deny(unsafe_code)]`; loom-gated atomics behind `cfg(loom)` for model checks only.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · crate-local guides in `docs/` (`composition.md`, `observability.md`, `gate.md`, `api-reference.md`, `architecture.md`)
- Canon `docs/PRODUCT_CANON.md` §4.2/§4.3/§11.2 (Circuit Breaker + Timeout + Retry-with-Backoff)
