//! Length-bounded line framing for the duplex plugin transport.
//!
//! Pure framing layer: it knows how to read one newline-delimited line
//! from an async reader without letting an untrusted plugin grow the
//! receive buffer until OOM, and it owns the live [`PluginHandle`] that
//! wraps the accepted stream. It has **no** knowledge of how a plugin is
//! spawned or how envelopes are dispatched — that lives in `spawn` and
//! `dispatch` respectively.
//!
//! ## Transport line-length caps (#316, 2026-04-14)
//!
//! Both the handshake and envelope read paths are length-capped. An
//! untrusted plugin that emits a newline-starved or gigabyte-sized line no
//! longer grows the receive buffer until OOM: the read returns a typed
//! [`SandboxError::PluginLineTooLarge`] / [`SandboxError::HandshakeLineTooLarge`]
//! and the plugin transport is **poisoned** — further send/recv calls on
//! the same handle fail fast with [`SandboxError::TransportPoisoned`]. The
//! outer dispatch also drops its cached handle on any error, so
//! defense-in-depth gives us both an instance-level invalidation
//! (`poisoned` flag inside `PluginHandle`) and a sandbox-level one
//! (`*self.handle.lock().await = None`).

use nebula_plugin_sdk::{
    protocol::{HostToPlugin, PluginToHost},
    transport::PluginStream,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::error::SandboxError;

/// Maximum bytes accepted for a single runtime envelope line
/// (JSON payload + trailing `\n`).
///
/// 1 MiB starting point. If real plugins need more, make this configurable
/// via [`crate::ProcessSandbox::new`] later rather than preemptively raising
/// the ceiling — a too-high cap defeats the purpose of the cap.
pub(crate) const ENVELOPE_LINE_CAP: usize = 1024 * 1024;

/// Maximum bytes buffered per plugin stderr log line before the overflow
/// is silently discarded (stderr is diagnostic log output, not protocol,
/// so truncation is acceptable here — but we still bound the read, because
/// an attacker-controlled plugin could otherwise emit a gigabyte of
/// newline-starved garbage and starve the host of memory).
pub(crate) const STDERR_LINE_CAP: usize = 8 * 1024;

/// Live connection to a running plugin process.
///
/// Owns the spawned [`Child`](tokio::process::Child) and the two halves of
/// the accepted stream (reader is wrapped in `BufReader` for efficient
/// line-delimited reads). When dropped, `kill_on_drop(true)` on the child
/// ensures the OS process is terminated; the socket/pipe is released by
/// `PluginStream`'s cleanup guard on the plugin side.
pub(crate) struct PluginHandle {
    /// Kept alive for `kill_on_drop` — dropping this struct SIGKILLs the
    /// child. Read nowhere; the underscore prefix silences dead-code
    /// warnings.
    _child: tokio::process::Child,
    /// Host-allocated temp directory holding the UDS socket on Unix.
    /// Kept alive for the lifetime of the handle so the directory (and
    /// its 0700 perms) survive as long as the plugin process. Dropping
    /// the `TempDir` removes the directory tree. `None` on Windows —
    /// named pipes aren't in a filesystem directory. Dead code at
    /// read-time; the `_` prefix silences warnings.
    _socket_dir: Option<tempfile::TempDir>,
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
    pub(crate) poisoned: bool,
}

impl PluginHandle {
    pub(crate) fn new(
        child: tokio::process::Child,
        stream: PluginStream,
        socket_dir: Option<tempfile::TempDir>,
    ) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        Self {
            _child: child,
            _socket_dir: socket_dir,
            reader: BufReader::new(read_half),
            writer: write_half,
            line_buf: Vec::with_capacity(512),
            poisoned: false,
        }
    }

    pub(crate) async fn send_envelope(
        &mut self,
        envelope: &HostToPlugin,
    ) -> Result<(), SandboxError> {
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

    pub(crate) async fn recv_envelope(&mut self) -> Result<PluginToHost, SandboxError> {
        if self.poisoned {
            return Err(SandboxError::TransportPoisoned);
        }
        self.line_buf.clear();
        match read_bounded_line(&mut self.reader, ENVELOPE_LINE_CAP, &mut self.line_buf).await {
            Ok(BoundedReadOutcome::Line { body_len }) => {
                let body = &self.line_buf[..body_len];
                serde_json::from_slice::<PluginToHost>(body).map_err(|e| {
                    // Parse error does not leave the stream in an unknown
                    // state — we consumed exactly one line. Do NOT poison
                    // here; the outer dispatch will still drop the handle
                    // to be safe, but a legitimate one-off parse failure
                    // should not prevent a retry on a fresh envelope.
                    SandboxError::MalformedEnvelope(e)
                })
            },
            Ok(BoundedReadOutcome::Eof) => {
                self.poisoned = true;
                Err(SandboxError::PluginClosed)
            },
            Ok(BoundedReadOutcome::Overflow { observed }) => {
                self.poisoned = true;
                Err(SandboxError::PluginLineTooLarge {
                    limit: ENVELOPE_LINE_CAP,
                    observed,
                })
            },
            Err(e) => {
                self.poisoned = true;
                Err(SandboxError::Transport(e))
            },
        }
    }

    /// Receive the next envelope as raw JSON bytes (no typed parse).
    ///
    /// Used by the metadata-probe path (see `ProcessSandbox::get_metadata_raw`):
    /// returning the raw bytes lets the caller first parse to
    /// `serde_json::Value` for a cheap version check, then re-parse the
    /// same bytes as `PluginToHost` — preserving the zero-copy / borrowed-
    /// string path that some `Deserialize` implementations (notably
    /// `domain_key::Key<T>`) require.
    ///
    /// Contract mirrors `recv_envelope`: same poisoning rules, same cap
    /// enforcement, same error taxonomy; only the final parse step is
    /// deferred to the caller.
    pub(crate) async fn recv_envelope_bytes(&mut self) -> Result<Vec<u8>, SandboxError> {
        if self.poisoned {
            return Err(SandboxError::TransportPoisoned);
        }
        self.line_buf.clear();
        match read_bounded_line(&mut self.reader, ENVELOPE_LINE_CAP, &mut self.line_buf).await {
            Ok(BoundedReadOutcome::Line { body_len }) => Ok(self.line_buf[..body_len].to_vec()),
            Ok(BoundedReadOutcome::Eof) => {
                self.poisoned = true;
                Err(SandboxError::PluginClosed)
            },
            Ok(BoundedReadOutcome::Overflow { observed }) => {
                self.poisoned = true;
                Err(SandboxError::PluginLineTooLarge {
                    limit: ENVELOPE_LINE_CAP,
                    observed,
                })
            },
            Err(e) => {
                self.poisoned = true;
                Err(SandboxError::Transport(e))
            },
        }
    }
}

/// Outcome of a single bounded line read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoundedReadOutcome {
    /// A complete newline-terminated line was read. `body_len` is the
    /// length of the line **excluding** the trailing `\n`; callers slice
    /// `&buf[..body_len]` directly, so the no-underflow / in-bounds
    /// invariant lives in the type rather than in a recomputed `- 1` at
    /// every call site.
    Line { body_len: usize },
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
pub(crate) async fn read_bounded_line<R>(
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
        // payload_len >= 1 here (n != 0 and the last byte is `\n`), so
        // `payload_len - 1` (body without the newline) cannot underflow.
        return Ok(BoundedReadOutcome::Line {
            body_len: payload_len - 1,
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

/// Read and discard bytes from `reader` until a newline is encountered
/// or the stream ends. Returns `true` if a newline was found (drain loop
/// should continue), `false` if the stream closed / errored (drain loop
/// should exit).
///
/// Bounds memory to a single small scratch buffer regardless of how
/// many bytes the plugin emits before the next newline. Used to recover
/// from an oversized stderr line without growing the line buffer past
/// `STDERR_LINE_CAP`.
pub(crate) async fn drop_until_newline<R>(reader: &mut R) -> bool
where
    R: AsyncBufRead + Unpin,
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
pub(crate) async fn drain_plugin_stderr(stderr: tokio::process::ChildStderr, plugin_name: String) {
    let mut reader = BufReader::new(stderr);
    let mut line_buf: Vec<u8> = Vec::with_capacity(256);
    loop {
        line_buf.clear();
        match read_bounded_line(&mut reader, STDERR_LINE_CAP, &mut line_buf).await {
            Ok(BoundedReadOutcome::Line { body_len }) => {
                let body = &line_buf[..body_len];
                let as_str = String::from_utf8_lossy(body);
                let sanitized = sanitize_plugin_string(as_str.trim());
                tracing::debug!(
                    plugin = %plugin_name,
                    stderr = %sanitized,
                    "plugin stderr"
                );
            },
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
            },
            Err(_) => return,
        }
    }
}

pub(crate) fn envelope_kind(env: &PluginToHost) -> &'static str {
    match env {
        PluginToHost::ActionResultOk { .. } => "action_result_ok",
        PluginToHost::ActionResultError { .. } => "action_result_error",
        PluginToHost::RpcCall { .. } => "rpc_call",
        PluginToHost::Log { .. } => "log",
        PluginToHost::MetadataResponse { .. } => "metadata_response",
    }
}

pub(crate) fn sanitize_plugin_string(s: &str) -> String {
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
        assert_eq!(outcome, BoundedReadOutcome::Line { body_len: 5 });
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
        assert_eq!(outcome, BoundedReadOutcome::Line { body_len: 7 });
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
            },
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
        assert_eq!(out1, BoundedReadOutcome::Line { body_len: 5 });
        assert_eq!(&buf[..6], b"first\n");

        buf.clear();
        let out2 = read_bounded_line(&mut reader, 1024, &mut buf)
            .await
            .unwrap();
        assert_eq!(out2, BoundedReadOutcome::Line { body_len: 6 });
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
                Ok(BoundedReadOutcome::Line { body_len }) => Ok(self.line_buf[..body_len].to_vec()),
                Ok(BoundedReadOutcome::Eof) => {
                    self.poisoned = true;
                    Err(SandboxError::PluginClosed)
                },
                Ok(BoundedReadOutcome::Overflow { observed }) => {
                    self.poisoned = true;
                    Err(SandboxError::PluginLineTooLarge {
                        limit: cap,
                        observed,
                    })
                },
                Err(e) => {
                    self.poisoned = true;
                    Err(SandboxError::Transport(e))
                },
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
            },
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
        assert_eq!(outcome, BoundedReadOutcome::Line { body_len: 9 });
        assert_eq!(buf, b"next-line\n");
    }
}
