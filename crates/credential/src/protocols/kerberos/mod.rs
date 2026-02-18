//! Kerberos protocol stub — planned for Phase 7.
//!
//! Full implementation requires `libkrb5` FFI or a pure-Rust Kerberos library.
//! This module reserves the namespace for `#[credential(extends = KerberosProtocol)]`.

use serde::{Deserialize, Serialize};

/// Kerberos authentication configuration.
///
/// Used with `#[kerberos(...)]` macro attribute once `KerberosProtocol` is implemented.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KerberosConfig {
    /// Kerberos realm (e.g. `EXAMPLE.COM`).
    pub realm: String,
    /// Key Distribution Center hostname.
    pub kdc: String,
    /// Service principal name (e.g. `HTTP/service.example.com`).
    pub service_principal: String,
}
