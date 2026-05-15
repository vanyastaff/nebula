//! Host-side handshake: socket allocation, address validation, and
//! correlation-id extraction.
//!
//! This module owns the host's side of the duplex handshake contract: it
//! pre-allocates the socket/pipe address the plugin must bind (#260),
//! validates the address the plugin announces against that pre-allocated
//! value, and pulls the correlation id out of host→plugin / plugin→host
//! envelopes (#285) so the dispatch layer can detect a stale or replayed
//! response. It has no spawn or transport-framing knowledge.

use std::time::Duration;

use nebula_plugin_sdk::protocol::{HostToPlugin, PluginToHost};

use crate::error::SandboxError;

/// Timeout for reading the plugin's handshake line from stdout.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// Maximum bytes accepted for the plugin handshake line.
///
/// A handshake is a short socket/pipe address string plus protocol version
/// — realistically under 100 bytes. 4 KiB gives ~40x headroom while still
/// bounding memory use far below anything that could stress the allocator.
pub(crate) const HANDSHAKE_LINE_CAP: usize = 4 * 1024;

/// Allocate a host-controlled socket address for the plugin to bind (#260).
///
/// On Unix: creates a tempdir with `0700` permissions (via `tempfile`) and
/// returns a socket path inside it. Keeping the `TempDir` alive is the
/// caller's responsibility — it must be stored on the resulting
/// [`PluginHandle`](crate::codec::PluginHandle) so the directory persists
/// for as long as the socket is in use and is cleaned up on drop.
///
/// Prefers `/tmp` when available over the platform temp dir (macOS
/// defaults to `/var/folders/...` paths that can exceed
/// [`transport::MAX_UNIX_SOCKET_PATH_BYTES`]). Falls back to the platform
/// temp dir, and skips any candidate whose resulting socket path would
/// overflow `sun_path`. This mirrors the plugin-side self-allocation
/// policy so the host and plugin cannot disagree on what's bindable.
///
/// On Windows: generates an unpredictable named-pipe path under
/// `\\.\pipe\LOCAL\` (session-scoped namespace). The pipe is bound by
/// the plugin and released when the plugin process exits; no
/// directory-level cleanup is needed.
pub(crate) fn allocate_host_socket_addr()
-> Result<(String, &'static str, Option<tempfile::TempDir>), SandboxError> {
    #[cfg(unix)]
    {
        use std::{os::unix::ffi::OsStrExt, path::PathBuf};

        use nebula_plugin_sdk::transport;

        // Candidate roots in preference order: short `/tmp` first (macOS
        // `/var/folders/…` often exceeds the `sun_path` cap), then the
        // platform temp dir. Deduplicate so `/tmp`-is-temp_dir callers
        // don't try the same root twice.
        let mut candidate_roots: Vec<PathBuf> = Vec::new();
        let short_tmp = PathBuf::from("/tmp");
        if short_tmp.is_dir() {
            candidate_roots.push(short_tmp);
        }
        let platform_tmp = std::env::temp_dir();
        if !candidate_roots.iter().any(|root| root == &platform_tmp) {
            candidate_roots.push(platform_tmp);
        }

        let mut last_alloc_error: Option<(PathBuf, std::io::Error)> = None;
        for root in candidate_roots {
            let dir = match tempfile::Builder::new()
                .prefix("nebula-plugin-host-")
                .tempdir_in(&root)
            {
                Ok(dir) => dir,
                Err(e) => {
                    last_alloc_error = Some((root, e));
                    continue;
                },
            };

            let socket_path = dir.path().join("sock");
            let socket_path_len = socket_path.as_os_str().as_bytes().len();
            if socket_path_len > transport::MAX_UNIX_SOCKET_PATH_BYTES {
                // Drop the oversized tempdir (Drop cleans up) and try
                // the next candidate root.
                continue;
            }

            let addr = socket_path
                .to_str()
                .ok_or_else(|| {
                    SandboxError::Spawn(String::from(
                        "plugin socket tempdir path is not valid UTF-8",
                    ))
                })?
                .to_owned();
            return Ok((addr, "unix", Some(dir)));
        }

        if let Some((root, e)) = last_alloc_error {
            return Err(SandboxError::Spawn(format!(
                "failed to allocate plugin socket tempdir in {}: {e}",
                root.display()
            )));
        }
        Err(SandboxError::Spawn(format!(
            "failed to allocate a Unix socket path within {} bytes",
            transport::MAX_UNIX_SOCKET_PATH_BYTES
        )))
    }
    #[cfg(windows)]
    {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let pipe = format!(r"\\.\pipe\LOCAL\nebula-plugin-host-{nonce}");
        Ok((pipe, "pipe", None))
    }
}

