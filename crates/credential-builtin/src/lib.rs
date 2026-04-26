//! # nebula-credential-builtin
//!
//! Built-in concrete credential types and the canonical
//! [`sealed_caps`] module per ADR-0035. Plugin authors depend on
//! `nebula-credential` (the contract crate); first-party concrete
//! types live here.
//!
//! ## Canonical `mod sealed_caps`
//!
//! Per ADR-0035 §3 (amended 2026-04-24-B), every crate that declares
//! capability phantom traits in `dyn` positions must provide a
//! crate-private `sealed_caps` module with **per-capability** inner
//! sealed traits. This crate is the canonical home for the built-in
//! capabilities; plugin crates declare their own `mod sealed_caps`
//! at their own crate root for capabilities they introduce.
//!
//! See `README.md` for the plugin-author onboarding guide.
#![forbid(unsafe_code)]

extern crate self as nebula_credential_builtin;

/// Canonical inner sealed traits for built-in capabilities.
///
/// Crate-private. External crates cannot impl these — they declare
/// their own `mod sealed_caps` per ADR-0035 §3.
///
/// `dead_code` is silenced for the П1 scaffold — these inner traits
/// are emitted into `dyn` positions by the `#[capability]` macro in
/// П3, at which point each becomes load-bearing (Tech Spec §16.1).
#[allow(dead_code)]
pub(crate) mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait SigningSealed {}
    pub trait TlsIdentitySealed {}
}

// Concrete credential types land here in П3 (per Tech Spec §16.1).
// П1 ships the empty scaffold so deny.toml + workspace member
// resolution settle ahead of the type-shape commits.
