# API

## Public Surface

- stable APIs:
  - planned `ClusterManager`, `ClusterNode`, `SchedulingStrategy`, `ClusterEvent` contracts
  - planned control-plane commands (join/leave/rebalance/failover)
- experimental APIs:
  - autoscaling and advanced affinity/placement policy extensions
- hidden/internal APIs:
  - consensus internals and replication protocol details

## Usage Patterns

- runtime requests placement for workflow execution.
- cluster manager tracks membership and selects target node.
- failover path reschedules affected workflows when nodes fail.

## Minimal Example

```rust
// planned API sketch
let execution = cluster.execute_workflow(workflow_id, input).await?;
```

## Advanced Example

```rust
// planned API sketch
cluster.update_strategy(SchedulingStrategy::ConsistentHash).await?;
cluster.rebalance().await?;
cluster.handle_node_failure(node_id).await?;
```

## Error Semantics

- retryable errors:
  - transient network/consensus/storage communication failures.
- fatal errors:
  - invalid cluster state, unsupported topology transitions, unrecoverable data inconsistency.
- validation errors:
  - invalid node metadata, strategy config, or unsafe cluster operation request.

## Compatibility Rules

- what changes require major version bump:
  - scheduling semantics and placement guarantees
  - consensus/state ownership model
- deprecation policy:
  - at least one minor release compatibility window for non-critical API transitions
