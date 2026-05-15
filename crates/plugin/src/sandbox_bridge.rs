//! `SandboxError` → `ActionError` classification.
//!
//! This is the single seam where the transport crate's typed
//! [`SandboxError`](nebula_sandbox::SandboxError) is mapped to the
//! engine's [`ActionError`](nebula_action::ActionError) taxonomy. It lives
//! in `nebula-plugin` because both consumers reach it via a legal
//! downward edge:
//!
//! - [`ProcessSandboxHandler`](crate::ProcessSandboxHandler) (in this crate) maps the result of a
//!   discovered plugin's transport call at the `StatelessHandler` boundary.
//! - the engine-side `impl SandboxRunner for ProcessSandbox` adapter (`nebula-engine`, which depends
//!   on this crate) maps the same error for the runner-trait path.
//!
//! Keeping one implementation avoids a second, drift-prone copy of the
//! retry/fatal policy.

use nebula_action::ActionError;
use nebula_sandbox::SandboxError;

/// Convert an internal [`SandboxError`] into the public [`ActionError`]
/// returned to the engine. Transport-level issues are fatal
/// (non-retryable) by design: once the plugin has misbehaved on the wire,
/// the next caller gets a fresh process, not a blind retry on the same
/// poisoned channel.
#[must_use]
pub fn sandbox_error_to_action_error(err: SandboxError) -> ActionError {
    match err {
        // Retryable: plugin crashed / exited with NO request bytes on a
        // running plugin for this attempt (pre-send / stale-on-entry).
        // Respawn-and-retry is safe — nothing was executed. UNCHANGED.
        SandboxError::PluginClosed => ActionError::retryable_from(err),
        // Fatal: the plugin closed AFTER the request was sent. The action
        // may already have run; resending would risk double-execution.
        // The engine's retry decision finalizes a fatal error before the
        // policy check, so "bytes reached the plugin ⇒ no re-dispatch" is
        // structural, not best-effort.
        SandboxError::PluginClosedAfterSend => ActionError::fatal_from(err),
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
        // `SandboxError` is `#[non_exhaustive]`. A future transport
        // failure mode reaches this boundary as fatal until it is
        // explicitly classified — never silently retried.
        _ => ActionError::fatal_from(err),
    }
}

#[cfg(test)]
mod tests {
    //! `SandboxError` → `ActionError` classification guards. These moved
    //! here (with the function) when the discovery cluster relocated from
    //! `nebula-sandbox` to `nebula-plugin`: the transport crate no longer
    //! knows about `ActionError`, and `nebula-plugin` is the single home
    //! shared by the handler and the engine runner adapter.

    use super::*;

    #[test]
    fn plugin_line_too_large_converts_to_fatal_action_error() {
        let sandbox_err = SandboxError::PluginLineTooLarge {
            limit: 1024,
            observed: 2048,
        };
        let ae = sandbox_error_to_action_error(sandbox_err);
        // Fatal classification is the contract: we do not want the
        // engine to quietly retry a DoS attempt on a fresh connection.
        assert!(
            matches!(ae, ActionError::Fatal { .. }),
            "PluginLineTooLarge must classify as Fatal, got {ae:?}",
        );
    }

    #[test]
    fn handshake_line_too_large_converts_to_fatal_action_error() {
        let sandbox_err = SandboxError::HandshakeLineTooLarge {
            limit: 4096,
            observed: 8192,
        };
        let ae = sandbox_error_to_action_error(sandbox_err);
        assert!(matches!(ae, ActionError::Fatal { .. }));
    }

    #[test]
    fn plugin_closed_converts_to_retryable_action_error() {
        // Plugin-closed (pre-send / stale-on-entry) is benign relative to
        // DoS and nothing was executed — retry to respawn is safe.
        // UNCHANGED behaviour; preserves Phase-1's safe respawn.
        let sandbox_err = SandboxError::PluginClosed;
        let ae = sandbox_error_to_action_error(sandbox_err);
        assert!(
            matches!(ae, ActionError::Retryable { .. }),
            "PluginClosed should classify as Retryable, got {ae:?}",
        );
    }

    #[test]
    fn plugin_closed_after_send_converts_to_fatal_action_error() {
        // Plugin closed AFTER the request was sent: the action may have
        // run. This MUST be fatal so the engine's retry decision
        // finalizes it without re-dispatch — the structural half of the
        // no-resend guarantee.
        let ae = sandbox_error_to_action_error(SandboxError::PluginClosedAfterSend);
        assert!(
            matches!(ae, ActionError::Fatal { .. }),
            "PluginClosedAfterSend must classify as Fatal (no resend), got {ae:?}",
        );
    }

    #[test]
    fn host_malformed_envelope_converts_to_fatal_action_error() {
        let parse_err = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("fixture must produce serde_json::Error");
        let ae = sandbox_error_to_action_error(SandboxError::HostMalformedEnvelope(parse_err));
        assert!(matches!(ae, ActionError::Fatal { .. }));
    }

    #[test]
    fn response_id_mismatch_converts_to_fatal_action_error() {
        let err = SandboxError::ResponseIdMismatch {
            expected: 42,
            got: 41,
        };
        let ae = sandbox_error_to_action_error(err);
        assert!(
            matches!(ae, ActionError::Fatal { .. }),
            "ResponseIdMismatch must classify as Fatal, got {ae:?}",
        );
    }

    #[test]
    fn handshake_addr_mismatch_converts_to_fatal_action_error() {
        let err = SandboxError::HandshakeAddrMismatch {
            expected: String::from("unix|/tmp/ok"),
            got: String::from("unix|/tmp/evil"),
        };
        let ae = sandbox_error_to_action_error(err);
        assert!(
            matches!(ae, ActionError::Fatal { .. }),
            "HandshakeAddrMismatch must classify as Fatal (no retry on forged handshake), got {ae:?}",
        );
    }
}
