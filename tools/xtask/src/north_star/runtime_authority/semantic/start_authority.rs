//! Predicates over atomic start and exact-routing observations.

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum StartAuthorityError {
    #[error("start-authority observation has missing, extra, or invalid fields")]
    Shape,
    #[error("start-authority producer or inventory version is unsupported")]
    Version,
    #[error("start-authority backend identity is invalid")]
    Backend,
    #[error("non-terminal execution does not retain its exact bundle/plan/flavor identity")]
    ExactIdentity,
    #[error("exact revision routing admitted a fallback, unpinned row, or untyped rejection")]
    ExactRouting,
    #[error("keyed acceptance did not converge to one durable drive identity")]
    KeyedAcceptance,
    #[error("failed or cross-tenant materialization changed durable state")]
    Atomicity,
}

pub(crate) fn verify_exact_identity(value: &Value) -> Result<(), StartAuthorityError> {
    let observations = root(value)?;
    let fields = observation(observations)?;
    let plan = digest(&fields["plan_revision"])?;
    let flavor = digest(&fields["flavor_revision"])?;
    if digest(&fields["execution_state_plan_revision"])? != plan
        || digest(&fields["execution_state_flavor_revision"])? != flavor
        || fields["bundle_revision"]
            .as_str()
            .is_none_or(|id| !prefixed_id(id, "ecb_"))
        || fields["materialized_execution_id"]
            .as_str()
            .is_none_or(|id| !prefixed_id(id, "exe_"))
        || fields["initial_execution_version"].as_u64() != Some(0)
    {
        return Err(StartAuthorityError::ExactIdentity);
    }
    verify_atomicity(fields)
}

pub(crate) fn verify_exact_routing(value: &Value) -> Result<(), StartAuthorityError> {
    let fields = observation(root(value)?)?;
    let route = object(
        &fields["exact_route"],
        &[
            "claimed_execution_id",
            "claimed_flavor_revision",
            "draining_revision_bundle_visible",
            "draining_revision_execution_visible",
            "draining_revision_outcome",
            "missing_revision_bundle_visible",
            "missing_revision_execution_visible",
            "missing_revision_outcome",
            "required_flavor_revision",
            "unscoped_or_unpinned_rows_left_untouched",
            "wrong_flavor_execution_id",
            "wrong_flavor_first_generation",
        ],
    )?;
    let required = digest(&route["required_flavor_revision"])?;
    if digest(&route["claimed_flavor_revision"])? != required
        || route["claimed_execution_id"] == route["wrong_flavor_execution_id"]
        || route["claimed_execution_id"]
            .as_str()
            .is_none_or(|id| !prefixed_id(id, "exe_"))
        || route["wrong_flavor_execution_id"]
            .as_str()
            .is_none_or(|id| !prefixed_id(id, "exe_"))
        || route["wrong_flavor_first_generation"].as_u64() != Some(1)
        || route["unscoped_or_unpinned_rows_left_untouched"].as_u64() != Some(2)
        || route["missing_revision_outcome"].as_str() != Some("plan-unavailable")
        || route["missing_revision_execution_visible"].as_bool() != Some(false)
        || route["missing_revision_bundle_visible"].as_bool() != Some(false)
        || route["draining_revision_outcome"].as_str() != Some("pair-not-admitted")
        || route["draining_revision_execution_visible"].as_bool() != Some(false)
        || route["draining_revision_bundle_visible"].as_bool() != Some(false)
    {
        return Err(StartAuthorityError::ExactRouting);
    }
    Ok(())
}

pub(crate) fn verify_keyed_acceptance(value: &Value) -> Result<(), StartAuthorityError> {
    let fields = observation(root(value)?)?;
    let winner = fields["keyed_winner_execution_id"]
        .as_str()
        .filter(|id| prefixed_id(id, "exe_"))
        .ok_or(StartAuthorityError::KeyedAcceptance)?;
    if fields["keyed_replay_execution_id"].as_str() != Some(winner)
        || fields["durable_drive_identities"].as_u64() != Some(1)
    {
        return Err(StartAuthorityError::KeyedAcceptance);
    }
    verify_atomicity(fields)
}

