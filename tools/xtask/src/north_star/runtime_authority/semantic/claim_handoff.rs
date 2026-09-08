//! Durable command handoff policy before action execution.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ClaimHandoffError {
    #[error("claim handoff observation has missing, extra, or invalid fields")]
    Shape,
    #[error("claim handoff producer or inventory version is unsupported")]
    Version,
    #[error("claim handoff backend or command inventory is invalid")]
    Inventory,
    #[error("claim handoff left a dispatch claim reclaimable, exhausted, or claimable")]
    ClaimLifetime,
    #[error("claim handoff control command did not complete its durable handoff")]
    Handoff,
}

pub(crate) fn verify(value: &Value) -> Result<(), ClaimHandoffError> {
    let root = object(
        value,
        &[
            "backend",
            "commands",
            "contract",
            "producer_version",
            "scenario_inventory_version",
        ],
    )?;
    if root["contract"].as_str() != Some("claim-handoff")
        || root["producer_version"].as_u64() != Some(1)
        || root["scenario_inventory_version"].as_u64() != Some(1)
    {
        return Err(ClaimHandoffError::Version);
    }
    if !matches!(
        root["backend"].as_str(),
        Some("in-memory" | "sqlite" | "postgresql")
    ) {
        return Err(ClaimHandoffError::Inventory);
    }
    let commands = root["commands"]
        .as_array()
        .ok_or(ClaimHandoffError::Inventory)?;
    if commands.len() != 3 {
        return Err(ClaimHandoffError::Inventory);
    }
    let mut seen = BTreeSet::new();
    for command in commands {
        let name = command
            .get("command")
            .and_then(Value::as_str)
            .ok_or(ClaimHandoffError::Inventory)?;
        if !matches!(name, "Start" | "Resume" | "Restart") || !seen.insert(name) {
            return Err(ClaimHandoffError::Inventory);
        }
        if name == "Start" {
            verify_start(command)?;
        } else {
            verify_control(command, name)?;
        }
    }
    Ok(())
}

fn verify_start(value: &Value) -> Result<(), ClaimHandoffError> {
    let fields = object(
        value,
        &[
            "action_calls_while_delivery_terminal",
            "claim_exhausted_while_action_blocked",
            "claim_reclaimed_while_action_blocked",
            "command",
            "competing_claim_count",
            "competing_execution_lease_acquired",
            "final_exhausted",
            "final_reclaimed",
        ],
    )?;
    if fields["command"].as_str() != Some("Start")
        || fields["competing_execution_lease_acquired"].as_bool() != Some(false)
        || fields["final_reclaimed"].as_u64() != Some(0)
        || fields["final_exhausted"].as_u64() != Some(0)
    {
        return Err(ClaimHandoffError::Handoff);
    }
    verify_common(fields)
}

fn verify_control(value: &Value, name: &str) -> Result<(), ClaimHandoffError> {
    let fields = object(
        value,
        &[
            "action_calls_while_delivery_terminal",
            "claim_exhausted_while_action_blocked",
            "claim_generation",
            "claim_reclaimed_while_action_blocked",
            "command",
            "competing_claim_count",
            "durable_handoff_outcome",
        ],
    )?;
    if fields["command"].as_str() != Some(name)
        || fields["claim_generation"]
            .as_u64()
            .is_none_or(|value| value == 0)
        || fields["durable_handoff_outcome"].as_str() != Some("Accepted")
    {
        return Err(ClaimHandoffError::Handoff);
    }
    verify_common(fields)
}

fn verify_common(fields: &Map<String, Value>) -> Result<(), ClaimHandoffError> {
    if fields["action_calls_while_delivery_terminal"].as_u64() != Some(1)
        || fields["claim_reclaimed_while_action_blocked"].as_u64() != Some(0)
        || fields["claim_exhausted_while_action_blocked"].as_u64() != Some(0)
        || fields["competing_claim_count"].as_u64() != Some(0)
    {
        return Err(ClaimHandoffError::ClaimLifetime);
    }
    Ok(())
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, ClaimHandoffError> {
    let fields = value.as_object().ok_or(ClaimHandoffError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(ClaimHandoffError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_observation_qualifies_and_a_reclaimed_claim_is_rejected() {
        let value = super::super::fixtures::claim_handoff();
        assert_eq!(super::verify(&value), Ok(()));
        let mut invalid = value;
        invalid["commands"][0]["claim_reclaimed_while_action_blocked"] = 1.into();
        assert_eq!(
            super::verify(&invalid),
            Err(super::ClaimHandoffError::ClaimLifetime)
        );
    }

    #[test]
    fn a_pass_flag_cannot_replace_command_observations() {
        assert_eq!(
            super::verify(&serde_json::json!({"passed":true})),
            Err(super::ClaimHandoffError::Shape)
        );
    }
}
