## Archived Ideas (from `docs/archive/architecture-v2.md`)

### Testing framework concepts

Historical draft described a richer testing surface:

- `NodeTestHarness` with resource mocking helpers
- `WorkflowTestHarness` with node-level execution expectations
- Integration environment bootstrapping (Postgres/Redis/engine)

```rust
#[tokio::test]
async fn test_node_execution() {
    let mut harness = NodeTestHarness::new();
    // setup mocks, execute node, assert outputs
}
```

These notes align with current SDK direction but are retained as ideas, not guaranteed APIs.

