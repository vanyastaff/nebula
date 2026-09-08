//! Same-processor ABA policy for both durable queues.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ClaimFencingError {
    #[error("claim fencing observation has missing, extra, or invalid fields")]
    Shape,
    #[error("claim fencing producer or inventory version is unsupported")]
    Version,
    #[error("claim fencing backend or queue inventory is invalid")]
    Inventory,
    #[error("claim fencing claim generation did not advance monotonically")]
    Generation,
    #[error("claim fencing stale claim changed or retained authority over the row")]
    StaleMutation,
}

pub(crate) fn verify(value: &Value) -> Result<(), ClaimFencingError> {
    let root = object(
        value,
        &[
            "backend",
            "contract",
            "producer_version",
            "queues",
            "scenario_inventory_version",
        ],
    )?;
    if root["contract"].as_str() != Some("claim-generation-fencing")
        || root["producer_version"].as_u64() != Some(1)
        || root["scenario_inventory_version"].as_u64() != Some(1)
    {
        return Err(ClaimFencingError::Version);
    }
    if !matches!(
        root["backend"].as_str(),
        Some("in-memory" | "sqlite" | "postgresql")
    ) {
        return Err(ClaimFencingError::Inventory);
    }
    let queues = root["queues"]
        .as_array()
        .ok_or(ClaimFencingError::Inventory)?;
    if queues.len() != 2 {
        return Err(ClaimFencingError::Inventory);
    }
    let mut seen = BTreeSet::new();
    for queue in queues {
        let fields = object(
            queue,
            &[
                "current_generation",
                "current_owner_completed_after_stale_attempts",
                "late_ack_fenced",
                "late_nack_fenced",
                "queue",
                "row_id",
                "same_processor",
                "stale_generation_mutation_count",
                "superseded_generation",
            ],
        )?;
        let name = fields["queue"]
            .as_str()
            .ok_or(ClaimFencingError::Inventory)?;
        if !matches!(name, "control" | "job") || !seen.insert(name) {
            return Err(ClaimFencingError::Inventory);
        }
        let row = fields["row_id"]
            .as_array()
            .ok_or(ClaimFencingError::Shape)?;
        if row.len() != 16
            || row
                .iter()
                .any(|byte| byte.as_u64().is_none_or(|byte| byte > 255))
        {
            return Err(ClaimFencingError::Shape);
        }
        let superseded = fields["superseded_generation"]
            .as_u64()
            .ok_or(ClaimFencingError::Generation)?;
        let current = fields["current_generation"]
            .as_u64()
            .ok_or(ClaimFencingError::Generation)?;
        if superseded == 0 || current <= superseded {
            return Err(ClaimFencingError::Generation);
        }
        if fields["same_processor"].as_bool() != Some(true)
            || fields["late_ack_fenced"].as_bool() != Some(true)
            || fields["late_nack_fenced"].as_bool() != Some(true)
            || fields["stale_generation_mutation_count"].as_u64() != Some(0)
            || fields["current_owner_completed_after_stale_attempts"].as_bool() != Some(true)
        {
            return Err(ClaimFencingError::StaleMutation);
        }
    }
    Ok(())
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, ClaimFencingError> {
    let fields = value.as_object().ok_or(ClaimFencingError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(ClaimFencingError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_observation_qualifies_and_a_false_fence_is_rejected() {
        let value = super::super::fixtures::claim_fencing();
        assert_eq!(super::verify(&value), Ok(()));
        let mut invalid = value;
        invalid["queues"][0]["late_ack_fenced"] = false.into();
        assert_eq!(
            super::verify(&invalid),
            Err(super::ClaimFencingError::StaleMutation)
        );
    }

    #[test]
    fn empty_and_authored_pass_claims_do_not_qualify() {
        assert_eq!(
            super::verify(&serde_json::json!({})),
            Err(super::ClaimFencingError::Shape)
        );
        assert_eq!(
            super::verify(&serde_json::json!({"passed":true})),
            Err(super::ClaimFencingError::Shape)
        );
    }
}
