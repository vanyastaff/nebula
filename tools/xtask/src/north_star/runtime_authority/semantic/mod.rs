//! Executable semantic predicates for runtime-authority observations.

mod activation_diagnostics;
mod checkpoint_reconnect;
mod claim_fencing;
mod claim_handoff;
#[cfg(test)]
pub(super) mod fixtures;
mod ordered_migrations;
mod persistence_authority;
mod remote_effects;
mod required_postgresql;
mod start_authority;

use serde_json::Value;
use std::path::Path;

use super::{Backend, GateBackend, RuntimeAuthorityGate, VerificationError, runtime_gate};

pub(super) fn verify(
    workspace: &Path,
    identity: &GateBackend,
    case: Option<&str>,
    events: &[Value],
) -> Result<(), VerificationError> {
    let [fragment] = events else {
        return Err(VerificationError::SemanticObservation);
    };
    let gate = runtime_gate(identity.gate)?;
    verify_backend(gate, identity.backend, fragment)?;
    let result = match (gate, case) {
        (RuntimeAuthorityGate::ExecutionIdentity, None) => {
            start_authority::verify_exact_identity(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::ExactRevisionRouting, None) => {
            start_authority::verify_exact_routing(fragment).map_err(|_| ())
        },
        // Missing and draining revisions share one exact-route observation,
        // so the routing predicate qualifies both policy cases.
        (RuntimeAuthorityGate::ExactRevisionRouting, Some("missing" | "draining")) => {
            start_authority::verify_exact_routing(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::ClaimGenerationFencing, None) => {
            claim_fencing::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::KeyedAcceptance, None) => {
            start_authority::verify_keyed_acceptance(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::ClaimHandoff, None) => {
            claim_handoff::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("checkpoint-reconnect")) => {
            checkpoint_reconnect::verify(fragment).map_err(|_| ())
        },
        (
            RuntimeAuthorityGate::PersistenceConformance,
            Some(
                case @ ("owner-fencing"
                | "atomic-transition"
                | "publication-atomicity"
                | "lease-recovery"),
            ),
        ) => persistence_authority::verify_case(fragment, case).map_err(|_| ()),
        (RuntimeAuthorityGate::PersistenceConformance, Some("tenant-isolation")) => {
            start_authority::verify_tenant_isolation(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("keyed-acceptance")) => {
            start_authority::verify_keyed_acceptance(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("exact-revision-routing")) => {
            start_authority::verify_exact_routing(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("claim-generation-fencing")) => {
            claim_fencing::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("backend-reinitialization"))
            if identity.backend == Some(Backend::InMemory) =>
        {
            checkpoint_reconnect::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("backend-reinitialization")) => {
            ordered_migrations::verify(workspace, fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("remote-effects")) => {
            remote_effects::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::RequiredPostgresql, None) => {
            required_postgresql::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::OrderedMigrations, Some("clean" | "previous-supported-version")) => {
            ordered_migrations::verify(workspace, fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::ActivationDiagnostics, None) => {
            activation_diagnostics::verify(fragment).map_err(|_| ())
        },
        (RuntimeAuthorityGate::RemoteEffects, None) => {
            remote_effects::verify(fragment).map_err(|_| ())
        },
        _ => Err(()),
    };
    result.map_err(|()| VerificationError::SemanticObservation)
}

/// Bind a fragment to the backend its artifact identity claims.
///
/// The exemption is keyed on the trusted gate, never on the fragment: reading
/// `contract` out of the payload let the untrusted document decide whether the
/// trusted backend identity was enforced at all.
fn verify_backend(
    gate: RuntimeAuthorityGate,
    backend: Option<Backend>,
    fragment: &Value,
) -> Result<(), VerificationError> {
    // The required-PostgreSQL gate observes the process, not a backend session:
    // its healthy and absent arms are recorded by the same producer and carry
    // no `backend` field of their own.
    if gate == RuntimeAuthorityGate::RequiredPostgresql {
        return Ok(());
    }
    let expected = match backend {
        Some(Backend::InMemory) => Some("in-memory"),
        Some(Backend::Sqlite) => Some("sqlite"),
        Some(Backend::Postgresql) => Some("postgresql"),
        None => None,
    };
    if let Some(expected) = expected {
        if fragment.get("backend").and_then(Value::as_str) != Some(expected) {
            return Err(VerificationError::SemanticObservation);
        }
    } else if fragment.get("backend").is_some() {
        return Err(VerificationError::SemanticObservation);
    }
    Ok(())
}
