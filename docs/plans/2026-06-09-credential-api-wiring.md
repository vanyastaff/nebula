# Wire nebula-api → CredentialService (ADR-0088 D4 P4 + D7)

Status: in progress (P1 only approved; re-gate before P2+)
Branch: `refactor/credential-facade-dyn-erasure`
Owner decision (2026-06-09): Option **C** (P4 dyn-erasure folded into the wiring,
one branch, expand-contract green-per-commit). Execution gated **P1-first**:
land the non-generic facade as its own green milestone, then re-approve the
api/server wiring.

## Goal / end state (ADR-0088 D7 target)

One persistence path. `CredentialService` is **non-generic**; nebula-api routes
all credential ops through it; `apps/server` composes a real service; the
split-brain store / raw-fallback / api `classify()` dup / `CredentialScopeLayer`
are deleted.

Authoritative ADR: **0088** (proposed). 0066/0051/0052 do not exist as files
(superseded by 0081, amended by 0088). Reactive-only (0084). External Vault /
proactive refresh / M12.4 bind-population are out of scope.

## Why P4 (dyn-erasure) is a prerequisite

On main, `CredentialService<B: CredentialStore, PS: PendingStateStore>` is generic
and `AppState.credential_service` is hard-typed to
`CredentialService<InMemoryStore, InMemoryPendingStore>`. Wiring api/server against
that monomorphization re-churns every call site the day a durable backend lands.
ADR-0088 D4 makes the facade non-generic (`Arc<dyn>` collaborators erased at
construction) so the backend can be swapped without churn. P4 is that erasure,
**in place** in `nebula-credential-runtime` (the crate-merge half of D4 is
INFEASIBLE — Exec→Core is cargo-deny-forbidden — and is dropped).

---

# P1 — dyn-erasure → non-generic facade  (THIS milestone)

### Design (validated against the live code)

Object-safety blockers: `CredentialStore` (credential/src/store.rs:166) is RPITIT;
`PendingStateStore` (credential/src/pending_store.rs:23) is RPITIT **and** every
non-`delete` method is generic over `<P: PendingState>`. Erase via hand-rolled
boxed-future bridge traits (no `async_trait`, no `bon` — matches
`credential/src/provider/future.rs`; records the ADR-0088 D4 bon-deviation).

**Erase the whole LayeredStore at the top** (one box at the resolver→store
boundary; the Audit/Cache/Encryption layers stay statically composed and
monomorphic, so cache hits never reach the box). Fix `PS = ErasedPendingStore`
so `DispatchOps` and the engine resolver need **zero** changes.

### New types — `crates/credential/src/erased.rs` (new module, exported from lib.rs)

