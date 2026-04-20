//! End-to-end smoke test for the Phase 1 slice-1c socket transport.
//!
//! Spawns the `nebula-echo-fixture` binary, reads its handshake line from
//! stdout, dials the announced UDS (Linux/macOS) or Named Pipe (Windows),
//! then exchanges envelopes over the resulting stream. Validates the full
//! handshake → dial → run loop path end-to-end without going through
//! `ProcessSandbox`.
//!
//! Slice 1b shipped the same test over raw stdio; slice 1c replaces the
//! transport layer with sockets. The envelope shape is unchanged.

use std::time::Duration;

use nebula_plugin_sdk::{
    protocol::{DUPLEX_PROTOCOL_VERSION, HostToPlugin, LogLevel, PluginToHost},
    transport::{self, PluginStream},
};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdout, Command},
};

const ECHO_BIN: &str = env!("CARGO_BIN_EXE_nebula-echo-fixture");
const OP_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

struct PluginProcess {
    child: Child,
    stream: PluginStream,
    line_buf: String,
    #[expect(
        dead_code,
        reason = "stderr pipe held to keep the child's stderr open; not read by tests"
    )]
    stderr: Option<ChildStderr>,
}

impl PluginProcess {
    async fn spawn() -> Self {
        let mut cmd = Command::new(ECHO_BIN);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().expect("failed to spawn echo fixture");

        // Read the handshake line from the fixture's stdout.
        let stdout: ChildStdout = child.stdout.take().expect("stdout piped");
        let mut stdout_reader = BufReader::new(stdout);
        let mut handshake = String::new();
        tokio::time::timeout(HANDSHAKE_TIMEOUT, stdout_reader.read_line(&mut handshake))
            .await
            .expect("handshake read timed out")
            .expect("handshake read failed");
        assert!(
            !handshake.is_empty(),
            "plugin exited before printing handshake line"
        );

        // Dial the announced transport.
        let stream = transport::dial(handshake.trim())
            .await
            .expect("failed to dial plugin transport");

        // Keep stderr piped but don't actively drain it in slice 1c tests.
        let stderr = child.stderr.take();

        Self {
            child,
            stream,
            line_buf: String::with_capacity(512),
            stderr,
        }
    }

    async fn send(&mut self, msg: &HostToPlugin) {
        let encoded = serde_json::to_string(msg).expect("serialize HostToPlugin");
        self.stream
            .write_all(encoded.as_bytes())
            .await
            .expect("write envelope");
        self.stream.write_all(b"\n").await.expect("write newline");
        self.stream.flush().await.expect("flush stream");
    }

    async fn recv(&mut self) -> PluginToHost {
        self.line_buf.clear();
        let mut byte = [0u8; 1];
        loop {
            let n = tokio::time::timeout(OP_TIMEOUT, self.stream.read(&mut byte))
                .await
                .expect("recv timed out")
                .expect("read failed");
            assert!(n > 0, "plugin closed stream unexpectedly");
            if byte[0] == b'\n' {
                break;
            }
            self.line_buf.push(byte[0] as char);
        }
        serde_json::from_str(self.line_buf.trim())
            .unwrap_or_else(|e| panic!("parse PluginToHost failed: {e} :: {:?}", self.line_buf))
    }

    async fn shutdown(mut self) {
        // Send Shutdown envelope; the plugin will return Ok(()) from its
        // event loop, the stream will close, and the process exits.
        self.send(&HostToPlugin::Shutdown).await;
        drop(self.stream);
        let status = tokio::time::timeout(OP_TIMEOUT, self.child.wait())
            .await
            .expect("child did not exit in time")
            .expect("child wait failed");
        assert!(
            status.success(),
            "plugin exited with non-zero status: {status}"
        );
    }
}

