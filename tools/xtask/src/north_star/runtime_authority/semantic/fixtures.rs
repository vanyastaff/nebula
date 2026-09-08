//! Stable synthetic observations used to exercise semantic predicates.

use std::path::Path;

use serde_json::{Value, json};

use super::super::{Backend, GateBackend, RuntimeAuthorityGate, runtime_gate};

pub(super) fn claim_fencing() -> Value {
    let queue = |name: &str, byte: u8| {
        json!({
            "queue": name, "row_id": vec![byte; 16], "superseded_generation": 1,
            "current_generation": 2, "same_processor": true, "late_ack_fenced": true,
            "late_nack_fenced": true, "stale_generation_mutation_count": 0,
            "current_owner_completed_after_stale_attempts": true
        })
    };
    json!({"backend":"in-memory","contract":"claim-generation-fencing","producer_version":1,
        "scenario_inventory_version":1,"queues":[queue("control", 1), queue("job", 2)]})
}

pub(super) fn claim_handoff() -> Value {
    let common = |command: &str| {
        json!({"command":command,"claim_generation":1,
            "durable_handoff_outcome":"Accepted","action_calls_while_delivery_terminal":1,
            "claim_reclaimed_while_action_blocked":0,"claim_exhausted_while_action_blocked":0,
            "competing_claim_count":0})
    };
    json!({"backend":"in-memory","contract":"claim-handoff","producer_version":1,
        "scenario_inventory_version":1,"commands":[{
            "command":"Start","action_calls_while_delivery_terminal":1,
            "claim_reclaimed_while_action_blocked":0,"claim_exhausted_while_action_blocked":0,
            "competing_claim_count":0,
            "competing_execution_lease_acquired":false,"final_exhausted":0,"final_reclaimed":0
        },common("Resume"),common("Restart")]})
}

pub(super) fn checkpoint_reconnect() -> Value {
    let execution_id = "exe_01M1YNKHRBF3T0EHDWSVNND35K";
    let plan = "51".repeat(32);
    let flavor = "95".repeat(32);
    let turn = |name: &str, node: &str, calls: u64, version: u64, status: &str| {
        json!({
            "turn":name,"execution_id":execution_id,"execution_version":version,"status":status,
            "action_calls_total":calls,"engine_recreated":true,
            "checkpoint_node":node,"checkpoint_output":{"identity":execution_id,"payload":"persisted predecessor output"},
            "executable_plan_revision_id":plan,"worker_flavor_revision_id":flavor
        })
    };
    json!({"backend":"in-memory","contract":"durable-checkpoint-reconnect","producer_version":1,
        "scenario_inventory_version":1,"turns":[turn("cold-start","predecessor",1,3,"Paused"),
        turn("warm-resume","successor",2,7,"Completed")]})
}

pub(super) fn persistence_authority() -> Value {
    json!({"backend":"in-memory","contract":"persistence-authority","producer_version":1,
        "scenario_inventory_version":1,
        "owner_fencing":{"stale_generation":0,"live_generation":1,"outcome":"FencedOut",
            "stale_mutation_count":0,"state_before":{},"state_after":{},"version_before":0,"version_after":0},
        "atomic_transition":{"owner_fence_generation":1,"transition_outcome":"Applied { new_version: 1 }",
            "execution_version":1,"state":{"s":"running"},"journal_entry_count":1,
            "journal_payload":{"event":"transition"},"outbox_claim_count":1,"outbox_command":"Cancel",
            "outbox_command_id":vec![1;16]},
        "lease_recovery":{"first_generation":1,"expired_generation":1,"recovered_generation":2,
            "recovered_generation_advanced":true,"same_holder_live_reacquire_granted":false},
        "publication_atomicity":{"created_workflow_version":1,"published_version":1,
            "stale_cas_outcome":"Conflict","row_version_after_stale_cas":1,
            "candidate_version_after_stale_cas":null,"duplicate_version_outcome":"Duplicate",
            "orphan_workflow_after_duplicate":false}})
}