```rust
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── Credential store bridge (easy: no per-method generics) ──
pub trait DynCredentialStore: Send + Sync {
    fn get<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<StoredCredential, StoreError>>;
    fn put<'a>(&'a self, credential: StoredCredential, mode: PutMode)
        -> BoxFut<'a, Result<StoredCredential, StoreError>>;
    fn delete<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<(), StoreError>>;
    fn list<'a>(&'a self, state_kind: Option<&'a str>) -> BoxFut<'a, Result<Vec<String>, StoreError>>;
    fn exists<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<bool, StoreError>>;
}
impl<T: CredentialStore> DynCredentialStore for T { /* Box::pin(CredentialStore::*(self, ..)) */ }

#[derive(Clone)]
pub struct ErasedCredentialStore(Arc<dyn DynCredentialStore>);
impl ErasedCredentialStore { pub fn new(inner: Arc<dyn DynCredentialStore>) -> Self }
impl CredentialStore for ErasedCredentialStore { /* forward to self.0.* (RPITIT returns the BoxFut) */ }

// ── Pending store bridge (hard: erase <P: PendingState> to a byte core) ──
pub trait DynPendingStateStore: Send + Sync {
    fn put_serialized<'a>(&'a self, credential_kind: &'a str, owner_id: &'a str,
        session_id: &'a str, data: Vec<u8>, expires_in: Duration)
        -> BoxFut<'a, Result<PendingToken, PendingStoreError>>;
    fn get_serialized<'a>(&'a self, token: &'a PendingToken)
        -> BoxFut<'a, Result<Vec<u8>, PendingStoreError>>;
    fn get_bound_serialized<'a>(&'a self, credential_kind: &'a str, token: &'a PendingToken,
        owner_id: &'a str, session_id: &'a str) -> BoxFut<'a, Result<Vec<u8>, PendingStoreError>>;
    fn consume_serialized<'a>(&'a self, credential_kind: &'a str, token: &'a PendingToken,
        owner_id: &'a str, session_id: &'a str) -> BoxFut<'a, Result<Vec<u8>, PendingStoreError>>;
    fn delete<'a>(&'a self, token: &'a PendingToken) -> BoxFut<'a, Result<(), PendingStoreError>>;
}
// Typed surface for free: serialize on put, deserialize on get/consume/get_bound.
impl<T: DynPendingStateStore + ?Sized> PendingStateStore for T {
    async fn put<P: PendingState>(&self, k, o, s, pending: P) -> Result<PendingToken, _> {
        let data = serde_json::to_vec(&pending).map_err(|e| Backend(Box::new(e)))?;
        let ttl  = pending.expires_in();
        self.put_serialized(k, o, s, data, ttl).await
    }
    async fn get<P: PendingState>(&self, token) -> Result<P, _> {
        let data = self.get_serialized(token).await?;
        serde_json::from_slice(&data).map_err(|e| Backend(Box::new(e)))
    }
    // get_bound / consume: same byte→typed deserialize; delete forwards.
}

#[derive(Clone)]
pub struct ErasedPendingStore(Arc<dyn DynPendingStateStore>);
impl ErasedPendingStore { pub fn new(inner: Arc<dyn DynPendingStateStore>) -> Self }
impl PendingStateStore for ErasedPendingStore { /* forward put<P>/get<P>/... to (*self.0) */ }
```

Coherence: after this change **no type impls `PendingStateStore` directly** — the
two `InMemoryPendingStore` copies impl `DynPendingStateStore`, and the blanket
gives them `PendingStateStore`. `ErasedPendingStore` forwards to the blanket on
`dyn DynPendingStateStore`. Add an object-safety probe test (mirror
`storage-port/tests/object_safe.rs`) asserting `Arc<dyn DynCredentialStore>` and
`Arc<dyn DynPendingStateStore>` construct.

### Per-file edits

| File | Change |
|------|--------|
| `crates/credential/src/erased.rs` | NEW — the four types above + probe tests |
| `crates/credential/src/lib.rs` | `pub mod erased;` + re-export `DynCredentialStore, DynPendingStateStore, ErasedCredentialStore, ErasedPendingStore` |
| `crates/storage/src/credential/pending.rs` | `impl PendingStateStore for InMemoryPendingStore` → `impl DynPendingStateStore` (byte core: `put_serialized` stores `data`+`Utc::now()+expires_in`; `get/consume/get_bound_serialized` return `Vec<u8>`, keep TTL eviction + 4-D binding checks; `delete` unchanged). Existing tests use the typed API → keep passing via blanket |
| `crates/credential-testutil/src/pending_store_memory.rs` | same inversion (impl `DynPendingStateStore`) |
| `crates/credential-runtime/src/service.rs` | `CredentialService<B,PS>` → non-generic `CredentialService`; fields `store: Arc<dyn DynCredentialStore>`, `resolver: CredentialResolver<ErasedCredentialStore>`, `pending: ErasedPendingStore`, `ops: Arc<DispatchOps<ErasedPendingStore>>`; `__from_parts` non-generic; `credential_store_handle(&self) -> Arc<dyn DynCredentialStore>`; method bodies unchanged (calls resolve via `DynCredentialStore`/blanket) |
| `crates/credential-runtime/src/builder.rs` | `CredentialServiceBuilder<B>` (drop PS); `pending_store: ErasedPendingStore`, `ops: Arc<DispatchOps<ErasedPendingStore>>`; `build()` → `Result<CredentialService, _>`, erases `Arc::new(layered) as Arc<dyn DynCredentialStore>`, `CredentialResolver::new(ErasedCredentialStore::new(store.clone()))` |
| `crates/credential-runtime/src/service.rs` test_support (~1270-1500) + tests (~1822) | wrap pending in `ErasedPendingStore`, build `DispatchOps<ErasedPendingStore>` via `register_all_builtin_ops::<ErasedPendingStore>`, return `CredentialService`; `type Svc = CredentialService` |
| `crates/api/src/state.rs` | field+setter+doc (lines ~256-259, ~1050): `Option<Arc<CredentialService>>` (drop `<InMemoryStore, InMemoryPendingStore>`). Keep the `nebula_storage::credential::{InMemoryStore, InMemoryPendingStore}` import (still used by `oauth_*_store`) |
| compile_fail `.stderr` (credential + credential-runtime) | regen **warm**, plain `cargo test`; NEVER `TRYBUILD=overwrite` |

