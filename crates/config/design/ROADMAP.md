# Roadmap

## Phase 1: Contract Baseline and Documentation (Done)

- completed:
  - SPEC-template docs and interaction contracts aligned
  - precedence/path semantics documented in API + fixtures
  - governance/migration requirements codified in contract tests
- residual risks:
  - downstream consumers may still rely on undocumented local conventions
- exit criteria:
  - docs and fixtures treated as source of truth in CI

## Phase 2: Compatibility and Validation Hardening (Done)

- completed:
  - compatibility fixtures for precedence/path/type conversion are present
  - direct validator trait bridge integrated into `ConfigValidator`
  - validator compatibility + governance contract tests added
- residual risks:
  - stricter validation may still expose latent config debt in late-adopting consumers
- exit criteria:
  - contract suite remains green across crate changes and releases

## Phase 3: Reliability and Reload Semantics (Mostly Done)

- completed:
  - atomic reload behavior verification
  - reload failure preservation of last-known-good state
  - failure-mode guidance documented
- remaining:
  - stronger watcher lifecycle/backoff guidance for high-frequency reload workloads
- risks:
  - race conditions in high-frequency reload scenarios
- exit criteria:
  - explicit watcher/backoff guidance and targeted stress tests

## Phase 4: Source Ecosystem Expansion (Next)

- deliverables:
  - production-ready remote/database/kv source adapters
  - security model for remote source auth and trust
- risks:
  - increased attack surface and operational complexity
- exit criteria:
  - source adapter contracts and security tests pass

## Metrics of Readiness

- correctness:
  - zero known precedence/path regression failures.
- latency:
  - bounded config read path latency for typed getters.
- throughput:
  - stable reload throughput under expected source counts.
- stability:
  - no flaky reload/validation contract tests.
- operability:
  - actionable logging and metadata visibility for source/load state.

## Release-Path Requirements

- every behavior-significant change must declare:
  - compatibility class (`additive` or `breaking`)
  - migration mapping reference (`docs/crates/config/MIGRATION.md`)
  - fixture delta (`crates/config/tests/fixtures/compat/*`)
- release checklist:
  1. contract tests pass for precedence/reload/path categories
  2. migration docs updated when contract changes
  3. downstream consumers verified against pinned fixtures
  4. config-validator shared category fixture remains compatible
