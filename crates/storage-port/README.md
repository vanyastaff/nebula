# nebula-storage-port

The **storage port** for Nebula: object-safe repository traits, port-local
DTO rows, the plain-data `Scope` value type, `StorageError`, and the
`TransitionBatch` atomic unit-of-work. It also owns the object-safe,
owner-bound `CredentialPersistence` technical contract and its port-local
owner/selector/row/error values, plus the technical exact plan/worker-flavor
catalog contract.

## What this crate is

A pure contract crate (Core tier). It declares *what* storage must do; it
does **not** implement any backend.

- **No sqlx.** No database driver, no migrations, no connection pool. Those
  live in the adapter crate `nebula-storage`.
- **Object-safe traits.** Every repository trait is `#[async_trait]` and
  `dyn`-compatible, because the engine/api consume storage as
  `Arc<dyn …>`. The per-call boxed-future allocation is noise next to the
  network/disk I/O every port call bottoms out in.
- **Port-local DTOs.** Row/record types are defined here and depend only on
  `serde_json::Value` — never on `ActionResult` or any higher-tier type
  (prevents a Core-tier dependency inversion).
- **Exact plan/flavor catalog.** `PlanFlavorCatalog` loads only a caller-pinned
  typed plan/flavor pair. `PlanFlavorCatalogWriter` owns immutable insertion
  only; `PlanFlavorCatalogAdmin` owns drain and guarded deletion. Runtime
  readers and contract installers receive no destructive lifecycle capability.
  The port stores non-empty opaque recorded-form bytes and never decodes
  plugin-owned descriptors.
- **Plain-data `Scope`.** `Scope { workspace_id, org_id }` is a value type
  with no policy. Resolving a `Scope` from a principal and enforcing
  cross-tenant denial for general Scope-taking stores is the job of
  `nebula-tenancy`. Credential persistence is the deliberate exception: every
  operation is directly bound to a mandatory `CredentialOwner` /
  `CredentialSelector`, while actor authorization happens above this port.
- **Technical, not branded.** These contracts are used directly by trusted
  workspace crates and first-party composition roots. They are not a supported
  integration surface; downstream authors depend on `nebula-sdk`.

## Layering

```text
engine / credential / api / apps ──depends on──▶ nebula-storage-port
                                                    ▲
nebula-storage (SQLite/Postgres + internal reference) ──implements──┘
nebula-tenancy (general Scope-taking decorators)      ──wraps───────┘
```

First-party deployment composition belongs under `apps/`. `nebula-api` is a
technical HTTP boundary, while tests may assemble reference adapters directly.
Credential persistence is not wrapped by tenancy; its selectors are mandatory
data but do not confer authority.

Revision-reference mutation is deliberately absent from the public catalog
roles. Live execution and rollback-window references belong to runtime
control and must be changed inside the same backend transaction as their
owning execution transition. Exposing standalone retain/release calls here
would make that atomicity optional. A Draining revision remains exactly
loadable by already-retained executions while rejecting new references;
guarded deletion is implemented by storage adapters from authoritative
reference rows, never mutable counters.

Credential writes are explicit `create`, version-fenced `replace`, and
version-fenced `tombstone` intents. The selector owns a typed global
`CredentialId`; terminal state is structural and cannot carry live-only data.
Generic overwrite and physical-delete operations are not part of the port.
Refresh-retry admission is also structural aggregate state: it is never stored
in user metadata or conflated with refresh-claim TTL. Replacements carry an
outer `CredentialMaterialTransition`: `Preserve { refresh_retry }` retains the
backend-owned material epoch while applying one explicit
preserve/clear/permanent/timed gate transition; `Advance` increments the epoch
and unconditionally clears the old gate. Backends initialize creates and
migrated rows at `CredentialMaterialEpoch::MIN`, reject epoch overflow
fail-closed, and evaluate timed gates against their authoritative clock.
`refresh_retry_snapshot` returns credential version, material epoch,
reauthentication state, and admission decision from one backend read; callers
must not reconstruct it from separate reads.
`Never` means “never retry this credential-material epoch”: an explicit
material replacement/reconnect or durable reauthentication decision uses
`Advance`, invalidates stale same-epoch finalizers, and clears the gate.
Reauthentication additionally blocks resolution through its own flag. The ban
is not global to the credential identity.

## Contract pointer

The architectural contract this crate satisfies is recorded in ADR-0072
(storage spec-16 port / adapter / tenancy), kept in the maintainers' private design vault.
