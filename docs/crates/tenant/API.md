# API

## Public Surface

- stable APIs:
  - planned `TenantManager`, `TenantContext`, `TenantQuota`, `PartitionStrategy` contracts
  - planned middleware/context extraction interface
- experimental APIs:
  - advanced adaptive quota policies and tenant-level autoscaling hooks
- hidden/internal APIs:
  - internal accounting store and background reconciliation paths

## Usage Patterns

- API/gateway extracts tenant identity and attaches validated `TenantContext`.
- runtime checks quota and policy before scheduling/allocating tenant work.
- storage/resource layers resolve partitioning and limits from tenant policy.

## Minimal Example

```rust
// planned API sketch
let tenant_ctx = tenant_manager.resolve_context(request).await?;
tenant_ctx.check_quota(ResourceType::Execution).await?;
```

## Advanced Example

```rust
// planned API sketch
let policy = tenant_manager.policy_for(tenant_id).await?;
let partition = policy.partition_strategy();
let quota = policy.quota();

runtime.apply_tenant_policy(&tenant_ctx, &policy).await?;
storage.bind_partition(tenant_id, partition).await?;
resource.apply_quota(tenant_id, quota).await?;
```

## Error Semantics

- retryable errors:
  - transient policy backend unavailable, optimistic quota contention failures
- fatal errors:
  - unknown/disabled tenant, policy violation, partition mismatch
- validation errors:
  - malformed tenant identity, invalid quota configuration, incompatible strategy

## Compatibility Rules

- what changes require major version bump:
  - tenant context schema and enforcement semantics
  - quota decision semantics and isolation defaults
- deprecation policy:
  - adapter-based transition for at least one minor release where possible