/// Verify that the handshake line's `kind|addr` pair matches the
/// host-allocated values from [`allocate_host_socket_addr`]. Returns
/// [`SandboxError::HandshakeAddrMismatch`] on any deviation.
///
/// Exact string comparison is used — the plugin is expected to echo the
/// address we passed via `NEBULA_PLUGIN_SOCKET_ADDR` verbatim. An
/// attacker-controlled plugin that prints a different address (to
/// redirect the host at a sibling plugin's UDS or pipe) fails this check
/// and never reaches `transport::dial`.
pub(crate) fn validate_handshake_addr(
    handshake_line: &str,
    expected_addr: &str,
    expected_kind: &'static str,
) -> Result<(), SandboxError> {
    let line = handshake_line.trim();
    let mut parts = line.splitn(3, '|');
    let _version = parts.next(); // already checked by the caller's length cap and UTF-8 decode
    let announced_kind = parts.next().unwrap_or("");
    let announced_addr = parts.next().unwrap_or("");
    if announced_kind != expected_kind || announced_addr != expected_addr {
        let got = if announced_kind.is_empty() && announced_addr.is_empty() {
            String::from("<malformed handshake>")
        } else {
            format!("{announced_kind}|{announced_addr}")
        };
        return Err(SandboxError::HandshakeAddrMismatch {
            expected: format!("{expected_kind}|{expected_addr}"),
            got,
        });
    }
    Ok(())
}

/// Correlation id carried by an outbound host→plugin envelope, if any.
/// `Shutdown` has no id (it's one-way, no response expected); every
/// other variant carries a `u64`.
pub(crate) fn request_id(env: &HostToPlugin) -> Option<u64> {
    match env {
        HostToPlugin::ActionInvoke { id, .. }
        | HostToPlugin::MetadataRequest { id }
        | HostToPlugin::Cancel { id }
        | HostToPlugin::RpcResponseOk { id, .. }
        | HostToPlugin::RpcResponseError { id, .. } => Some(*id),
        HostToPlugin::Shutdown => None,
    }
}

/// Correlation id carried by an inbound plugin→host envelope, if any.
/// `Log` is one-way and carries no correlation id; every other variant
/// carries a `u64`.
pub(crate) fn response_id(env: &PluginToHost) -> Option<u64> {
    match env {
        PluginToHost::ActionResultOk { id, .. }
        | PluginToHost::ActionResultError { id, .. }
        | PluginToHost::RpcCall { id, .. }
        | PluginToHost::MetadataResponse { id, .. } => Some(*id),
        PluginToHost::Log { .. } => None,
    }
}

/// Variant of [`response_id`] that works on a raw `serde_json::Value`.
///
/// Mirrors the typed path used by `try_dispatch` for the raw-value metadata
/// probe. Returns `None` when the field is absent (`Log` variant) or when
/// the value isn't a `u64`.
pub(crate) fn response_id_from_value(value: &serde_json::Value) -> Option<u64> {
    value.get("id").and_then(serde_json::Value::as_u64)
}

#[cfg(test)]
mod tests {
    //! Handshake address-validation and correlation-id regression guards
    //! (#260 forged-handshake, #285 stale-response detection).

    use nebula_action::ActionError;

    use super::*;
    use crate::error::sandbox_error_to_action_error;

    // ---- #260 forged-handshake regression guard ----------------------

    #[test]
    fn validate_handshake_addr_accepts_matching_pair() {
        let line = "NEBULA-PROTO-3|unix|/tmp/nebula-plugin-host-abc/sock\n";
        let result = validate_handshake_addr(line, "/tmp/nebula-plugin-host-abc/sock", "unix");
        assert!(result.is_ok(), "matching addr+kind must be accepted");
    }

    #[test]
    fn validate_handshake_addr_rejects_forged_sibling_socket() {
        // The exact #260 scenario: a compromised plugin prints a path
        // that belongs to a DIFFERENT plugin's socket tree. The host
        // allocated `/tmp/nebula-plugin-host-ours/sock`, but the plugin
        // announces `/tmp/nebula-plugin-host-other/sock`. Must fail
        // BEFORE `dial` so the host never connects to the sibling.
        let line = "NEBULA-PROTO-3|unix|/tmp/nebula-plugin-host-other/sock\n";
        let err = validate_handshake_addr(line, "/tmp/nebula-plugin-host-ours/sock", "unix")
            .expect_err("forged sibling path must be rejected");
        match err {
            SandboxError::HandshakeAddrMismatch { expected, got } => {
                assert!(
                    expected.contains("/tmp/nebula-plugin-host-ours/sock"),
                    "expected field should contain the host-allocated addr, got {expected:?}",
                );
                assert!(
                    got.contains("/tmp/nebula-plugin-host-other/sock"),
                    "got field should contain the announced addr, got {got:?}",
                );
            },
            other => panic!("expected HandshakeAddrMismatch, got {other:?}"),
        }
    }

    #[test]
    fn validate_handshake_addr_rejects_mismatched_kind() {
        // Plugin tries to smuggle a pipe address on a Unix host (or
        // vice versa). Kind mismatch is a protocol violation and must
        // be rejected with the same error type.
        let line = "NEBULA-PROTO-3|pipe|some-pipe-name\n";
        let err = validate_handshake_addr(line, "/tmp/nebula/sock", "unix")
            .expect_err("kind mismatch must be rejected");
        assert!(matches!(err, SandboxError::HandshakeAddrMismatch { .. }));
    }

