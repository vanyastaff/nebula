# Roadmap

## Phase 1: Isolation and Spill (Current Focus)

**Deliverables:**
- Restore isolation level logic: resolve_isolation_level from ActionMetadata
- Route IsolationLevel::CapabilityGated/Isolated through SandboxRunner
- Implement SpillToBlob: blob storage abstraction, reference in ActionResult
- Enforce max_total_execution_bytes (engine or runtime)

**Risks:**
- ActionMetadata may not have isolation/capabilities yet
- Blob storage adds dependency

**Exit criteria:**
- Trusted actions run directly; isolated actions run via sandbox
- SpillToBlob writes to blob, returns ref; consumer fetches on demand
- max_total_execution_bytes enforced

---

## Phase 2: Trigger Lifecycle (Optional)

**Deliverables:**
- Trigger lifecycle orchestration: activate, deactivate, listen
- Trigger types (webhook, schedule, Kafka) live in **nebula-action**; runtime executes them like any action
- Integration with engine for event-driven workflow start
- See _archive/from-archive/nebula-complete-docs-part3/nebula-runtime.md for legacy target design

**Risks:**
- Coordination between engine, runtime, action (triggers)
- Event bus integration

**Exit criteria:**
- Webhook trigger (in action) activates workflow on HTTP POST
- Schedule trigger fires at cron times
- Triggers deactivate on workflow deactivation

---

## Phase 3: Coordination (Multi-Runtime)

**Deliverables:**
- WorkflowCoordinator for workflow-to-runtime assignment
- RuntimeRegistry for discovery
- Load balancing, failover (see archive)
- Single-node default; multi-node optional

**Risks:**
- Distributed systems complexity
- May be out of scope for MVP

**Exit criteria:**
- Multiple runtime instances; coordinator assigns workflows
- Runtime failure triggers reassignment

---

## Phase 4: Health and Observability

**Deliverables:**
- HealthMonitor for runtime components
- Graceful shutdown
- Runtime metrics (beyond action metrics): queue depth, active executions

**Risks:**
- Overlap with telemetry

**Exit criteria:**
- /health endpoint or equivalent
- Clean shutdown of in-flight actions

---

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| **Correctness** | All tests pass; engine integration stable |
| **Latency** | execute_action overhead < 1ms |
| **Throughput** | Scale with engine concurrency |
| **Stability** | No panics; errors propagated |
| **Operability** | Telemetry events; metrics |
