//! Required-PostgreSQL matrix semantics from actual child-process reports.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use thiserror::Error;

#[cfg(test)]
#[path = "required_postgresql/tests.rs"]
mod tests;

const CASE: &str = "create_get_roundtrip::case_3_postgres";
const SCENARIOS: [&str; 4] = [
    "absent_url",
    "unreachable_database",
    "healthy_database",
    "disabled_feature",
];

#[cfg(test)]
pub(super) fn fixture() -> Value {
    let mut enabled: Value =
        super::super::json::decode(include_bytes!("required_postgresql/postgres_observed.json"))
            .expect("the committed enabled-feature observation is valid bounded JSON");
    let mut disabled: Value =
        super::super::json::decode(include_bytes!("required_postgresql/disabled_observed.json"))
            .expect("the committed disabled-feature observation is valid bounded JSON");
    enabled["scenarios"]
        .as_array_mut()
        .expect("the committed observation has a scenario array")
        .append(
            disabled["scenarios"]
                .as_array_mut()
                .expect("the committed observation has a scenario array"),
        );
    enabled
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RequiredPostgresqlError {
    #[error("required PostgreSQL observation has missing, extra, or invalid fields")]
    Shape,
    #[error("required PostgreSQL producer or inventory version is unsupported")]
    Version,
    #[error("required PostgreSQL scenario inventory is missing, duplicated, or unexpected")]
    Inventory,
    #[error("required PostgreSQL child settings, identity, or exit are inconsistent")]
    Process,
    #[error("required PostgreSQL child report does not prove the exact required matrix case ran")]
    Report,
    #[error("required PostgreSQL failure cause does not match the required absence condition")]
    Diagnostic,
}

/// Verify the merged four-scenario observation after authenticated, bounded JSON admission.
pub(crate) fn verify(value: &Value) -> Result<(), RequiredPostgresqlError> {
    let root = object(
        value,
        &[
            "contract",
            "producer_version",
            "scenario_inventory_version",
            "scenarios",
        ],
    )?;
    if root["contract"].as_str() != Some("required-postgresql")
        || root["producer_version"].as_u64() != Some(2)
        || root["scenario_inventory_version"].as_u64() != Some(1)
    {
        return Err(RequiredPostgresqlError::Version);
    }
    let scenarios = root["scenarios"]
        .as_array()
        .ok_or(RequiredPostgresqlError::Inventory)?;
    if scenarios.len() != SCENARIOS.len() {
        return Err(RequiredPostgresqlError::Inventory);
    }
    let mut seen = BTreeSet::new();
    for scenario in scenarios {
        let fields = object(scenario, &["scenario", "events"])?;
        let name = fields["scenario"]
            .as_str()
            .ok_or(RequiredPostgresqlError::Inventory)?;
        if !SCENARIOS.contains(&name) || !seen.insert(name) {
            return Err(RequiredPostgresqlError::Inventory);
        }
        let events = fields["events"]
            .as_array()
            .ok_or(RequiredPostgresqlError::Process)?;
        if events.len() != 1 {
            return Err(RequiredPostgresqlError::Process);
        }
        let event = object(
            &events[0],
            &[
                "sequence",
                "kind",
                "test_case",
                "required_postgres",
                "postgres_feature",
                "exit_code",
                "stdout",
                "stderr",
            ],
        )?;
        let healthy = name == "healthy_database";
        if event["sequence"].as_u64() != Some(0)
            || event["kind"].as_str() != Some("process_exited")
            || event["test_case"].as_str() != Some(CASE)
            || event["required_postgres"].as_bool() != Some(true)
            || event["postgres_feature"].as_bool() != Some(name != "disabled_feature")
            || event["exit_code"].as_i64() != Some(if healthy { 0 } else { 101 })
        {
            return Err(RequiredPostgresqlError::Process);
        }
        let stdout = output(&event["stdout"])?;
        let stderr = output(&event["stderr"])?;
        verify_report(stdout, healthy)?;
        verify_diagnostic(stderr, name)?;
    }
    Ok(())
}

fn output(value: &Value) -> Result<&str, RequiredPostgresqlError> {
    let text = value.as_str().ok_or(RequiredPostgresqlError::Shape)?;
    // Match the enclosing bounded decoder; process output is never an
    // escape hatch around its per-string limit.
    if text.len() > 4096 || text.contains('\u{1b}') {
        return Err(RequiredPostgresqlError::Shape);
    }
    Ok(text)
}

fn verify_report(stdout: &str, healthy: bool) -> Result<(), RequiredPostgresqlError> {
    let lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() != if healthy { 3 } else { 6 } || lines[0] != "running 1 test" {
        return Err(RequiredPostgresqlError::Report);
    }
    let case_line = if healthy {
        "test create_get_roundtrip::case_3_postgres ... ok"
    } else {
        "test create_get_roundtrip::case_3_postgres ... FAILED"
    };
    if lines[1] != case_line {
        return Err(RequiredPostgresqlError::Report);
    }
    if !healthy && (lines[2] != "failures:" || lines[3] != "failures:" || lines[4] != CASE) {
        return Err(RequiredPostgresqlError::Report);
    }
    let summary = lines.last().ok_or(RequiredPostgresqlError::Report)?;
    let prefix = if healthy {
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; "
    } else {
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; "
    };
    let tail = summary
        .strip_prefix(prefix)
        .ok_or(RequiredPostgresqlError::Report)?;
    let (filtered, duration) = tail
        .split_once(" filtered out; finished in ")
        .ok_or(RequiredPostgresqlError::Report)?;
    if filtered.is_empty()
        || !filtered.bytes().all(|byte| byte.is_ascii_digit())
        || filtered.parse::<u64>().is_err()
    {
        return Err(RequiredPostgresqlError::Report);
    }
    let duration = duration
        .strip_suffix('s')
        .ok_or(RequiredPostgresqlError::Report)?;
    let (seconds, fraction) = duration
        .split_once('.')
        .ok_or(RequiredPostgresqlError::Report)?;
    if seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() != 2
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(RequiredPostgresqlError::Report);
    }
    let elapsed = duration
        .parse::<f64>()
        .map_err(|_| RequiredPostgresqlError::Report)?;
    if !elapsed.is_finite() || elapsed > 45.0 {
        return Err(RequiredPostgresqlError::Report);
    }
    Ok(())
}

