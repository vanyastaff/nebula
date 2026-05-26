---
name: nebula-credential-testutil
role: Test shims for downstream credential consumers
status: stable
last-reviewed: 2026-05-26
related: [nebula-credential, nebula-credential-runtime]
---

# nebula-credential-testutil

## Purpose

`nebula-credential-testutil` ships the **test fixtures, mock adapters,
and assertion helpers** that downstream crates use to exercise the
`nebula-credential` contract surface without standing up a full
credential runtime.

Extracted from `nebula-credential` during the M12.2 stabilize sweep
(2026-05-20) so the contract crate itself stays free of `test-util`
gated code paths. See `docs/MATURITY.md` for the extraction rationale
and `nebula-credential` README for the parent contract.

## What's inside

- **In-memory `CredentialStore` mock** — round-trips encrypted credential
  rows without persistence; covers create / read / update / delete +
  rotation semantics for adapter tests.
- **Snapshot fixtures** — pre-built `StoredCredential` rows that mirror
  the canonical OAuth2 / API-key / mTLS / Basic shapes the workspace
  ships, ready to feed into resolver / dispatch tests.
- **Assertion helpers** — `assert_no_plaintext_drop` and friends that
  verify the zeroization invariants on `Drop` paths without manually
  reaching into `SecretBox` internals.
- **`scheme` builders** — minimal `AuthScheme` factories so resource /
  action tests can construct credential bindings without depending on
  `nebula-credential-builtin`.

## Layer

This crate sits in the **Business** layer alongside `nebula-credential`
itself; the `deny.toml` `[bans].deny[].wrappers` allowlist locks the
exact consumer set. It is **test-only** (`publish = false`) — production
crates must depend on `nebula-credential` directly, never on this
helper.

## Out of scope

- Production credential storage (see `nebula-storage` adapter + the
  `nebula-credential` contract).
- First-party credential type catalog (see `nebula-credential-builtin`).
- Runtime facade / dispatch coordination (see `nebula-credential-runtime`,
  ADR-0066).

## Related

- `crates/credential/` — contract crate this helper supports.
- `crates/credential-runtime/` — runtime facade (ADR-0066) that
  consumes this in its own tests.
- `crates/credential-vault/` — Vault-backed backend; uses these
  fixtures in its integration tests.
- `docs/MATURITY.md` — extraction record (2026-05-20).
