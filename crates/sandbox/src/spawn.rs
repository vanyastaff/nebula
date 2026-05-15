//! Plugin process spawn + dial.
//!
//! Spawns the plugin binary with a hardened, host-controlled environment
//! (`env_clear` + explicit transport env, `kill_on_drop`, Linux Landlock /
//! rlimit `pre_exec`), reads and validates its handshake line, dials the
//! announced transport, and returns a live
//! [`PluginHandle`](crate::codec::PluginHandle).
//!
//! This is the **only** module in the crate that contains `unsafe`: the
//! single `pre_exec` block that runs between `fork()` and `exec()` on
//! Linux. The crate-level `#![deny(unsafe_code)]` stays in force; this
//! module carries one narrowly-scoped `#[expect(unsafe_code, ...)]` around
//! the real `pre_exec` call.
//!
//! Security enforcement (ADR 0006):
//! - `env_clear()` + explicit env allowlist
//! - `pre_exec` landlock + configurable rlimits (Linux)
//! - stderr size limit for log capture
//! - `kill_on_drop` on the spawned child → plugin process dies with the sandbox

use std::path::Path;

use nebula_plugin_sdk::transport::{self, ENV_SOCKET_ADDR, ENV_SOCKET_KIND};
use tokio::{io::BufReader, process::Command};

use crate::{
    codec::{
        BoundedReadOutcome, PluginHandle, drain_plugin_stderr, read_bounded_line,
        sanitize_plugin_string,
    },
    error::SandboxError,
    handshake::{
        HANDSHAKE_LINE_CAP, HANDSHAKE_TIMEOUT, allocate_host_socket_addr, validate_handshake_addr,
    },
    os_sandbox::LinuxRlimits,
};