#[tokio::test]
async fn echo_roundtrip_simple_string() {
    let mut p = PluginProcess::spawn().await;
    p.send(&HostToPlugin::ActionInvoke {
        id: 1,
        action_key: "echo".into(),
        input: json!("hello"),
    })
    .await;
    let resp = p.recv().await;
    match resp {
        PluginToHost::ActionResultOk { id, output } => {
            assert_eq!(id, 1);
            assert_eq!(output, json!("hello"));
        },
        other => panic!("expected ActionResultOk, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn echo_roundtrip_structured_object() {
    let mut p = PluginProcess::spawn().await;
    let payload = json!({
        "nested": { "count": 42, "name": "test" },
        "array": [1, 2, 3],
        "bool": true,
        "null": null
    });
    p.send(&HostToPlugin::ActionInvoke {
        id: 7,
        action_key: "echo".into(),
        input: payload.clone(),
    })
    .await;
    let resp = p.recv().await;
    match resp {
        PluginToHost::ActionResultOk { id, output } => {
            assert_eq!(id, 7);
            assert_eq!(output, payload);
        },
        other => panic!("expected ActionResultOk, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn echo_handles_multiple_invocations_sequentially() {
    let mut p = PluginProcess::spawn().await;
    for i in 0..5u64 {
        p.send(&HostToPlugin::ActionInvoke {
            id: i,
            action_key: "echo".into(),
            input: json!({"iteration": i}),
        })
        .await;
        let resp = p.recv().await;
        match resp {
            PluginToHost::ActionResultOk { id, output } => {
                assert_eq!(id, i);
                assert_eq!(output, json!({"iteration": i}));
            },
            other => panic!("iteration {i}: expected ActionResultOk, got {other:?}"),
        }
    }
    p.shutdown().await;
}

#[tokio::test]
async fn unknown_action_returns_error() {
    let mut p = PluginProcess::spawn().await;
    p.send(&HostToPlugin::ActionInvoke {
        id: 1,
        action_key: "does_not_exist".into(),
        input: json!({}),
    })
    .await;
    let resp = p.recv().await;
    match resp {
        PluginToHost::ActionResultError {
            id,
            code,
            retryable,
            ..
        } => {
            assert_eq!(id, 1);
            assert_eq!(code, "UNKNOWN_ACTION");
            assert!(!retryable);
        },
        other => panic!("expected ActionResultError, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn metadata_request_returns_plugin_info() {
    let mut p = PluginProcess::spawn().await;
    p.send(&HostToPlugin::MetadataRequest { id: 99 }).await;
    let resp = p.recv().await;
    match resp {
        PluginToHost::MetadataResponse {
            id,
            protocol_version,
            manifest,
            actions,
        } => {
            assert_eq!(id, 99);
            assert_eq!(protocol_version, DUPLEX_PROTOCOL_VERSION);
            assert_eq!(manifest.key().as_str(), "com.nebula.echo");
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].key, "echo");
        },
        other => panic!("expected MetadataResponse, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn plugin_exits_on_host_disconnect() {
    // Spawn, drop the stream (simulating host disconnect), verify the
    // plugin exits cleanly within a grace window.
    let mut p = PluginProcess::spawn().await;
    drop(p.stream);
    let status = tokio::time::timeout(OP_TIMEOUT, p.child.wait())
        .await
        .expect("child did not exit after host disconnect")
        .expect("child wait");
    assert!(status.success(), "plugin exited with {status}");
}

#[tokio::test]
async fn malformed_json_line_is_skipped_not_fatal() {
    let mut p = PluginProcess::spawn().await;
    // Send a garbage line first.
    p.stream
        .write_all(b"this is not valid json\n")
        .await
        .expect("write garbage");
    p.stream.flush().await.expect("flush");

    // Then send a valid invocation — it should still work.
    p.send(&HostToPlugin::ActionInvoke {
        id: 1,
        action_key: "echo".into(),
        input: json!("recovered"),
    })
    .await;
    let resp = p.recv().await;
    match resp {
        PluginToHost::ActionResultOk { id, output } => {
            assert_eq!(id, 1);
            assert_eq!(output, json!("recovered"));
        },
        other => panic!("expected ActionResultOk after recovery, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn cancel_and_rpc_response_messages_are_ignored_cleanly() {
    // Slice 1c ignores Cancel / RpcResponseOk / RpcResponseError — they
    // should neither crash the plugin nor produce a response.
    let mut p = PluginProcess::spawn().await;
    p.send(&HostToPlugin::Cancel { id: 1 }).await;
    p.send(&HostToPlugin::RpcResponseOk {
        id: 2,
        result: json!({}),
    })
    .await;
    p.send(&HostToPlugin::RpcResponseError {
        id: 3,
        code: "X".into(),
        message: "y".into(),
    })
    .await;

    // Follow with a real invocation; if the ignore-path was buggy the plugin
    // would either produce unexpected output here or be in a broken state.
    p.send(&HostToPlugin::ActionInvoke {
        id: 10,
        action_key: "echo".into(),
        input: json!("still alive"),
    })
    .await;
    let resp = p.recv().await;
    match resp {
        PluginToHost::ActionResultOk { id, output } => {
            assert_eq!(id, 10);
            assert_eq!(output, json!("still alive"));
        },
        other => panic!("expected ActionResultOk, got {other:?}"),
    }
    p.shutdown().await;
}

// Suppress unused-import warning: LogLevel is kept for the future slice 1d
// test for `Log` envelope round-trips.
#[allow(dead_code)]
fn _referenced_levels() {
    let _ = LogLevel::Info;
}
