# nebula-resilience — Agent orientation
> Local guide for `crates/resilience/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).
> Human contributors: the README's [Contributing](README.md#contributing) section is the
> entry point; this file is the machine-facing subset of the same rules.

**Purpose:** In-process stability-patterns pipeline (retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, load-shed) that action authors compose at outbound call sites; retry filtering is driven by `nebula-error::Classify`.
**Layer:** Cross-cutting — depends only downward (root AGENTS.md -> Layered Dependency Map); only Nebula dep is `nebula-error`.

## Common Tasks

| Task | Steps |
|------|-------|
| Add resilience to an outbound call | Compose patterns via `ResiliencePipeline<E>` / `PipelineBuilder` in `src/pipeline/`. The doc examples on those types are the guide; there is no separate prose manual. |
| Understand retry semantics | Two layers, never merged: this crate retries transient outbound calls inside one action attempt; the engine separately owns operator-declared node re-execution with persisted attempt accounting. `nebula-error::Classify::retry_hint()` classifies failures but does not authorize retry across an ambiguous remote-effect boundary (canon §11.2–§11.3). |
| Add a new resilience pattern | Add standalone module, integrate into `PipelineBuilder`, add to `src/lib.rs` re-exports. Add criterion bench in `benches/` (declare `required-features = ["bench-internals"]` if it measures crate internals). |
| Touch the circuit breaker | One accounting model: consecutive-failure counters with leaky forgiveness + slow-call rate. `circuit_state()` reads; `try_acquire()` mutates (probe slots, Open→HalfOpen) — never use it as a predicate. |

## Commands

- `cargo check -p nebula-resilience --all-features` and `cargo check -p nebula-resilience --all-targets --no-default-features` exercise the optional/default-free shapes separately.
- `cargo test -p nebula-resilience --doc` — the rustdoc examples are the reference documentation and must compile.
- benches: `cargo bench -p nebula-resilience --features bench-internals` (retry, hedge, latency_tracker, compose need that feature; the rest do not).
- features: `serde` (default), `bench-internals`.
- asm check for a hot function: `cargo asm -p nebula-resilience --lib "<symbol>"` (needs a release build; symbols are listed by `cargo asm -p nebula-resilience --lib` with no argument). A perf change to a hot path carries before/after asm and a number, not just a green bench.

## Key files

- `src/lib.rs` — crate docs + re-export surface (the public API map)
- `src/pipeline/` — `builder.rs` (`PipelineBuilder`) + `executor.rs` (`ResiliencePipeline`); composes the patterns
- `src/error.rs` — `CallError<E>` (`#[non_exhaustive]`, no type erasure); per-pattern variants
- `src/classifier.rs` + `src/context.rs` — `ErrorClassifier` (Classify seam) and `CallContext` (cancel/deadline/scope)
- `src/circuit_breaker.rs` · `src/retry.rs` · `src/bulkhead.rs` · `src/hedge.rs` — standalone patterns; `src/rate_limiter/` holds the trait plus one module per algorithm
- `src/fallback.rs` — strategies + the single `orchestrate_fallback` shared by `FallbackExecutor` and the pipeline
- `src/gate.rs` — cooperative-shutdown barrier; `src/events.rs` — `EventSink` observability hooks
- `src/policy.rs` — `PolicySource` / `LoadSignal` seams (no in-repo consumer yet; host-wired)

## Conventions & never-do

- **ADR-0068 / canon §11.2 define two retry layers.** This crate owns
  in-action outbound-call retry; the engine owns operator-declared node retry
  with persisted attempt accounting. Keep the trigger boundary explicit and
  obey canon §11.3 after an ambiguous remote effect.
