# Cross-Spec Consistency Fixes

> Results of full audit across all 15 spec documents.

---

## RED — Implementation Blockers

### R1. SandboxRunner trait not object-safe
**Problem:** `fn execute(...) -> impl Future<...> + Send` is not object-safe. Can't use `Box<dyn SandboxRunner>`.
**Fix:** Change to `async_trait` or explicit `Pin<Box<dyn Future<...> + Send>>`:
```rust
pub trait SandboxRunner: Send + Sync {
    fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + '_>>;
    
    fn isolation_level(&self) -> IsolationLevel;
}
```
Same fix applies to `BlobStorage` trait in runtime-v2.

### R2. rkyv incompatible with serde_json::Value
**Problem:** `ExecutionState` and `StoredCredential` contain `serde_json::Value` fields. `Value` does NOT implement rkyv traits. Zero-copy claim is false for these types.
**Fix:** Scope rkyv to metadata-only types (no Value fields):
- Credential cache: rkyv for `CredentialMetadata` (key, kind, version, timestamps). Decrypt+deserialize for actual secret data.
- Execution cache: rkyv for `ExecutionMetadata` (id, status, node statuses). `Value`-containing outputs stay as `rmp-serde`.
- Update serialization spec Section 2.4 to clarify this limitation.
- Adjust performance claim from "0.1us pointer cast for all data" to "0.1us for metadata envelope, ~5us for full state with Value deserialization".

### R3. nebula-plugin needs nebula-action dependency
**Problem:** `ActionDescriptor.factory` returns `Arc<dyn InternalHandler>` from nebula-action, but nebula-plugin doesn't depend on nebula-action.
**Fix:** Add `nebula-action` to plugin's Cargo.toml. Both are Business layer — valid per architecture. Document the transitive dependency fan-out (plugin → action → credential, resource, parameter).

---

## YELLOW — Inconsistencies

### Y1. OwnerId type — inline amendment W1 into struct definition
All references to `owner_id` across workflow-v2, api-v1, and business-governance must use `OwnerId` newtype (from nebula-core), never `Option<String>`.

### Y2. Credential access naming — standardize to `credential()`
`ActionContext::credential::<S>(key)` is the public API. Internally delegates to `CredentialAccessor::resolve_typed()`. Document in action-v2 Section 3.

### Y3. ActionDescriptor contains ActionMetadata
```rust
pub struct ActionDescriptor {
    pub metadata: ActionMetadata,  // contains key, name, version, parameters, dependencies
    pub factory: Box<dyn Fn() -> Arc<dyn InternalHandler> + Send + Sync>,
}
```
Plugin registration uses `descriptor.metadata` for the registry. No duplicate fields.

### Y4. BlobStorage — two methods, one streaming abstraction
```rust
pub trait BlobStorage: Send + Sync {
    fn write(&self, data: &[u8], content_type: &str) 
        -> Pin<Box<dyn Future<Output = Result<BlobRef, RuntimeError>> + Send + '_>>;
    fn write_stream(&self, chunks: Pin<Box<dyn Stream<Item = Bytes> + Send>>, content_type: &str)
        -> Pin<Box<dyn Future<Output = Result<BlobRef, RuntimeError>> + Send + '_>>;
    fn read(&self, blob_ref: &BlobRef)
        -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RuntimeError>> + Send + '_>>;
}
```

### Y5. Engine uses NodeOutput from serialization spec
Add cross-reference: engine-v1 Section 1 states "Node outputs stored as `NodeOutput` per serialization-strategy-design.md Section 2.1. `ExecutionRepo::save_node_output` accepts `&Value` (serialized from `NodeOutput.as_value()`)."

### Y6. expression-v1-design.md — MUST BE WRITTEN
v1 blocker. Contents: grammar, built-in functions, memory budget (RT-1), value redaction in errors (RT-2), step limit (already implemented), inline caching (breakthrough #1), batch evaluation (breakthrough #6).

### Y7. Engine refreshes by CredentialId, not CredentialKey
Engine resolves `CredentialId` from node configuration (workflow node params reference a specific credential instance), then calls `credential_resolver.refresh(credential_id)`.

### Y8. ActionMetadata gains dependencies field
```rust
pub struct ActionMetadata {
    // ... existing fields ...
    pub dependencies: ActionDependencies,  // NEW — from #[derive(Action)] attributes
}
```
Sandbox reads `metadata.dependencies.credential_keys()` — now valid.

### Y9. Plugin manifest gains isolation field
```toml
[plugin]
# ... existing fields ...
isolation = "capability_gated"  # none | capability_gated | isolated
```

### Y10. simd-json performance claim corrected
When deserializing into `serde_json::Value`, simd-json gives ~1.3-1.5x improvement (not 2-4x). The 2-4x claim applies only to `simd_json::OwnedValue`. Update serialization spec Section 2.2 and performance budget table.

---

## Missing Spec — Expression v1 (v1 BLOCKER)

Needed: `2026-04-06-expression-v1-design.md` covering:
1. Formal grammar for `{{ }}` template expressions
2. Supported operators and precedence
3. Built-in functions catalog (string, math, array, object, datetime)
4. Variable resolution ($node, $input, $execution, $workflow, $now, $today)
5. Memory budget per evaluation (RT-1)
6. Value redaction in error messages (RT-2)
7. Step limit (already implemented)
8. Inline caching design (breakthrough #1)
9. Batch evaluation API (breakthrough #6)
10. Security sandbox (function allowlist/denylist, no filesystem/network access)