pub(crate) fn verify_tenant_isolation(value: &Value) -> Result<(), StartAuthorityError> {
    let fields = observation(root(value)?)?;
    if fields["foreign_scope_bundle_visible"].as_bool() != Some(false) {
        return Err(StartAuthorityError::Atomicity);
    }
    Ok(())
}

fn root(value: &Value) -> Result<&Value, StartAuthorityError> {
    let fields = object(
        value,
        &[
            "backend",
            "contract",
            "observations",
            "producer_version",
            "scenario_inventory_version",
        ],
    )?;
    if fields["contract"].as_str() != Some("start-authority")
        || fields["producer_version"].as_u64() != Some(1)
        || fields["scenario_inventory_version"].as_u64() != Some(1)
    {
        return Err(StartAuthorityError::Version);
    }
    if !matches!(
        fields["backend"].as_str(),
        Some("in-memory" | "sqlite" | "postgresql")
    ) {
        return Err(StartAuthorityError::Backend);
    }
    Ok(&fields["observations"])
}

fn observation(value: &Value) -> Result<&Map<String, Value>, StartAuthorityError> {
    object(
        value,
        &[
            "bundle_revision",
            "conflict_durable_delta",
            "drain_race_accepted_count",
            "durable_drive_identities",
            "exact_route",
            "execution_state_flavor_revision",
            "execution_state_plan_revision",
            "fingerprint_mismatch_durable_delta",
            "flavor_revision",
            "foreign_scope_bundle_visible",
            "initial_execution_version",
            "keyed_replay_execution_id",
            "keyed_winner_execution_id",
            "live_references_after_terminal",
            "materialized_execution_id",
            "plan_revision",
        ],
    )
}

fn verify_atomicity(fields: &Map<String, Value>) -> Result<(), StartAuthorityError> {
    let accepted = fields["drain_race_accepted_count"]
        .as_u64()
        .ok_or(StartAuthorityError::Atomicity)?;
    if !matches!(accepted, 2 | 3)
        || fields["live_references_after_terminal"].as_u64() != Some(accepted - 1)
        || fields["fingerprint_mismatch_durable_delta"].as_u64() != Some(0)
        || fields["conflict_durable_delta"].as_u64() != Some(0)
        || fields["foreign_scope_bundle_visible"].as_bool() != Some(false)
    {
        return Err(StartAuthorityError::Atomicity);
    }
    Ok(())
}

fn digest(value: &Value) -> Result<&str, StartAuthorityError> {
    value
        .as_str()
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or(StartAuthorityError::ExactIdentity)
}

fn prefixed_id(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() == prefix.len() + 26
        && value[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase())
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, StartAuthorityError> {
    let fields = value.as_object().ok_or(StartAuthorityError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(StartAuthorityError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_observation_qualifies_and_missing_revision_visibility_is_rejected() {
        let value = super::super::fixtures::start_authority();
        assert_eq!(super::verify_exact_identity(&value), Ok(()));
        assert_eq!(super::verify_exact_routing(&value), Ok(()));
        assert_eq!(super::verify_keyed_acceptance(&value), Ok(()));
        let mut invalid = value;
        invalid["observations"]["exact_route"]["missing_revision_execution_visible"] = true.into();
        assert_eq!(
            super::verify_exact_routing(&invalid),
            Err(super::StartAuthorityError::ExactRouting)
        );
    }

    #[test]
    fn authored_success_claim_is_not_evidence() {
        let value = serde_json::json!({"passed":true});
        assert_eq!(
            super::verify_exact_identity(&value),
            Err(super::StartAuthorityError::Shape)
        );
        assert_eq!(
            super::verify_exact_routing(&value),
            Err(super::StartAuthorityError::Shape)
        );
        assert_eq!(
            super::verify_keyed_acceptance(&value),
            Err(super::StartAuthorityError::Shape)
        );
    }
}
