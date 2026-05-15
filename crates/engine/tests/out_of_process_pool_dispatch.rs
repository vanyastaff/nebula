//! Gated end-to-end: a discovered out-of-process plugin action dispatched
//! through the engine-owned plugin pool with a `Lease`.
//!
//! Behind the `out-of-process-plugins` Cargo feature (default OFF, never in
//! the default CI profile). With the feature on AND a non-empty
//! `plugin_dirs`, `discover_into_registry` scans the directory via
//! `nebula_plugin::discovery::discover_directory`, registers a pooled
//! factory per action, and a factory dispatch round-trips through
//! `PluginSupervisor::acquire` → `Lease` → `ProcessSandbox` → the live
//! plugin process; `shutdown()` then drains the warm process.
//!
//! # Running
//!
//! ```bash
//! cargo build -p nebula-plugin-sdk --bin nebula-plugin-schema-fixture
//! cargo nextest run -p nebula-engine --features out-of-process-plugins \
//!     --test out_of_process_pool_dispatch --run-ignored all
//! ```
//!
//! It is `#[ignore]` because it needs the pre-built fixture binary; the
//! binding gates (3-crate nextest, workspace check, deny, clippy) run
//! without it.

#![cfg(feature = "out-of-process-plugins")]

use std::{path::PathBuf, sync::Arc, time::Duration};

use nebula_action::{ActionResult, testing::TestContextBuilder};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox, OutOfProcessConfig,
    SandboxRunner, discover_into_registry,
};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::PluginRegistry;
use nebula_workflow::NodeDefinition;
use serde_json::json;

fn fixture_binary_path() -> PathBuf {
    let bin_name = if cfg!(windows) {
        "nebula-plugin-schema-fixture.exe"
    } else {
        "nebula-plugin-schema-fixture"
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
    // Discovered actions dispatch through the pooled factory (the
    // `IsolationLevel::None` arm → `erased.dispatch`), never through
    // `self.sandbox`. This in-process sandbox is only here to satisfy
    // `ActionRuntime::try_new`; it is never invoked on this path.
    let executor: nebula_engine::ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    Arc::new(InProcessSandbox::new(executor))
}

#[tokio::test]
#[ignore = "requires pre-built fixture binary; run `cargo build -p nebula-plugin-sdk --bin nebula-plugin-schema-fixture` first, then `cargo nextest run -p nebula-engine --features out-of-process-plugins --test out_of_process_pool_dispatch --run-ignored all`"]
async fn discovered_plugin_action_round_trips_through_engine_pool() {
    let src_binary = fixture_binary_path();
    assert!(
        src_binary.exists(),
        "fixture binary not built: {}",
        src_binary.display(),
    );

    let scan_dir = tempfile::tempdir().expect("temp dir");
    let dest_binary = scan_dir
        .path()
        .join(src_binary.file_name().expect("binary file name"));
    std::fs::copy(&src_binary, &dest_binary).expect("copy fixture binary");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest_binary)
            .expect("stat fixture")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest_binary, perms).expect("chmod fixture");
    }

    // `sdk = "*"` matches any host SDK version.
    std::fs::write(
        scan_dir.path().join("plugin.toml"),
        "[nebula]\nsdk = \"*\"\n",
    )
    .expect("write plugin.toml");

    // Gate OPEN: feature compiled in + a non-empty plugin_dirs.
    let config = OutOfProcessConfig {
        plugin_dirs: vec![scan_dir.path().to_path_buf()],
        default_timeout: Duration::from_secs(5),
        max_processes_per_key: 2,
    };

    let action_registry = Arc::new(ActionRegistry::new());
    let mut plugin_registry = PluginRegistry::new();
    let supervisor = discover_into_registry(&config, &mut plugin_registry, &action_registry)
        .await
        .expect("valid pool capacity");

    // The schema fixture exposes `com.author.schema.describe` and replies
    // `{ "received": <input> }`.
    let action_key = "com.author.schema.describe";
    assert!(
        action_registry.get_by_str(action_key).is_none(), // legacy handler map: discovered actions register as factories
        "discovered action must register via the factory path, not the legacy handler map"
    );
    assert!(
        action_registry
            .get_factory(&nebula_core::ActionKey::new(action_key).unwrap())
            .is_some(),
        "discover_into_registry must register a pooled factory for the discovered action"
    );

    let runtime = Arc::new(
        ActionRuntime::try_new(
            Arc::clone(&action_registry),
            no_op_sandbox(),
            DataPassingPolicy::default(),
            MetricsRegistry::new(),
        )
        .expect("runtime"),
    );

    let node = NodeDefinition::new(
        nebula_core::NodeKey::new("describe_node").expect("node key"),
        "describe_node",
        action_key,
    )
    .expect("node definition");

    let ctx = TestContextBuilder::new().build();
    let input = json!({ "name": "ada", "age": 36 });

    // Dispatch through the factory path: this acquires a pool Lease, spawns
    // (or reuses) the ProcessSandbox, and round-trips the envelope to the
    // live fixture process.
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.execute_action_with_node(&node, None, input.clone(), &ctx, None),
    )
    .await
    .expect("pooled out-of-process dispatch must not hang")
    .expect("pooled out-of-process dispatch must succeed");

    match result {
        ActionResult::Success { output } => {
            assert_eq!(
                output.as_value().expect("value output"),
                &json!({ "received": input }),
                "the pooled ProcessSandbox round-trip must return the fixture's reply verbatim"
            );
        },
        other => panic!("expected ActionResult::Success, got {other:?}"),
    }

    // A second dispatch on the same key must reuse the warm pooled process
    // (no panic, identical reply) — exercises the Lease return-to-idle path.
    let result2 = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.execute_action_with_node(&node, None, json!({ "name": "grace" }), &ctx, None),
    )
    .await
    .expect("second pooled dispatch must not hang")
    .expect("second pooled dispatch must succeed");
    match result2 {
        ActionResult::Success { output } => {
            assert_eq!(
                output.as_value().expect("value output"),
                &json!({ "received": { "name": "grace" } }),
                "warm pooled process must serve the second dispatch correctly"
            );
        },
        other => panic!("expected ActionResult::Success, got {other:?}"),
    }

    // The supervisor owns the pools; shutdown drains the warm process
    // kept alive across the two dispatches (one pooled conn for this
    // single (binary, empty-scope) key) and SIGKILLs its child.
    assert_eq!(
        supervisor.shutdown(),
        1,
        "shutdown must drain the one warm pooled plugin process"
    );
}
