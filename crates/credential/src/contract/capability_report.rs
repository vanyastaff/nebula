//! Capability detection at registration.
//!
//! Stage 5 stubs `compute_capabilities` to return `Capabilities::empty()`
//! so `CredentialRegistry` (Tech Spec §15.6) compiles standalone. Stage 7
//! (Tech Spec §15.8) fills in real detection via the per-credential
//! `plugin_capability_report::*` constants emitted by the
//! `#[plugin_credential]` macro.
//!
//! # Bitflag set
//!
//! The five capability flags mirror the sub-trait surface introduced
//! in Stage 3 (Tech Spec §15.4):
//!
//! | Flag | Sub-trait |
//! |------|-----------|
//! | `INTERACTIVE` | `Interactive` |
//! | `REFRESHABLE` | `Refreshable` |
//! | `REVOCABLE`   | `Revocable` |
//! | `TESTABLE`    | `Testable` |
//! | `DYNAMIC`     | `Dynamic` |

use bitflags::bitflags;

bitflags! {
    /// Capability set computed at registration time. Authoritative source
    /// of capability discovery once Stage 7 (§15.8) lands — supersedes
    /// the metadata-field reads that previously controlled `iter_compatible`
    /// filtering and refresh-dispatcher entry points.
    #[derive(Clone, Copy, Default, Debug, Eq, PartialEq, Hash)]
    pub struct Capabilities: u8 {
        /// Credential implements [`Interactive`](crate::Interactive).
        const INTERACTIVE = 1 << 0;
        /// Credential implements [`Refreshable`](crate::Refreshable).
        const REFRESHABLE = 1 << 1;
        /// Credential implements [`Revocable`](crate::Revocable).
        const REVOCABLE = 1 << 2;
        /// Credential implements [`Testable`](crate::Testable).
        const TESTABLE = 1 << 3;
        /// Credential implements [`Dynamic`](crate::Dynamic).
        const DYNAMIC = 1 << 4;
    }
}

/// Capability detection stub for Stage 5. Returns
/// [`Capabilities::empty()`] until Stage 7 (Tech Spec §15.8) wires
/// real detection via the macro-emitted `plugin_capability_report::*`
/// constants on each `Credential` type.
///
/// This shim exists to keep the registry compile-clean during the
/// staged rollout; the type parameter `C` is intentionally unbounded so
/// every concrete credential satisfies it without requiring the full
/// detection plumbing.
#[doc(hidden)]
pub fn compute_capabilities<C>() -> Capabilities {
    Capabilities::empty()
}
