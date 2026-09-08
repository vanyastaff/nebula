//! Persistence ownership, fencing, and atomic-write observation policy.

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum PersistenceAuthorityError {
    #[error("persistence-authority observation has missing, extra, or invalid fields")]
    Shape,
    #[error("persistence-authority producer, inventory, or backend is unsupported")]
    Version,
    #[error("a stale execution owner retained mutation authority")]
    OwnerFencing,
    #[error("lease recovery did not fence the expired ownership generation")]
    LeaseRecovery,
    #[error("state, journal, and outbox did not commit as one transition")]
    AtomicTransition,
    #[error("workflow and publication writes did not roll back as one unit")]
    PublicationAtomicity,
}

#[cfg(test)]
pub(crate) fn verify(value: &Value) -> Result<(), PersistenceAuthorityError> {
    let root = root(value)?;
    verify_owner_fencing(&root["owner_fencing"])?;
    verify_lease_recovery(&root["lease_recovery"])?;
    verify_atomic_transition(&root["atomic_transition"])?;
    verify_publication_atomicity(&root["publication_atomicity"])
}

pub(crate) fn verify_case(value: &Value, case: &str) -> Result<(), PersistenceAuthorityError> {
    let root = root(value)?;
    match case {
        "owner-fencing" => verify_owner_fencing(&root["owner_fencing"]),
        "lease-recovery" => verify_lease_recovery(&root["lease_recovery"]),
        "atomic-transition" => verify_atomic_transition(&root["atomic_transition"]),
        "publication-atomicity" => verify_publication_atomicity(&root["publication_atomicity"]),
        _ => Err(PersistenceAuthorityError::Shape),
    }
}

fn root(value: &Value) -> Result<&Map<String, Value>, PersistenceAuthorityError> {
    let root = object(
        value,
        &[
            "atomic_transition",
            "backend",
            "contract",
            "lease_recovery",
            "owner_fencing",
            "producer_version",
            "publication_atomicity",
            "scenario_inventory_version",
        ],
    )?;
    if root["contract"].as_str() != Some("persistence-authority")
        || root["producer_version"].as_u64() != Some(1)
        || root["scenario_inventory_version"].as_u64() != Some(1)
        || !matches!(
            root["backend"].as_str(),
            Some("in-memory" | "sqlite" | "postgresql")
        )
    {
        return Err(PersistenceAuthorityError::Version);
    }
    Ok(root)
}

fn verify_owner_fencing(value: &Value) -> Result<(), PersistenceAuthorityError> {
    let fields = object(
        value,
        &[
            "live_generation",
            "outcome",
            "stale_generation",
            "stale_mutation_count",
            "state_after",
            "state_before",
            "version_after",
            "version_before",
        ],
    )?;
    if fields["live_generation"]
        .as_u64()
        .is_none_or(|value| value == 0)
        || fields["stale_generation"].as_u64() != Some(0)
        || !matches!(
            fields["outcome"].as_str(),
            Some("FencedOut" | "VersionConflict { actual: 0 }")
        )
        || fields["stale_mutation_count"].as_u64() != Some(0)
        || fields["version_before"] != fields["version_after"]
        || fields["state_before"] != fields["state_after"]
    {
        return Err(PersistenceAuthorityError::OwnerFencing);
    }
    Ok(())
}

fn verify_lease_recovery(value: &Value) -> Result<(), PersistenceAuthorityError> {
    let fields = object(
        value,
        &[
            "expired_generation",
            "first_generation",
            "recovered_generation",
            "recovered_generation_advanced",
            "same_holder_live_reacquire_granted",
        ],
    )?;
    let expired = fields["expired_generation"]
        .as_u64()
        .ok_or(PersistenceAuthorityError::LeaseRecovery)?;
    let recovered = fields["recovered_generation"]
        .as_u64()
        .ok_or(PersistenceAuthorityError::LeaseRecovery)?;
    if fields["first_generation"]
        .as_u64()
        .is_none_or(|value| value == 0)
        || fields["same_holder_live_reacquire_granted"].as_bool() != Some(false)
        || fields["recovered_generation_advanced"].as_bool() != Some(true)
        || recovered <= expired
    {
        return Err(PersistenceAuthorityError::LeaseRecovery);
    }
    Ok(())
}

fn verify_atomic_transition(value: &Value) -> Result<(), PersistenceAuthorityError> {
    let fields = object(
        value,
        &[
            "execution_version",
            "journal_entry_count",
            "journal_payload",
            "outbox_claim_count",
            "outbox_command",
            "outbox_command_id",
            "owner_fence_generation",
            "state",
            "transition_outcome",
        ],
    )?;
    let command_id = fields["outbox_command_id"]
        .as_array()
        .ok_or(PersistenceAuthorityError::AtomicTransition)?;
    if fields["transition_outcome"]
        .as_str()
        .is_none_or(|outcome| !outcome.starts_with("Applied"))
        || fields["execution_version"].as_u64() != Some(1)
        || fields["state"] != serde_json::json!({"s":"running"})
        || fields["journal_entry_count"].as_u64() != Some(1)
        || fields["journal_payload"] != serde_json::json!({"event":"transition"})
        || fields["outbox_claim_count"].as_u64() != Some(1)
        || fields["outbox_command"].as_str() != Some("Cancel")
        || command_id.len() != 16
        || !command_id.iter().all(|byte| byte.as_u64() == Some(1))
        || fields["owner_fence_generation"]
            .as_u64()
            .is_none_or(|value| value == 0)
    {
        return Err(PersistenceAuthorityError::AtomicTransition);
    }
    Ok(())
}

fn verify_publication_atomicity(value: &Value) -> Result<(), PersistenceAuthorityError> {
    let fields = object(
        value,
        &[
            "candidate_version_after_stale_cas",
            "created_workflow_version",
            "duplicate_version_outcome",
            "orphan_workflow_after_duplicate",
            "published_version",
            "row_version_after_stale_cas",
            "stale_cas_outcome",
        ],
    )?;
    if fields["created_workflow_version"].as_u64() != Some(1)
        || fields["published_version"].as_u64() != Some(1)
        || fields["stale_cas_outcome"].as_str() != Some("Conflict")
        || fields["row_version_after_stale_cas"].as_u64() != Some(1)
        || !fields["candidate_version_after_stale_cas"].is_null()
        || fields["duplicate_version_outcome"].as_str() != Some("Duplicate")
        || fields["orphan_workflow_after_duplicate"].as_bool() != Some(false)
    {
        return Err(PersistenceAuthorityError::PublicationAtomicity);
    }
    Ok(())
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, PersistenceAuthorityError> {
    let fields = value.as_object().ok_or(PersistenceAuthorityError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(PersistenceAuthorityError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_observation_qualifies_and_a_stale_mutation_is_rejected() {
        let value = super::super::fixtures::persistence_authority();
        assert_eq!(super::verify(&value), Ok(()));
        let mut invalid = value;
        invalid["owner_fencing"]["stale_mutation_count"] = 1.into();
        assert_eq!(
            super::verify(&invalid),
            Err(super::PersistenceAuthorityError::OwnerFencing)
        );
    }

    #[test]
    fn authored_success_claim_is_not_evidence() {
        assert_eq!(
            super::verify(&serde_json::json!({"passed":true})),
            Err(super::PersistenceAuthorityError::Shape)
        );
    }
}
