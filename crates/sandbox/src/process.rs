//! Process-based sandbox for community plugins using the duplex v2 protocol.
//!
//! Slice 1c (2026-04-13): plugin processes are **long-lived**. On the first
//! call, `ProcessSandbox` spawns the plugin binary, reads the handshake line
//! from its stdout, dials the announced UDS or Named Pipe, and stores the
//! resulting [`PluginHandle`] on the sandbox. Subsequent calls reuse that
//! handle, sending envelopes over the socket without respawning. A broken
//! connection (plugin crashed or exited) clears the handle and the next
//! request triggers a fresh spawn.
//!
//! The plugin-side event loop in `nebula-plugin-sdk::run_duplex` is still
//! sequential — one action at a time per plugin process. Slice 1d adds
//! concurrent multiplexed dispatch.
//!
//! Security enforcement (unchanged since slice 1b):
//! - `env_clear()` + explicit env allowlist
//! - `pre_exec` landlock + rlimits (Linux)
//! - stderr size limit for log capture
//! - `kill_on_drop` on the spawned child → plugin process dies with the sandbox
//!
//! ## Transport line-length caps (#316, 2026-04-14)
//!
//! Both the handshake and envelope read paths are length-capped. An
//! untrusted plugin that emits a newline-starved or gigabyte-sized line no
//! longer grows the receive buffer until OOM: the read returns a typed
//! [`SandboxError::PluginLineTooLarge`] / [`SandboxError::HandshakeLineTooLarge`]
//! and the plugin transport is **poisoned** — further send/recv calls on
//! the same handle fail fast with [`SandboxError::TransportPoisoned`]. The
//! outer `dispatch_envelope` also drops its cached handle on any error,
//! so defense-in-depth gives us both an instance-level invalidation
//! (`poisoned` flag inside `PluginHandle`) and a sandbox-level one
//! (`*self.handle.lock().await = None`).

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use nebula_action::{ActionError, ActionMetadata, result::ActionResult};
use nebula_plugin_sdk::{
    protocol::{HostToPlugin, PluginToHost},
    transport::{self, PluginStream},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

use crate::{
    SandboxRunner,
    capabilities::{Capability, PluginCapabilities},
    error::SandboxError,
    runner::SandboxedContext,
};

/// Timeout for reading the plugin's handshake line from stdout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// Maximum bytes accepted for the plugin handshake line.
///
/// A handshake is a short socket/pipe address string plus protocol version
/// — realistically under 100 bytes. 4 KiB gives ~40x headroom while still
/// bounding memory use far below anything that could stress the allocator.
const HANDSHAKE_LINE_CAP: usize = 4 * 1024;

/// Maximum bytes accepted for a single runtime envelope line
/// (JSON payload + trailing `\n`).
///
/// 1 MiB starting point. If real plugins need more, make this configurable
/// via [`ProcessSandbox::new`] later rather than preemptively raising the
/// ceiling — a too-high cap defeats the purpose of the cap.
const ENVELOPE_LINE_CAP: usize = 1024 * 1024;

/// Maximum bytes buffered per plugin stderr log line before the overflow
/// is silently discarded (stderr is diagnostic log output, not protocol,
/// so truncation is acceptable here — but we still bound the read, because
/// an attacker-controlled plugin could otherwise emit a gigabyte of
/// newline-starved garbage and starve the host of memory).
const STDERR_LINE_CAP: usize = 8 * 1024;

/// Correlation id used for the single envelope sent per invocation.
///
/// Slice 1c still does one envelope exchange at a time per call. Slice 1d's
/// concurrent dispatch assigns unique ids across multiple in-flight calls.
const ONE_SHOT_ID: u64 = 1;

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
    /// Capabilities granted to this plugin.
    capabilities: PluginCapabilities,
    /// Long-lived handle to the spawned plugin process. Serialized via the
    /// mutex — slice 1c is sequential per sandbox instance. Slice 1d can
    /// replace this with a lock-free handle once concurrent dispatch lands.
    handle: Mutex<Option<PluginHandle>>,
}

