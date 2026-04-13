//! End-to-end smoke test for the Phase 1 slice-1a duplex protocol.
//!
//! Spawns the `nebula-echo-fixture` binary (a sibling bin target of this
//! crate), sends it `HostToPlugin::ActionInvoke` envelopes over its stdin,
//! reads `PluginToHost::ActionResultOk` envelopes from its stdout, and
//! asserts round-trip correctness.
//!
//! This test does **not** go through `ProcessSandbox` — it validates the
//! wire protocol and the plugin-sdk's `run_duplex` event loop in isolation.
//! Slice 1b will wire the same protocol into `ProcessSandbox`.

use std::time::Duration;

use nebula_plugin_sdk::protocol::{DUPLEX_PROTOCOL_VERSION, HostToPlugin, LogLevel, PluginToHost};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
};

const ECHO_BIN: &str = env!("CARGO_BIN_EXE_nebula-echo-fixture");
const OP_TIMEOUT: Duration = Duration::from_secs(5);

struct PluginProcess {
    child: tokio::process::Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    line_buf: String,
}

impl PluginProcess {
    async fn spawn() -> Self {
        let mut cmd = Command::new(ECHO_BIN);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().expect("failed to spawn echo fixture");
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        Self {
            child,
            stdin,
            stdout,
            line_buf: String::with_capacity(512),
        }
    }

    async fn send(&mut self, msg: &HostToPlugin) {
        let encoded = serde_json::to_string(msg).expect("serialize HostToPlugin");
        self.stdin
            .write_all(encoded.as_bytes())
            .await
            .expect("write envelope");
        self.stdin.write_all(b"\n").await.expect("write newline");
        self.stdin.flush().await.expect("flush stdin");
    }

    async fn recv(&mut self) -> PluginToHost {
        self.line_buf.clear();
        let n = tokio::time::timeout(OP_TIMEOUT, self.stdout.read_line(&mut self.line_buf))
            .await
            .expect("recv timeout")
            .expect("read_line io");
        assert!(n > 0, "plugin closed stdout unexpectedly");
        serde_json::from_str(self.line_buf.trim())
            .unwrap_or_else(|e| panic!("parse PluginToHost failed: {e} :: {:?}", self.line_buf))
    }

    async fn shutdown(mut self) {
        self.send(&HostToPlugin::Shutdown).await;
        drop(self.stdin);
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
        }
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
        }
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
            }
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
        }
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
            plugin_key,
            plugin_version,
            actions,
        } => {
            assert_eq!(id, 99);
            assert_eq!(protocol_version, DUPLEX_PROTOCOL_VERSION);
            assert_eq!(plugin_key, "com.nebula.echo");
            assert_eq!(plugin_version, "0.1.0");
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].key, "echo");
        }
        other => panic!("expected MetadataResponse, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn plugin_exits_on_stdin_eof() {
    // Spawn, drop stdin, verify the plugin exits cleanly.
    let mut p = PluginProcess::spawn().await;
    drop(p.stdin);
    let status = tokio::time::timeout(OP_TIMEOUT, p.child.wait())
        .await
        .expect("child did not exit after stdin drop")
        .expect("child wait");
    assert!(status.success(), "plugin exited with {status}");
}

#[tokio::test]
async fn malformed_json_line_is_skipped_not_fatal() {
    let mut p = PluginProcess::spawn().await;
    // Send a garbage line first.
    p.stdin
        .write_all(b"this is not valid json\n")
        .await
        .expect("write garbage");
    p.stdin.flush().await.expect("flush");

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
        }
        other => panic!("expected ActionResultOk after recovery, got {other:?}"),
    }
    p.shutdown().await;
}

#[tokio::test]
async fn cancel_and_rpc_response_messages_are_ignored_cleanly() {
    // Slice 1a ignores Cancel / RpcResponseOk / RpcResponseError — they should
    // neither crash the plugin nor produce a response.
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
        }
        other => panic!("expected ActionResultOk, got {other:?}"),
    }
    p.shutdown().await;
}

// Suppress warning about the unused LogLevel import — the `use` is there so
// that future slice-1a tests (when we add Log-stream assertions) can use it
// without another import line. Delete when Log tests land.
#[allow(dead_code)]
fn _referenced_levels() {
    let _ = LogLevel::Info;
}
