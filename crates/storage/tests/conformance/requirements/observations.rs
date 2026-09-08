//! Retained process observations from the real PostgreSQL matrix entry point.

use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

const CASE: &str = "create_get_roundtrip::case_3_postgres";

fn observe(scenario: &str, command: &mut Command) -> Value {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("actual matrix child starts");
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("required matrix child timed out; timeout is not absence evidence");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.stdout.len() <= 32 * 1024);
    assert!(output.stderr.len() <= 32 * 1024);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("running 1 test"));
    assert!(!stderr.contains("skipping Postgres case"));
    assert!(!stdout.contains("0 passed; 0 failed"));
    json!({
        "scenario": scenario,
        "events": [{
            "sequence": 0,
            "kind": "process_exited",
            "test_case": CASE,
            "required_postgres": true,
            "postgres_feature": cfg!(feature = "postgres"),
            "exit_code": output.status.code().expect("signal death is not gate evidence"),
            "stdout": stdout,
            "stderr": stderr
        }]
    })
}

fn required_case() -> Command {
    let mut command = super::postgres_case();
    command
        .args(["--color", "never"])
        .env("NEBULA_REQUIRE_POSTGRES", "1")
        .env("RUST_BACKTRACE", "0")
        .env("RUST_LIB_BACKTRACE", "0");
    command
}

fn retain(scenarios: Vec<Value>) {
    if let Some(path) = std::env::var_os("NEBULA_POSTGRES_REQUIREMENT_OBSERVATIONS_PATH") {
        assert_eq!(std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(), Ok("1"));
        let path = std::path::Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let bytes = serde_json::to_vec_pretty(&json!({
            "producer_version": 2,
            "contract": "required-postgresql",
            "scenario_inventory_version": 1,
            "scenarios": scenarios
        }))
        .unwrap();
        assert!(bytes.len() <= 256 * 1024);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
}

#[cfg(feature = "postgres")]
#[test]
fn required_postgres_process_observations() {
    let absent = observe("absent_url", &mut required_case());
    assert_eq!(absent["events"][0]["exit_code"], 101);
    assert!(
        absent["events"][0]["stderr"]
            .as_str()
            .unwrap()
            .contains("NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL")
    );
    // Port zero cannot identify a listening TCP service. This never falls back
    // to a developer's configured database or assumes a spare ephemeral port.
    let unavailable = observe(
        "unreachable_database",
        required_case().env(
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:0/nebula_absent?sslmode=disable",
        ),
    );
    assert_eq!(unavailable["events"][0]["exit_code"], 101);
    let mut scenarios = vec![absent, unavailable];
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let healthy = observe("healthy_database", required_case().env("DATABASE_URL", url));
        assert_eq!(healthy["events"][0]["exit_code"], 0);
        assert!(
            healthy["events"][0]["stdout"]
                .as_str()
                .unwrap()
                .contains("1 passed; 0 failed; 0 ignored")
        );
        scenarios.push(healthy);
    } else {
        assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
        assert!(std::env::var_os("NEBULA_POSTGRES_REQUIREMENT_OBSERVATIONS_PATH").is_none());
    }
    retain(scenarios);
}

#[cfg(not(feature = "postgres"))]
#[test]
fn required_postgres_feature_absence_observations() {
    let disabled = observe("disabled_feature", &mut required_case());
    assert_eq!(disabled["events"][0]["exit_code"], 101);
    assert!(
        disabled["events"][0]["stderr"]
            .as_str()
            .unwrap()
            .contains("NEBULA_REQUIRE_POSTGRES is set but the postgres feature is disabled")
    );
    retain(vec![disabled]);
}
