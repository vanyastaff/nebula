//! Sandbox-specific error types.
//!
//! These errors describe failures of the plugin transport layer
//! ([`crate::ProcessSandbox`]) independently of the broader
//! [`nebula_action::ActionError`] classification. They are converted into
//! `ActionError::Fatal` / `ActionError::Retryable` at the public boundary
//! via `ActionError::fatal_from` / `retryable_from`, which preserves the
//! full source chain for logging and metrics.
//!
//! A dedicated type lets the caller distinguish "plugin is misbehaving /
//! attempting DoS" (`PluginLineTooLarge`, `HandshakeLineTooLarge`) from
//! "plugin exited / transport broken" (`PluginClosed`,
//! `Transport`) — valuable signal for security dashboards.

use std::io;

/// Errors produced by the plugin transport layer of
/// [`crate::ProcessSandbox`].
///
/// Marked `#[non_exhaustive]` so new failure modes (e.g. future per-message
/// framing violations) can be added without a semver break.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SandboxError {
    /// The plugin emitted a single line that exceeded the envelope size
    /// cap without yielding a newline. The transport connection is
    /// poisoned after this error and the surrounding
    /// [`crate::ProcessSandbox`] will clear its cached handle so the next
    /// call respawns the plugin.
    #[error(
        "plugin envelope line exceeded cap of {limit} bytes (observed {observed}) — \
         possible DoS or protocol violation; connection poisoned"
    )]
    PluginLineTooLarge {
        /// Configured byte cap for a single envelope line.
        limit: usize,
        /// Number of bytes actually consumed before the cap was hit.
        /// Always `> limit` (we read one extra byte to distinguish
        /// "exactly at cap" from "over cap").
        observed: usize,
    },

    /// The plugin emitted an oversized handshake line. Triggers the same
    /// hard failure as an oversized envelope but is reported separately
    /// for observability — a handshake failure points at plugin startup,
    /// an envelope failure points at runtime protocol abuse.
    #[error(
        "plugin handshake line exceeded cap of {limit} bytes (observed {observed}) — \
         refusing to dial announced transport"
    )]
    HandshakeLineTooLarge {
        /// Configured byte cap for the handshake line.
        limit: usize,
        /// Number of bytes consumed before the cap was hit.
        observed: usize,
    },

    /// The plugin closed its end of the transport without sending a
    /// response envelope. Signals abnormal plugin exit, not a protocol
    /// violation.
    #[error("plugin closed transport without sending a response envelope")]
    PluginClosed,

    /// An operation was attempted on a transport connection that was
    /// previously poisoned by an oversize read. This is an internal
    /// safeguard — the surrounding `ProcessSandbox` clears the cached
    /// handle on any error, so reaching this variant means defense-in-depth
    /// has fired.
    #[error("plugin transport is poisoned and must not be reused")]
    TransportPoisoned,

    /// Underlying I/O failure on the transport read/write.
    #[error("plugin transport I/O error")]
    Transport(#[source] io::Error),

    /// Plugin sent bytes that did not decode as a valid envelope.
    #[error("plugin sent malformed envelope")]
    MalformedEnvelope(#[source] serde_json::Error),

    /// Host failed to serialize an outbound envelope before writing it
    /// to the plugin transport.
    #[error("host failed to serialize outbound envelope")]
    HostMalformedEnvelope(#[source] serde_json::Error),

    /// The plugin announced a handshake address that does not match the
    /// host-allocated one. Protects against the "forged handshake →
    /// hijack sibling plugin socket" attack (#260).
    #[error(
        "plugin announced handshake address `{got}` but host expected `{expected}` — \
         refusing to dial; a compromised plugin may be attempting to redirect \
         to another process's socket"
    )]
    HandshakeAddrMismatch {
        /// The address the host set via the `NEBULA_PLUGIN_SOCKET_ADDR`
        /// env var before spawn.
        expected: String,
        /// The address announced by the plugin in its handshake line.
        got: String,
    },
}
