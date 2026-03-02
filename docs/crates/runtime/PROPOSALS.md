# Proposals

## P001: Triggers in nebula-action (Rejected: nebula-trigger)

**Type:** N/A

**Motivation:** Archive described TriggerManager as separate subsystem. Considered extracting to nebula-trigger.

**Decision:** Triggers are action types; they live in `nebula-action`. No separate nebula-trigger crate. Runtime executes trigger actions like any other action; engine/API coordinates trigger lifecycle (activate/deactivate).

**Status:** Rejected — triggers in nebula-action

---

## P002: BlobStorage trait for SpillToBlob

**Type:** Non-breaking

**Motivation:** SpillToBlob needs somewhere to write large outputs. Should be pluggable (local fs, S3, etc.).

**Proposal:** Define `BlobStorage` trait in ports or runtime:

```rust
pub trait BlobStorage: Send + Sync {
    async fn write(&self, data: &[u8]) -> Result<BlobRef, Error>;
    async fn read(&self, ref: &BlobRef) -> Result<Vec<u8>, Error>;
}
```

DataPassingPolicy or runtime receives optional `Arc<dyn BlobStorage>`. When SpillToBlob and storage present, overflow writes to blob, returns BlobRef in output.

**Expected benefits:** Pluggable backends; no hard dependency on S3/fs.

**Costs:** New trait; integration with ActionResult output type.

**Risks:** BlobRef format; TTL/cleanup.

**Compatibility impact:** Additive.

**Status:** Draft

---

## P003: Resource injection into NodeContext

**Type:** Breaking (potential)

**Motivation:** Actions may need database connections, HTTP clients. Resource manager provides pooled instances.

**Proposal:** Context (currently NodeContext; target ActionContext per CONSTITUTION P-001) or execute_action receives optional `ResourceProvider`. Runtime passes to context; actions call `ctx.get_resource::<DbPool>()`. Engine already has resource_manager; could pass to runtime. Prefer aligning with ActionContext/TriggerContext migration so resource injection uses the same context type.

**Expected benefits:** Actions get typed resources; pooling handled by resource crate.

**Costs:** Context API change; engine-runtime contract.

**Risks:** May already be partially supported; check current context type.

**Compatibility impact:** Additive if optional.

**Status:** Defer (check current state)
