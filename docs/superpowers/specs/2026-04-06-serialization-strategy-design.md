# Nebula Serialization Strategy — Cross-Cutting Spec

## Goal

Define where each serialization technique is used across the Nebula stack. Replace blanket `serde_json::Value` with purpose-specific formats: `RawValue` for lazy parsing, `Bytes` for zero-copy sharing, `rkyv` for zero-copy persistence, `rmp-serde` for compact storage, `simd-json` for hot parse paths.

## Philosophy

- **External boundaries = JSON.** User-facing APIs, webhooks, workflow definitions in/out — always `serde_json::Value` or JSON strings. Human-readable, debuggable.
- **Internal = fastest format for the job.** Node-to-node, persistence, cache, IPC — use specialized formats. Never exposed to users.
- **Lazy by default.** Don't parse what you don't need. `RawValue` defers parsing until a node actually reads a field.
- **Zero-copy where schema is known.** `rkyv` for execution state, credential cache, DAG snapshots — types we control.

---

## 1. Data Flow Architecture

```
Webhook/API request
    ↓ simd_json::from_slice() — fast parse at boundary
    ↓
Box<RawValue> — unparsed JSON, valid but not tree-structured
    ↓ engine passes to node
    ↓
Node execution:
    ↓ serde_json::from_str::<MyInput>(raw.get()) — parse only into typed struct
    ↓ node does work
    ↓
ActionResult<Value> — output as serde_json::Value
    ↓ engine serializes to RawValue for next node
    ↓
Arc<RawValue> — shared between downstream nodes (fan-out, zero-copy)
    ↓
Next node deserializes only what it needs
    ...
    ↓
Final output → serde_json::Value → API response / storage
```

---

## 2. Per-Layer Strategy

### 2.1 Node-to-Node Data Passing

**Current:** `serde_json::Value` cloned per downstream consumer.

**New:** `Arc<RawValue>` — serialized once from node output, shared by reference.

```rust
/// What the engine stores per node output
pub struct NodeOutput {
    /// Raw JSON bytes — not parsed into a tree.
    /// Shared across all downstream consumers via Arc.
    pub raw: Arc<RawValue>,
    /// Lazy-initialized parsed Value (for expression evaluation).
    parsed: OnceLock<Value>,
}

impl NodeOutput {
    /// Create from an action's output Value.
    pub fn from_value(value: &Value) -> Self {
        let json_string = serde_json::to_string(value).expect("Value is always serializable");
        Self {
            raw: Arc::from(RawValue::from_string(json_string).expect("valid JSON")),
            parsed: OnceLock::new(),
        }
    }

    /// Get as RawValue for pass-through or selective parsing.
    pub fn as_raw(&self) -> &RawValue {
        &self.raw
    }

    /// Get as parsed Value (lazy — parsed on first access).
    pub fn as_value(&self) -> &Value {
        self.parsed.get_or_init(|| {
            serde_json::from_str(self.raw.get()).expect("RawValue is always valid JSON")
        })
    }

    /// Deserialize directly into a typed struct (skip Value intermediate).
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(self.raw.get())
    }
}
```

**Impact:** 3-node chain on 10KB payload: current = 3 full parses + 2 deep clones. New = 1 serialize + 0 parses (if pass-through) or 1 partial parse (if node reads fields).

**Where:** `nebula-engine` node output storage. `nebula-expression` evaluation context.

### 2.2 Webhook / API Ingest

**Current:** `serde_json::from_slice(&body)`

**New:** `simd_json::from_slice(&mut body)` on hot paths (webhook receiver, API bulk endpoints).

```rust
// Feature-gated: simd-json on x86_64, fallback to serde_json on other archs
#[cfg(target_arch = "x86_64")]
fn parse_json(body: &mut [u8]) -> Result<Value, Error> {
    simd_json::from_slice(body).map_err(Into::into)
}

#[cfg(not(target_arch = "x86_64"))]
fn parse_json(body: &mut [u8]) -> Result<Value, Error> {
    serde_json::from_slice(body).map_err(Into::into)
}
```