- Retry/transient-vs-permanent is decided by `nebula-error::Classify::retry_hint()`, never by per-call folklore in action bodies.
- **Never apply hedge to an effecting call.** The effect driver accounts each provider invocation against a minted `OperationCallId` and policy budget; speculative duplication bypasses that accounting. Hedge is for read-only/idempotent lookups only.
- NOT a durable control plane (in-process only — durable cancel/dispatch lives in `execution_control_queue`) and NOT a metrics exporter (events feed `nebula-metrics` via sinks, not the reverse).
- **One exception to "in-process only": the GCRA limit-store *contract*.** `rate_limiter::gcra::LimitStore` (and its `ErasedLimitStore` facade) lives here so limit consumers never depend on storage, and so shared stores implement the exact same `gcra::step` math. This crate ships only `MemoryLimitStore`; database/Redis stores implement the trait in storage crates, which may depend on this crate — never the reverse. Stores run each `step` transition atomically on **their own clock**; no store may read a caller's clock. Keys reaching a store are opaque: namespacing, tenant isolation and hashing of personal data are the caller's job. Every store applies `step::effective_rate` (a busy key enforces the stricter of the rates its callers declare; an idle key forgets) and must pass `gcra::conformance::run_all` — the `conformance` feature exposes that kit to storage crates' tests; it is test support and panics on a violation.
- GCRA invariants are pinned by `rate_limiter/gcra/tests.rs` (window bound, `per_window` quota, FIFO, refusal consumes nothing, tail-only cancel without ABA). `RateLimiter::acquire` on `Gcra` stays fail-fast like every other limiter; waiting is `Gcra::until_ready` / `LimitStore::reserve`, never a changed `acquire`.
- `CallError<E>` keeps the caller's `E` — no forced mapping, no `Box<dyn Error>` erasure; keep variants additive (`#[non_exhaustive]`).
- Never report a panicked or aborted attempt as `Cancelled`; `CallError::TaskPanicked` exists for that distinction.
- No `unsafe` in this crate (`#![deny(unsafe_code)]`).
- **Float hot paths assume the default x86-64 target, which has no hardware FMA.** `mul_add` lowers to a ~30-cycle `call fma` there, so `retry.rs` and `rate_limiter/` use explicit multiply+add under `#[expect(clippy::suboptimal_flops, reason = …)]`. Do not "fix" those back to `mul_add` without checking `cargo asm` for the target CI builds.
- **`delay_for`'s general exponential path must stay free of soft-float libcalls** (`__powidf2`, `__floattidf`). `powi_nonnegative` mirrors compiler-rt's operation order; `exponential_general_path_matches_powi_oracle` is the numeric oracle.
- **The circuit breaker reads its instant source only when `slow_call_threshold` is set.** `call_skips_instant_reads_without_slow_threshold` pins it; adding an unconditional `Instant::now()` back would silently cost ~20ns per call in the default config.
- Rustdoc is the reference documentation. Do not reintroduce a `docs/` prose folder; update the doc comment instead.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Cancellation/deadline composition | [cancel_safety](tests/cancel_safety.rs), [call_context_contracts](tests/call_context_contracts.rs), [pipeline](tests/pipeline.rs). |
| Limiter/backoff behavior | [rate_limiter](tests/rate_limiter.rs), [proptest_backoff](tests/proptest_backoff.rs); retry-budget bounds live in `src/retry_tests.rs`. GCRA math and stores: `src/rate_limiter/gcra/tests.rs` (property tests + paused clock). |
| Circuit-breaker accounting | `src/circuit_breaker_tests.rs` plus [circuit_breaker](tests/circuit_breaker.rs); Layer-1/Layer-2 backoff parity is pinned in `crates/engine/src/engine/tests.rs`. |
| Hot-path timing or float math | `src/retry_tests.rs` (`exponential_general_path_matches_powi_oracle`), `src/circuit_breaker_tests.rs` (`call_skips_instant_reads_without_slow_threshold`), `tests/proptest_backoff.rs`; plus before/after `cargo bench` and `cargo asm` for the symbol touched. |

## See also

- `README.md` — purpose, role, terminology table, contribution rules, non-goals
- Root [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — toolchain, commits, PR policy
- Canon [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §4.2/§4.3/§11.2–§11.3 (Circuit Breaker + Timeout + Retry-with-Backoff)
