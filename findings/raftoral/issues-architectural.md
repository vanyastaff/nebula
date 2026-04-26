# Issues — raftoral

## Issue count

GitHub reports: 0 open issues, 0 closed issues.
Repository created 2025-09-22, last updated 2025-12-29.
Stars: 6. Forks: 0.

The project has no tracked issues or discussions. This is consistent with a solo pre-release project (v0.2.0, single maintainer, no community yet).

## Cited issues / pain points — from docs and roadmap instead

Because there are zero GitHub issues, the following pain points are sourced from the project's own `docs/FEATURE_PLAN.md` and README "Current Limitations" section.

### LP-1 — In-memory workflow state lost on restart
Source: `docs/FEATURE_PLAN.md` §5 ("Persistent Checkpoint History — Current State: All in-memory").
The checkpoint queues and history live in `WorkflowStateMachine::checkpoint_history: HashMap<String, VecDeque<Vec<u8>>>` (`src/workflow/state_machine.rs`). A node crash drops all in-flight checkpoint data not yet captured in a Raft snapshot.

### LP-2 — No built-in compensation / rollback
Source: README "Current Limitations" — "No built-in compensation/rollback (implement in workflow logic)".
Users must implement saga-style undo inside workflow closures; there is no framework equivalent to Temporal's `workflow.compensate()` or Nebula's planned rollback hook.

### LP-3 — Rust-only; requires deterministic execution
Source: README. Workflow functions must be identical on all nodes and produce the same operation sequence from the same input. Non-deterministic code (randomness, system time reads outside checkpoints) silently corrupts distributed state.

### LP-4 — Single execution cluster load balancing not yet implemented
Source: `src/full_node/workflow_service.rs` line ~55: `// For now, use the default execution cluster (cluster_id=1) // Future: Implement load balancing across multiple execution clusters`. All workflows go to cluster 1.

### LP-5 — No workflow termination API yet
Source: `docs/FEATURE_PLAN.md` §1.2 — `WorkflowCommand::WorkflowTerminate` described as planned for v0.2.0. Checked: `src/workflow/state_machine.rs` — `WorkflowCommand` enum has `WorkflowStart`, `WorkflowEnd`, `SetCheckpoint`, `OwnerChange` but no `Terminate` variant.
