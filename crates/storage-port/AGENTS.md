# nebula-storage-port — Agent orientation
> Local guide for `crates/storage-port/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Pure storage contract — object-safe `#[async_trait]` repository traits, port-local DTO rows, plain-data `Scope`, `StorageError`, the exact plan/flavor catalog roles, and the `TransitionBatch` atomic unit-of-work. No backend code.
**Layer:** Core — the only product dependency is `nebula-core`; no SQL driver or higher-layer domain dependency.

## Key files

- `src/lib.rs` — crate root; re-exports `Scope`, `StorageError`, `FencingToken`, `TransitionBatch{,Builder,Outcome}`
- `src/batch.rs` — `TransitionBatch`: private fields, builder-only construction; `commit` writes state+outbox+journal in one CAS+fencing-gated transaction
- `src/store/mod.rs` — ISP-segregated object-safe role traits, including `CredentialPersistence`
- `src/dto/` — private-field lifecycle DTOs, typed `CredentialSelector`, bounded `CredentialVersion`, structural live/tombstoned records, and opaque exact plan/flavor records
- `src/scope.rs` — plain-data `Scope { workspace_id, org_id }`; `src/ids.rs` — re-exported core ULIDs + lease `FencingToken`

## Conventions & never-do

- This crate declares *what* storage does; **never implement a backend here** (adapters live in `nebula-storage`). `nebula-tenancy` enforces policy for the general Scope-taking stores; credential persistence is owner-bound directly and intentionally has no tenancy decorator.
- DTOs never depend on higher-tier domain types. Opaque payloads use `serde_json::Value`/bytes, and credential rows/selectors remain port-local so this crate never imports `nebula-credential`.
- `Scope` is a value type with **no policy**; resolving it from a principal and general cross-tenant denial belong to `nebula-tenancy`, not here. `CredentialOwner`/`CredentialSelector` are also data, not actor authority; their public technical constructors must not be exposed through HTTP or `nebula-sdk`.
- Credential persistence exposes only explicit `create`, version-fenced `replace`, and version-fenced `tombstone`; never restore generic overwrite or physical delete. Its refresh-retry gate and material epoch are structural aggregate state (never metadata or claim TTL). The backend authors epochs: create/migration starts at `CredentialMaterialEpoch::MIN`; `CredentialMaterialTransition::Preserve { refresh_retry }` retains the epoch and applies the explicit gate transition; `Advance` increments the epoch and unconditionally clears the gate; overflow fails closed. Admission is evaluated against the backend clock.
- Every repository trait stays `#[async_trait]` + `dyn`-compatible (consumed as `Arc<dyn …>`); keep `TransitionBatch` fields private and builder-only so a transition can't skip scope/CAS/fencing.
- A port change must include its adapters, applicable tenancy decorators, and consumers. Object-safety tests prove the contract compiles; storage conformance proves its backend behavior. Neither substitutes for the other.
- `PlanFlavorCatalog` loads only an exact typed pair; `PlanFlavorCatalogWriter` inserts only; `PlanFlavorCatalogAdmin` owns drain/delete only. Never merge installer and destructive lifecycle authority, and never add public retain/release/reference mutation: execution-owned references must compose inside their owning backend transaction.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Object-safe trait changes | [object_safe](tests/object_safe.rs), [credential_persistence_object_safe](tests/credential_persistence_object_safe.rs); then storage conformance for implementations. |
| Shared-resource runtime DTOs and roles | [resource_subscription_dto](tests/resource_subscription_dto.rs), [object_safe](tests/object_safe.rs), then `nebula-storage` resource fanout conformance. |
| DTO/batch/catalog contracts | [dto](tests/dto.rs), [batch](tests/batch.rs), [revision_catalog_api_contract](tests/revision_catalog_api_contract.rs). |
| Credential lifecycle perimeter | [credential_lifecycle_api_contract](tests/credential_lifecycle_api_contract.rs), [credential_lifecycle_surface_guard](tests/credential_lifecycle_surface_guard.rs), [credential_secret_debug_contract](tests/credential_secret_debug_contract.rs). |

## See also

- `README.md` — full design · ADR-0072 (port/adapter/tenancy contract) · ADR-0041 (RefreshClaimStore shape)
