## Archived Ideas (from `docs/archive/architecture-v2.md`)

### Specialized execution traits draft

`architecture-v2` proposed specialized traits on top of base `Action`:

- `StreamingAction` for stream output execution
- `BatchAction` for vectorized execution
- `StatefulAction` with explicit save/load state hooks

```rust
#[async_trait]
pub trait StreamingAction: Action {
    type Item: Send + Sync;

    async fn execute_stream(
        &self,
        input: Self::Input,
        context: &mut ExecutionContext,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Item, Self::Error>> + Send>>, Self::Error>;
}
```

### Extended `ActionResult` variants draft

Historical list included advanced orchestration variants:

- `Suspend { state, resume_condition }`
- `Fork(Vec<ForkBranch<T>>)`
- `Join { wait_for, merge_strategy }`
- `Delegate { workflow_id, input, wait }`
- `Error { error, recovery }`

These are not treated as current contract, but kept as backlog ideas for engine-level flow
control extensions.

