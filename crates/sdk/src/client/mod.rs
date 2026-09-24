//! Curated contracts for clients of Nebula's public product API.
//!
//! These types describe public product state. They do not expose runtime
//! authority, persistence handles, or orchestration internals.

pub mod credential;

#[cfg(feature = "http")]
pub mod http;
