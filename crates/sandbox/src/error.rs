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

use nebula_action::ActionError;

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

    /// The plugin's response envelope carried a correlation id that did
    /// not match the id the host sent in the request (#285). Signals a
    /// stale response (e.g. a late reply to a cancelled call) or a
    /// protocol violation. The transport is poisoned after this error
    /// so the next request respawns the plugin on a fresh connection —
    /// defense-in-depth against cross-request id confusion.
    #[error(
        "plugin response id mismatch: sent {expected}, got {got} — \
         possible stale response or protocol violation; connection poisoned"
    )]
    ResponseIdMismatch {
        /// The correlation id the host placed in the outgoing request.
        expected: u64,
        /// The correlation id the plugin echoed back in its response.
        got: u64,
    },

    /// Landlock ruleset construction or enforcement failed (Linux only).
    /// Produced pre-`fork` while preparing the child sandbox — this is a
    /// spawn-time hardening failure, not a transport/protocol error.
    #[error("landlock setup failed: {0}")]
    Landlock(String),

    /// Resource-limit (`setrlimit`) configuration failed (Linux only),
    /// also a pre-`fork` spawn-time hardening failure.
    #[error("rlimit setup failed: {0}")]
    Rlimit(String),

    /// The plugin binary could not be spawned, the handshake could not be
    /// read/validated, or the announced transport could not be dialled.
    /// This is a spawn-time failure before any envelope round-trip — a
    /// blind respawn-retry on the same misconfigured host/binary would
    /// fail identically.
    #[error("plugin spawn/dial failed: {0}")]
    Spawn(String),

    /// The plugin replied with an `ActionResultError` envelope. Carries the
    /// plugin-reported code/message and its own retry hint. The host maps
    /// this to the public `ActionError` classification at the engine-side
    /// runner boundary; `retryable` is the plugin's advice, not a transport
    /// fact.
    #[error("plugin returned an action error [{code}]: {message}")]
    PluginActionError {
        /// Plugin-supplied error code.
        code: String,
        /// Plugin-supplied human-readable message (sanitized before use).
        message: String,
        /// Whether the plugin marked this error as retryable.
        retryable: bool,
    },

    /// The plugin returned an envelope kind that does not match what the
    /// host requested (e.g. a `log` line where an `ActionResult*` was
    /// expected).
    #[error("plugin returned unexpected envelope (got {kind})")]
    UnexpectedEnvelope {
        /// The envelope kind the plugin sent instead.
        kind: String,
    },

    /// The per-call envelope round-trip exceeded its wall-clock deadline.
    /// The connection is dropped (state is undefined after a partial
    /// write); the engine-side adapter decides retry policy.
    #[error("plugin {plugin} timed out on {envelope} after {timeout:?}")]
    Timeout {
        /// Plugin binary path (display form) that timed out.
        plugin: String,
        /// Envelope tag (`action_invoke` / `metadata_request` / `other`).
        envelope: String,
        /// The configured per-call timeout that elapsed.
        timeout: std::time::Duration,
    },

    /// The dispatch was cancelled mid-round-trip via the engine
    /// cancellation token. The connection is dropped; the action must not
    /// be silently resent (it may already be running on the plugin).
    #[error("plugin dispatch cancelled")]
    Cancelled,
}

/// Convert an internal [`SandboxError`] into the public `ActionError` the
/// engine-side sandbox runner adapter returns. Transport-level issues are
/// fatal (non-retryable) by design: once the plugin has misbehaved on the
/// wire, the next caller gets a fresh process, not a blind retry on the
/// same poisoned channel.
pub(crate) fn sandbox_error_to_action_error(err: SandboxError) -> ActionError {
    match err {
        // Retryable: plugin crashed / exited, respawn path is safe.
        SandboxError::PluginClosed => ActionError::retryable_from(err),
        // Timeout surfaces as retryable so the engine's higher-level retry
        // policy can decide; the sandbox itself never silently retries.
        SandboxError::Timeout { .. } => ActionError::retryable_from(err),
        // Cancellation must round-trip as the canonical cancelled error so
        // the engine honours its standard cancellation path.
        SandboxError::Cancelled => ActionError::Cancelled,
        // The plugin itself classified this error; honour its retry hint.
        SandboxError::PluginActionError { retryable: true, .. } => ActionError::retryable_from(err),
        SandboxError::PluginActionError { retryable: false, .. } => ActionError::fatal_from(err),
        // Fatal: DoS / protocol-abuse signals. Do not paper over with retry.
        SandboxError::PluginLineTooLarge { .. }
        | SandboxError::HandshakeLineTooLarge { .. }
        | SandboxError::HandshakeAddrMismatch { .. }
        | SandboxError::ResponseIdMismatch { .. }
        | SandboxError::TransportPoisoned
        | SandboxError::Transport(_)
        | SandboxError::MalformedEnvelope(_)
        | SandboxError::HostMalformedEnvelope(_)
        | SandboxError::UnexpectedEnvelope { .. }
        // Pre-`fork` spawn-time hardening / spawn failures. They reach the
        // public boundary as fatal; a retry on the same misconfigured host
        // or binary would fail identically.
        | SandboxError::Landlock(_)
        | SandboxError::Rlimit(_)
        | SandboxError::Spawn(_) => ActionError::fatal_from(err),
    }
}
