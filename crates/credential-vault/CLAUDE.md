# nebula-credential-vault — Claude Code orientation
> Agent quick-map for `crates/credential-vault/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** First-party HashiCorp Vault backend implementing `nebula-credential`'s `ExternalProvider`/`LeasedProvider` traits — KV v2 static reads + dynamic secrets with lease renew/revoke (ADR-0081).
**Layer:** Business (credential backend) — depends only on `nebula-credential` (shared infra) downward; composition roots wire it in as `Arc<dyn ExternalProvider>`.

## Commands
- `cargo check -p nebula-credential-vault`
- `cargo nextest run -p nebula-credential-vault`  ·  doctests: `cargo test -p nebula-credential-vault --doc`
- Integration tests use `wiremock` (mock Vault HTTP) + `nebula-storage` cache layer: see `tests/cache_integration.rs`.

## Key files
- `src/lib.rs` — crate docs (path-prefix convention), module wiring, re-exports `VaultConfig`/`VaultError`/`VaultProvider`.
- `src/provider.rs` — `VaultProvider` (the `ExternalProvider`/`LeasedProvider` impl), `VaultConfig`, construction-time `VaultError`, HTTP routing + error classification.
- `src/wire.rs` — serde envelopes for Vault responses (`KvV2Envelope`, `DynamicSecretEnvelope`, `LeaseRenewEnvelope`).

## Conventions & never-do
- **Path routing is explicit, never sniffed:** bare `path` → KV v2 (`from_secret`, no lease); `dyn/<rest>` prefix → dynamic read (`with_lease`). Do NOT add "try KV then fall back" heuristics — operators control routing via the stored reference.
- **Error mapping is contractual (ADR-0051 fall-through):** only HTTP 404 → `ProviderError::NotFound` (lets `ExternalProviderChain` fall through); 403→`AccessDenied`, 5xx/transport→`Unavailable`, other 4xx/decode→`Backend` — all short-circuit. Don't widen what maps to `NotFound`.
- **Never leak the auth token:** `Debug` on `VaultProvider` omits the bearer token (address + KV mount only); the token rides the `X-Vault-Token` header. Keep it out of logs/spans/Debug.
- Every issued `LeaseHandle` is attributed to `PROVIDER_NAME = "vault"` (must match `provider_name` so `handles_lease` routing works).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, path-convention table, error-classification table, cache-layer composition example.
- `docs/adr/0081-m6-resource-credential-integration.md` — provider trait surface + fall-through rationale.
- `crates/credential/src/provider/` — the trait definitions this crate implements.
- `crates/storage/src/credential/provider_cache.rs` — `ProviderCacheLayer` this backend composes with.