/// Spawn the plugin binary, read and parse its handshake line, dial the
/// announced transport, and return a fresh
/// [`PluginHandle`](crate::codec::PluginHandle).
///
/// Only reads the two pieces of sandbox state it needs (`binary`,
/// `linux_rlimits`); it does not touch the cached handle or id counter,
/// which keeps the spawn path independent of dispatch state.
pub(crate) async fn spawn_and_dial(
    binary: &Path,
    linux_rlimits: LinuxRlimits,
) -> Result<PluginHandle, SandboxError> {
    // rlimits are only consumed by the Linux `pre_exec` hardening block
    // below; on other platforms they are accepted for a uniform signature
    // but intentionally not applied.
    #[cfg(not(target_os = "linux"))]
    let _ = linux_rlimits;

    // #260: host allocates the plugin's socket address up-front and
    // passes it via env so the child cannot forge a handshake that
    // redirects the host at a sibling plugin's socket. `socket_dir`
    // is `Some` on Unix (TempDir that owns the 0700 parent) and
    // `None` on Windows (named pipes aren't in a filesystem dir).
    let (expected_addr, kind, socket_dir) = allocate_host_socket_addr()?;

    // `env_clear()` then only the host-controlled transport env: a
    // Phase-era plugin inherits **no** host environment. A
    // host-authored, scope-keyed env allowlist returns only with the
    // broker (ADR-0025 §6), never as a plugin-declared capability.
    let mut cmd = Command::new(binary);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .env_clear()
        .env(ENV_SOCKET_ADDR, &expected_addr)
        .env(ENV_SOCKET_KIND, kind);

    // Apply OS-level hardening in the child between fork() and exec()
    // (Linux only). The entire Landlock ruleset + rlimit snapshot is
    // built HERE, pre-fork, on the host thread — every allocation
    // happens before fork.
    #[cfg(target_os = "linux")]
    {
        let mut prepared = crate::os_sandbox::PreparedSandbox::prepare(linux_rlimits)
            .map_err(|e| SandboxError::Spawn(format!("sandbox prepare failed: {e}")))?;

        // SAFETY: the closure runs between fork() and exec(). It calls
        // only setrlimit(2) and landlock_restrict_self(2) on data
        // allocated above (pre-fork) — no allocation, no serde, no
        // PathFd::new, no tracing on the success path. This is the
        // structural fix for the post-fork allocator-lock deadlock
        // class: a multi-threaded parent may hold the allocator lock
        // at fork, so the child must not re-enter the allocator.
        #[expect(
            unsafe_code,
            reason = "pre_exec: only setrlimit + landlock_restrict_self on pre-fork-allocated data; no allocation in child"
        )]
        unsafe {
            cmd.pre_exec(move || prepared.apply_in_child());
        }
    }

    let mut child = cmd.spawn().map_err(|e| {
        SandboxError::Spawn(format!("failed to spawn plugin {}: {e}", binary.display()))
    })?;

    // Spawn a background task that drains the plugin's stderr and logs
    // each line via `tracing`. We do this BEFORE reading the handshake
    // so that any crash diagnostics the plugin writes during startup
    // are captured.
    //
    // Fire-and-forget: the returned JoinHandle is intentionally
    // dropped (#270). The task's lifecycle is bounded implicitly via
    // the chain:
    //
    //   cmd.kill_on_drop(true)             (above, ~"kill_on_drop")
    //     → dropping PluginHandle drops Child
    //     → Child's Drop terminates the plugin process
    //       (SIGKILL on Unix, TerminateProcess on Windows — both
    //       close the child's stderr pipe handle as a side effect)
    //     → plugin's stderr pipe closes
    //     → drain_plugin_stderr observes EOF and returns
    //     → detached task completes and is reaped
    //
    // If a future refactor removes `kill_on_drop(true)`, make sure
    // the replacement shutdown path still reliably terminates the
    // child so stderr closes and the drainer reaches EOF, and/or
    // keep an owned JoinHandle that can be aborted explicitly on
    // ProcessSandbox::drop. The leak risk is not "explicit
    // shutdown" by itself; it is any shutdown path that leaves the
    // stderr drainer with no guaranteed completion signal — at
    // that point tokio's task slab grows unbounded per plugin
    // spawn.
    if let Some(stderr) = child.stderr.take() {
        let plugin_name = binary.display().to_string();
        tokio::spawn(drain_plugin_stderr(stderr, plugin_name));
    }

    // Read the handshake line from child stdout with a hard timeout
    // AND a hard length cap — a malicious plugin must not be able to
    // hold this task blocking on memory growth.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SandboxError::Spawn(String::from("failed to open plugin stdout")))?;
    let mut stdout_reader = BufReader::new(stdout);
    let mut handshake_buf: Vec<u8> = Vec::with_capacity(256);

    let read_result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        read_bounded_line(&mut stdout_reader, HANDSHAKE_LINE_CAP, &mut handshake_buf).await
    })
    .await;

    let outcome = read_result.map_err(|_| {
        SandboxError::Spawn(format!(
            "plugin {} handshake timeout after {HANDSHAKE_TIMEOUT:?}",
            binary.display()
        ))
    })?;

    let body_len = match outcome {
        Ok(BoundedReadOutcome::Line { body_len }) => body_len,
        Ok(BoundedReadOutcome::Eof) => {
            return Err(SandboxError::Spawn(format!(
                "plugin {} exited before printing handshake line",
                binary.display()
            )));
        },
        Ok(BoundedReadOutcome::Overflow { observed }) => {
            tracing::warn!(
                plugin = %binary.display(),
                limit = HANDSHAKE_LINE_CAP,
                observed,
                "plugin handshake exceeded cap — refusing to dial",
            );
            return Err(SandboxError::HandshakeLineTooLarge {
                limit: HANDSHAKE_LINE_CAP,
                observed,
            });
        },
        Err(e) => {
            return Err(SandboxError::Spawn(format!(
                "plugin {} handshake read error: {e}",
                binary.display()
            )));
        },
    };

    // Strip the trailing newline and decode as UTF-8 for the dial
    // address. We do this AFTER the cap check so we never run UTF-8
    // validation on an unbounded buffer.
    let handshake_bytes = &handshake_buf[..body_len];
    let handshake_line = std::str::from_utf8(handshake_bytes).map_err(|e| {
        SandboxError::Spawn(format!(
            "plugin {} handshake line is not valid UTF-8: {e}",
            binary.display()
        ))
    })?;

    let sanitized_handshake = sanitize_plugin_string(handshake_line.trim());
    tracing::debug!(
        plugin = %binary.display(),
        handshake = %sanitized_handshake,
        "plugin handshake received"
    );

    // #260: validate the announced address against what we pre-allocated
    // before dialling. A compromised plugin that prints a sibling
    // plugin's socket path here is rejected, not connected to.
    if let Err(err) = validate_handshake_addr(handshake_line, &expected_addr, kind) {
        tracing::warn!(
            plugin = %binary.display(),
            expected = %expected_addr,
            error = %err,
            "plugin handshake address mismatch — refusing to dial",
        );
        return Err(err);
    }

    // Dial the announced transport.
    let stream = transport::dial(handshake_line)
        .await
        .map_err(|e| SandboxError::Spawn(format!("plugin transport dial failed: {e}")))?;

    Ok(PluginHandle::new(child, stream, socket_dir))
}
