//! Long-lived envelope dispatch for the process sandbox.
//!
//! [`ProcessSandbox`] spawns the plugin binary once (via
//! [`spawn_and_dial`](crate::spawn::spawn_and_dial)), caches the resulting
//! [`PluginHandle`](crate::codec::PluginHandle), and round-trips envelopes
//! over it with a per-call timeout and a cancellation race. A broken
//! connection clears the handle so the next call respawns.
//!
//! Slice 1c (2026-04-13): plugin processes are **long-lived**. On the first
//! call, `ProcessSandbox` spawns the plugin binary, reads the handshake
//! line from its stdout, dials the announced UDS or Named Pipe, and stores
//! the resulting [`PluginHandle`](crate::codec::PluginHandle) on the
//! sandbox. Subsequent calls reuse that handle, sending envelopes over the
//! socket without respawning. A broken connection (plugin crashed or
//! exited) clears the handle and the next request triggers a fresh spawn.
//!
//! The plugin-side event loop in `nebula-plugin-sdk::run_duplex` is still
//! sequential — one action at a time per plugin process. Slice 1d adds
//! concurrent multiplexed dispatch.
//!
//! The defense-in-depth poisoning contract (#316) is shared with `codec`:
//! the cached handle is dropped on any error here, and the
//! [`PluginHandle`](crate::codec::PluginHandle) also flips its own
//! `poisoned` flag — either layer alone prevents reuse of a desynced
//! transport.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use nebula_plugin_sdk::protocol::{HostToPlugin, PluginToHost};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    codec::{PluginHandle, envelope_kind, sanitize_plugin_string},
    error::SandboxError,
    handshake::{request_id, response_id, response_id_from_value},
    os_sandbox::LinuxRlimits,
    spawn::spawn_and_dial,
};

/// Map a plugin response envelope to the transport-level result value.
///
/// `ActionResultOk` → the raw output `Value`. `ActionResultError` → a
/// [`SandboxError::PluginActionError`] carrying the plugin's own code /
/// message / retry hint. Any other envelope kind →
/// [`SandboxError::UnexpectedEnvelope`]. The engine-side runner adapter
/// owns the `SandboxError` → `ActionError` / `Value` → `ActionResult`
/// classification; this crate stays free of `ActionError` on the transport
/// path.
fn action_response_to_value(envelope: PluginToHost) -> Result<serde_json::Value, SandboxError> {
    match envelope {
        PluginToHost::ActionResultOk { output, .. } => Ok(output),
        PluginToHost::ActionResultError {
            code,
            message,
            retryable,
            ..
        } => Err(SandboxError::PluginActionError {
            code: sanitize_plugin_string(&code),
            message: sanitize_plugin_string(&message),
            retryable,
        }),
        other => Err(SandboxError::UnexpectedEnvelope {
            kind: envelope_kind(&other).to_owned(),
        }),
    }
}

/// Process sandbox: spawns the plugin binary once and keeps the connection
/// alive for the lifetime of this sandbox instance.
///
/// Each `ProcessSandbox` owns a long-lived `PluginHandle` behind a
/// `Mutex`. The first invocation spawns the child and dials the socket;
/// subsequent invocations reuse the same handle. A connection error on
/// write or read invalidates the handle and the next call respawns.
pub struct ProcessSandbox {
    /// Path to the plugin binary.
    binary: PathBuf,
    /// Per-call timeout (envelope round-trip wall clock).
    timeout: Duration,
    /// Linux child-process resource limits (ignored on non-Linux).
    linux_rlimits: LinuxRlimits,
    /// Long-lived handle to the spawned plugin process. Serialized via the
    /// mutex — slice 1c is sequential per sandbox instance. Slice 1d can
    /// replace this with a lock-free handle once concurrent dispatch lands.
    handle: Mutex<Option<PluginHandle>>,
    /// Monotonic correlation id source (#285). Each outbound envelope
    /// gets a fresh id; `try_dispatch` verifies the response echoes it
    /// back. A stale response (e.g. late reply to a cancelled call)
    /// therefore can't be silently mis-associated with a fresh request
    /// — ID mismatch poisons the transport.
    ///
    /// Persisted across plugin respawns — the plugin only sees a
    /// monotone sequence from its own perspective (fresh process,
    /// fresh socket, fresh id stream), but the host never reuses an
    /// id across invocations within this sandbox instance's lifetime.
    next_id: AtomicU64,
}

