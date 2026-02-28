# Decisions

## D001: tracing-first Observability

**Status:** Adopt

**Context:** Need structured logging with spans, async compatibility, and ecosystem maturity for a Rust workflow platform.

**Decision:** Use `tracing` + `tracing-subscriber` as primary abstraction.

**Alternatives considered:** `log` crate only (no spans, weaker structure); `slog` (smaller ecosystem).

**Trade-offs:** Larger dependency surface; re-exports of tracing macros for ergonomics.

**Consequences:** All logging goes through tracing; `log-compat` feature bridges legacy `log` callers.

**Migration impact:** None; initial design.

**Validation plan:** All examples and tests use tracing macros.

---

## D002: Feature-gated Integrations

**Status:** Adopt

**Context:** Binary size and deployment flexibility vary (CLI vs server, with/without OTLP).

**Decision:** Keep telemetry/file/metrics/sentry optional behind feature flags.

**Alternatives considered:** Monolithic build; separate crates per integration.

**Trade-offs:** Feature matrix complexity; conditional compilation in docs.

**Consequences:** Consumers enable only needed features; `full` feature for convenience.

**Migration impact:** None.

**Validation plan:** `cargo check --all-features`; CI with default and full.

---

## D003: Panic-isolated Hook Dispatch

**Status:** Adopt

**Context:** Hooks are third-party extensions; one faulty hook must not break logging.

**Decision:** Catch panics in hook lifecycle and event dispatch via `catch_unwind`.

**Alternatives considered:** Let panics propagate; abort on hook panic.

**Trade-offs:** Panic cost on hot path; possible silent hook failures.

**Consequences:** Hook bugs do not crash process; monitoring should track hook errors.

**Migration impact:** None.

**Validation plan:** Unit test with panicking hook verifies emission continues.

---

## D004: Async-safe Context Propagation

**Status:** Adopt

**Context:** Workflow execution spans `.await`; context (request/user/session) must persist.

**Decision:** Use task-local context in async mode, thread-local in sync mode.

**Alternatives considered:** Manual context passing; global only.

**Trade-offs:** Tokio dependency for async; two code paths.

**Consequences:** `with_context!` and `current_contexts()` work across `.await`.

**Migration impact:** None.

**Validation plan:** Async integration test with context across await.

---

## D005: Config-first Initialization

**Status:** Adopt

**Context:** Predictable behavior in multi-environment and high-load deployments.

**Decision:** Expose explicit `Config` + presets (`from_env`, `development`, `production`) rather than hardcoded global behavior.

**Alternatives considered:** Env-only; builder-only without presets.

**Trade-offs:** More API surface; clearer contract.

**Consequences:** `auto_init` uses presets; production configs are explicit.

**Migration impact:** None.

**Validation plan:** Config round-trip and preset tests.

---

## D006: Staged Hook Budget Policy

**Status:** Adopt

**Context:** Phase 2 needs bounded hook execution behavior without immediate async queue complexity.

**Decision:** Keep current staged model:
- v1: `HookPolicy::Bounded` uses inline dispatch with execution-budget diagnostics.
- v2: async offload queue and drop accounting are deferred until explicit rollout.

**Alternatives considered:** Ship only full async offload; keep inline-only policy indefinitely.

**Trade-offs:** Faster delivery and lower risk now; incomplete latency isolation until v2.

**Consequences:** Operators can detect slow hooks today; throughput isolation remains future work.

**Migration impact:** None for current API; future async mode remains opt-in.

**Validation plan:** `tests/hook_policy.rs` plus runtime docs for budget/backpressure semantics.