fn verify_diagnostic(stderr: &str, scenario: &str) -> Result<(), RequiredPostgresqlError> {
    if scenario == "healthy_database" {
        return if stderr.is_empty() {
            Ok(())
        } else {
            Err(RequiredPostgresqlError::Diagnostic)
        };
    }
    let expected = match scenario {
        "absent_url" => "NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL is absent or non-Unicode",
        "disabled_feature" => "NEBULA_REQUIRE_POSTGRES is set but the postgres feature is disabled",
        "unreachable_database" => "connect Postgres (DATABASE_URL): PoolTimedOut",
        _ => return Err(RequiredPostgresqlError::Inventory),
    };
    let lines = stderr
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() != 3
        || lines[1] != expected
        || lines[2]
            != "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace"
    {
        return Err(RequiredPostgresqlError::Diagnostic);
    }
    let header = lines[0]
        .strip_prefix("thread 'create_get_roundtrip::case_3_postgres' (")
        .ok_or(RequiredPostgresqlError::Diagnostic)?;
    let (thread, location) = header
        .split_once(") panicked at crates/storage/tests/conformance/mod.rs:")
        .ok_or(RequiredPostgresqlError::Diagnostic)?;
    if thread.is_empty() || !thread.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RequiredPostgresqlError::Diagnostic);
    }
    let location = location
        .strip_suffix(':')
        .ok_or(RequiredPostgresqlError::Diagnostic)?;
    let coordinates: [&str; 2] = location
        .split_once(':')
        .ok_or(RequiredPostgresqlError::Diagnostic)?
        .into();
    if !coordinates.iter().all(|number| {
        !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
            && number.parse::<u64>().is_ok_and(|number| number > 0)
    }) {
        return Err(RequiredPostgresqlError::Diagnostic);
    }
    Ok(())
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, RequiredPostgresqlError> {
    let fields = value.as_object().ok_or(RequiredPostgresqlError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(RequiredPostgresqlError::Shape);
    }
    Ok(fields)
}