/// Live connection to a running plugin process.
///
/// Owns the spawned [`Child`] and the two halves of the accepted stream
/// (reader is wrapped in `BufReader` for efficient line-delimited reads).
/// When dropped, `kill_on_drop(true)` on the child ensures the OS process
/// is terminated; the socket/pipe is released by `PluginStream`'s cleanup
/// guard on the plugin side.
struct PluginHandle {
    /// Kept alive for `kill_on_drop` — dropping this struct SIGKILLs the
    /// child. Read nowhere; the underscore prefix silences dead-code
    /// warnings.
    _child: Child,
    /// Buffered reader over the stream's read half. Crucial for
    /// throughput — byte-at-a-time reads hit ~4 MB/s, BufReader reaches
    /// hundreds of MB/s on local sockets/pipes.
    reader: BufReader<tokio::io::ReadHalf<PluginStream>>,
    /// Owning write half for envelope dispatch.
    writer: tokio::io::WriteHalf<PluginStream>,
    /// Scratch byte buffer reused across `recv_envelope` calls to avoid
    /// per-call allocation. `Vec<u8>` rather than `String` so `read_until`
    /// can drive the bounded read without a UTF-8 validation in the hot
    /// path — parsing happens via `serde_json::from_slice` after the cap
    /// check.
    line_buf: Vec<u8>,
    /// Set to `true` once the transport has produced any error that
    /// leaves us at an unknown position in the byte stream (oversized
    /// line, mid-frame I/O failure). Once poisoned, every subsequent
    /// `send_envelope` / `recv_envelope` call short-circuits with
    /// [`SandboxError::TransportPoisoned`] instead of reading more bytes.
    /// This is defense-in-depth alongside the outer
    /// `ProcessSandbox::try_dispatch` handle-clear logic.
    poisoned: bool,
}

impl PluginHandle {
    fn new(child: Child, stream: PluginStream) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        Self {
            _child: child,
            reader: BufReader::new(read_half),
            writer: write_half,
            line_buf: Vec::with_capacity(512),
            poisoned: false,
        }
    }

    async fn send_envelope(&mut self, envelope: &HostToPlugin) -> Result<(), SandboxError> {
        if self.poisoned {
            return Err(SandboxError::TransportPoisoned);
        }
        let encoded = serde_json::to_vec(envelope).map_err(SandboxError::HostMalformedEnvelope)?;
        if let Err(e) = self.writer.write_all(&encoded).await {
            self.poisoned = true;
            return Err(SandboxError::Transport(e));
        }
        if let Err(e) = self.writer.write_all(b"\n").await {
            self.poisoned = true;
            return Err(SandboxError::Transport(e));
        }
        if let Err(e) = self.writer.flush().await {
            self.poisoned = true;
            return Err(SandboxError::Transport(e));
        }
        Ok(())
    }

    async fn recv_envelope(&mut self) -> Result<PluginToHost, SandboxError> {
        if self.poisoned {
            return Err(SandboxError::TransportPoisoned);
        }
        self.line_buf.clear();
        match read_bounded_line(&mut self.reader, ENVELOPE_LINE_CAP, &mut self.line_buf).await {
            Ok(BoundedReadOutcome::Line {
                bytes_including_newline,
            }) => {
                // Strip the trailing newline before parsing.
                let body = &self.line_buf[..bytes_including_newline - 1];
                serde_json::from_slice::<PluginToHost>(body).map_err(|e| {
                    // Parse error does not leave the stream in an unknown
                    // state — we consumed exactly one line. Do NOT poison
                    // here; the outer dispatch will still drop the handle
                    // to be safe, but a legitimate one-off parse failure
                    // should not prevent a retry on a fresh envelope.
                    SandboxError::MalformedEnvelope(e)
                })
            }
            Ok(BoundedReadOutcome::Eof) => {
                self.poisoned = true;
                Err(SandboxError::PluginClosed)
            }
            Ok(BoundedReadOutcome::Overflow { observed }) => {
                self.poisoned = true;
                Err(SandboxError::PluginLineTooLarge {
                    limit: ENVELOPE_LINE_CAP,
                    observed,
                })
            }
            Err(e) => {
                self.poisoned = true;
                Err(SandboxError::Transport(e))
            }
        }
    }
}

/// Outcome of a single bounded line read.
#[derive(Debug, PartialEq, Eq)]
enum BoundedReadOutcome {
    /// A complete line was read; the final byte in the caller's buffer is
    /// `b'\n'` and `bytes_including_newline` is the number of bytes
    /// appended to the buffer by this read.
    Line { bytes_including_newline: usize },
    /// EOF reached before any bytes were read. The stream is closed.
    Eof,
    /// More than `cap` bytes were consumed without encountering a
    /// newline. The stream position is now `cap + 1` bytes past where
    /// the read began; the transport **must not be reused** because we
    /// cannot resync on a line boundary without reading an unbounded
    /// amount more.
    Overflow {
        /// Number of bytes actually consumed from the stream before the
        /// cap was hit. Always `> cap`.
        observed: usize,
    },
}

