# Implementation Plan: nebula-expression

**Crate**: `nebula-expression` | **Path**: `crates/expression` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The expression crate evaluates template expressions and dynamic values in workflow definitions — resolving `{{ node.output.field }}` references, applying functions, and coercing types. It provides strict/compatibility evaluation modes and cache observability. Current focus is Phase 3 (scale/performance): cache tuning, hot evaluator paths, and lightweight observability.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (optional)
**Key Dependencies**: `nebula-core`, `nebula-parameter`, `serde_json`
**Testing**: `cargo test -p nebula-expression`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ✅ Done | Stable public API, evaluator safety guards, doc alignment |
| Phase 2: Runtime Hardening | ✅ Done | Error context, integration tests, context resolution edge cases |
| Phase 3: Scale and Performance | 🔄 In Progress | Cache tuning, hot-path optimization; `cache_overview()` available |
| Phase 4: Ecosystem and DX | ⬜ Planned | Strict mode rollout, migration tooling, production examples |

## Phase Details

### Phase 1: Contract and Safety Baseline ✅

**Goal**: Formalize stable public API; lock evaluator safety guards; align docs.

**Exit Criteria** (met):
- Stable contract docs validated by integration tests

### Phase 2: Runtime Hardening ✅

**Goal**: Improve error context; strengthen integration tests; harden context resolution edge cases.

**Exit Criteria** (met):
- Deterministic failures; no unresolved contract ambiguities

### Phase 3: Scale and Performance 🔄

**Goal**: Benchmark-driven cache tuning; optimize hot evaluator paths; expose cache observability.

**Deliverables**:
- Benchmark-driven cache tuning guidance
- Optimized hot evaluator paths and template rendering overhead
- Lightweight cache observability metrics (available: `ExpressionEngine::cache_overview()`)

**Exit Criteria**:
- Measurable throughput/latency gains with semantic parity

**Risks**:
- Optimization changes accidentally altering evaluation semantics

### Phase 4: Ecosystem and DX

**Goal**: Strict/compatibility evaluation modes; migration tooling; production examples.

**Deliverables**:
- Strict mode foundation (`EvaluationPolicy::with_strict_mode(true)`) — expand coverage
- Migration tooling for function/grammar evolution
- Production examples for common workflow patterns

**Exit Criteria**:
- Clear operator guidance; low-friction adoption path

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-parameter` (value types)
- **Depended by**: `nebula-action` (expression evaluation in context), `nebula-engine`, `nebula-workflow`

## Verification

- [ ] `cargo check -p nebula-expression`
- [ ] `cargo test -p nebula-expression`
- [ ] `cargo clippy -p nebula-expression -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-expression`
