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
up a real backend (Vault, Postgres, an OAuth IdP) depend on this crate
in their `dev-dependencies`.

The crate was extracted from `nebula-credential` during the M12.2
stabilize sweep (2026-05-20) so the contract crate itself stays free
of `test-util` gated code paths. See `docs/MATURITY.md` for the
extraction rationale.

## Public surface

The full crate surface fits in a single import block:

```rust
use nebula_credential_testutil::{
    InMemoryStore,        // implements nebula_credential::store::CredentialStore
    InMemoryPendingStore, // implements nebula_credential::store::PendingCredentialStore
    in_memory_pair,       // -> (InMemoryStore, InMemoryPendingStore)
};
```

That is the full export list as of v0.1.0
(`crates/credential-testutil/src/lib.rs`). Both stores are simple
`Mutex`-backed `HashMap` implementations sufficient for unit and
integration tests of credential consumers; they intentionally do not
emulate persistence, encryption, or concurrent eviction semantics.

## Layer

Sits in the **Business** layer alongside `nebula-credential` itself;
the `deny.toml` `[bans].deny[].wrappers` allowlist locks the exact
consumer set. It is **test-only** (`publish = false`) — production
crates must depend on `nebula-credential` directly, never on this
helper.

Current consumers (dev-deps only):

- `nebula-credential-runtime` — facade tests under the `test-util`
  feature, plus its own integration suite.
- `nebula-tenancy` — scope-enforcement tests over the in-memory
  stores.

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
- `crates/credential-runtime/` — runtime facade; primary consumer.
- `crates/tenancy/` — scope-enforcement decorator; secondary consumer.
- ADR-0081 — M6 resource/credential integration (absorbs the earlier
  ADR-0042–0045, 0051, 0066–0067 cascade per `docs/adr/README.md`).
- `docs/MATURITY.md` — extraction record (2026-05-20).