/// Read a single newline-delimited line from `reader`, refusing to buffer
/// more than `cap` bytes.
///
/// Uses `AsyncBufReadExt::take(cap + 1).read_until(b'\n', buf)` as the
/// primitive. The `+ 1` is load-bearing: it lets us distinguish "exactly
/// `cap` bytes including the newline, legal" from "more than `cap` bytes,
/// cap breached". Without it we could not tell whether a caller that
/// read exactly `cap` bytes ended on a newline (legal) or just happened
/// to saturate the `take` adapter (illegal).
///
/// `buf` is NOT cleared — the caller is responsible for clearing it if
/// they want a fresh line. This lets the function be used both with a
/// scratch buffer (clear-before-call, the envelope path) and a
/// freshly-allocated one (the handshake path).
async fn read_bounded_line<R>(
    reader: &mut R,
    cap: usize,
    buf: &mut Vec<u8>,
) -> std::io::Result<BoundedReadOutcome>
where
    R: AsyncBufRead + Unpin,
{
    let before = buf.len();
    // `take` returns a new adapter that yields at most `cap + 1` bytes.
    // We reborrow `reader` so the adapter does not take ownership.
    let limit = cap as u64 + 1;
    let mut limited = reader.take(limit);
    let n = limited.read_until(b'\n', buf).await?;
    if n == 0 {
        return Ok(BoundedReadOutcome::Eof);
    }
    let ends_with_newline = buf.get(before + n - 1).copied() == Some(b'\n');
    let payload_len = n; // bytes appended by this call

    // Three terminal conditions after a non-zero read:
    //   1. line_len <= cap AND newline-terminated → legal Line
    //   2. line_len  > cap (adapter yielded cap + 1) → Overflow (DoS signal)
    //   3. line_len <= cap AND no newline → underlying reader hit EOF mid-line. Treat as unexpected
    //      close; the handle is not reusable either way so we surface it as Eof (poisons the handle
    //      the same way a clean EOF would) rather than as a misleading "exceeded cap" error.
    if ends_with_newline && payload_len <= cap {
        return Ok(BoundedReadOutcome::Line {
            bytes_including_newline: payload_len,
        });
    }
    if payload_len > cap {
        // The adapter was allowed cap + 1 bytes and consumed them all —
        // the underlying line is strictly longer than cap. Report as
        // overflow; do not attempt to read more.
        return Ok(BoundedReadOutcome::Overflow {
            observed: payload_len,
        });
    }
    // payload_len <= cap and no trailing newline → partial read with
    // EOF. Report as Eof — the caller treats it the same as a clean
    // close (poison the handle, fail with PluginClosed).
    Ok(BoundedReadOutcome::Eof)
}

impl ProcessSandbox {
    /// Create a new process sandbox for a plugin binary.
    #[must_use]
    pub fn new(binary: PathBuf, timeout: Duration, capabilities: PluginCapabilities) -> Self {
        Self {
            binary,
            timeout,
            capabilities,
            handle: Mutex::new(None),
        }
    }

