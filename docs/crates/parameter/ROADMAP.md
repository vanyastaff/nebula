# Roadmap

`nebula-parameter` roadmap targets stronger contracts between schema, UI, and runtime execution.

## Phase 1: Contract and Safety Baseline

- **Deliverables:** Align docs/examples with actual API; publish stable naming conventions for keys/paths; document required vs nullable behavior per kind; schema lint pass (P-004)
- **Risks:** Lint may surface breaking schema issues in existing definitions
- **Exit criteria:** All consumers pass lint; error code stability documented

## Phase 2: Runtime Hardening

- **Deliverables:** Benchmark deep nested object/list validation; optimize recursive path building and error allocation; stress tests for large collections and high error counts; deterministic error ordering (P-002)
- **Risks:** Performance regressions in hot paths
- **Exit criteria:** Benchmarks in CI; no allocation regression in common cases

## Phase 3: Scale and Performance

- **Deliverables:** Improve typed extraction helpers; clearer conversion contracts for numbers/integers/decimals; reduce ambiguity in "any"-typed flows; optional typed value layer (P-001)
- **Risks:** Typed layer migration complexity
- **Exit criteria:** Typed API available; migration path documented

## Phase 4: Ecosystem and DX

- **Deliverables:** Formalize dependency graph extraction from display rules; detect cycles/contradictory visibility at schema build time; diagnostics for unreachable parameters; ValidationRule versioning (P-005); ParameterKey newtype (P-003)
- **Risks:** Display rule analysis may be expensive for large schemas
- **Exit criteria:** Display rule lint; version metadata in persisted schemas

## Metrics of Readiness

- **Correctness:** All validation paths covered; error codes stable
- **Latency:** Validation &lt;1ms for typical node configs
- **Throughput:** N/A (sync, per-request)
- **Stability:** No breaking changes without MIGRATION.md
- **Operability:** Workspace baseline Rust 1.93
