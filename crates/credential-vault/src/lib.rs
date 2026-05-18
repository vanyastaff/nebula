//! # nebula-credential-vault
//!
//! HashiCorp Vault backend for Nebula's [`ExternalProvider`] /
//! [`LeasedProvider`] trait surface. Supports the two canonical
//! Vault read shapes:
//!
//! - **KV v2 static secrets** (`/v1/{mount}/data/{path}`) — the default path
//!   shape. Returns a [`ProviderResolution::from_secret`] with no lease.
//! - **Dynamic secrets** (`/v1/{mount}/creds/{role}` and the equivalent
//!   `aws/`, `pki/`, etc. endpoints) — opt-in via the `dyn/` path prefix
//!   convention. Returns a [`ProviderResolution::with_lease`] carrying a
//!   [`LeaseHandle`] attributed to `"vault"`.
//!
//! Lease lifecycle (`/sys/leases/renew`, `/sys/leases/revoke`) is exposed
//! through the [`LeasedProvider`] sub-trait — surfaced to composed
//! providers (chain / cache layer) via the
//! [`ExternalProvider::lease_renewal`] capability discovery hook.
//!
//! # Path convention
//!
//! [`ExternalReference::path`] is interpreted by prefix:
//!
//! | Prefix | Backend route | Resolution shape |
//! |-------------|------------------------------|---------------------------------------------------|
//! | _(none)_ | `GET /v1/{kv_mount}/data/{path}` | `from_secret` (no lease) |
//! | `dyn/<rest>`| `GET /v1/{rest}` | `with_lease` (lease_id + TTL from response) |
//!
//! `ExternalReference::version` is honoured for KV v2 (added as
//! `?version=N`). `ExternalReference::field` is interpreted as a JSON-pointer-
//! free field lookup against the response's data map: if set, only that
//! field is returned; otherwise the entire data map is JSON-encoded.
//!
//! [`ExternalProvider`]: nebula_credential::provider::ExternalProvider
//! [`LeasedProvider`]: nebula_credential::provider::LeasedProvider
//! [`ProviderResolution::from_secret`]: nebula_credential::provider::ProviderResolution::from_secret
//! [`ProviderResolution::with_lease`]: nebula_credential::provider::ProviderResolution::with_lease
//! [`LeaseHandle`]: nebula_credential::provider::LeaseHandle
//! [`ExternalProvider::lease_renewal`]: nebula_credential::provider::ExternalProvider::lease_renewal
//! [`ExternalReference::path`]: nebula_credential::provider::ExternalReference::path
#![forbid(unsafe_code)]

mod provider;
mod wire;

pub use provider::{VaultConfig, VaultError, VaultProvider};
