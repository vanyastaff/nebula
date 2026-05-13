# nebula-credential-vault

HashiCorp Vault backend for Nebula's `ExternalProvider` / `LeasedProvider`
trait surface (ADR-0051). First concrete `LeasedProvider` implementation —
companion to the `ProviderCacheLayer` (in `nebula-storage`) and the
`ExternalProviderChain` (in `nebula-credential`).

Plugin authors and downstream code depend on the contract crate
(`nebula-credential`). This crate ships the first-party Vault backend that
composition roots wire in via `Arc<dyn ExternalProvider>`.

## What it does

- **KV v2 reads** — `GET /v1/{mount}/data/{path}?version=N`. Returns a
  `ProviderResolution::from_secret` with no lease metadata.
- **Dynamic secrets** — `GET /v1/{mount}/creds/{role}` and any equivalent
  Vault dynamic-secret endpoint (`database/`, `aws/`, `gcp/`, …). Returns a
  `ProviderResolution::with_lease` carrying a `LeaseHandle` attributed to
  `"vault"`.
- **Lease lifecycle** — `LeasedProvider::renew` (`PUT /v1/sys/leases/renew`)
  and `LeasedProvider::revoke` (`PUT /v1/sys/leases/revoke`). Surfaced to
  composed providers (chain / cache layer) via the
  `ExternalProvider::lease_renewal` capability discovery hook.

## Path convention

The `ExternalReference::path` field is interpreted by prefix:

| `path` shape                 | Backend route                       | Resolution            |
|------------------------------|-------------------------------------|-----------------------|
| `<secret-path>`              | `GET /v1/{kv_mount}/data/{path}`    | `from_secret`         |
| `dyn/<mount>/creds/<role>`   | `GET /v1/<mount>/creds/<role>`      | `with_lease`          |
| `dyn/<anything>`             | `GET /v1/<anything>`                | `with_lease`          |

`ExternalReference::version` is honoured for KV v2 reads (added as
`?version=N`). `ExternalReference::field` is a top-level key lookup against
the response data — if set, only that field's value is returned; otherwise
the entire data map is JSON-encoded (BTreeMap → sorted keys, so the output
is deterministic).

The `dyn/` prefix is deliberately explicit: no path-shape sniffing, no
"try KV first then fall back". Operators control routing through the
reference value stored in their credential record.

## Configuration

```rust
use nebula_credential::SecretString;
use nebula_credential_vault::{VaultConfig, VaultProvider};
use url::Url;

let config = VaultConfig::new(
    Url::parse("https://vault.example.com:8200/")?,
    SecretString::new(std::env::var("VAULT_TOKEN")?),
);
let provider = VaultProvider::new(config)?;
```

Defaults: `kv_mount = "secret"` (Vault's `vault kv` CLI default),
`request_timeout = 10s`. Override via the public `VaultConfig` fields.
`VaultProvider::with_client` lets you reuse a caller-managed
`reqwest::Client` — useful when the composition root pins a shared
connection pool or CA bundle.

## Composing with the cache layer

`nebula-storage::credential::ProviderCacheLayer` wraps any
`Arc<dyn ExternalProvider>` and honours per-entry TTLs. Dynamic
resolutions carry a Vault-supplied `lease_duration`, so they cache
automatically; KV v2 reads need either an explicit `default_ttl` in the
config or an inner provider that supplies one.

```rust
use std::{sync::Arc, time::Duration};
use nebula_credential::provider::ExternalProvider;
use nebula_credential_vault::{VaultConfig, VaultProvider};
use nebula_storage::credential::{ProviderCacheConfig, ProviderCacheLayer};

let provider = VaultProvider::new(VaultConfig::new(addr, token))?;
let cached = ProviderCacheLayer::new(
    Arc::new(provider) as Arc<dyn ExternalProvider>,
    ProviderCacheConfig {
        max_entries: 1_000,
        default_ttl: Duration::from_secs(60),
    },
);
```

`cache.lease_renewal()` returns a `LeasedProvider` view that invalidates
cached entries holding the matching lease id before forwarding revoke /
renew to Vault. See `tests/cache_integration.rs` for the round-trip
verification.

## Error classification

Mapped per ADR-0051's fall-through semantics — only `NotFound` causes an
`ExternalProviderChain` to fall through to the next provider; every other
variant short-circuits.

| Vault outcome           | Mapped to                       |
|-------------------------|---------------------------------|
| HTTP 404                | `ProviderError::NotFound`       |
| HTTP 403                | `ProviderError::AccessDenied`   |
| HTTP 5xx, transport err | `ProviderError::Unavailable`    |
| Other 4xx, decode err   | `ProviderError::Backend`        |

The `Debug` impl on `VaultProvider` deliberately omits the auth token —
the address and KV mount are safe to surface for diagnostics, the bearer
token is not.

## See also

- [`docs/adr/0051-external-provider-redesign.md`][adr0051] — design rationale and the
  trait surface.
- [`crates/credential/src/provider/`][credential-provider] — the trait definitions.
- [`crates/storage/src/credential/provider_cache.rs`][cache] — the cache layer this
  backend composes with.

[adr0051]: ../../docs/adr/0051-external-provider-redesign.md
[credential-provider]: ../credential/src/provider/
[cache]: ../storage/src/credential/provider_cache.rs
