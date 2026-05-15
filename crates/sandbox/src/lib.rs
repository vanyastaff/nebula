#![deny(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-sandbox — Process Sandbox Transport (Correctness Boundary)
//!
//! The host side of the duplex plugin transport (ADR 0006). A leaf crate:
//! it spawns a plugin binary, dials its announced socket, and round-trips
//! JSON envelopes with a per-call timeout + cancellation race.
//!
//! **This is not a security boundary against malicious native code.**
//! Canon §12.6 is the normative statement: child-process execution provides
//! OS-namespace separation via a duplex JSON envelope over UDS / Named Pipe.
//! WASM / WASI is an explicit non-goal (§12.6). There is **no** per-plugin
//! capability/scope model here — egress, credential, and filesystem
//! mediation is the broker's responsibility (ADR-0025), not this crate.
//!
//! Discovery, the `RemoteAction`/`ProcessSandboxHandler` registry adapters,
//! and the `SandboxError` → `ActionError` mapping live in `nebula-plugin`
//! (host-registry population belongs with the registry). The
//! `SandboxRunner` runner abstraction lives in `nebula-engine` (the
//! dispatcher that owns it). This crate has no Business-tier dependency.
//!
//! ## Key types
//!
//! - `ProcessSandbox` — child-process execution via JSON envelope (ADR 0006). Transport methods
//!   return `SandboxError`.
//! - `os_sandbox` — Linux Landlock + rlimit child hardening (fixed system
//!   paths; no per-plugin grant).
//! - `SandboxError` — typed transport error.
//!
//! ## Canon
//!
//! - §7.1 plugin packaging: this crate is the host side of the duplex transport;
//!   `nebula-plugin-sdk` is the plugin side.
//! - §12.6 isolation honesty: correctness boundary, not attacker-grade; no
//!   false-capability surface is advertised.
//!
//! See `crates/sandbox/README.md` for the real isolation roadmap and ADR 0006
//! status.

pub mod error;
pub mod os_sandbox;

mod codec;
mod dispatch;
mod handshake;
mod spawn;

pub use dispatch::ProcessSandbox;
pub use error::SandboxError;
