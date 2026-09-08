//! Round-trip coverage for the producer half of the evidence pipeline.
//!
//! `build` had no tests at all, so nothing checked that the assembler can
//! actually find every observation the compiled policy demands. Two defects
//! reached CI review through exactly that gap: observation reports written to
//! a directory the assembler never reads, and a gate whose `required_ci` did
//! not list the job producing its own artifact.
//!
//! Synthetic observations come from the semantic predicate fixtures, allowing
//! the producer and an independent verifier invocation to complete successfully.

use std::{fs, path::Path};

use serde_json::json;

use super::{BundleRequest, VerificationError, build};
use crate::north_star::runtime_authority::{
    Backend, GateBackend, RunnerIdentity, RuntimeAuthorityGate, semantic, verify,
};

fn write(root: &Path, relative: &str, value: &serde_json::Value) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

/// Every observation the compiled policy can ask `fragment` for.
fn observation_root(root: &Path, workspace: &Path) {
    let backend_identities = [
        (Backend::InMemory, "in-memory"),
        (Backend::Sqlite, "sqlite"),
        (Backend::Postgresql, "postgresql"),
    ];
    let contracts = [
        ("start-authority", RuntimeAuthorityGate::ExecutionIdentity),
        (
            "claim-fencing",
            RuntimeAuthorityGate::ClaimGenerationFencing,
        ),
        ("claim-handoff", RuntimeAuthorityGate::ClaimHandoff),
        (
            "checkpoint-reconnect",
            RuntimeAuthorityGate::PersistenceConformance,
        ),
        (
            "persistence-authority",
            RuntimeAuthorityGate::PersistenceConformance,
        ),
        ("remote-effects", RuntimeAuthorityGate::RemoteEffects),
    ];
    for (backend, backend_name) in backend_identities {
        for (contract, gate) in contracts {
            let case = match contract {
                "checkpoint-reconnect" => Some("checkpoint-reconnect"),
                "persistence-authority" => Some("owner-fencing"),
                _ => None,
            };
            let identity = GateBackend {
                gate: gate.into(),
                backend: Some(backend),
            };
            write(
                root,
                &format!("{contract}/{backend_name}.json"),
                &semantic::fixtures::for_observation(workspace, &identity, case),
            );
        }
    }
    for (backend, backend_name) in [
        (Backend::Sqlite, "sqlite"),
        (Backend::Postgresql, "postgresql"),
    ] {
        let identity = GateBackend {
            gate: RuntimeAuthorityGate::OrderedMigrations.into(),
            backend: Some(backend),
        };
        write(
            root,
            &format!("ordered-migrations/{backend_name}.json"),
            &semantic::fixtures::for_observation(workspace, &identity, Some("clean")),
        );
    }
    write(
        root,
        "required-postgresql/enabled.json",
        &serde_json::from_slice(include_bytes!(
            "semantic/required_postgresql/postgres_observed.json"
        ))
        .unwrap(),
    );
    write(
        root,
        "required-postgresql/disabled.json",
        &serde_json::from_slice(include_bytes!(
            "semantic/required_postgresql/disabled_observed.json"
        ))
        .unwrap(),
    );
    write(
        root,
        "activation-diagnostics/report.json",
        &serde_json::from_slice(include_bytes!(
            "semantic/activation_diagnostics/observed.json"
        ))
        .unwrap(),
    );
}

fn workspace() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn request(directory: &Path, job_id: &str) -> BundleRequest {
    BundleRequest {
        workspace_root: workspace(),
        observation_root: directory.join("observations"),
        artifact_root: directory.join("bundle"),
        expected_provenance: directory.join("expected.json"),
        source_revision: "a".repeat(40),
        repository: "fixture/repository".to_owned(),
        workflow_path: ".github/workflows/test-matrix.yml".to_owned(),
        job_id: job_id.to_owned(),
        run_id: "1234".to_owned(),
        run_attempt: 1,
        toolchain: "synthetic-toolchain-version".to_owned(),
    }
}

#[test]
fn a_complete_observation_root_builds_and_independently_verifies() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = workspace();
    observation_root(&directory.path().join("observations"), &workspace);
    let request = request(directory.path(), "postgres-conformance");

    let producer_summary = build(&request).unwrap();
    let verifier_summary = verify(
        &workspace,
        &directory.path().join("bundle"),
        &directory.path().join("expected.json"),
        &RunnerIdentity {
            source_revision: request.source_revision,
            repository: request.repository,
            run_id: request.run_id,
            run_attempt: request.run_attempt,
        },
    )
    .unwrap();

    assert_eq!(producer_summary, verifier_summary);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&verifier_summary).unwrap()["status"],
        json!("verified")
    );
    assert!(directory.path().join("expected.json").is_file());
    assert!(
        directory
            .path()
            .join("bundle/keyed-acceptance/in-memory.json")
            .is_file()
    );
}

#[test]
fn a_missing_producer_report_fails_closed_instead_of_assembling_a_partial_bundle() {
    for absent in [
        "start-authority/sqlite.json",
        "ordered-migrations/postgresql.json",
        "required-postgresql/disabled.json",
        "activation-diagnostics/report.json",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("observations");
        observation_root(&root, &workspace());
        fs::remove_file(root.join(absent)).unwrap();

        assert_eq!(
            build(&request(directory.path(), "postgres-conformance")),
            Err(VerificationError::ArtifactRead),
            "a bundle missing `{absent}` must not assemble"
        );
    }
}

#[test]
fn an_empty_observation_root_cannot_produce_a_bundle() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("observations")).unwrap();

    assert_eq!(
        build(&request(directory.path(), "postgres-conformance")),
        Err(VerificationError::ArtifactRead)
    );
}

#[test]
fn assembling_under_a_job_that_does_not_produce_the_bundle_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    observation_root(&directory.path().join("observations"), &workspace());

    assert_eq!(
        build(&request(directory.path(), "tests")),
        Err(VerificationError::ProvenanceMismatch),
        "the aggregation job runs no tests and cannot be an artifact's producer"
    );
}
