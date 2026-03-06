# Next Tasks: nebula-resilience

Scope: post-Phase-8 backlog for `nebula-resilience` focused on cross-crate integration, production readiness, and documentation consistency.

## Priority Backlog

- [ ] RSL-N001 Wire resilience into runtime execution path
  - Goal: apply timeout/retry/circuit-breaker policies around action execution in `nebula-runtime`.
  - Done when: runtime can execute with `ResilienceManager` wrappers and integration tests cover success, retry, timeout, and breaker-open outcomes.

- [ ] RSL-N002 Wire resilience into engine orchestration boundaries
  - Goal: ensure `nebula-engine` uses resilience policies at node execution boundaries (not only local ad-hoc timeouts).
  - Done when: engine integration tests verify node-level policy enforcement and expected DAG behavior under transient failures.

- [ ] RSL-N003 Define policy handoff contract (`action` -> `runtime` -> `resilience`)
  - Goal: map `ActionError::Retryable`, backoff hints, and action metadata to concrete resilience policy inputs.
  - Done when: one documented contract exists and contract tests validate deterministic mapping.

- [ ] RSL-N004 Add end-to-end cancellation/timeout semantics tests
  - Goal: validate cancellation and timeout behavior across `engine + runtime + resilience` together.
  - Done when: workspace integration tests cover cancel-before-start, cancel-during-retry, timeout-during-backoff, and cleanup guarantees.

- [ ] RSL-N005 Export resilience telemetry to shared metrics pipeline
  - Goal: connect resilience events/counters to `nebula-telemetry` naming and labels.
  - Done when: metric schema is documented, emitted in integration tests, and queried via existing telemetry flow.

- [ ] RSL-N006 Add distributed operations guidance for horizontal scale
  - Goal: document strategy for per-instance vs global limits (noted as future in reliability docs).
  - Done when: `crates/resilience/docs/RELIABILITY.md` contains clear deployment patterns for multi-node rate limiting and breaker tuning.

- [ ] RSL-N007 Add CI performance guardrails for hot resilience benches
  - Goal: fail CI on significant regressions in key bench suites (`manager`, `retry`, `bulkhead`, `timeout`, `fallback`, `hedge`).
  - Done when: thresholds are versioned and a CI job reports regression deltas.

- [ ] RSL-N008 Add production chaos profile for resilience composition
  - Goal: create repeatable chaos scenarios for overload, retry storms, and degraded downstream dependencies.
  - Done when: scripted scenario suite exists and produces a pass/fail summary based on SLO-oriented criteria.

- [ ] RSL-N009 Fix roadmap/task source-of-truth drift
  - Goal: align resilience planning docs where status/link mismatches exist.
  - Done when: `docs/crates/resilience/*` and top-level `docs/TASKS.md` reference consistent current status and valid links.

- [ ] RSL-N010 Prepare Phase 9 roadmap package
  - Goal: convert this backlog into phased milestones with owners, risks, and exit criteria.
  - Done when: a formal roadmap/plan set exists and is linked from `docs/ROADMAP.md` crate index.

## Suggested Execution Order

1. RSL-N001
2. RSL-N003
3. RSL-N002
4. RSL-N004
5. RSL-N005
6. RSL-N007
7. RSL-N006
8. RSL-N008
9. RSL-N009
10. RSL-N010

## Validation Commands

```bash
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo bench -p nebula-resilience
```