**Impact:** 2-4x faster for payloads >1KB. Negligible for small payloads.

**Where:** `nebula-webhook` ingest, `nebula-api` bulk endpoints.

### 2.3 Execution State Persistence

**Current:** `serde_json` to Postgres JSONB.

**New:** Dual format:
- **Postgres:** `rmp-serde` (MessagePack) to `BYTEA` column — 30-50% smaller, 2x faster serialize/deserialize.
- **In-memory cache:** `rkyv` archive — zero-copy read on cache hit.

```rust
/// Execution state stored in Postgres
pub struct PersistedExecutionState {
    /// MessagePack-encoded execution state
    pub data: Vec<u8>,
    /// Format version for forward compatibility
    pub format: StorageFormat,
}

#[derive(Clone, Copy)]
pub enum StorageFormat {
    /// MessagePack via rmp-serde (default for new writes)
    MessagePack,
    /// Legacy JSON (read-only, for migration)
    Json,
}

impl PersistedExecutionState {
    pub fn serialize(state: &ExecutionState) -> Result<Self> {
        Ok(Self {
            data: rmp_serde::to_vec(state)?,
            format: StorageFormat::MessagePack,
        })
    }

    pub fn deserialize(&self) -> Result<ExecutionState> {
        match self.format {
            StorageFormat::MessagePack => Ok(rmp_serde::from_slice(&self.data)?),
            StorageFormat::Json => Ok(serde_json::from_slice(&self.data)?),
        }
    }
}
```

**Where:** `nebula-storage` PgExecutionRepo, `nebula-execution` state persistence.

### 2.4 Credential Cache

**Current:** `serde_json::Value` in moka LRU cache.

**New:** `rkyv` archived bytes — zero-copy read from cache.

```rust
/// Cached credential — rkyv-archived for zero-copy access
pub struct CachedCredential {
    /// rkyv-serialized bytes
    bytes: AlignedVec,
}

impl CachedCredential {
    pub fn archive(stored: &StoredCredential) -> Self {
        Self {
            bytes: rkyv::to_bytes::<rkyv::rancor::Error>(stored).expect("archivable"),
        }
    }

    /// Zero-copy access — no deserialization
    pub fn get(&self) -> &ArchivedStoredCredential {
        rkyv::access::<ArchivedStoredCredential, rkyv::rancor::Error>(&self.bytes).expect("valid archive")
    }
}
```

**Impact:** Cache hit = pointer cast, no parsing. For 50K Telegram bot tokens in cache — significant.

**Where:** `nebula-credential` CacheLayer.

### 2.5 Binary Data Fan-Out

**Current:** `Vec<u8>` cloned per downstream node.

**New:** `bytes::Bytes` — reference-counted, zero-copy clone.

```rust
/// Binary payload shared between nodes
pub struct BinaryPayload {
    pub data: Bytes,          // Arc-backed, clone = refcount bump
    pub content_type: String,
    pub size: u64,
}

impl Clone for BinaryPayload {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),  // just Arc::clone — no memcpy
            content_type: self.content_type.clone(),
            size: self.size,
        }
    }
}
```

**Where:** `nebula-action` BinaryData, `nebula-runtime` SpillToBlob, media processing chains.

### 2.6 Expression Evaluation Context

**Current:** `HashMap<Arc<str>, Arc<Value>>` — each node output fully parsed.

**New:** `HashMap<Arc<str>, Arc<RawValue>>` — lazy parse only referenced paths.

```rust
pub struct EvaluationContext {
    /// Node outputs as raw JSON — parsed only when $node.field is accessed
    nodes: HashMap<Arc<str>, Arc<RawValue>>,
    // ...
}

impl EvaluationContext {
    pub fn resolve_variable(&self, name: &str) -> Option<Value> {
        match name {
            "node" => {
                // Only now parse what's needed
                let mut obj = serde_json::Map::new();
                for (key, raw) in &self.nodes {
                    let value: Value = serde_json::from_str(raw.get()).ok()?;
                    obj.insert(key.to_string(), value);
                }
                Some(Value::Object(obj))
            }
            // ...
        }
    }
}
```