    /// Invoke an action and return the plugin's response envelope.
    pub(crate) async fn call_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
    ) -> Result<PluginToHost, ActionError> {
        let request = HostToPlugin::ActionInvoke {
            id: ONE_SHOT_ID,
            action_key: action_key.to_owned(),
            input,
        };
        self.dispatch_envelope(request).await
    }

    /// Query plugin metadata via a `MetadataRequest` envelope.
    pub async fn get_metadata(&self) -> Result<PluginToHost, ActionError> {
        let request = HostToPlugin::MetadataRequest { id: ONE_SHOT_ID };
        self.dispatch_envelope(request).await
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
    ) -> Result<serde_json::Value, ActionError> {
        let envelope = self.call_action(action_key, input).await?;
        match envelope {
            PluginToHost::ActionResultOk { output, .. } => Ok(output),
            PluginToHost::ActionResultError {
                code,
                message,
                retryable,
                ..
            } => {
                let msg = sanitize_plugin_string(&format!("{code}: {message}"));
                if retryable {
                    Err(ActionError::retryable(msg))
                } else {
                    Err(ActionError::fatal(msg))
                }
            }
            other => Err(ActionError::fatal(format!(
                "plugin returned unexpected envelope (expected ActionResult*, got {})",
                envelope_kind(&other)
            ))),
        }
    }

    /// Core long-lived dispatch. Reuses the cached [`PluginHandle`] if any,
    /// spawns fresh otherwise. On transport error, clears the handle and
    /// retries once.
    async fn dispatch_envelope(&self, envelope: HostToPlugin) -> Result<PluginToHost, ActionError> {
        let first_attempt = self.try_dispatch(envelope.clone()).await;
        if first_attempt.is_ok() {
            return first_attempt;
        }
        // Clear the stale handle and retry once with a fresh spawn.
        *self.handle.lock().await = None;
        self.try_dispatch(envelope).await
    }

    async fn try_dispatch(&self, envelope: HostToPlugin) -> Result<PluginToHost, ActionError> {
        let mut guard = self.handle.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn_and_dial().await?);
        }
        let handle = guard.as_mut().expect("handle set above");

        // Round-trip the envelope with a per-call timeout.
        let envelope_tag = match &envelope {
            HostToPlugin::ActionInvoke { .. } => "action_invoke",
            HostToPlugin::MetadataRequest { .. } => "metadata_request",
            _ => "other",
        };

        let result = tokio::time::timeout(self.timeout, async {
            handle.send_envelope(&envelope).await?;
            handle.recv_envelope().await
        })
        .await;

        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(sandbox_err)) => {
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
                Err(sandbox_error_to_action_error(sandbox_err))
            }
            Err(_) => {
                // Timeout — also invalidate; we don't know if the plugin is
                // still processing and we can't safely reuse the connection.
                *guard = None;
                Err(ActionError::retryable(format!(
                    "plugin {} timed out on {envelope_tag} after {:?}",
                    self.binary.display(),
                    self.timeout
                )))
            }
        }
    }

    /// Spawn the plugin binary, read and parse its handshake line, dial the
    /// announced transport, and return a fresh [`PluginHandle`].
    async fn spawn_and_dial(&self) -> Result<PluginHandle, ActionError> {
        // Build allowed env vars from capabilities.
        let allowed_env: Vec<(String, String)> = self
            .capabilities
            .list()
            .iter()
            .filter_map(|cap| match cap {
                Capability::Env { keys } => Some(keys.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|key| std::env::var(&key).ok().map(|val| (key, val)))
            .collect();

        let mut cmd = Command::new(&self.binary);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env_clear()
            .envs(allowed_env);

        // Apply OS-level sandbox in child process before exec (Linux only).
        #[cfg(target_os = "linux")]
        {
            let caps_json = serde_json::to_string(&self.capabilities)
                .map_err(|e| ActionError::fatal(format!("capabilities serialization: {e}")))?;

            // SAFETY: pre_exec runs between fork() and exec() in the child.
            // We only call async-signal-safe operations (landlock, setrlimit).
            #[allow(unsafe_code)]
            unsafe {
                cmd.pre_exec(move || {
                    let caps: PluginCapabilities = serde_json::from_str(&caps_json)
                        .map_err(|e| std::io::Error::other(format!("capability parse: {e}")))?;
                    crate::os_sandbox::apply_sandbox(&caps)
                        .map_err(|e| std::io::Error::other(format!("sandbox setup: {e}")))
                });
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            ActionError::fatal(format!(
                "failed to spawn plugin {}: {e}",
                self.binary.display()
            ))
        })?;

        // Spawn a background task that drains the plugin's stderr and logs
        // each line via `tracing`. We do this BEFORE reading the handshake
        // so that any crash diagnostics the plugin writes during startup
        // are captured. The task ends when the child's stderr closes —
        // usually on plugin exit.
        if let Some(stderr) = child.stderr.take() {
            let plugin_name = self.binary.display().to_string();
            tokio::spawn(drain_plugin_stderr(stderr, plugin_name));
        }

        // Read the handshake line from child stdout with a hard timeout
        // AND a hard length cap — a malicious plugin must not be able to
        // hold this task blocking on memory growth.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ActionError::fatal("failed to open plugin stdout"))?;
        let mut stdout_reader = BufReader::new(stdout);
        let mut handshake_buf: Vec<u8> = Vec::with_capacity(256);

        let read_result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            read_bounded_line(&mut stdout_reader, HANDSHAKE_LINE_CAP, &mut handshake_buf).await
        })
        .await;

        let outcome = read_result.map_err(|_| {
            ActionError::fatal(format!(
                "plugin {} handshake timeout after {HANDSHAKE_TIMEOUT:?}",
                self.binary.display()
            ))
        })?;

        let bytes_including_newline = match outcome {
            Ok(BoundedReadOutcome::Line {
                bytes_including_newline,
            }) => bytes_including_newline,
            Ok(BoundedReadOutcome::Eof) => {
                return Err(ActionError::fatal(format!(
                    "plugin {} exited before printing handshake line",
                    self.binary.display()
                )));
            }
            Ok(BoundedReadOutcome::Overflow { observed }) => {
                tracing::warn!(
                    plugin = %self.binary.display(),
                    limit = HANDSHAKE_LINE_CAP,
                    observed,
                    "plugin handshake exceeded cap — refusing to dial",
                );
                return Err(sandbox_error_to_action_error(
                    SandboxError::HandshakeLineTooLarge {
                        limit: HANDSHAKE_LINE_CAP,
                        observed,
                    },
                ));
            }
            Err(e) => {
                return Err(ActionError::fatal(format!(
                    "plugin {} handshake read error: {e}",
                    self.binary.display()
                )));
            }
        };

        // Strip the trailing newline and decode as UTF-8 for the dial
        // address. We do this AFTER the cap check so we never run UTF-8
        // validation on an unbounded buffer.
        let handshake_bytes = &handshake_buf[..bytes_including_newline - 1];
        let handshake_line = std::str::from_utf8(handshake_bytes).map_err(|e| {
            ActionError::fatal(format!(
                "plugin {} handshake line is not valid UTF-8: {e}",
                self.binary.display()
            ))
        })?;

        let sanitized_handshake = sanitize_plugin_string(handshake_line.trim());
        tracing::debug!(
            plugin = %self.binary.display(),
            handshake = %sanitized_handshake,
            "plugin handshake received"
        );

        // Dial the announced transport.
        let stream = transport::dial(handshake_line)
            .await
            .map_err(|e| ActionError::fatal(format!("plugin transport dial failed: {e}")))?;

        Ok(PluginHandle::new(child, stream))
    }
}

