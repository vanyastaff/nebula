//! Exact persisted state, output, and routing survive engine reconstruction.

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum CheckpointReconnectError {
    #[error("checkpoint observation has missing, extra, or invalid fields")]
    Shape,
    #[error("checkpoint producer or inventory version is unsupported")]
    Version,
    #[error("cold and warm turns do not preserve one execution and exact revisions")]
    Identity,
    #[error("checkpoint or output changed across restart")]
    Checkpoint,
    #[error("engine reconstruction evidence is incomplete")]
    Recreation,
}

pub(crate) fn verify(value: &Value) -> Result<(), CheckpointReconnectError> {
    let root = object(
        value,
        &[
            "backend",
            "contract",
            "producer_version",
            "scenario_inventory_version",
            "turns",
        ],
    )?;
    if root["contract"].as_str() != Some("durable-checkpoint-reconnect")
        || root["producer_version"].as_u64() != Some(1)
        || root["scenario_inventory_version"].as_u64() != Some(1)
    {
        return Err(CheckpointReconnectError::Version);
    }
    let backend = root["backend"]
        .as_str()
        .ok_or(CheckpointReconnectError::Shape)?;
    if !matches!(backend, "in-memory" | "sqlite" | "postgresql") {
        return Err(CheckpointReconnectError::Shape);
    }
    let turns = root["turns"]
        .as_array()
        .ok_or(CheckpointReconnectError::Shape)?;
    if turns.len() != 2 {
        return Err(CheckpointReconnectError::Shape);
    }
    let cold = turn(&turns[0], "cold-start", "Paused", "predecessor", 1)?;
    let warm = turn(&turns[1], "warm-resume", "Completed", "successor", 2)?;
    for key in [
        "execution_id",
        "executable_plan_revision_id",
        "worker_flavor_revision_id",
    ] {
        if cold[key] != warm[key] {
            return Err(CheckpointReconnectError::Identity);
        }
    }
    let execution = cold["execution_id"]
        .as_str()
        .filter(|id| id.starts_with("exe_") && id.len() == 30)
        .ok_or(CheckpointReconnectError::Identity)?;
    for key in ["executable_plan_revision_id", "worker_flavor_revision_id"] {
        digest(&cold[key])?;
    }
    if cold["checkpoint_output"] != warm["checkpoint_output"]
        || cold["checkpoint_output"]["identity"].as_str() != Some(execution)
        || cold["checkpoint_output"]["payload"].as_str() != Some("persisted predecessor output")
        || warm["execution_version"].as_u64() <= cold["execution_version"].as_u64()
    {
        return Err(CheckpointReconnectError::Checkpoint);
    }
    if cold["engine_recreated"].as_bool() != Some(true)
        || warm["engine_recreated"].as_bool() != Some(true)
    {
        return Err(CheckpointReconnectError::Recreation);
    }
    Ok(())
}

fn turn<'a>(
    value: &'a Value,
    expected_turn: &str,
    status: &str,
    node: &str,
    calls: u64,
) -> Result<&'a Map<String, Value>, CheckpointReconnectError> {
    let fields = object(
        value,
        &[
            "action_calls_total",
            "checkpoint_node",
            "checkpoint_output",
            "engine_recreated",
            "executable_plan_revision_id",
            "execution_id",
            "execution_version",
            "status",
            "turn",
            "worker_flavor_revision_id",
        ],
    )?;
    if fields["turn"].as_str() != Some(expected_turn)
        || fields["status"].as_str() != Some(status)
        || fields["checkpoint_node"].as_str() != Some(node)
        || fields["action_calls_total"].as_u64() != Some(calls)
        || fields["execution_version"].as_u64().is_none()
    {
        return Err(CheckpointReconnectError::Checkpoint);
    }
    let output = object(&fields["checkpoint_output"], &["identity", "payload"])?;
    if output["identity"].as_str().is_none() || output["payload"].as_str().is_none() {
        return Err(CheckpointReconnectError::Checkpoint);
    }
    Ok(fields)
}

fn digest(value: &Value) -> Result<(), CheckpointReconnectError> {
    if value.as_str().is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }) {
        Ok(())
    } else {
        Err(CheckpointReconnectError::Identity)
    }
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, CheckpointReconnectError> {
    let fields = value.as_object().ok_or(CheckpointReconnectError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(CheckpointReconnectError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_observation_qualifies_and_a_repeated_action_is_rejected() {
        let value = super::super::fixtures::checkpoint_reconnect();
        assert_eq!(super::verify(&value), Ok(()));
        let mut invalid = value;
        invalid["turns"][1]["action_calls_total"] = 3.into();
        assert_eq!(
            super::verify(&invalid),
            Err(super::CheckpointReconnectError::Checkpoint)
        );
    }

    #[test]
    fn an_authored_result_cannot_replace_restart_snapshots() {
        assert_eq!(
            super::verify(&serde_json::json!({"passed":true})),
            Err(super::CheckpointReconnectError::Shape)
        );
    }
}