**Where:** `nebula-expression` context.rs.

---

## 3. Dependency Impact

| Crate | New Dependencies |
|-------|-----------------|
| nebula-engine | `bytes` (already in workspace) |
| nebula-expression | none (uses serde_json RawValue, already available) |
| nebula-storage | `rmp-serde` |
| nebula-credential | `rkyv` (cache layer only, feature-gated) |
| nebula-webhook | `simd-json` (feature-gated, x86_64 only) |
| nebula-api | `simd-json` (feature-gated) |
| nebula-action | `bytes` |
| nebula-runtime | `bytes` |

Feature gates:
```toml
[features]
default = []
simd-json = ["dep:simd-json"]    # opt-in, x86_64 only
rkyv-cache = ["dep:rkyv"]        # opt-in, for credential cache
msgpack-storage = ["dep:rmp-serde"]  # opt-in, for Postgres storage
```

---

## 4. Migration Path

| Phase | What | Breaking? |
|-------|------|-----------|
| 1 | `NodeOutput` with `Arc<RawValue>` in engine | No — internal to engine |
| 2 | Expression context uses `Arc<RawValue>` | No — internal |
| 3 | `Bytes` for binary payloads in action/runtime | No — BinaryData internal |
| 4 | `rmp-serde` for execution state storage | No — StorageFormat enum, reads both |
| 5 | `rkyv` for credential cache | No — feature-gated, CacheLayer internal |
| 6 | `simd-json` for webhook ingest | No — feature-gated, drop-in |

**Zero breaking changes.** All optimizations are internal to their respective crates. External API stays `serde_json::Value`.

---

## 5. Performance Budget

| Path | Current | Target | Technique |
|------|---------|--------|-----------|
| 10KB payload through 5-node chain | 5 full parses + 4 deep clones | 1 serialize + 0-5 partial parses + 0 clones | `Arc<RawValue>` |
| Webhook ingest (10KB) | ~15μs parse | ~4μs parse | `simd-json` |
| Execution checkpoint (50KB state) | ~30μs serialize, ~25μs deserialize | ~15μs ser, ~12μs deser | `rmp-serde` |
| Credential cache hit | ~5μs deserialize | ~0.1μs (pointer cast) | `rkyv` |
| Binary 1MB fan-out to 3 nodes | 3MB memcpy | 0 bytes copied | `Bytes` |

---

## 6. Integration with Existing Specs

### Parameter v4
No change. Parameters are small JSON schemas — `serde_json::Value` is fine. `ParameterValues` stays as-is.

### Credential v3
Add `rkyv` feature gate for `CacheLayer`. `StoredCredential` derives `rkyv::Archive` behind feature flag. Decrypted credential material never rkyv-archived (stays `SecretString` with zeroize).

### Resource v2
`AuthorizeCallback` receives typed `R::Auth` (already in spec). Internal rotation event payload can use `rmp-serde` for compact EventBus messages.

### Action v2
`ActionResult<Value>` stays as-is (actions produce `Value`). Engine wraps output in `NodeOutput(Arc<RawValue>)` after action returns. Actions never see `RawValue` — this is engine-internal.

`BinaryData` gains `Bytes` internally for zero-copy fan-out. Action authors still use `Vec<u8>` — conversion to `Bytes` happens in the adapter.

### Runtime v2
`BlobStorage::write_stream` (from Instagram feedback) naturally fits with `Bytes` chunks:
```rust
async fn write_stream(
    &self,
    chunks: impl Stream<Item = Bytes> + Send,
    content_type: &str,
) -> Result<BlobRef, RuntimeError>;
```

### Workflow v2
`WorkflowDefinition` can be `rkyv`-archived for fast loading from disk cache. JSON remains canonical format for storage/API. `rkyv` cache is a read optimization.

---

## 7. Not In Scope

- Replacing `serde_json::Value` in public API (stays forever — it IS the interface)
- `flatbuffers` / `capnp` (schema codegen overhead not justified)
- `memmap2` for workflow loading (Postgres is the primary store, not files)
- Custom binary format (MessagePack + rkyv cover all needs)
