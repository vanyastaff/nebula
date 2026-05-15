//! Cascade-class regression: an out-of-process plugin invocation whose
//! request reached the plugin (`sent == true`) but whose response never
//! arrived (plugin died) MUST execute EXACTLY ONCE across the full
//! frontier-retry path — the engine must never re-dispatch it even under
//! a multi-attempt retry policy.
//!
//! This is the structural no-resend guarantee: a `sent == true` close maps
//! `SandboxError::PluginClosedAfterSend` → fatal `ActionError`, and
//! `compute_retry_decision` finalizes a fatal just-recorded error before
//! the policy check. The negative control proves the short-circuit is
//! narrow: a *retryable* plugin failure still retries through the frontier
//! (the deterministic stand-in for the unchanged pre-send
//! `PluginClosed → Retryable` class).
//!
//! Behind the `out-of-process-plugins` feature; `#[ignore]` because it
//! needs the pre-built fixture binary. The binding gates run without it.
//!
//! # Running
//!
//! ```bash
//! cargo build -p nebula-plugin-sdk --bin nebula-plugin-resend-fixture
//! cargo nextest run -p nebula-engine --features out-of-process-plugins \
//!     --test sandbox_pool_no_resend --run-ignored all
//! ```

#![cfg(feature = "out-of-process-plugins")]

use std::{path::PathBuf, sync::Arc, time::Duration};

use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox, OutOfProcessConfig,
    SandboxRunner, WorkflowEngine, discover_into_registry,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::PluginRegistry;
use nebula_workflow::{NodeDefinition, RetryConfig, Version, WorkflowConfig, WorkflowDefinition};

fn fixture_binary_path() -> PathBuf {
    let bin_name = if cfg!(windows) {
        "nebula-plugin-resend-fixture.exe"
    } else {
        "nebula-plugin-resend-fixture"
    };
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .join("target")
        .join(profile)
        .join(bin_name)
}

fn no_op_sandbox() -> Arc<dyn SandboxRunner> {
    let executor: nebula_engine::ActionExecutor = Arc::new(|_ctx, _meta, input| {
        Box::pin(async move { Ok(nebula_action::ActionResult::success(input)) })
    });
    Arc::new(InProcessSandbox::new(executor))
}

/// Copy the fixture into a fresh temp dir, write `plugin.toml`, and return
/// `(tempdir, counter_path)`. The fixture appends one byte to
/// `resend-counter` next to its own executable per received invocation.
fn stage_fixture() -> (tempfile::TempDir, PathBuf) {
    let src = fixture_binary_path();
    assert!(
        src.exists(),
        "fixture not built: {} — run `cargo build -p nebula-plugin-sdk \
         --bin nebula-plugin-resend-fixture`",
        src.display(),
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let dest = dir.path().join(src.file_name().expect("binary name"));
    std::fs::copy(&src, &dest).expect("copy fixture");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).expect("chmod");
    }

    std::fs::write(dir.path().join("plugin.toml"), "[nebula]\nsdk = \"*\"\n")
        .expect("write plugin.toml");

    let counter = dir.path().join("resend-counter");
    (dir, counter)
}

fn receipt_count(counter: &PathBuf) -> usize {
    std::fs::read(counter).map(|b| b.len()).unwrap_or(0)
}

async fn engine_with_pool(scan_dir: &std::path::Path) -> WorkflowEngine {
    // Pool capacity N >= 2 per the cascade-class requirement.
    let config = OutOfProcessConfig {
        plugin_dirs: vec![scan_dir.to_path_buf()],
        default_timeout: Duration::from_secs(5),
        max_processes_per_key: 2,
    };
    let action_registry = Arc::new(ActionRegistry::new());
    let mut plugin_registry = PluginRegistry::new();
    discover_into_registry(&config, &mut plugin_registry, &action_registry).await;

    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            Arc::clone(&action_registry),
            no_op_sandbox(),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("runtime"),
    );
    WorkflowEngine::new(runtime, metrics).expect("engine")
}

fn workflow(action_key: &str) -> WorkflowDefinition {
    let mut node = NodeDefinition::new(
        nebula_core::NodeKey::new("n").expect("node key"),
        "n",
        action_key,
    )
    .expect("node");
    // A 3-attempt policy: if the engine wrongly re-dispatched the
    // after-send failure, the counter would reach 3, not 1.
    node.retry_policy = Some(RetryConfig::fixed(3, 1));

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::id::WorkflowId::new(),
        name: "no-resend".to_owned(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![node],
        connections: vec![],
        variables: Default::default(),
        config: WorkflowConfig::default(),
        trigger: None,
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: 1,
    }
}

/// POSITIVE: `sent == true` then EOF ⇒ fatal ⇒ the action executes
/// EXACTLY ONCE across the full frontier-retry path, despite a 3-attempt
/// policy and an N=2 pool.
#[tokio::test]
#[ignore = "requires pre-built fixture binary; run `cargo build -p nebula-plugin-sdk --bin nebula-plugin-resend-fixture` first, then `cargo nextest run -p nebula-engine --features out-of-process-plugins --test sandbox_pool_no_resend --run-ignored all`"]
async fn after_send_close_executes_action_exactly_once() {
    let (dir, counter) = stage_fixture();
    let engine = engine_with_pool(dir.path()).await;
    let wf = workflow("com.nebula.resend.crash_after_recv");

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow drives to a terminal state");

    // The plugin died after receiving the request — a fatal, non-resendable
    // failure. The node fails (not retried), and crucially the plugin
    // received the invocation EXACTLY ONCE.
    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "an after-send plugin close is a fatal node failure"
    );
    assert_eq!(
        receipt_count(&counter),
        1,
        "STRUCTURAL NO-RESEND: the action must execute exactly once even \
         under a 3-attempt retry policy — a second receipt would mean the \
         engine re-dispatched a request that had already reached the plugin"
    );
}

/// NEGATIVE CONTROL: a *retryable* plugin failure (not fatal) still
/// retries through the frontier — proving the fatal short-circuit is
/// narrow and does not break the unchanged pre-send / `PluginClosed →
/// Retryable` path.
#[tokio::test]
#[ignore = "requires pre-built fixture binary; run `cargo build -p nebula-plugin-sdk --bin nebula-plugin-resend-fixture` first, then `cargo nextest run -p nebula-engine --features out-of-process-plugins --test sandbox_pool_no_resend --run-ignored all`"]
async fn retryable_failure_still_retries_through_frontier() {
    let (dir, counter) = stage_fixture();
    let engine = engine_with_pool(dir.path()).await;
    let wf = workflow("com.nebula.resend.fail_retryable");

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow drives to a terminal state");

    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "a persistently-retryable failure exhausts the policy and fails"
    );
    assert_eq!(
        receipt_count(&counter),
        3,
        "NEGATIVE CONTROL: a retryable (non-fatal) failure must still be \
         retried up to max_attempts (3) — the fatal short-circuit must NOT \
         finalize a merely-retryable error"
    );
}
