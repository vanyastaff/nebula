//! Wire types for the subset of the Vault HTTP API this crate calls.
//!
//! Only the response shapes we actually consume are modelled. Vault returns
//! a JSON envelope with a common top-level `data` field; the KV v2 secret
//! engine nests another `data` inside that envelope (and a sibling
//! `metadata` object), while dynamic-secret engines put credentials
//! directly in `data` and carry lease metadata at the envelope root
//! (`lease_id`, `lease_duration`, `renewable`).
//!
//! Fields not consumed here are silently ignored at decode time
//! (`serde_json` default behaviour), which keeps the wire surface tolerant
//! to Vault server version drift.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top-level envelope returned by `GET /v1/{mount}/data/{path}`.
///
/// Vault wraps the actual KV v2 payload in a `data.data` nested object so
/// that secret metadata (`data.metadata.version`, etc.) lives at a sibling
/// path without collision against arbitrary user keys.
#[derive(Debug, Deserialize)]
pub(crate) struct KvV2Envelope {
    pub(crate) data: KvV2Data,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KvV2Data {
    /// The actual key/value map stored at the secret path. `serde_json::Value`
    /// rather than `BTreeMap<String, String>` because Vault permits nested
    /// JSON objects as values, and the field-extraction logic should
    /// surface those verbatim rather than fail to deserialize them.
    pub(crate) data: BTreeMap<String, serde_json::Value>,
}

/// Envelope for dynamic-secret responses
/// (`GET /v1/{mount}/creds/{role}` and equivalents on `aws/`, `gcp/`, …).
///
/// Carries credentials in `data` and lease metadata at the envelope root.
#[derive(Debug, Deserialize)]
pub(crate) struct DynamicSecretEnvelope {
    /// Vault-assigned lease identifier — opaque, used for `renew`/`revoke`.
    pub(crate) lease_id: String,
    /// Lease duration in seconds. Vault sends `0` when the issued credential
    /// is non-leasable; we still build a `LeaseHandle` so the caller can
    /// distinguish "issued but eternal" from "static KV value".
    pub(crate) lease_duration: u64,
    /// The credential payload — same shape as KV v2 `data.data`, but not
    /// nested under another `data` layer because dynamic secrets don't
    /// carry version metadata.
    pub(crate) data: BTreeMap<String, serde_json::Value>,
}

/// Response shape for `PUT /v1/sys/leases/renew`. Vault echoes the lease
/// id and the (possibly clamped) new TTL; the secret payload is **not**
/// included — renew is metadata-only by design.
#[derive(Debug, Deserialize)]
pub(crate) struct LeaseRenewEnvelope {
    pub(crate) lease_id: String,
    pub(crate) lease_duration: u64,
}
