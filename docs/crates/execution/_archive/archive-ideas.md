## Archived Ideas (from `architecture-v2`)

### Execution models (design draft)

```rust
pub enum ExecutionModel {
    Sequential(SequentialExecution),
    Parallel(ParallelExecution),
    Streaming(StreamingExecution),
    Batch(BatchExecution),
    Reactive(ReactiveExecution),
    Distributed(DistributedExecution),
}
```

### Stateful execution manager (design draft)

```rust
pub struct ExecutionState {
    pub id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub status: ExecutionStatus,
    pub node_states: HashMap<NodeId, NodeState>,
    pub variables: Variables,
    pub checkpoints: Vec<Checkpoint>,
}
```

### Transaction and persistence ideas

- Transactional updates around state transitions (`begin/commit/rollback`).
- Versioned state persistence for resume/replay workflows.
- Checkpoint-based recovery for long-running and distributed runs.

These notes are historical and should be treated as backlog/design reference.

