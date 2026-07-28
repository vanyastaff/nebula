use std::{fs, path::PathBuf, process::Command};

#[test]
fn live_north_star_registry_and_schema_validate() {
    let output = Command::new(env!("CARGO_BIN_EXE_nebula-xtask"))
        .args(["north-star-gates", "validate"])
        .current_dir(workspace_root().join("tools/xtask"))
        .output()
        .expect("nebula-xtask runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        b"{\"registry_version\":1,\"schema_version\":1,\"gate_count\":22,\"status\":\"valid\"}\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn required_pull_request_path_runs_gate_validation_before_selection() {
    let workflow = fs::read_to_string(workspace_root().join(".github/workflows/test-matrix.yml"))
        .expect("required test workflow is readable");
    let gate_validation = workflow
        .find("cargo xtask north-star-gates validate")
        .expect("required workflow invokes North Star validation");
    let package_selection = workflow
        .find("cargo xtask \"${plan_args[@]}\"")
        .expect("required workflow invokes metadata-driven package selection");

    assert!(
        gate_validation < package_selection,
        "gate validation must run before package selection without participating in it"
    );
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("xtask manifest has a workspace root")
        .to_path_buf()
}