    #[test]
    fn validate_handshake_addr_rejects_malformed_handshake() {
        // Missing kind/addr entirely → mismatch error (with a clear
        // "<malformed handshake>" marker on the got side).
        let line = "NEBULA-PROTO-3\n";
        let err = validate_handshake_addr(line, "/tmp/nebula/sock", "unix")
            .expect_err("malformed handshake must be rejected");
        match err {
            SandboxError::HandshakeAddrMismatch { got, .. } => {
                assert!(
                    got.contains("malformed"),
                    "got should flag the malformed handshake, was {got:?}",
                );
            },
            other => panic!("expected HandshakeAddrMismatch, got {other:?}"),
        }
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

    #[cfg(unix)]
    #[test]
    fn allocate_host_socket_addr_gives_distinct_tempdirs() {
        // Two successive allocations must produce distinct addresses and
        // distinct tempdirs — otherwise two concurrent plugin spawns
        // would race on the same socket path.
        let (a, kind_a, dir_a) = allocate_host_socket_addr().expect("alloc a");
        let (b, kind_b, dir_b) = allocate_host_socket_addr().expect("alloc b");
        assert_eq!(kind_a, "unix");
        assert_eq!(kind_b, "unix");
        assert_ne!(a, b, "two allocations must produce distinct socket paths");
        assert_ne!(
            dir_a.as_ref().map(|d| d.path().to_path_buf()),
            dir_b.as_ref().map(|d| d.path().to_path_buf()),
            "two allocations must produce distinct tempdirs",
        );
    }

    #[cfg(windows)]
    #[test]
    fn allocate_host_socket_addr_gives_distinct_pipe_names() {
        let (a, kind_a, dir_a) = allocate_host_socket_addr().expect("alloc a");
        let (b, kind_b, dir_b) = allocate_host_socket_addr().expect("alloc b");
        assert_eq!(kind_a, "pipe");
        assert_eq!(kind_b, "pipe");
        assert!(dir_a.is_none(), "no tempdir on windows");
        assert!(dir_b.is_none(), "no tempdir on windows");
        assert_ne!(a, b, "two allocations must produce distinct pipe names");
        assert!(
            a.starts_with(r"\\.\pipe\LOCAL\nebula-plugin-host-"),
            "pipe name must carry the host-plugin prefix, was {a:?}",
        );
    }

    // ---- #285 monotonic-id + id-matching regression tests ------------

    #[test]
    fn request_id_extracts_all_id_bearing_variants() {
        let action = HostToPlugin::ActionInvoke {
            id: 7,
            action_key: String::from("k"),
            input: serde_json::json!({}),
        };
        let meta = HostToPlugin::MetadataRequest { id: 8 };
        let cancel = HostToPlugin::Cancel { id: 9 };
        let rpc_ok = HostToPlugin::RpcResponseOk {
            id: 10,
            result: serde_json::json!({}),
        };
        let rpc_err = HostToPlugin::RpcResponseError {
            id: 11,
            code: String::from("c"),
            message: String::from("m"),
        };
        let shutdown = HostToPlugin::Shutdown;

        assert_eq!(request_id(&action), Some(7));
        assert_eq!(request_id(&meta), Some(8));
        assert_eq!(request_id(&cancel), Some(9));
        assert_eq!(request_id(&rpc_ok), Some(10));
        assert_eq!(request_id(&rpc_err), Some(11));
        assert_eq!(
            request_id(&shutdown),
            None,
            "Shutdown is one-way and has no correlation id",
        );
    }

    #[test]
    fn response_id_extracts_all_id_bearing_variants() {
        let ok = PluginToHost::ActionResultOk {
            id: 42,
            output: serde_json::json!({}),
        };
        let err = PluginToHost::ActionResultError {
            id: 43,
            code: String::from("c"),
            message: String::from("m"),
            retryable: false,
        };
        let rpc = PluginToHost::RpcCall {
            id: 44,
            verb: String::from("v"),
            params: serde_json::json!({}),
        };
        let meta = PluginToHost::MetadataResponse {
            id: 45,
            protocol_version: 3,
            manifest: nebula_metadata::PluginManifest::builder("k", "K")
                .build()
                .unwrap(),
            actions: Vec::new(),
        };
        let log = PluginToHost::Log {
            level: nebula_plugin_sdk::protocol::LogLevel::Info,
            message: String::from("hi"),
            fields: serde_json::json!({}),
        };

        assert_eq!(response_id(&ok), Some(42));
        assert_eq!(response_id(&err), Some(43));
        assert_eq!(response_id(&rpc), Some(44));
        assert_eq!(response_id(&meta), Some(45));
        assert_eq!(
            response_id(&log),
            None,
            "Log is one-way and has no correlation id",
        );
    }
}