/// Classification of a [`ProcessSandbox::try_dispatch`] failure.
///
/// `dispatch_envelope` uses this to decide whether a failed attempt is
/// eligible for a single respawn-and-retry. Only failures that demonstrably
/// occurred **before** any envelope bytes landed on a running plugin
/// process are retried — any other failure (cancellation, timeout,
/// mid-round-trip transport error, protocol violation) is terminal because
/// a retry would risk re-invoking a non-idempotent action on the plugin
/// side. See `dispatch_envelope` docs and the #257 review for the full
/// rationale.
#[derive(Debug)]
enum TryDispatchError {
    /// The first attempt observed a stale handle (plugin crashed or
    /// exited between calls). No envelope bytes reached a running plugin
    /// process, so the outer [`ProcessSandbox::dispatch_envelope`] is
    /// safe to respawn and retry exactly once.
    Respawnable(SandboxError),
    /// Terminal for this dispatch — either cancellation, timeout, a
    /// mid-round-trip transport error, a protocol violation, or a spawn
    /// failure. Must NOT be retried silently; the engine's higher-level
    /// retry policy (if any) remains free to retry externally, but the
    /// sandbox itself has to surface the error as-is so cancellation and
    /// fatal classifications round-trip correctly.
    Terminal(SandboxError),
}

impl TryDispatchError {
    /// Classify a transport-layer [`SandboxError`], given whether the
    /// outbound envelope was already written to the plugin (`sent`).
    ///
    /// [`SandboxError::PluginClosed`] is respawn-eligible **only** when
    /// `sent == false` — i.e. no envelope bytes reached a running plugin
    /// for this attempt. Once `send_envelope` has succeeded the plugin may
    /// have received and begun executing a non-idempotent action before
    /// dying; resending it on a fresh process would double-execute. The
    /// original #257 logic treated every `PluginClosed` as safe to
    /// respawn, conflating "stale handle on entry" with "EOF observed by
    /// `recv` after a successful send"; the `sent` bit separates them.
    ///
    /// An EOF observed *after* a successful send is re-typed to the
    /// distinct [`SandboxError::PluginClosedAfterSend`] variant so the
    /// no-resend guarantee survives the crate boundary structurally:
    /// `nebula-plugin`'s classifier maps that variant to a fatal
    /// `ActionError` and the engine's retry decision finalizes it without
    /// re-dispatch. Plain [`SandboxError::PluginClosed`] is kept *only*
    /// for the `!sent` / pre-send / stale-on-entry case, preserving the
    /// safe respawn path. Every other variant is terminal regardless and
    /// passes through unchanged.
    fn from_sandbox_error_after_send(err: SandboxError, sent: bool) -> Self {
        match (sent, &err) {
            // No bytes reached a running plugin → safe to respawn-retry.
            (false, SandboxError::PluginClosed) => Self::Respawnable(err),
            // EOF after a successful send: the action may have run. Re-type
            // to the distinct variant so "bytes reached the plugin ⇒ never
            // re-dispatch" is enforced by type across the crate boundary,
            // not by a `bool` that does not cross it.
            (true, SandboxError::PluginClosed) => {
                Self::Terminal(SandboxError::PluginClosedAfterSend)
            },
            // Every other failure mode is terminal as before.
            _ => Self::Terminal(err),
        }
    }

    /// `true` if [`ProcessSandbox::dispatch_envelope`] is allowed to
    /// respawn the plugin and retry this envelope once.
    #[cfg(test)]
    fn is_respawnable(&self) -> bool {
        matches!(self, Self::Respawnable(_))
    }

    /// Unwrap the carried [`SandboxError`] once the dispatch-level retry
    /// decision has been made.
    fn into_sandbox_error(self) -> SandboxError {
        match self {
            Self::Respawnable(err) | Self::Terminal(err) => err,
        }
    }
}

/// Outcome of a [`race_cancel_timeout`] call.
#[derive(Debug, PartialEq, Eq)]
enum RaceOutcome<T> {
    /// The inner future produced a value within the deadline and before
    /// the cancellation token fired.
    Ready(T),
    /// The wall-clock deadline elapsed first. The inner future was
    /// dropped mid-flight; callers must assume its side effects are
    /// partially applied (writes may have reached the peer).
    Timeout,
    /// The optional cancellation token fired first. Same partial-side-effect
    /// caveat as `Timeout` applies — the race only wins a snapshot of
    /// progress, not a clean rollback.
    Cancelled,
}

