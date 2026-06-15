# nebula-storage-port

The **storage port** for Nebula: object-safe repository traits, port-local
DTO rows, the plain-data `Scope` value type, `StorageError`, and the
`TransitionBatch` atomic unit-of-work.

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
- **Plain-data `Scope`.** `Scope { workspace_id, org_id }` is a value type
  with no policy. Resolving a `Scope` from a principal and enforcing
  cross-tenant denial is the job of `nebula-tenancy`.

## Layering

```text
engine / api / core  ──depends on──▶  nebula-storage-port  (this crate)
                                             ▲
nebula-storage (adapters: InMemory/SQLite/Postgres)  ──implements──┘
nebula-tenancy (scope-enforcing decorators)          ──wraps──┘
```

Only composition roots (api `AppState`, the knife test) wire the concrete
adapter and the tenancy decorator together.

## Contract pointer

The architectural contract this crate satisfies is recorded in ADR-0072
(storage spec-16 port / adapter / tenancy), kept in the maintainers' private design vault.
