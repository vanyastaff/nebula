---
name: nebula-credential-testutil
role: In-memory test doubles for credential storage
status: stable
last-reviewed: 2026-05-26
related: [nebula-credential, nebula-credential-runtime, nebula-tenancy]
---

# nebula-credential-testutil

## Purpose

`nebula-credential-testutil` ships **in-memory test doubles** for the
two credential-storage ports defined by `nebula-credential`. Downstream
crates that need to exercise the credential contract without standing
up a real backend (Vault, Postgres, an OAuth IdP) depend on this crate.

The crate was extracted from `nebula-credential` during the M12.2
stabilize sweep (2026-05-20) so the contract crate itself stays free
of `test-util` gated code paths. See `docs/MATURITY.md` for the
extraction rationale.

## Public surface

```rust
use nebula_credential_testutil::{
    InMemoryStore,        // impls nebula_credential::store::CredentialStore
    InMemoryPendingStore, // impls nebula_credential::pending_store::PendingStateStore
    in_memory_pair,       // -> (InMemoryStore, InMemoryPendingStore)
};

// The two backing modules are also re-exported publicly:
use nebula_credential_testutil::{pending_store_memory, store_memory};
```

That is the full public surface as of v0.1.0 (verify with
`grep '^pub' crates/credential-testutil/src/lib.rs`). Both stores
hold their data in `Arc<tokio::sync::RwLock<HashMap<...>>>`. Cloning
either store produces another handle to the same backing map
(cheap `Arc` clone), so tests can share a single store across multiple
task handles. All data is lost when the last clone drops.

Both shims are **behaviour-identical** to the production
`InMemoryStore` / `InMemoryPendingStore` in
`nebula_storage::credential::*`; they live here only so the
`nebula-credential` contract crate does not have to export
`#[cfg(test)]`-style code itself. Production composition roots should
import the `nebula-storage` variants.

## Layer

Sits in the **Business** layer alongside `nebula-credential` itself.
The `deny.toml` `[bans].deny[].wrappers` allowlist locks the exact
consumer set. The crate is **internal-only** (`publish = false`).

Real consumers (`rg credential-testutil crates/*/Cargo.toml`):

- **`nebula-credential-runtime`** — declared in two roles in its
  manifest: an optional normal dependency activated by the
  `test-util` feature **and** a `dev-dependency` for its own test
  suite.
- **`nebula-tenancy`** — `dev-dependency` for scope-enforcement tests
  over the in-memory stores.

## Out of scope

- Production credential storage (see `nebula-storage` adapter + the
  `nebula-credential` contract).
- First-party credential type catalog (see `nebula-credential-builtin`).
- Runtime facade / dispatch coordination (see
  `nebula-credential-runtime`).
- Snapshot fixtures, redaction assertions, scheme builders, or any
  other helper not exported by `lib.rs` today — those are *not* part
  of this crate.

## Related

- `crates/credential/` — contract crate this helper supports.
- `crates/credential-runtime/` — runtime facade; primary consumer
  (both via the `test-util` feature and dev-deps).
- `crates/tenancy/` — scope-enforcement decorator; dev-deps consumer.
- ADR-0081 — M6 resource/credential integration (absorbs the earlier
  ADR-0042–0045, 0051, 0066–0067 cascade per `docs/adr/README.md`).
- `docs/MATURITY.md` — extraction record (2026-05-20).
