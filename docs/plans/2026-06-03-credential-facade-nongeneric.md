# Credential facade — non-generic redesign (ADR-0088 D4)

**Branch:** `refactor/credential-facade-nongeneric` (off `origin/main` @ `ae2ec1de`)
**Goal:** make `CredentialService<B, PS>` a non-generic struct holding type-erased
collaborators, so `nebula_api::AppState.credential_service` drops the
`<InMemoryStore, InMemoryPendingStore>` leak and a durable backend can be wired
later without generic churn. Implements ADR-0088 **D4** (facade redesign; the
crate-merge half of D4 is already proven infeasible — runtime stays an Exec crate).

## Surface (verified against `ae2ec1de`)

`CredentialService<B, PS>` (`credential-runtime/src/service.rs:142`) threads the
two params through only 4 of 8 fields:

| field | generic carrier | erase to |
|-------|-----------------|----------|
| `store: Arc<LayeredStore<B>>` | `B` (decorator stack `AuditLayer<CacheLayer<EncryptionLayer<B>>>`) | `Arc<dyn DynCredentialStore>` |
| `resolver: CredentialResolver<LayeredStore<B>>` | `B` (engine, generic + generic method `resolve<C>`) | resolver over the erased store |
| `ops: Arc<DispatchOps<B, PS>>` | `B` **phantom**, `PS` real | `Arc<DispatchOps<PS-erased>>` |
| `pending: PS` | `PS` real | `Arc<dyn DynPendingStateStore>` |

Already erased: `registry: Arc<CredentialRegistry>`, `observer: Arc<dyn CredentialObserver>`,
`lease: LeaseLifecycle`, `source: StateSource`.

Object-safety (verified): `CredentialStore` (`credential/src/store.rs:166`) is
RPITIT ⇒ not dyn-safe. `PendingStateStore` (`credential/src/pending_store.rs:23`)
is RPITIT **and** every method is generic over `<P: PendingState>` ⇒ doubly not
dyn-safe (hardest bridge — `P` must be erased to a byte/`Value` surface). All other
collaborators (`KeyProvider`/`AuditSink`/`ScopeResolver`/`CredentialObserver`/
`ExternalProvider`/`RefreshClaimStore`) are already dyn-safe.

Sole non-test consumer: `api/src/state.rs:259` (field) + `:1050` (`with_credential_service`,
**zero callers**). All other construction is `credential-runtime` `test_support`
(`service.rs:1274-1483`) + tests. So the call-site blast radius is tiny; the cost
is the internal dyn-erasure.

## Decisions

- **bon builder: skipped.** Not a workspace dep, zero prior art, new proc-macro on
  a Core/Exec crate (deny-wrappers + MSRV-1.95 verification risk). The existing
  hand-rolled `CredentialServiceBuilder::new(...)` already makes mandatory-collaborator
  omission a compile error (positional by-value args) — the same guarantee bon's
  typestate gives. Deviation from ADR-0088 D4's literal "bon-typestate builder";
  recorded here + in the PR (ADRs are revisable when the letter adds risk for no moat).
- **Store bridge: manual `Pin<Box<dyn Future + Send>>` (ProviderFuture-style),** not
  `async_trait`, to match the workspace's existing zero-alloc bridge
  (`credential/src/provider/future.rs`) and the no-shim ethos. One bridge trait
  `DynCredentialStore` in `nebula-credential`, blanket-impl'd `impl<T: CredentialStore>
  DynCredentialStore for T`, erased once at the `LayeredStore` boundary.
- **Pending-store bridge:** erase `<P: PendingState>` to a serialized surface
  (`&[u8]`/`Value`) at the `DynPendingStateStore` boundary; the typed generic methods
  become a blanket extension over the dyn core.
- **Registry∩ops gate (P2): `build()` returns `Result`** (consistent with the
  registry's own `DuplicateKey` Result), not a panic. Subset scoped to the four
  ops-modeled capabilities (refresh/test/revoke/interactive); DYNAMIC excluded.

## Sequence (each commit whole-workspace-green: clippy -D warnings + nextest + rustdoc)

- **P1 — drop `DispatchOps` phantom `B`.** `DispatchOps<B,PS>` → `DispatchOps<PS>`
  (`_backend: PhantomData<fn()->B>` removed); cascade `register_*_ops<C,B,PS>` →
  `<C,PS>`, the facade/builder field types, the `test_support` turbofish, lib re-exports.
  Pure simplification, contained to `credential-runtime`.
- **P2 — registry∩ops build-gate.** Add a per-key capability accessor to `DispatchOps`
  (`capabilities_of(key) -> Capabilities` from which `Option` closures are `Some`);
  `build()` asserts `registry.capabilities ⊆ ops-keys` per registered key, returns
  `Result`. Update the ~6 `test_support`/wiring call sites.
- **P3 — delete dead `CredentialStoreHandle::Layered` arm.** Minimal: drop the enum
  arm + 4 dead match arms + `list_owned` helper + the orphaned `LayeredStore` import in
  `api/transport/credential.rs`; leave `Scoped` as the sole path. Do NOT remove the
  `credential_service` field/setter (openspec `oauth-providers-from-operator-secrets`
  design keeps it for a future plane). Update module prose.
- **P4 — dyn-erasure (the substance, may be multi-commit).**
  1. `DynCredentialStore` bridge trait + blanket impl + object-safety probe (mirror
     `storage-port/tests/object_safe.rs`). nebula-credential.
  2. `DynPendingStateStore` bridge (erase `P`). nebula-credential (+ the second copy
     in `storage/src/credential/pending.rs`).
  3. `LayeredStore` decorators compose over the erased store; `CredentialResolver`
     runs over the erased store (engine).
  4. `CredentialService` + builder go non-generic; `api/state.rs` field/setter drop
     the params; regenerate the `compile_fail` `.stderr` (warm, never `TRYBUILD=overwrite`).

## Gates per crate (run from the worktree, absolute toolchain on PATH)
`cargo check -p nebula-credential-runtime --features test-util` ·
`cargo nextest run -p nebula-credential-runtime --features test-util` ·
`cargo check -p nebula-credential -p nebula-storage -p nebula-engine -p nebula-api` ·
`RUSTDOCFLAGS=-D warnings cargo doc -p <touched> --no-deps` · then pre-push (full clippy + crate-diff nextest).

## Out of scope (later ADR-0088 steps)
Step 5 (engine trim: public forced-refresh, delete deprecated String-id L1) ·
Step 6 (API thin-edge: OAuth2→credential, split-brain delete) · Step 7 (dead SQL
row-model) · Step 8 (delete old `Credential` + 5 sub-traits). Bind-population (D6/M12.4)
is a separate track.
