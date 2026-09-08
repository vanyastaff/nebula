//! Durable effect policy over ledger state and the provider's independent commit log.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use thiserror::Error;

const SCENARIOS: [&str; 5] = [
    "applied",
    "opaque-ambiguity-recovery",
    "stable-key-retry",
    "crashed-invocation-recovery",
    "stale-owner-fence",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RemoteEffectError {
    #[error("remote effect observation has missing, extra, or invalid fields")]
    Shape,
    #[error("remote effect producer or inventory version is unsupported")]
    Version,
    #[error("remote effect backend or scenario inventory is invalid")]
    Inventory,
    #[error("remote effect provider log is not bound to the storage-minted operation identity")]
    Identity,
    #[error("remote effect provider log contains a stale or duplicate committed business effect")]
    CommittedEffect,
    #[error("remote effect durable protocol state differs from the required scenario")]
    Protocol,
}

pub(crate) fn verify(value: &Value) -> Result<(), RemoteEffectError> {
    let root = object(
        value,
        &[
            "backend",
            "contract",
            "producer_version",
            "scenario_inventory_version",
            "scenarios",
        ],
    )?;
    if root["contract"].as_str() != Some("remote-effect-protocol")
        || root["producer_version"].as_u64() != Some(1)
        || root["scenario_inventory_version"].as_u64() != Some(1)
    {
        return Err(RemoteEffectError::Version);
    }
    if !matches!(
        root["backend"].as_str(),
        Some("in-memory" | "sqlite" | "postgresql")
    ) {
        return Err(RemoteEffectError::Inventory);
    }
    let scenarios = root["scenarios"]
        .as_array()
        .ok_or(RemoteEffectError::Inventory)?;
    if scenarios.len() != SCENARIOS.len() {
        return Err(RemoteEffectError::Inventory);
    }
    let mut indexed = BTreeMap::new();
    for scenario in scenarios {
        let name = scenario
            .get("scenario")
            .and_then(Value::as_str)
            .ok_or(RemoteEffectError::Inventory)?;
        if !SCENARIOS.contains(&name) || indexed.insert(name, scenario).is_some() {
            return Err(RemoteEffectError::Inventory);
        }
    }
    verify_ordinary(indexed["applied"], "applied")?;
    verify_ordinary(
        indexed["opaque-ambiguity-recovery"],
        "opaque-ambiguity-recovery",
    )?;
    verify_ordinary(indexed["stable-key-retry"], "stable-key-retry")?;
    verify_crash(indexed["crashed-invocation-recovery"])?;
    verify_fence(indexed["stale-owner-fence"])
}

fn verify_identity(fields: &Map<String, Value>) -> Result<(usize, usize), RemoteEffectError> {
    let operation = fields["operation_id"]
        .as_str()
        .filter(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or(RemoteEffectError::Identity)?;
    let calls = string_array(&fields["provider_calls"])?;
    let commits = string_array(&fields["provider_commits"])?;
    if calls.iter().chain(&commits).any(|id| *id != operation) {
        return Err(RemoteEffectError::Identity);
    }
    if fields["unique_business_effects"].as_u64() != Some(1)
        || commits.len() != 1
        || commits.len() > calls.len()
    {
        return Err(RemoteEffectError::CommittedEffect);
    }
    Ok((calls.len(), commits.len()))
}

fn verify_ordinary(value: &Value, name: &str) -> Result<(), RemoteEffectError> {
    let fields = object(
        value,
        &[
            "calls_after_first_turn",
            "execution_completed",
            "invocations",
            "known_evidence",
            "operation_id",
            "phase",
            "provider_calls",
            "provider_commits",
            "queries",
            "recovery_attempted",
            "scenario",
            "unique_business_effects",
        ],
    )?;
    if fields["scenario"].as_str() != Some(name) {
        return Err(RemoteEffectError::Inventory);
    }
    let (calls, _) = verify_identity(fields)?;
    let expected = match name {
        "applied" => (1, 1, "Resolved", true, true, false),
        "opaque-ambiguity-recovery" => (1, 1, "OutcomeUnknown", false, false, true),
        "stable-key-retry" => (2, 2, "Resolved", true, true, false),
        _ => return Err(RemoteEffectError::Inventory),
    };
    if calls != expected.0
        || fields["calls_after_first_turn"].as_u64() != Some(expected.0 as u64)
        || fields["invocations"].as_u64() != Some(expected.1)
        || fields["phase"].as_str() != Some(expected.2)
        || fields["known_evidence"].as_bool() != Some(expected.3)
        || fields["execution_completed"].as_bool() != Some(expected.4)
        || fields["recovery_attempted"].as_bool() != Some(expected.5)
        || fields["queries"].as_u64() != Some(0)
    {
        return Err(RemoteEffectError::Protocol);
    }
    Ok(())
}

fn verify_crash(value: &Value) -> Result<(), RemoteEffectError> {
    let fields = object(
        value,
        &[
            "calls_after_recovery",
            "calls_before_recovery",
            "engine_recreated",
            "operation_id",
            "phase",
            "provider_calls",
            "provider_commits",
            "scenario",
            "unique_business_effects",
        ],
    )?;
    let (calls, _) = verify_identity(fields)?;
    if fields["scenario"].as_str() != Some("crashed-invocation-recovery")
        || calls != 1
        || fields["calls_before_recovery"].as_u64() != Some(1)
        || fields["calls_after_recovery"].as_u64() != Some(1)
        || fields["engine_recreated"].as_bool() != Some(true)
        || fields["phase"].as_str() != Some("InvocationOutstanding")
    {
        return Err(RemoteEffectError::Protocol);
    }
    Ok(())
}

fn verify_fence(value: &Value) -> Result<(), RemoteEffectError> {
    let fields = object(
        value,
        &[
            "operation_id",
            "phase_after_stale_response",
            "phase_before_stale_response",
            "provider_calls",
            "provider_commits",
            "scenario",
            "stale_storage_mutations",
            "successor_fence_differs",
            "unique_business_effects",
        ],
    )?;
    let (calls, _) = verify_identity(fields)?;
    if fields["scenario"].as_str() != Some("stale-owner-fence")
        || calls != 1
        || fields["phase_before_stale_response"].as_str() != Some("InvocationOutstanding")
        || fields["phase_after_stale_response"].as_str() != Some("InvocationOutstanding")
        || fields["successor_fence_differs"].as_bool() != Some(true)
        || fields["stale_storage_mutations"].as_u64() != Some(0)
    {
        return Err(RemoteEffectError::Protocol);
    }
    Ok(())
}

fn string_array(value: &Value) -> Result<Vec<&str>, RemoteEffectError> {
    value
        .as_array()
        .ok_or(RemoteEffectError::Shape)?
        .iter()
        .map(|value| value.as_str().ok_or(RemoteEffectError::Shape))
        .collect()
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, RemoteEffectError> {
    let fields = value.as_object().ok_or(RemoteEffectError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(RemoteEffectError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
pub(super) fn fixture() -> Value {
    let ordinary = |name: &str, calls: usize, phase: &str, evidence: bool| {
        serde_json::json!({
            "calls_after_first_turn": calls,
            "execution_completed": phase == "Resolved",
            "invocations": calls,
            "known_evidence": evidence,
            "operation_id": "0123456789abcdef0123456789abcdef",
            "phase": phase,
            "provider_calls": vec!["0123456789abcdef0123456789abcdef"; calls],
            "provider_commits": ["0123456789abcdef0123456789abcdef"],
            "queries": 0,
            "recovery_attempted": name == "opaque-ambiguity-recovery",
            "scenario": name,
            "unique_business_effects": 1
        })
    };
    serde_json::json!({
        "backend": "in-memory",
        "contract": "remote-effect-protocol",
        "producer_version": 1,
        "scenario_inventory_version": 1,
        "scenarios": [
            ordinary("applied", 1, "Resolved", true),
            ordinary("opaque-ambiguity-recovery", 1, "OutcomeUnknown", false),
            ordinary("stable-key-retry", 2, "Resolved", true),
            {"calls_after_recovery":1,"calls_before_recovery":1,"engine_recreated":true,"operation_id":"0123456789abcdef0123456789abcdef","phase":"InvocationOutstanding","provider_calls":["0123456789abcdef0123456789abcdef"],"provider_commits":["0123456789abcdef0123456789abcdef"],"scenario":"crashed-invocation-recovery","unique_business_effects":1},
            {"operation_id":"0123456789abcdef0123456789abcdef","phase_after_stale_response":"InvocationOutstanding","phase_before_stale_response":"InvocationOutstanding","provider_calls":["0123456789abcdef0123456789abcdef"],"provider_commits":["0123456789abcdef0123456789abcdef"],"scenario":"stale-owner-fence","stale_storage_mutations":0,"successor_fence_differs":true,"unique_business_effects":1}
        ]
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn exact_protocol_and_provider_log_qualify() {
        assert_eq!(super::verify(&super::fixture()), Ok(()));
    }

    #[test]
    fn duplicate_commit_stale_identity_and_reinvocation_after_crash_fail() {
        let mut duplicate = super::fixture();
        duplicate["scenarios"][0]["provider_commits"] = json!([
            "0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef"
        ]);
        assert_eq!(
            super::verify(&duplicate),
            Err(super::RemoteEffectError::CommittedEffect)
        );

        let mut stale = super::fixture();
        stale["scenarios"][4]["provider_commits"][0] = json!("ffffffffffffffffffffffffffffffff");
        assert_eq!(
            super::verify(&stale),
            Err(super::RemoteEffectError::Identity)
        );

        let mut reinvoked = super::fixture();
        reinvoked["scenarios"][3]["provider_calls"]
            .as_array_mut()
            .unwrap()
            .push(json!("0123456789abcdef0123456789abcdef"));
        assert_eq!(
            super::verify(&reinvoked),
            Err(super::RemoteEffectError::Protocol)
        );
    }
}