`ops.rs` and `nebula-engine` need **no changes** (DispatchOps stays generic,
fixed to `ErasedPendingStore` at the service; resolver stays generic over the
store, fixed to `ErasedCredentialStore`). `deny.toml` unchanged (no new edges).

### Green gate (per crate, bare commands — Windows: no `cd &&`/pipe chains)

`cargo check -p` then `clippy -p … -- -D warnings` then `nextest run -p` for, in order:
`nebula-credential` → `nebula-storage` → `nebula-credential-testutil` →
`nebula-credential-runtime` (`--features test-util`) → `nebula-api`.
Doctests on `nebula-credential` / `nebula-credential-runtime`. Warm + plain
`cargo test` for trybuild. No `unwrap/expect/panic` in lib code.

### Risks
- **Pending byte round-trip**: serialize/deserialize must be exactly the
  pre-existing serde_json path; property-test typed↔byte equivalence.
- **Boxed-future lifetimes**: tie `&self` and borrowed args to one `'a` in the
  Dyn trait (`fn get<'a>(&'a self, id: &'a str) -> BoxFut<'a, …>`).
- **Hot-path box**: `resolve_for_slot` p99 ≤ 1ms — one box at the store top;
  verify in P6 bench, erasure depth is reversible.
- **Green-per-commit** under cross-crate erasure: commit at each per-crate-green.

---

# P2–P6 (deferred; re-gate after P1)

- **P2 server composes service**: deps + deny wrappers; build `CredentialService`
  in `run_transport` (`EnvKeyProvider::from_env` fail-closed, real/doc'd
  `AuditSink`, `EventMetricObserver`, `register_builtins` +
  `register_all_builtin_ops::<ErasedPendingStore>`, real shutdown
  `CancellationToken` → ctrl_c + axum graceful); `with_credential_service`.
  Note: prod `InMemoryPendingStore`/`InMemoryStore` live in
  `nebula_storage::credential` (not testutil).
- **P3 api rewire CRUD/lifecycle/discovery**: route `transport/credential.rs`
  through `state.credential_service`; map `CredentialServiceError`→`ApiError`
  (RFC 9457); honest-503 stubs become real; delete api `classify()` dup.
- **P4 OAuth migration**: two-phase raw-bytes write → facade interactive
  acquisition (`resolve`→`Pending`→`continue_resolve`); audit `owner_id=None`
  admin-bypass (facade `TenantScope` has no None).
- **P5 delete CredentialScopeLayer**: service mandatory, drop fallback,
  `git rm tenancy/src/credential_scope.rs`, **transfer its ~20 tenant-isolation
  tests** to facade coverage first; update deny.toml/Cargo.toml.
- **P6 observability DoD + docs**: spans/metrics/invariants on live paths;
  criterion p99 gate; ADR-0088 status + deviations.
