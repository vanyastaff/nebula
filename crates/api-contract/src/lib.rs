//! Versioned HTTP wire data shared by the server and curated SDK client.
//!
//! This technical contract ships lockstep with Nebula. The supported Rust
//! product surface remains `nebula-sdk`. Domain authority, storage adapters,
//! HTTP clients and server frameworks are deliberately absent.

#![warn(missing_docs)]

pub mod v1;