pub(super) fn start_authority() -> Value {
    let plan = "33".repeat(32);
    let flavor = "34".repeat(32);
    let route_flavor = "49".repeat(32);
    let winner = "exe_01M1YNJFP8NR2RDQF0NW5CKD9E";
    json!({"backend":"in-memory","contract":"start-authority","producer_version":1,
        "scenario_inventory_version":1,"observations":{
            "bundle_revision":"ecb_01M1YNJFP8GN4SFKWJ84W0MK5P","plan_revision":plan,
            "flavor_revision":flavor,"execution_state_plan_revision":plan,
            "execution_state_flavor_revision":flavor,"durable_drive_identities":1,
            "initial_execution_version":0,"materialized_execution_id":"exe_01M1YNJFP80V42BDSCYCMWTS28",
            "foreign_scope_bundle_visible":false,"keyed_winner_execution_id":winner,
            "keyed_replay_execution_id":winner,"fingerprint_mismatch_durable_delta":0,
            "conflict_durable_delta":0,"drain_race_accepted_count":2,"live_references_after_terminal":1,
            "exact_route":{"claimed_execution_id":"exe_01M1YNJFPBY4KVQG4MT6FX9F6G",
                "wrong_flavor_execution_id":"exe_01M1YNJFPBAE80JDZ6WC0506PC",
                "claimed_flavor_revision":route_flavor,"required_flavor_revision":route_flavor,
                "missing_revision_outcome":"plan-unavailable","missing_revision_execution_visible":false,
                "missing_revision_bundle_visible":false,"draining_revision_outcome":"pair-not-admitted",
                "draining_revision_execution_visible":false,"draining_revision_bundle_visible":false,
                "unscoped_or_unpinned_rows_left_untouched":2,"wrong_flavor_first_generation":1}}})
}

pub(in super::super) fn for_observation(
    workspace: &Path,
    identity: &GateBackend,
    case: Option<&str>,
) -> Value {
    let gate = runtime_gate(identity.gate).expect("the fixture identity names a runtime gate");
    let mut fragment = match (gate, case) {
        (RuntimeAuthorityGate::ExecutionIdentity | RuntimeAuthorityGate::KeyedAcceptance, None)
        | (RuntimeAuthorityGate::ExactRevisionRouting, None | Some("missing" | "draining"))
        | (
            RuntimeAuthorityGate::PersistenceConformance,
            Some("tenant-isolation" | "keyed-acceptance" | "exact-revision-routing"),
        ) => start_authority(),
        (RuntimeAuthorityGate::ClaimGenerationFencing, None)
        | (RuntimeAuthorityGate::PersistenceConformance, Some("claim-generation-fencing")) => {
            claim_fencing()
        },
        (RuntimeAuthorityGate::ClaimHandoff, None) => claim_handoff(),
        (RuntimeAuthorityGate::PersistenceConformance, Some("checkpoint-reconnect")) => {
            checkpoint_reconnect()
        },
        (
            RuntimeAuthorityGate::PersistenceConformance,
            Some(
                "owner-fencing" | "atomic-transition" | "publication-atomicity" | "lease-recovery",
            ),
        ) => persistence_authority(),
        (RuntimeAuthorityGate::PersistenceConformance, Some("backend-reinitialization"))
            if identity.backend == Some(Backend::InMemory) =>
        {
            checkpoint_reconnect()
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("backend-reinitialization"))
        | (RuntimeAuthorityGate::OrderedMigrations, Some(_)) => {
            super::ordered_migrations::fixture(workspace, backend_name(identity.backend))
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("remote-effects"))
        | (RuntimeAuthorityGate::RemoteEffects, None) => super::remote_effects::fixture(),
        (RuntimeAuthorityGate::RequiredPostgresql, None) => {
            return super::required_postgresql::fixture();
        },
        (RuntimeAuthorityGate::ActivationDiagnostics, None) => {
            return super::activation_diagnostics::fixture();
        },
        _ => panic!("the compiled policy requested an unsupported semantic fixture"),
    };
    fragment["backend"] = backend_name(identity.backend).into();
    fragment
}

fn backend_name(backend: Option<Backend>) -> &'static str {
    match backend {
        Some(Backend::InMemory) => "in-memory",
        Some(Backend::Sqlite) => "sqlite",
        Some(Backend::Postgresql) => "postgresql",
        None => panic!("backend-independent fixtures return before backend selection"),
    }
}
