# Roadmap

## Phase 1: Contract Baseline and Documentation

- deliverables:
  - full SPEC-template docs and interaction contracts
  - precedence/path semantics formally documented
- risks:
  - hidden assumptions in consumers
- exit criteria:
  - docs accepted as source of truth

## Phase 2: Compatibility and Validation Hardening

- deliverables:
  - compatibility fixtures for precedence, path access, and type conversion
  - stricter validation integration patterns for consumers
- risks:
  - stricter validation can expose latent config debt
- exit criteria:
  - CI contract suite passes for representative consuming crates

## Phase 3: Reliability and Reload Semantics

- deliverables:
  - atomic reload behavior verification
  - watcher lifecycle and reload backoff strategy guidance
  - failure-mode runbooks
- risks:
  - race conditions in high-frequency reload scenarios
- exit criteria:
  - reload failure preserves last-known-good config in all tested scenarios

## Phase 4: Source Ecosystem Expansion

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
