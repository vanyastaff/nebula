# nebula-cluster Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

In a distributed Nebula deployment, multiple nodes (API, workers) form a cluster. Membership, scheduling, and failover must be coordinated so that workflow tasks are not lost and execution is consistent. A dedicated cluster crate owns membership, consensus-backed control-plane state, and distributed scheduling contracts.

**nebula-cluster is the planned distributed execution and coordination layer.**

It answers: *How do nodes discover each other, how is control-plane state (e.g. task ownership, membership) kept consistent, and how does scheduling and failover work across the fleet?*

```
Nodes join cluster → membership protocol
    ↓
Control-plane state (task assignment, membership) via consensus or leader
    ↓
Scheduling and failover: deterministic decisions; safe transitions
```

Contract: deterministic control plane; safe membership transitions; observable behavior. Crate is planned; not yet implemented.

---

## User Stories

### Story 1 — Node Joins and Leaves Safely (P1)

A worker or API node joins the cluster and is visible to scheduler. On shutdown, it leaves and hands off owned tasks or state. No split-brain; membership view converges.

**Acceptance**: Membership protocol with join/leave; safe transition; handoff or reassignment of owned work.

### Story 2 — Distributed Scheduling and Failover (P1)

Scheduler assigns tasks to nodes. If a node fails, tasks are reassigned. Decisions are deterministic and observable so that operators can reason about behavior.

**Acceptance**: Scheduling contract (who runs what); failover semantics (at-most-once, at-least-once, exactly-once) documented; idempotency where needed.

### Story 3 — Consensus-Backed State (P2)

Control-plane state (e.g. task ownership, config) is replicated via consensus (Raft, etc.) or leader so that restarts and failures do not lose critical state.

**Acceptance**: State machine or key-value with consensus; documented consistency guarantees.

---

## Core Principles

### I. Cluster Does Not Own Workflow or Execution Logic

**Cluster owns membership and coordination. Engine and execution own workflow and execution state model.**

**Rationale**: Separation of concerns. Cluster is infrastructure; engine is domain.

### II. Deterministic Scheduling and Failover

**Scheduling and failover decisions are deterministic for the same inputs. Observable for ops.**

**Rationale**: Reproducibility and debugging.

### III. Safe Membership Transitions

**Join and leave do not cause split-brain or task loss. Handoff and reassignment are defined.**

**Rationale**: Rolling deploys and scaling require safe transitions.

---

## Production Vision

Fleet of nodes; membership protocol; consensus-backed control plane; distributed scheduler and failover. Observable metrics and events. From archives: consensus, scheduling, failover, autoscaling. Gaps: implement crate; define wire and trait contracts with runtime/worker/storage.

### Key gaps

| Gap | Priority |
|-----|----------|
| Implement crates/cluster | Critical |
| Consensus backend and state machine | High |
| Scheduling and failover contract with worker/runtime | High |
| Membership and handoff protocol | High |

---

## Non-Negotiables

1. **Cluster owns membership and coordination** — not workflow/execution logic.
2. **Deterministic control plane** — same view ⇒ same decisions.
3. **Safe membership transitions** — no split-brain; task handoff defined.
4. **Breaking consensus/scheduling semantics = major + MIGRATION.md**.

---

## Governance

- **MINOR**: Additive scheduling/metrics APIs.
- **MAJOR**: Consensus/state-machine or scheduling semantics change; MIGRATION.md required.