/// Race `fut` against a wall-clock deadline and (optionally) a
/// [`CancellationToken`]. Used by [`ProcessSandbox::try_dispatch`] to
/// honour both the per-call plugin timeout and the engine-wide
/// cancellation contract — see #257.
///
/// When `cancel` is `Some`, the select is `biased` so a token that is
/// already cancelled on entry is observed on the very first poll, before
/// the roundtrip future is driven. When `cancel` is `None` (public helpers
/// like `invoke` / `get_metadata` that run outside an execution context)
/// the helper degrades to a plain `tokio::time::timeout`.
async fn race_cancel_timeout<F, T>(
    fut: F,
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> RaceOutcome<T>
where
    F: Future<Output = T>,
{
    let timed = tokio::time::timeout(timeout, fut);
    match cancel {
        Some(token) => {
            tokio::select! {
                biased;
                () = token.cancelled() => RaceOutcome::Cancelled,
                r = timed => match r {
                    Ok(v) => RaceOutcome::Ready(v),
                    Err(_) => RaceOutcome::Timeout,
                },
            }
        },
        None => match timed.await {
            Ok(v) => RaceOutcome::Ready(v),
            Err(_) => RaceOutcome::Timeout,
        },
    }
}

impl ProcessSandbox {
    /// Create a new process sandbox for a plugin binary.
    #[must_use]
    pub fn new(binary: PathBuf, timeout: Duration) -> Self {
        Self {
            binary,
            timeout,
            linux_rlimits: LinuxRlimits::default(),
            handle: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    /// Reserve the next monotonic correlation id for an outbound
    /// envelope (#285). Uses `Relaxed` ordering — id allocation has no
    /// happens-before requirement against any other memory op; we only
    /// need uniqueness, which `fetch_add` guarantees regardless of
    /// ordering.
    fn next_envelope_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Override Linux child-process rlimits for this sandbox instance.
    ///
    /// On non-Linux platforms the provided limits are ignored.
    #[must_use]
    pub fn with_linux_rlimits(mut self, linux_rlimits: LinuxRlimits) -> Self {
        self.linux_rlimits = linux_rlimits;
        self
    }

    /// The plugin binary this sandbox dispatches to.
    ///
    /// Exposed for the engine-side runner adapter's tracing spans; not on
    /// any hot path.
    #[must_use]
    pub fn binary(&self) -> &std::path::Path {
        &self.binary
    }

    /// Invoke an action and return the plugin's response envelope.
    pub(crate) async fn call_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
        cancel: Option<&CancellationToken>,
    ) -> Result<PluginToHost, SandboxError> {
        let request = HostToPlugin::ActionInvoke {
            id: self.next_envelope_id(),
            action_key: action_key.to_owned(),
            input,
        };
        self.dispatch_envelope(request, cancel).await
    }

    /// Invoke an action, racing the round-trip against a cancellation
    /// token, and return the unwrapped output value.
    ///
    /// This is the transport entry point the engine-side `SandboxRunner`
    /// adapter uses for `IsolationLevel::CapabilityGated|Isolated`
    /// dispatch. It stays free of `ActionError`: the `SandboxError` ->
    /// `ActionError` and `Value` -> `ActionResult` mapping is the engine
    /// adapter's responsibility.
    pub async fn invoke_with_cancel(
        &self,
        action_key: &str,
        input: serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value, SandboxError> {
        let envelope = self.call_action(action_key, input, Some(cancel)).await?;
        action_response_to_value(envelope)
    }

    /// Query plugin metadata via a `MetadataRequest` envelope.
    pub async fn get_metadata(&self) -> Result<PluginToHost, SandboxError> {
        let request = HostToPlugin::MetadataRequest {
            id: self.next_envelope_id(),
        };
        self.dispatch_envelope(request, None).await
    }

    /// Query plugin metadata and return the response as raw JSON bytes,
    /// **before** attempting the strongly-typed `PluginToHost` deserialize.
    ///
    /// Purpose: let the caller check `protocol_version` before the typed
    /// parse runs. If a v2 plugin sends its old envelope shape (flat
    /// `plugin_key` / `plugin_version` fields, no `manifest`) against a
    /// v3 host, this method returns the raw bytes; the caller parses to
    /// `serde_json::Value` for the version check, and then — if the
    /// version matches — re-parses the same bytes into `PluginToHost`.
    ///
    /// Returning bytes rather than `serde_json::Value` preserves the
    /// zero-copy / borrowed-`&str` deserialize path that some `Deserialize`
    /// impls rely on (notably `domain_key::Key<T>` and therefore
    /// `PluginKey`), which would otherwise fail with
    /// `"expected a borrowed string"` when going through
    /// `serde_json::from_value`.
    ///
    /// Used by `discover_directory` in the `discovery` module (private path).
    pub async fn get_metadata_raw(&self) -> Result<Vec<u8>, SandboxError> {
        let request = HostToPlugin::MetadataRequest {
            id: self.next_envelope_id(),
        };
        self.dispatch_envelope_bytes(request, None).await
    }

    /// High-level action invocation for host code outside the engine flow
    /// (diagnostics, examples, integration tests, ad-hoc CLI invocations).
    ///
    /// Sends an `ActionInvoke` envelope to the (possibly already-spawned)
    /// plugin process, awaits the matching `ActionResult*` envelope, and
    /// returns the unwrapped output value.
    ///
    /// Production action execution in the engine still goes through the
    /// `SandboxRunner::execute` trait method, which wraps cancellation,
    /// metadata plumbing, and integration with `ActionRuntime`.
    pub async fn invoke(
        &self,
        action_key: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        let envelope = self.call_action(action_key, input, None).await?;
        action_response_to_value(envelope)
    }

    /// Core long-lived dispatch. Reuses the cached [`PluginHandle`] if any,
    /// spawns fresh otherwise.
    ///
    /// Retry policy (review feedback on #257): we retry **only** when the
    /// first attempt failed with [`TryDispatchError::Respawnable`] —
    /// concretely, [`SandboxError::PluginClosed`] (stale handle, plugin
    /// crashed between calls). Every other failure is terminal on the
    /// first attempt:
    ///
    /// - `Cancelled` is returned as-is to honour the engine's cancellation contract (re-sending
    ///   could duplicate a non-idempotent action after the caller gave up — see #257 review).
    /// - Timeout, protocol violations (`ResponseIdMismatch`, `PluginLineTooLarge`,
    ///   `HandshakeLineTooLarge`, `TransportPoisoned`, `MalformedEnvelope`), and I/O failures
    ///   mid-round-trip are treated as terminal because the stream position is undefined after a
    ///   partial write; a blind retry would risk duplicate side-effects on the plugin process.
    async fn dispatch_envelope(
        &self,
        envelope: HostToPlugin,
        cancel: Option<&CancellationToken>,
    ) -> Result<PluginToHost, SandboxError> {
        match self.try_dispatch(envelope.clone(), cancel).await {
            Ok(response) => Ok(response),
            Err(TryDispatchError::Respawnable(_)) => {
                // Stale handle — the plugin crashed or exited before we
                // sent this envelope. Respawning and resending is safe
                // because no bytes reached a running plugin process.
                *self.handle.lock().await = None;
                match self.try_dispatch(envelope, cancel).await {
                    Ok(response) => Ok(response),
                    Err(err) => Err(err.into_sandbox_error()),
                }
            },
            Err(err) => Err(err.into_sandbox_error()),
        }
    }

    /// Variant of [`dispatch_envelope`] that returns the response as raw
    /// JSON bytes (no strongly-typed `PluginToHost` parse).
    ///
    /// Used by [`get_metadata_raw`](Self::get_metadata_raw): lets the caller
    /// inspect `protocol_version` before committing to the typed parse,
    /// which otherwise fails with a confusing "missing field" message on
    /// a version-mismatched envelope.
    async fn dispatch_envelope_bytes(
        &self,
        envelope: HostToPlugin,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, SandboxError> {
        match self.try_dispatch_bytes(envelope.clone(), cancel).await {
            Ok(response) => Ok(response),
            Err(TryDispatchError::Respawnable(_)) => {
                *self.handle.lock().await = None;
                match self.try_dispatch_bytes(envelope, cancel).await {
                    Ok(response) => Ok(response),
                    Err(err) => Err(err.into_sandbox_error()),
                }
            },
            Err(err) => Err(err.into_sandbox_error()),
        }
    }

    async fn try_dispatch(
        &self,
        envelope: HostToPlugin,
        cancel: Option<&CancellationToken>,
    ) -> Result<PluginToHost, TryDispatchError> {
        let mut guard = self.handle.lock().await;
        if guard.is_none() {
            // Spawn failure before any bytes are on the wire — no
            // side-effect risk, but respawn-retrying a binary that just
            // failed to start almost never helps. Classify as terminal.
            let handle = spawn_and_dial(&self.binary, self.linux_rlimits)
                .await
                .map_err(TryDispatchError::Terminal)?;
            *guard = Some(handle);
        }
        let Some(handle) = guard.as_mut() else {
            // Unreachable in practice: we just set `*guard = Some(..)`
            // above. Prefer a typed error over `expect(..)` so a
            // hypothetical logic bug surfaces through the engine's
            // standard error path instead of panicking inside the
            // sandbox lock.
            return Err(TryDispatchError::Terminal(SandboxError::Spawn(
                String::from("process sandbox handle missing after successful spawn"),
            )));
        };

        // Round-trip the envelope with a per-call timeout AND a race
        // against the engine's cancellation token. Without the cancel
        // arm, a cancelled workflow would have to wait out `self.timeout`
        // on a hung or slow plugin before the engine could reclaim the
        // slot — see #257.
        let envelope_tag = match &envelope {
            HostToPlugin::ActionInvoke { .. } => "action_invoke",
            HostToPlugin::MetadataRequest { .. } => "metadata_request",
            _ => "other",
        };
        // Remember the outbound correlation id so we can validate the
        // response echoes it back (#285).
        let expected_id = request_id(&envelope);

        // The `bool` is `sent`: `false` if the failure happened at/before
        // the write (no bytes reached the plugin), `true` once the write
        // succeeded (the action may have run — no blind respawn-retry).
        let roundtrip = async {
            handle
                .send_envelope(&envelope)
                .await
                .map_err(|e| (false, e))?;
            handle.recv_envelope().await.map_err(|e| (true, e))
        };

        let outcome = race_cancel_timeout(roundtrip, self.timeout, cancel).await;

        match outcome {
            RaceOutcome::Ready(Ok(response)) => {
                if let (Some(expected), Some(got)) = (expected_id, response_id(&response))
                    && expected != got
                {
                    tracing::warn!(
                        plugin = %self.binary.display(),
                        envelope = %envelope_tag,
                        expected,
                        got,
                        "plugin response id mismatch — poisoning handle",
                    );
                    *guard = None;
                    // Protocol violation — must not retry. A stale
                    // response on a fresh connection is indistinguishable
                    // from an attacker replaying a prior reply, and a
                    // retry on a fresh handle would still send a fresh
                    // request the plugin may already have processed.
                    return Err(TryDispatchError::Terminal(
                        SandboxError::ResponseIdMismatch { expected, got },
                    ));
                }
                Ok(response)
            },
            RaceOutcome::Ready(Err((sent, sandbox_err))) => {
                // Transport/protocol error — invalidate the handle so the
                // next call respawns. Log PluginLineTooLarge at warn so it
                // shows up in security dashboards.
                if matches!(
                    sandbox_err,
                    SandboxError::PluginLineTooLarge { .. }
                        | SandboxError::HandshakeLineTooLarge { .. }
                        | SandboxError::TransportPoisoned
                ) {
                    tracing::warn!(
                        plugin = %self.binary.display(),
                        envelope = %envelope_tag,
                        error = %sandbox_err,
                        "plugin transport poisoned — clearing handle and forcing respawn",
                    );
                }
                *guard = None;
                Err(TryDispatchError::from_sandbox_error_after_send(
                    sandbox_err,
                    sent,
                ))
            },
            RaceOutcome::Timeout => {
                // Timeout — also invalidate; we don't know if the plugin is
                // still processing and we can't safely reuse the connection.
                // Classified as terminal: silently retrying after the
                // engine already gave up on this call would risk the
                // plugin running the action twice (#257 review).
                *guard = None;
                Err(TryDispatchError::Terminal(SandboxError::Timeout {
                    plugin: self.binary.display().to_string(),
                    envelope: envelope_tag.to_owned(),
                    timeout: self.timeout,
                }))
            },
            RaceOutcome::Cancelled => {
                // Cancellation observed mid-round-trip. We may have
                // written part of an envelope to the plugin; the stream
                // position is undefined, so drop the handle and force a
                // respawn on the next call. Surface as
                // [`SandboxError::Cancelled`] so the engine-side adapter
                // maps it to the canonical cancellation path — and
                // crucially do NOT retry (would duplicate work the engine
                // already asked us to abort; see #257 review).
                *guard = None;
                tracing::debug!(
                    plugin = %self.binary.display(),
                    envelope = %envelope_tag,
                    "plugin dispatch cancelled via CancellationToken; clearing handle",
                );
                Err(TryDispatchError::Terminal(SandboxError::Cancelled))
            },
        }
    }

    /// Variant of [`try_dispatch`](Self::try_dispatch) that reads the
    /// plugin's response as raw JSON bytes instead of parsing to
    /// `PluginToHost`.
    ///
    /// Identical transport / cancel / timeout semantics; only the inbound
    /// parse step differs. Correlation-id matching happens by parsing the
    /// bytes once into `serde_json::Value` solely to pull `.id`; the
    /// caller re-parses the bytes into the target type (see
    /// `dispatch_envelope_bytes` callers).
    async fn try_dispatch_bytes(
        &self,
        envelope: HostToPlugin,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, TryDispatchError> {
        let mut guard = self.handle.lock().await;
        if guard.is_none() {
            let handle = spawn_and_dial(&self.binary, self.linux_rlimits)
                .await
                .map_err(TryDispatchError::Terminal)?;
            *guard = Some(handle);
        }
        let Some(handle) = guard.as_mut() else {
            return Err(TryDispatchError::Terminal(SandboxError::Spawn(
                String::from("process sandbox handle missing after successful spawn"),
            )));
        };

        let envelope_tag = match &envelope {
            HostToPlugin::ActionInvoke { .. } => "action_invoke",
            HostToPlugin::MetadataRequest { .. } => "metadata_request",
            _ => "other",
        };
        let expected_id = request_id(&envelope);

        // `bool` is `sent` (see `try_dispatch`): false at/before write,
        // true once the write succeeded.
        let roundtrip = async {
            handle
                .send_envelope(&envelope)
                .await
                .map_err(|e| (false, e))?;
            handle.recv_envelope_bytes().await.map_err(|e| (true, e))
        };

        let outcome = race_cancel_timeout(roundtrip, self.timeout, cancel).await;

        match outcome {
            RaceOutcome::Ready(Ok(response_bytes)) => {
                // Cheap reparse to `Value` just for the id check; the
                // caller will parse the bytes again into the strongly-
                // typed target. Parsing twice is acceptable because the
                // metadata-probe path runs once per plugin lifetime.
                if let Some(expected) = expected_id {
                    // Use a match rather than `map_err(...)?` so we can
                    // clear `*guard` on parse failure before returning.
                    // A `?` return would skip the guard-clear and leave a
                    // cached-but-poisoned handle for the next call.
                    let value: serde_json::Value = match serde_json::from_slice(&response_bytes) {
                        Ok(v) => v,
                        Err(e) => {
                            *guard = None;
                            // Response bytes were received, so the action
                            // ran — terminal, never a silent resend.
                            return Err(TryDispatchError::from_sandbox_error_after_send(
                                SandboxError::MalformedEnvelope(e),
                                true,
                            ));
                        },
                    };
                    if let Some(got) = response_id_from_value(&value)
                        && expected != got
                    {
                        tracing::warn!(
                            plugin = %self.binary.display(),
                            envelope = %envelope_tag,
                            expected,
                            got,
                            "plugin response id mismatch — poisoning handle",
                        );
                        *guard = None;
                        return Err(TryDispatchError::Terminal(
                            SandboxError::ResponseIdMismatch { expected, got },
                        ));
                    }
                }
                Ok(response_bytes)
            },
            RaceOutcome::Ready(Err((sent, sandbox_err))) => {
                if matches!(
                    sandbox_err,
                    SandboxError::PluginLineTooLarge { .. }
                        | SandboxError::HandshakeLineTooLarge { .. }
                        | SandboxError::TransportPoisoned
                ) {
                    tracing::warn!(
                        plugin = %self.binary.display(),
                        envelope = %envelope_tag,
                        error = %sandbox_err,
                        "plugin transport poisoned — clearing handle and forcing respawn",
                    );
                }
                *guard = None;
                Err(TryDispatchError::from_sandbox_error_after_send(
                    sandbox_err,
                    sent,
                ))
            },
            RaceOutcome::Timeout => {
                *guard = None;
                Err(TryDispatchError::Terminal(SandboxError::Timeout {
                    plugin: self.binary.display().to_string(),
                    envelope: envelope_tag.to_owned(),
                    timeout: self.timeout,
                }))
            },
            RaceOutcome::Cancelled => {
                *guard = None;
                tracing::debug!(
                    plugin = %self.binary.display(),
                    envelope = %envelope_tag,
                    "plugin dispatch cancelled via CancellationToken; clearing handle",
                );
                Err(TryDispatchError::Terminal(SandboxError::Cancelled))
            },
        }
    }
}

/// Drop the cached handle on sandbox drop so the child is killed promptly.
///
/// `kill_on_drop(true)` on the spawned `Command` handles this at the OS
/// level — the destructor of `PluginHandle.child` sends SIGKILL. We add no
/// extra cleanup here; the `Arc<ProcessSandbox>` in the engine's handler
/// table owns the lifetime.
impl Drop for ProcessSandbox {
    fn drop(&mut self) {
        tracing::debug!(
            plugin = %self.binary.display(),
            "ProcessSandbox dropped; plugin child will be killed by kill_on_drop"
        );
    }
}

#[cfg(test)]
mod tests {
    //! Dispatch-level regression guards: the cancellation/timeout race
    //! (#257), monotonic correlation ids (#285), and the narrowed
    //! respawn-retry policy. The `SandboxError` → `ActionError`
    //! classification guards moved to `nebula-engine` with
    //! `sandbox_error_to_action_error` (the transport crate no longer
    //! knows about `ActionError`).

    use std::path::PathBuf;

    use super::*;

    // ---- race_cancel_timeout (#257 regression guard) -----------------

    #[tokio::test]
    async fn race_cancel_timeout_returns_ready_when_future_completes_first() {
        let fut = async { 42u32 };
        let outcome = race_cancel_timeout(fut, Duration::from_secs(1), None).await;
        assert_eq!(outcome, RaceOutcome::Ready(42));
    }

    #[tokio::test]
    async fn race_cancel_timeout_returns_timeout_without_cancel_arm() {
        // A future that never completes, no cancel token → must time out
        // within roughly the configured duration.
        let fut = std::future::pending::<()>();
        let outcome = race_cancel_timeout(fut, Duration::from_millis(25), None).await;
        assert_eq!(outcome, RaceOutcome::Timeout);
    }

    #[tokio::test]
    async fn race_cancel_timeout_observes_pre_cancelled_token_promptly() {
        // If the cancellation token is already cancelled when we enter
        // the race, the biased select must observe it on first poll
        // WITHOUT polling the inner future. This is the core fix for
        // #257: a cancelled workflow does not wait for `timeout`.
        let token = CancellationToken::new();
        token.cancel();

        let start = std::time::Instant::now();
        // `pending` future so the only way out is the cancel arm.
        let fut = std::future::pending::<()>();
        let outcome = race_cancel_timeout(fut, Duration::from_secs(30), Some(&token)).await;
        let elapsed = start.elapsed();

        assert_eq!(outcome, RaceOutcome::Cancelled);
        assert!(
            elapsed < Duration::from_millis(200),
            "pre-cancelled token must resolve promptly, took {elapsed:?}",
        );
    }

    #[tokio::test]
    async fn race_cancel_timeout_wins_when_cancel_fires_mid_flight() {
        // Future that never completes; fire the token shortly after the
        // race starts. The cancel arm must win, not the timeout.
        let token = CancellationToken::new();
        let cancel_clone = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel_clone.cancel();
        });

        let fut = std::future::pending::<()>();
        // Long enough timeout that the cancel arm is the one that wins.
        let outcome = race_cancel_timeout(fut, Duration::from_secs(30), Some(&token)).await;
        assert_eq!(outcome, RaceOutcome::Cancelled);
    }

    #[tokio::test]
    async fn race_cancel_timeout_ready_wins_over_never_fired_cancel() {
        // Token exists but is never cancelled → the inner future's
        // Ready result wins normally.
        let token = CancellationToken::new();
        let fut = async { "ok" };
        let outcome = race_cancel_timeout(fut, Duration::from_secs(1), Some(&token)).await;
        assert_eq!(outcome, RaceOutcome::Ready("ok"));
    }

    #[tokio::test]
    async fn race_cancel_timeout_timeout_wins_over_never_fired_cancel() {
        // Token exists but is never cancelled → deadline still applies.
        let token = CancellationToken::new();
        let fut = std::future::pending::<()>();
        let outcome = race_cancel_timeout(fut, Duration::from_millis(25), Some(&token)).await;
        assert_eq!(outcome, RaceOutcome::Timeout);
    }

    // ---- #285 monotonic-id + id-matching regression tests ------------

    #[test]
    fn next_envelope_id_is_monotonic_and_unique() {
        let sandbox = ProcessSandbox::new(PathBuf::from("/nonexistent"), Duration::from_secs(1));
        // Starts at 1 (not 0) so a default-zeroed response id is
        // visibly stale.
        let first = sandbox.next_envelope_id();
        let second = sandbox.next_envelope_id();
        let third = sandbox.next_envelope_id();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(third, 3);
    }

    // ---- #257 review: narrowed dispatch retry policy -----------------
    //
    // The `dispatch_envelope` retry must only fire for
    // [`SandboxError::PluginClosed`]. Retrying on `Cancelled`, `Timeout`,
    // protocol violations, or mid-round-trip transport errors could
    // double-invoke a non-idempotent action on the plugin side after the
    // engine already gave up on the call.

    #[test]
    fn eof_after_send_is_terminal_and_distinct_variant_by_type() {
        // PluginClosed observed by recv AFTER a successful send: the
        // plugin may have received and begun a non-idempotent action
        // before dying. Resending on a fresh process would
        // double-execute, so this MUST be terminal AND re-typed to the
        // distinct `PluginClosedAfterSend` variant so the no-resend
        // guarantee is structural across the crate boundary (the `sent`
        // bool does not cross it; the variant does).
        let tde = TryDispatchError::from_sandbox_error_after_send(SandboxError::PluginClosed, true);
        assert!(
            !tde.is_respawnable(),
            "EOF after a successful send must not silently resend the action, got {tde:?}",
        );
        match tde.into_sandbox_error() {
            SandboxError::PluginClosedAfterSend => {},
            other => panic!(
                "sent==true PluginClosed must re-type to PluginClosedAfterSend, got {other:?}"
            ),
        }
    }

    #[test]
    fn stale_handle_before_send_is_respawnable_and_keeps_plain_variant() {
        // PluginClosed with no bytes written for this attempt: nothing
        // reached a running plugin, so respawn-and-retry is safe and the
        // variant stays the plain `PluginClosed` (NOT re-typed) so the
        // safe pre-send respawn path is preserved.
        let tde =
            TryDispatchError::from_sandbox_error_after_send(SandboxError::PluginClosed, false);
        assert!(
            tde.is_respawnable(),
            "no bytes reached a running plugin — respawn must be allowed, got {tde:?}",
        );
        match tde.into_sandbox_error() {
            SandboxError::PluginClosed => {},
            other => panic!("!sent PluginClosed must stay PluginClosed, got {other:?}"),
        }
    }

    #[test]
    fn plugin_line_too_large_classifies_as_terminal() {
        // DoS / protocol-abuse signal — MUST NOT be retried. A retry
        // would simply respawn and forward another opportunity to abuse
        // the cap; the security dashboard would also see one warn per
        // attempt instead of a single clean failure.
        let tde = TryDispatchError::from_sandbox_error_after_send(
            SandboxError::PluginLineTooLarge {
                limit: 1024,
                observed: 2048,
            },
            true,
        );
        assert!(
            !tde.is_respawnable(),
            "PluginLineTooLarge must be Terminal (no retry), got {tde:?}",
        );
    }

    #[test]
    fn response_id_mismatch_classifies_as_terminal() {
        // Protocol violation: a stale response must poison the call
        // rather than silently retrying onto a fresh connection.
        let tde = TryDispatchError::from_sandbox_error_after_send(
            SandboxError::ResponseIdMismatch {
                expected: 42,
                got: 41,
            },
            true,
        );
        assert!(
            !tde.is_respawnable(),
            "ResponseIdMismatch must be Terminal, got {tde:?}",
        );
    }

    #[test]
    fn transport_poisoned_classifies_as_terminal() {
        let tde =
            TryDispatchError::from_sandbox_error_after_send(SandboxError::TransportPoisoned, true);
        assert!(
            !tde.is_respawnable(),
            "TransportPoisoned must be Terminal, got {tde:?}",
        );
    }

    #[test]
    fn handshake_line_too_large_classifies_as_terminal() {
        let tde = TryDispatchError::from_sandbox_error_after_send(
            SandboxError::HandshakeLineTooLarge {
                limit: 4096,
                observed: 8192,
            },
            true,
        );
        assert!(
            !tde.is_respawnable(),
            "HandshakeLineTooLarge must be Terminal, got {tde:?}",
        );
    }

    #[test]
    fn malformed_envelope_classifies_as_terminal() {
        // The plugin spoke but produced a non-envelope. The outer
        // handle is already dropped by try_dispatch; retrying would
        // spawn fresh and blindly resend — but the reviewer's concern
        // is that any envelope that reached the plugin may have had a
        // side effect. Classify terminal to match the general rule.
        let parse_err = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("fixture must produce serde_json::Error");
        let tde = TryDispatchError::from_sandbox_error_after_send(
            SandboxError::MalformedEnvelope(parse_err),
            true,
        );
        assert!(
            !tde.is_respawnable(),
            "MalformedEnvelope must be Terminal, got {tde:?}",
        );
    }

    #[test]
    fn host_malformed_envelope_classifies_as_terminal() {
        // Host-side serialize failure — no bytes on the wire, but no
        // point retrying a deterministic host bug.
        let parse_err = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("fixture must produce serde_json::Error");
        let tde = TryDispatchError::from_sandbox_error_after_send(
            SandboxError::HostMalformedEnvelope(parse_err),
            true,
        );
        assert!(
            !tde.is_respawnable(),
            "HostMalformedEnvelope must be Terminal, got {tde:?}",
        );
    }

    #[test]
    fn terminal_and_respawnable_into_sandbox_error_round_trip() {
        // Both classifications must carry the underlying SandboxError
        // through unmodified — the classification only governs the
        // dispatch-level retry decision, never the error surfaced to
        // the engine-side adapter.
        let respawn = TryDispatchError::Respawnable(SandboxError::PluginClosed);
        assert!(matches!(
            respawn.into_sandbox_error(),
            SandboxError::PluginClosed
        ));
        let terminal = TryDispatchError::Terminal(SandboxError::Cancelled);
        assert!(matches!(
            terminal.into_sandbox_error(),
            SandboxError::Cancelled
        ));
    }
}
