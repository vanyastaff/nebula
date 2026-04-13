#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Plugin Protocol
//!
//! Typed wire protocol for Nebula community plugins (process-isolated binaries).
//!
//! Plugin authors never import this crate directly — they use
//! [`nebula-plugin-sdk`](https://docs.rs/nebula-plugin-sdk) which wraps the
//! envelope types behind an ergonomic `PluginHandler` trait. The host side
//! (`nebula-sandbox`) imports this crate to (de)serialize plugin responses.
//!
//! ## Protocol shape
//!
//! Bidirectional line-delimited JSON envelope stream over the plugin's
//! stdin/stdout. Each line is one JSON object tagged by `kind`. See the
//! [`duplex`] module for the full message catalog.
//!
//! - Host → plugin: [`duplex::HostToPlugin`] — `ActionInvoke`, `Cancel`, `MetadataRequest`,
//!   `RpcResponseOk/Error`, `Shutdown`.
//! - Plugin → host: [`duplex::PluginToHost`] — `ActionResultOk/Error`, `RpcCall`, `Log`,
//!   `MetadataResponse`.
//!
//! Protocol version: [`duplex::DUPLEX_PROTOCOL_VERSION`].
//!
//! Phase 1 implementation progress: see
//! `docs/plans/2026-04-13-sandbox-phase1-broker.md`.

pub mod duplex;
