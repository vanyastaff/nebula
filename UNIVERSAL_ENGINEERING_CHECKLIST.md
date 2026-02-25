# Universal Engineering Checklist (for any project)

## 0) Scope and intent

- [ ] Problem statement is clear and testable.
- [ ] Non-goals are explicitly documented.
- [ ] API/UX impact is described.
- [ ] Backward-compatibility expectations are stated.

---

## 1) Domain invariants (business correctness)

- [ ] Core invariants are explicitly listed (what must always be true).
- [ ] Critical multi-step updates are atomic (all-or-nothing).
- [ ] No partial writes on failure paths.
- [ ] Data model consistency is preserved across all operations.
- [ ] Replace/update semantics are explicit (replace vs merge vs reject).
- [ ] Duplicate inputs are handled deterministically (idempotent or rejected).
- [ ] Cycles/illegal references are prevented where applicable.
- [ ] Error categories reflect real causes (validation vs runtime vs internal).
- [ ] Retryability semantics are explicit and correct.

---

## 2) Concurrency and async protocols

- [ ] Ownership is clear for all long-lived components/tasks.
- [ ] Start/stop/shutdown lifecycle is defined for each background process.
- [ ] Cancellation is wired through all long-running operations.
- [ ] No lock is held while waiting on external/slow operations.
- [ ] Resource acquisition and release are balanced (no leaks).
- [ ] Backpressure strategy is explicit (queue, reject, timeout, shed load).
- [ ] Timeouts are defined for network/IO/locks and surfaced in errors.
- [ ] Concurrent updates cannot corrupt shared state.
- [ ] Race-prone paths are tested (register/update/shutdown, etc).
- [ ] System behavior under partial failure is deterministic.

---

## 3) State machines and transitions

- [ ] States are explicit (documented enum/table/diagram).
- [ ] Allowed transitions are explicit and enforced.
- [ ] Invalid transitions fail fast with clear errors.
- [ ] Terminal states and recovery rules are explicit.
- [ ] State transitions are observable (logs/events/metrics).
- [ ] Side effects on transition are idempotent or guarded.

---

## 4) Data integrity and persistence

- [ ] Schema/versioning strategy exists (if persistence involved).
- [ ] Migrations are reversible or safely roll-forward.
- [ ] Serialization/deserialization is validated.
- [ ] Corrupted/partial data is handled safely.
- [ ] Read-after-write consistency expectations are documented.
- [ ] Clock/time assumptions are explicit (UTC, monotonic timers, etc).

---

## 5) API and contract quality

- [ ] Public API behavior is stable and documented.
- [ ] Input validation is complete and early.
- [ ] Error responses are structured and actionable.
- [ ] Idempotency expectations are documented for write operations.
- [ ] Pagination/filter/sorting semantics are deterministic.
- [ ] Breaking changes are versioned and communicated.

---

## 6) Security and privacy

- [ ] Threat model considered for this change.
- [ ] Authentication and authorization checks are correct and complete.
- [ ] Least-privilege principle applied.
- [ ] Secrets are never hardcoded or logged.
- [ ] Sensitive data is encrypted in transit and at rest (where needed).
- [ ] PII handling complies with policy/regulatory requirements.
- [ ] Audit trail exists for privileged operations.
- [ ] Dependency vulnerabilities checked and triaged.

---

## 7) Reliability and resilience

- [ ] Retries use bounded policy with backoff + jitter.
- [ ] Circuit breaker / fail-open vs fail-closed strategy is explicit.
- [ ] Graceful degradation path is defined.
- [ ] Startup, warmup, and readiness are well-defined.
- [ ] Shutdown is graceful and bounded by timeout.
- [ ] Single point of failure identified and mitigated.
- [ ] Disaster recovery assumptions documented.

---

## 8) Observability and operability

- [ ] Structured logs include correlation/request IDs.
- [ ] Metrics cover throughput, latency, errors, saturation.
- [ ] Traces cover critical cross-service paths.
- [ ] Health checks reflect real readiness/liveness.
- [ ] Alert thresholds and SLO implications are considered.
- [ ] Runbook/update notes provided for operators.

---

## 9) Performance and capacity

- [ ] Baseline performance measured (before/after where relevant).
- [ ] Latency and throughput targets are documented.
- [ ] Memory and CPU impact evaluated.
- [ ] N+1 / unbounded loops / unbounded buffers avoided.
- [ ] Hot paths profiled or reasoned with evidence.
- [ ] Capacity limits and scaling bottlenecks identified.

---

## 10) Testing strategy

- [ ] Unit tests cover happy path + edge cases.
- [ ] Integration tests cover real component boundaries.
- [ ] Concurrency/race tests for shared-state paths.
- [ ] Property/fuzz tests for parser/validator/state logic (where useful).
- [ ] Failure-injection tests (timeouts, network errors, partial failures).
- [ ] Regression test added for each bug fixed.
- [ ] Deterministic test behavior (no flaky timing assumptions).

---

## 11) Code quality and maintainability

- [ ] Naming is clear and intention-revealing.
- [ ] Modules have single responsibility.
- [ ] Complex logic has comments for invariants and reasoning.
- [ ] Dead code / stale TODOs removed or tracked.
- [ ] Lint/format rules pass.
- [ ] Dependency additions are justified.
- [ ] Public docs/examples updated with changes.

---

## 12) Delivery and change management

- [ ] Rollout plan defined (feature flag/canary/phased rollout if needed).
- [ ] Rollback plan exists and is tested/thought through.
- [ ] Migration plan for existing users/data exists.
- [ ] Monitoring plan during rollout is defined.
- [ ] Stakeholders informed of behavioral changes.
- [ ] Post-release verification checklist prepared.

---

## 13) Minimal PR gate (fast universal gate)

- [ ] Build passes.
- [ ] Tests pass.
- [ ] Lint/format pass.
- [ ] No known critical security issue introduced.
- [ ] Invariants list updated if behavior changed.
- [ ] One new regression test for non-trivial fixes.