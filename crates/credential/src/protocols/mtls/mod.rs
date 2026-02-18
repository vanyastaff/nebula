//! mTLS (Mutual TLS) protocol stub — planned for Phase 7.
//!
//! Full implementation requires `rustls` client certificate support.
//! This module reserves the namespace for `#[credential(extends = MtlsProtocol)]`.

use serde::{Deserialize, Serialize};

/// mTLS client certificate configuration.
///
/// Used with `#[mtls(...)]` macro attribute once `MtlsProtocol` is implemented.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MtlsConfig {
    /// PEM-encoded client certificate chain.
    pub certificate: String,
    /// PEM-encoded private key (stored encrypted at rest).
    pub private_key: String,
    /// Optional PEM-encoded CA certificate for server verification.
    pub ca_certificate: Option<String>,
}
