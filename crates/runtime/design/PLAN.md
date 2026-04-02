# Implementation Plan: nebula-runtime

**Crate**: `nebula-runtime` | **Path**: `crates/runtime` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The runtime crate manages action execution isolation and routing — choosing whether an action runs trusted/direct or sandboxed, handling result spilling to blob storage for large outputs, and optionally orchestrating trigger lifecycle. Current focus is Phase 1: isolation level routing, SpillToBlob, and enforcing `max_total_execution_bytes`.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-action`, `nebula-core`, `nebula-execution`, `nebula-storage`, `nebula-system`
**Testing**: `cargo test -p nebula-runtime`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Isolation and Spill | 🔄 In Progress | restore_isolation_level, SandboxRunner routing, SpillToBlob |
| Phase 2: Trigger Lifecycle | ⬜ Planned | webhook/schedule triggers, engine event-driven start |
| Phase 3: Multi-Runtime Coordination | ⬜ Planned | WorkflowCoordinator, RuntimeRegistry, failover |
| Phase 4: Health and Observability | ⬜ Planned | HealthMonitor, graceful shutdown, queue/execution metrics |

## Phase Details

### Phase 1: Isolation and Spill

**Goal**: Route actions by isolation level; implement SpillToBlob for large results; enforce byte budget.

**Deliverables**:
- `resolve_isolation_level` from `ActionMetadata`
- Route `CapabilityGated`/`Isolated` through `SandboxRunner`
- `SpillToBlob`: blob storage abstraction + reference in `ActionResult`
- `max_total_execution_bytes` enforcement

**Exit Criteria**:
- Trusted actions run directly; isolated actions go through sandbox
- SpillToBlob writes to blob, returns ref; consumer fetches on demand
- `max_total_execution_bytes` enforced

**Risks**:
- `ActionMetadata` may not have isolation/capabilities fields yet
- Blob storage adds external dependency

**Dependencies**: `nebula-action` Phase 2 (context model), `nebula-storage`

### Phase 2: Trigger Lifecycle (Optional)

**Goal**: Trigger activation/deactivation lifecycle integrated with engine.

**Deliverables**:
- Trigger lifecycle: activate, deactivate, listen
- Trigger types (webhook, schedule, Kafka) live in `nebula-action`; runtime executes them
- Integration with engine for event-driven workflow start

**Exit Criteria**:
- Webhook trigger activates workflow on HTTP POST
- Schedule trigger fires at cron times
- Triggers deactivate on workflow deactivation

**Risks**:
- Coordination complexity between engine, runtime, action

### Phase 3: Multi-Runtime Coordination

**Goal**: Multiple runtime instances with workflow assignment, discovery, failover.

**Deliverables**:
- `WorkflowCoordinator` for workflow-to-runtime assignment
- `RuntimeRegistry` for discovery
- Load balancing and failover

**Exit Criteria**:
- Multiple runtime instances; coordinator assigns workflows
- Runtime failure triggers reassignment

**Risks**: Distributed systems complexity; may be out of scope for MVP

### Phase 4: Health and Observability

**Goal**: HealthMonitor, graceful shutdown, runtime-level metrics.

**Deliverables**:
- `HealthMonitor` for runtime components
- Graceful shutdown — drain in-flight actions
- Runtime metrics: queue depth, active executions

**Exit Criteria**:
- `/health` endpoint equivalent
- Clean shutdown of in-flight actions

## Inter-Crate Dependencies

- **Depends on**: `nebula-action` (action execution, trigger types), `nebula-execution`, `nebula-storage` (blob), `nebula-system` (pressure), `nebula-core`
- **Depended by**: `nebula-engine` (delegation), `nebula-api` (trigger management), `nebula-sdk`

## Verification

- [ ] `cargo check -p nebula-runtime`
- [ ] `cargo test -p nebula-runtime`
- [ ] `cargo clippy -p nebula-runtime -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-runtime`
