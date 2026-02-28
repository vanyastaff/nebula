# Decisions

## D001: Generic key-value Storage trait

**Status:** Adopt

**Context:** Need abstraction over different backends. Archive had domain-specific StorageBackend (save_workflow, load_execution). Generic key-value is more reusable.

**Decision:** `Storage<Key, Value>` trait with get, set, delete, exists. Consumers use `Storage<WorkflowId, Workflow>` or `MemoryStorageTyped<Workflow>`. Key and value types chosen by consumer.

**Alternatives considered:**
- Domain-specific trait — couples storage to workflow/execution; rejected
- Single Key=String, Value=Vec<u8> — too low-level; MemoryStorageTyped adds typed layer

**Trade-offs:** Generic; flexible. Domain layers build adapters (key format, serialization).

**Consequences:** No built-in workflow/execution methods; consumers implement repos.

**Migration impact:** None; current design.

**Validation plan:** Consumer integration when execution/workflow use storage.

---

## D002: MemoryStorage + MemoryStorageTyped

**Status:** Adopt

**Context:** Need in-memory backend for dev/test. Support both raw bytes and typed (JSON) values.

**Decision:** MemoryStorage for String → Vec<u8>; MemoryStorageTyped<T> wraps MemoryStorage, uses serde_json for T. Two types; clear separation.

**Alternatives considered:**
- Single MemoryStorage with generic Value — would need Serialize/Deserialize bounds on all; binary use case awkward
- Only MemoryStorageTyped — raw bytes need base64 or similar; extra indirection

**Trade-offs:** Two types; MemoryStorageTyped depends on MemoryStorage. Covers binary and JSON.

**Consequences:** All typed storage uses JSON; binary storage uses Vec<u8>.

**Migration impact:** None.

**Validation plan:** Unit tests for both.

---

## D003: StorageError with NotFound, Serialization, Backend

**Status:** Adopt

**Context:** Need error taxonomy. get returns Option; when is NotFound used?

**Decision:** StorageError::NotFound for operations that require key to exist (future: strict get, update). get returns Ok(None) when key absent. Serialization for serde errors. Backend for connection/timeout/other.

**Alternatives considered:**
- get returns Err(NotFound) when absent — breaks Option semantics; get is "optional"
- Single Backend variant — loses discrimination for serialization

**Trade-offs:** NotFound may be underused currently; get uses Option.

**Consequences:** Consumers use Ok(None) for missing key; Err(NotFound) for strict ops.

**Migration impact:** None.

**Validation plan:** Error handling tests.

---

## D004: Optional postgres, redis, s3 features

**Status:** Adopt

**Context:** Not all deployments need all backends. Minimize default deps.

**Decision:** Default feature set is empty (memory only). postgres, redis, s3 are optional features. Cargo.toml has sqlx, redis, aws-sdk-s3 as optional.

**Alternatives considered:**
- All backends default — bloats deps for simple use
- Separate crates per backend — more crates; current approach is simpler

**Trade-offs:** Backends not implemented yet; features are placeholders.

**Consequences:** Implementations to be added under features.

**Migration impact:** None.

**Validation plan:** Feature flags work; backends compile when enabled.

---

## D005: No list/scan in initial trait

**Status:** Adopt

**Context:** Key-value stores often support list/prefix scan. Needed for "list all workflows".

**Decision:** Initial Storage trait has get, set, delete, exists only. List/prefix scan deferred to Phase 2. May add `list_prefix` or separate `ListableStorage` trait.

**Alternatives considered:**
- Add list to trait now — not all backends support efficiently; S3 list is different from Redis SCAN
- Domain-specific list_workflows — would require domain in storage; rejected

**Trade-offs:** Consumers cannot list keys with current trait; workaround: maintain index elsewhere.

**Consequences:** Phase 2 adds list support.

**Migration impact:** Additive when added.

**Validation plan:** Design list API; implement for Memory, Postgres, Redis, S3.
