# nebula-runtime v2 — Design Spec

## Goal

Complete the runtime implementation: activate sandbox routing, implement SpillToBlob, integrate with action v2 registry and handler adapters. Additive changes — no redesign needed.

## Philosophy

- **Thin orchestration layer.** Runtime sits between engine (scheduling) and action (execution). No domain logic.
- **Additive, not breaking.** Current API is correct. New features are additions.
- **Sandbox when ready.** Isolation routing activates once ActionMetadata carries isolation_level.

## Current State

1,156 LOC, well-tested. Working: registry lookup, execution with telemetry, per-node data limits (10MB default). Not working: sandbox routing (TODO on line 90), SpillToBlob (logs warning only), max_total_execution_bytes (defined but unenforced).

---

## 1. Sandbox Routing (Phase 1 — blocked on action v2)

Currently all actions bypass sandbox. Fix when `ActionMetadata.isolation_level` lands:

```rust
async fn execute_action(&self, key: &str, input: Value, ctx: ActionContext)
    -> Result<ActionResult<Value>, RuntimeError>
{
    let handler = self.registry.get(key)
        .ok_or(RuntimeError::ActionNotFound { key: key.into() })?;

    let metadata = handler.metadata();

    let result = match metadata.isolation_level {
        IsolationLevel::None => {
            handler.execute(input, &ctx).await
        }
        IsolationLevel::CapabilityGated | IsolationLevel::Isolated => {
            let sandboxed_ctx = SandboxedContext::new(ctx, metadata.capabilities());
            self.sandbox.execute(handler.as_ref(), input, &sandboxed_ctx).await
        }
    };

    self.enforce_data_limit(&result?)?;
    Ok(result)
}
```

`SandboxedContext` wraps ActionContext with capability checks — undeclared resource/credential access returns `ActionError::SandboxViolation`.

---

## 2. SpillToBlob (Phase 1)

Replace warning-only with actual blob storage:

```rust
/// Trait for external blob storage (S3, local filesystem, etc.)
pub trait BlobStorage: Send + Sync {
    async fn write(&self, data: &[u8], content_type: &str) -> Result<BlobRef, RuntimeError>;
    async fn read(&self, blob_ref: &BlobRef) -> Result<Vec<u8>, RuntimeError>;
}

/// Reference to externally stored data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRef {
    pub uri: String,
    pub size_bytes: u64,
    pub content_type: String,
}
```

In `enforce_data_limit()`:
```rust
fn enforce_data_limit(&self, result: &ActionResult<Value>) -> Result<(), RuntimeError> {
    let size = estimate_size(result);
    if size <= self.policy.max_node_output_bytes {
        return Ok(());
    }

    match self.policy.strategy {
        LargeDataStrategy::Reject => Err(RuntimeError::DataLimitExceeded { size, limit }),
        LargeDataStrategy::SpillToBlob => {
            if let Some(blob) = &self.blob_storage {
                let blob_ref = blob.write(&serialize(result)?, "application/json").await?;
                // Replace inline data with BlobRef in result
                Ok(())
            } else {
                Err(RuntimeError::DataLimitExceeded { size, limit })
            }
        }
    }
}
```

---

## 3. Action v2 Registry Integration

Current `ActionRegistry` already has `register_stateless::<A>()`. For action v2:

```rust
impl ActionRegistry {
    /// Register any Action + StatelessAction with auto-adapter.
    pub fn register_action<A>(&self, action: A) -> Result<(), RuntimeError>
    where
        A: Action + StatelessAction + Send + Sync + 'static,
    {
        let key = action.metadata().key.clone();
        let version = action.metadata().version;
        let handler = Arc::new(StatelessAdapter::new(action));
        self.handlers.insert(VersionedActionKey(key, version), handler);
        Ok(())
    }

    /// Version-aware lookup (from action v2 spec).
    pub fn get_versioned(&self, key: &ActionKey, version: &InterfaceVersion)
        -> Option<Arc<dyn InternalHandler>>;

    pub fn get_latest(&self, key: &ActionKey)
        -> Option<Arc<dyn InternalHandler>>;
}
```

---

## 4. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Sandbox | Bypassed (TODO) | Routes by IsolationLevel |
| SpillToBlob | Warning only | BlobStorage trait + actual spill |
| Registry | Basic DashMap | Version-aware lookup |
| Max total bytes | Defined, unenforced | Engine tracks via execution budget |
| BlobRef | Not exists | New type for external data references |

---

## 5. Not In Scope

- Trigger lifecycle (Phase 2 — needs engine trigger scheduler)
- Multi-runtime coordination (Phase 3)
- Health monitor / graceful shutdown (Phase 4)
- WASM / container sandbox (Phase 3 per ADR 008)
- Max total execution bytes enforcement (engine concern, not runtime)

---

## Post-Conference Round 2 Amendments

### RT1. Enforce max_total_execution_bytes (Meta)
Engine accumulates output bytes across nodes via ExecutionBudget. Promote RTM-T007 to v1 blocker.

### RT2. BlobStorage::write_stream with AsyncRead (Instagram)
Large binary payloads stream to blob storage without full memory buffering.

### RT3. Deserialization recursion limit (Notion)
Default depth limit 128 on all serde_json deserialization at runtime boundary.

### RT4. Correlation ID in ActionContext (Datadog)
`ActionContext` injects `execution_id` into the tracing span so all log lines within a node execution carry it. All cross-layer operations (credential resolution, resource acquisition, action execution) inherit this span — enabling full request-scoped correlation.

### RT5. Metric cardinality limit (Datadog)
Metric labels sourced from user input (node names, workflow names) are sanitized and bounded. `MetricsRegistry` rejects labels with cardinality >10,000 unique values per metric. Labels from system constants (action_key, resource_key, credential_key) are bounded by registry size.

### RT6. Latency target (Grafana)
Stated performance contract: 3-node workflow with 1KB payloads, no external I/O calls: **< 1ms end-to-end** (DAG resolution + scheduling + action dispatch + serialize). Benchmark this as CI gate.

---

## Serialization Strategy

See `2026-04-06-serialization-strategy-design.md` for cross-cutting serialization decisions affecting this crate.
