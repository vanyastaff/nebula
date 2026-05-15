//! Sandbox runner abstraction — the engine-side boundary between the
//! action dispatcher and the isolation transport.
//!
//! The dispatcher owns the runner trait: it is the consumer that decides,
//! per [`IsolationLevel`](nebula_action::IsolationLevel), whether an action
//! runs in-process (trusted built-ins) or through the out-of-process
//! transport (community plugins). The transport crate (`nebula-sandbox`)
//! stays free of `nebula_action`: the `SandboxError` -> `ActionError` and
//! `Value` -> `ActionResult` mapping lives here, in the adapter that bridges
//! [`ProcessSandbox`](nebula_sandbox::ProcessSandbox) to [`SandboxRunner`].
//!
//! ## Key types
//!
//! - [`SandboxRunner`] — execute an action within an isolation boundary.
//! - [`InProcessSandbox`] — trusted in-process dispatch; no isolation.
//! - [`SandboxedContext`] — cooperative cancellation check for the runner.
//! - [`ActionExecutor`] — registry-lookup-and-invoke callback.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, result::ActionResult};
use nebula_sandbox::{ProcessSandbox, SandboxError};
use tokio_util::sync::CancellationToken;

/// Sandboxed execution context wrapping an [`ActionContext`].
///
/// Provides a cooperative cancellation check before action execution.
pub struct SandboxedContext {
    cancellation: CancellationToken,
}

impl SandboxedContext {
    /// Build sandbox metadata from an action context.
    pub fn new(context: &dyn ActionContext) -> Self {
        Self {
            cancellation: context.cancellation().clone(),
        }
    }

    /// Check whether execution has been cancelled.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Borrow the cancellation token for long-running dispatch paths that
    /// need to `select!` against it (e.g. the process-sandbox plugin
    /// round-trip).
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

/// Trait for executing actions within an isolation boundary.
///
/// Implementations provide different isolation levels:
/// - [`InProcessSandbox`] — trusted, in-process (built-in actions)
/// - [`ProcessSandbox`](nebula_sandbox::ProcessSandbox) — child-process dispatch over a JSON
///   envelope with Linux OS-level hardening (community plugins)
///
/// WASM is an explicit non-goal — see `docs/PRODUCT_CANON.md` §12.6.
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Execute an action within the sandbox.
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}

/// Boxed future returned by the action executor.
pub type ActionExecutorFuture = std::pin::Pin<
    Box<dyn Future<Output = Result<ActionResult<serde_json::Value>, ActionError>> + Send>,
>;

/// Callback type for executing an action (registry lookup + invoke).
pub type ActionExecutor = Arc<
    dyn Fn(SandboxedContext, &ActionMetadata, serde_json::Value) -> ActionExecutorFuture
        + Send
        + Sync,
>;

/// In-process sandbox: runs actions in the same process (cooperative
/// cancellation check only — no isolation).
///
/// Suitable for first-party (built-in) actions that are trusted code.
/// Untrusted/community plugins run out-of-process via
/// [`ProcessSandbox`](nebula_sandbox::ProcessSandbox).
pub struct InProcessSandbox {
    executor: ActionExecutor,
}

impl InProcessSandbox {
    /// Create a new in-process sandbox with the given action executor.
    pub fn new(executor: ActionExecutor) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl SandboxRunner for InProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        tracing::debug!(
            action_key = %metadata.base.key,
            "executing action in-process"
        );
        context.check_cancelled()?;
        let result = (self.executor)(context, metadata, input).await;
        if let Err(e) = &result {
            tracing::warn!(action_key = %metadata.base.key, error = %e, "action failed");
        }
        result
    }
}

/// Convert an internal [`SandboxError`] into the public [`ActionError`] the
/// sandbox runner trait returns. Transport-level issues are fatal
/// (non-retryable) by design: once the plugin has misbehaved on the wire,
/// the next caller gets a fresh process, not a blind retry on the same
/// poisoned channel.
fn sandbox_error_to_action_error(err: SandboxError) -> ActionError {
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
        // `SandboxError` is `#[non_exhaustive]`. A future transport
        // failure mode reaches this boundary as fatal until it is
        // explicitly classified — never silently retried.
        _ => ActionError::fatal_from(err),
    }
}

/// Adapter: drive an out-of-process [`ProcessSandbox`] as a
/// [`SandboxRunner`].
///
/// This is the single place the transport's `SandboxError` is classified
/// into the engine's `ActionError` taxonomy and the plugin output `Value`
/// is wrapped in an `ActionResult`. The transport crate owns neither —
/// keeping `nebula-sandbox` a Business-dependency-free leaf.
#[async_trait]
impl SandboxRunner for ProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        context.check_cancelled()?;

        let action_key = metadata.base.key.as_str();

        tracing::debug!(
            action_key = %action_key,
            plugin = %self.binary().display(),
            "executing action in process sandbox"
        );

        self.invoke_with_cancel(action_key, input, context.cancellation())
            .await
            .map(ActionResult::success)
            .map_err(sandbox_error_to_action_error)
    }
}

#[cfg(test)]
mod tests {
    //! `SandboxError` -> `ActionError` classification guards. These
    //! moved here with `sandbox_error_to_action_error` when the runner
    //! adapter relocated from `nebula-sandbox` to the engine: the
    //! transport crate no longer knows about `ActionError`.

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
        // Plugin-closed is benign relative to DoS — retry to respawn is safe.
        let sandbox_err = SandboxError::PluginClosed;
        let ae = sandbox_error_to_action_error(sandbox_err);
        assert!(
            matches!(ae, ActionError::Retryable { .. }),
            "PluginClosed should classify as Retryable, got {ae:?}",
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