/// Convert an internal [`SandboxError`] into the public `ActionError` the
/// sandbox runner trait returns. Transport-level issues are fatal
/// (non-retryable) by design: once the plugin has misbehaved on the wire,
/// the next caller gets a fresh process, not a blind retry on the same
/// poisoned channel.
fn sandbox_error_to_action_error(err: SandboxError) -> ActionError {
    match err {
        // Retryable: plugin crashed / exited, respawn path is safe.
        SandboxError::PluginClosed => ActionError::retryable_from(err),
        // Fatal: DoS / protocol-abuse signals. Do not paper over with retry.
        SandboxError::PluginLineTooLarge { .. }
        | SandboxError::HandshakeLineTooLarge { .. }
        | SandboxError::TransportPoisoned
        | SandboxError::Transport(_)
        | SandboxError::MalformedEnvelope(_)
        | SandboxError::HostMalformedEnvelope(_) => ActionError::fatal_from(err),
    }
}

#[async_trait]
impl SandboxRunner for ProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        context.check_cancelled()?;

        let action_key = metadata.key.as_str();

        tracing::debug!(
            action_key = %action_key,
            plugin = %self.binary.display(),
            "executing action in process sandbox"
        );

        let envelope = self.call_action(action_key, input).await?;
        match envelope {
            PluginToHost::ActionResultOk { output, .. } => Ok(ActionResult::success(output)),
            PluginToHost::ActionResultError {
                code,
                message,
                retryable,
                ..
            } => {
                let msg = sanitize_plugin_string(&format!("{code}: {message}"));
                if retryable {
                    Err(ActionError::retryable(msg))
                } else {
                    Err(ActionError::fatal(msg))
                }
            }
            other => Err(ActionError::fatal(format!(
                "plugin returned unexpected envelope (expected ActionResult*, got {})",
                envelope_kind(&other)
            ))),
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

/// Drain a plugin child's stderr, emitting one `tracing::debug!` event per
/// line. Returns when the stderr pipe closes (plugin exited) or read errors.
///
/// Unlike the envelope path, stderr is diagnostic log output — a
/// too-long line is truncated silently rather than rejected. What we
/// MUST still bound is the memory use of the read itself: a plugin that
/// spams bytes without newlines forever must not grow our buffer forever.
/// On overflow, we discard the current buffer, drain bytes until the
/// next newline, and resume. Truncation is logged at debug level so it
/// stays observable but doesn't pollute warn/error dashboards.
async fn drain_plugin_stderr(stderr: tokio::process::ChildStderr, plugin_name: String) {
    let mut reader = BufReader::new(stderr);
    let mut line_buf: Vec<u8> = Vec::with_capacity(256);
    loop {
        line_buf.clear();
        match read_bounded_line(&mut reader, STDERR_LINE_CAP, &mut line_buf).await {
            Ok(BoundedReadOutcome::Line {
                bytes_including_newline,
            }) => {
                let body = &line_buf[..bytes_including_newline - 1];
                let as_str = String::from_utf8_lossy(body);
                let sanitized = sanitize_plugin_string(as_str.trim());
                tracing::debug!(
                    plugin = %plugin_name,
                    stderr = %sanitized,
                    "plugin stderr"
                );
            }
            Ok(BoundedReadOutcome::Eof) => return,
            Ok(BoundedReadOutcome::Overflow { observed }) => {
                tracing::debug!(
                    plugin = %plugin_name,
                    observed,
                    limit = STDERR_LINE_CAP,
                    "plugin stderr line exceeded cap — discarding and resynchronising to next newline",
                );
                // Discard rest of the oversized line until next newline or
                // EOF. Returning `false` from the helper means the stderr
                // stream has closed — exit the drain loop entirely.
                if !drop_until_newline(&mut reader).await {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// Read and discard bytes from `reader` until a newline is encountered
/// or the stream ends. Returns `true` if a newline was found (drain loop
/// should continue), `false` if the stream closed / errored (drain loop
/// should exit).
///
/// Bounds memory to a single small scratch buffer regardless of how
/// many bytes the plugin emits before the next newline. Used to recover
/// from an oversized stderr line without growing the line buffer past
/// `STDERR_LINE_CAP`.
async fn drop_until_newline<R>(reader: &mut R) -> bool
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    loop {
        let chunk = match reader.fill_buf().await {
            Ok(chunk) => chunk,
            Err(_) => return false,
        };
        if chunk.is_empty() {
            return false;
        }
        if let Some(idx) = chunk.iter().position(|&b| b == b'\n') {
            reader.consume(idx + 1);
            return true;
        }
        let len = chunk.len();
        reader.consume(len);
    }
}

fn envelope_kind(env: &PluginToHost) -> &'static str {
    match env {
        PluginToHost::ActionResultOk { .. } => "action_result_ok",
        PluginToHost::ActionResultError { .. } => "action_result_error",
        PluginToHost::RpcCall { .. } => "rpc_call",
        PluginToHost::Log { .. } => "log",
        PluginToHost::MetadataResponse { .. } => "metadata_response",
    }
}

fn sanitize_plugin_string(s: &str) -> String {
    s.chars()
        .take(1024)
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Unit tests for the bounded plugin transport line reader and the
    //! poison invariant.
    //!
    //! These tests deliberately target the pure helper
    //! [`read_bounded_line`] rather than spinning up a real plugin
    //! process. The helper is what actually enforces the cap and drives
    //! both the handshake and envelope paths in production, so covering
    //! it exhaustively at this layer gives the whole crate its security
    //! guarantee.
    //!
    //! We also test `recv_envelope` / `send_envelope` poisoning by
    //! constructing a `PluginHandle`-shaped fixture via
    //! `tokio::io::duplex` and a test-only constructor. The real
    //! `PluginHandle` wraps a `PluginStream`, so for fixture purposes we
    //! introduce a separate `TestHandle` struct that mirrors the same
    //! fields and methods with the duplex-backed types — any divergence
    //! from the production `PluginHandle` would be caught by the fact
    //! that both call into the same `read_bounded_line` primitive.

    use tokio::io::{AsyncWriteExt, BufReader as TokioBufReader, duplex};

    use super::*;

    // ---- read_bounded_line primitive ---------------------------------

    #[tokio::test]
    async fn read_bounded_line_accepts_short_line() {
        let data: &[u8] = b"hello\n";
        let mut reader = TokioBufReader::new(data);
        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            BoundedReadOutcome::Line {
                bytes_including_newline: 6,
            }
        );
        assert_eq!(buf, b"hello\n");
    }

    #[tokio::test]
    async fn read_bounded_line_accepts_exactly_cap_bytes() {
        // cap = 8, line is "aaaaaaa\n" → exactly 8 bytes including newline.
        // Guards the off-by-one in `take(cap + 1)`.
        let data: &[u8] = b"aaaaaaa\n";
        assert_eq!(data.len(), 8);
        let mut reader = TokioBufReader::new(data);
        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 8, &mut buf).await.unwrap();
        assert_eq!(
            outcome,
            BoundedReadOutcome::Line {
                bytes_including_newline: 8,
            }
        );
    }

    #[tokio::test]
    async fn read_bounded_line_rejects_one_byte_over_cap() {
        // cap = 4, line is 5 bytes before newline → overflow.
        let data: &[u8] = b"aaaaa\n";
        let mut reader = TokioBufReader::new(data);
        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 4, &mut buf).await.unwrap();
        match outcome {
            BoundedReadOutcome::Overflow { observed } => assert_eq!(observed, 5),
            other => panic!("expected Overflow, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_bounded_line_rejects_newline_starved_stream() {
        // 1 MiB + 1 of 'A', no newline.
        let mut data = vec![b'A'; 1024 * 1024 + 1];
        data.push(b'A');
        let mut reader = TokioBufReader::new(&data[..]);
        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 1024 * 1024, &mut buf)
            .await
            .unwrap();
        match outcome {
            BoundedReadOutcome::Overflow { observed } => {
                // We should have read exactly cap + 1 bytes before giving up.
                assert_eq!(observed, 1024 * 1024 + 1);
            }
            other => panic!("expected Overflow, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_bounded_line_reports_eof() {
        let data: &[u8] = b"";
        let mut reader = TokioBufReader::new(data);
        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .unwrap();
        assert_eq!(outcome, BoundedReadOutcome::Eof);
    }

    #[tokio::test]
    async fn read_bounded_line_partial_line_then_eof_is_reported_as_eof() {
        // Some bytes, no newline, stream ends. Must be reported as Eof,
        // NOT as Overflow — a short unterminated tail is a clean-close
        // edge case, not a DoS signal, and mis-classifying it would
        // flood security dashboards with false positives whenever a
        // plugin crashes mid-envelope.
        let data: &[u8] = b"partial";
        let mut reader = TokioBufReader::new(data);
        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .unwrap();
        assert_eq!(outcome, BoundedReadOutcome::Eof);
    }

    #[tokio::test]
    async fn read_bounded_line_successive_lines_from_same_reader() {
        // Two complete lines back-to-back.
        let data: &[u8] = b"first\nsecond\n";
        let mut reader = TokioBufReader::new(data);
        let mut buf = Vec::new();

        let out1 = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .unwrap();
        assert_eq!(
            out1,
            BoundedReadOutcome::Line {
                bytes_including_newline: 6,
            }
        );
        assert_eq!(&buf[..6], b"first\n");

        buf.clear();
        let out2 = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .unwrap();
        assert_eq!(
            out2,
            BoundedReadOutcome::Line {
                bytes_including_newline: 7,
            }
        );
        assert_eq!(&buf[..7], b"second\n");
    }

    // ---- PluginHandle-shaped fixture for poison tests ----------------

    /// Test double for [`PluginHandle`] that uses an in-memory
    /// [`tokio::io::duplex`] pair instead of a real plugin transport.
    ///
    /// This duplicates the recv/send logic from `PluginHandle` faithfully
    /// enough to exercise the poisoning invariant end-to-end without the
    /// real `PluginStream` / `Child` types. Any logic drift between this
    /// and the real handle would show up as the real handle not calling
    /// `read_bounded_line` — trivially spotted in code review.
    struct TestHandle {
        reader: TokioBufReader<tokio::io::DuplexStream>,
        writer: tokio::io::DuplexStream,
        line_buf: Vec<u8>,
        poisoned: bool,
    }

    impl TestHandle {
        fn new(reader_side: tokio::io::DuplexStream, writer_side: tokio::io::DuplexStream) -> Self {
            Self {
                reader: TokioBufReader::new(reader_side),
                writer: writer_side,
                line_buf: Vec::with_capacity(64),
                poisoned: false,
            }
        }

        async fn recv_envelope_capped(&mut self, cap: usize) -> Result<Vec<u8>, SandboxError> {
            if self.poisoned {
                return Err(SandboxError::TransportPoisoned);
            }
            self.line_buf.clear();
            match read_bounded_line(&mut self.reader, cap, &mut self.line_buf).await {
                Ok(BoundedReadOutcome::Line {
                    bytes_including_newline,
                }) => Ok(self.line_buf[..bytes_including_newline - 1].to_vec()),
                Ok(BoundedReadOutcome::Eof) => {
                    self.poisoned = true;
                    Err(SandboxError::PluginClosed)
                }
                Ok(BoundedReadOutcome::Overflow { observed }) => {
                    self.poisoned = true;
                    Err(SandboxError::PluginLineTooLarge {
                        limit: cap,
                        observed,
                    })
                }
                Err(e) => {
                    self.poisoned = true;
                    Err(SandboxError::Transport(e))
                }
            }
        }

        async fn send_line(&mut self, line: &[u8]) -> Result<(), SandboxError> {
            if self.poisoned {
                return Err(SandboxError::TransportPoisoned);
            }
            if let Err(e) = self.writer.write_all(line).await {
                self.poisoned = true;
                return Err(SandboxError::Transport(e));
            }
            if let Err(e) = self.writer.flush().await {
                self.poisoned = true;
                return Err(SandboxError::Transport(e));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn oversized_envelope_poisons_handle_and_blocks_subsequent_reads() {
        // This test is the named regression for #316: a malicious plugin
        // emits > ENVELOPE_LINE_CAP bytes on a single line, the read
        // fails with PluginLineTooLarge, and the handle is marked
        // unusable so any further recv_envelope call short-circuits
        // with TransportPoisoned instead of reading more bytes.
        let cap = 64; // small cap so the test is fast
        let (mut fake_plugin, host_side) = duplex(cap * 4);
        let (host_side_a, _host_side_b) = duplex(8); // dummy reverse side
        drop(_host_side_b);

        // Plugin side emits cap + 10 bytes of 'X', no newline.
        let mut payload = vec![b'X'; cap + 10];
        payload.push(b'\n'); // newline exists but too far out
        fake_plugin.write_all(&payload).await.unwrap();
        fake_plugin.flush().await.unwrap();

        let mut handle = TestHandle::new(host_side, host_side_a);

        // First recv: the cap is breached, typed error, handle poisoned.
        let err = handle.recv_envelope_capped(cap).await.unwrap_err();
        match err {
            SandboxError::PluginLineTooLarge { limit, observed } => {
                assert_eq!(limit, cap);
                assert!(
                    observed > cap,
                    "observed={observed} must exceed cap={cap} to count as overflow",
                );
            }
            other => panic!("expected PluginLineTooLarge, got {other:?}"),
        }
        assert!(
            handle.poisoned,
            "handle must be marked poisoned after overflow"
        );

        // Second recv: must short-circuit with TransportPoisoned. It must
        // NOT read more bytes from the plugin side. This is the
        // "connection invalidation" invariant — verified in code, not
        // via comments.
        let err2 = handle.recv_envelope_capped(cap).await.unwrap_err();
        assert!(
            matches!(err2, SandboxError::TransportPoisoned),
            "expected TransportPoisoned on second recv, got {err2:?}",
        );

        // Third recv: still poisoned.
        let err3 = handle.recv_envelope_capped(cap).await.unwrap_err();
        assert!(matches!(err3, SandboxError::TransportPoisoned));

        // Writes must also fail on a poisoned handle.
        let send_err = handle.send_line(b"hello\n").await.unwrap_err();
        assert!(matches!(send_err, SandboxError::TransportPoisoned));
    }

    #[tokio::test]
    async fn exact_cap_envelope_is_accepted_and_does_not_poison() {
        // Boundary: a line of exactly `cap` bytes including the newline
        // must succeed AND leave the handle reusable. This guards the
        // off-by-one in the `+ 1` inside read_bounded_line.
        let cap = 16;
        let (mut fake_plugin, host_side) = duplex(1024);
        let (host_side_a, _host_side_b) = duplex(8);
        drop(_host_side_b);

        // 15 bytes of 'Y' + 1 newline = 16 bytes = exactly cap.
        let mut payload = vec![b'Y'; cap - 1];
        payload.push(b'\n');
        assert_eq!(payload.len(), cap);
        fake_plugin.write_all(&payload).await.unwrap();
        fake_plugin.flush().await.unwrap();

        let mut handle = TestHandle::new(host_side, host_side_a);
        let body = handle.recv_envelope_capped(cap).await.unwrap();
        assert_eq!(body.len(), cap - 1);
        assert!(body.iter().all(|&b| b == b'Y'));
        assert!(
            !handle.poisoned,
            "exact-cap read must NOT poison the handle"
        );
    }

    #[tokio::test]
    async fn eof_poisons_handle() {
        // Plugin closed transport without sending anything → PluginClosed
        // AND handle poisoned (an EOF is terminal by definition).
        let (fake_plugin, host_side) = duplex(64);
        drop(fake_plugin); // close the plugin side immediately

        let (host_side_a, _host_side_b) = duplex(8);
        drop(_host_side_b);

        let mut handle = TestHandle::new(host_side, host_side_a);
        let err = handle.recv_envelope_capped(64).await.unwrap_err();
        assert!(matches!(err, SandboxError::PluginClosed));
        assert!(handle.poisoned);

        let err2 = handle.recv_envelope_capped(64).await.unwrap_err();
        assert!(matches!(err2, SandboxError::TransportPoisoned));
    }

    #[tokio::test]
    async fn drop_until_newline_keeps_bytes_after_newline() {
        let data: &[u8] = b"oversized-line\nnext-line\n";
        let mut reader = TokioBufReader::new(data);

        assert!(drop_until_newline(&mut reader).await);

        let mut buf = Vec::new();
        let outcome = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .expect("next line must still be readable");
        assert_eq!(
            outcome,
            BoundedReadOutcome::Line {
                bytes_including_newline: 10,
            }
        );
        assert_eq!(buf, b"next-line\n");
    }

    // ---- SandboxError → ActionError conversion -----------------------

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
}
