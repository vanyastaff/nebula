//! Deterministic assembly of raw behavior reports into provenance-bound artifacts.

use std::{fs, io::Write as _, path::PathBuf};

use serde_json::Value;

use super::{
    Backend, CiProvenance, EnvironmentProvenance, ExpectedArtifact, ExpectedProvenance,
    FORMAT_VERSION, GateBackend, InputProvenance, Observation, ObservationArtifact, POLICY_SOURCE,
    REGISTRY, RuntimeAuthorityGate, VerificationError, json, loader, policy, runtime_gate,
};

pub(crate) struct BundleRequest {
    pub(crate) workspace_root: PathBuf,
    pub(crate) observation_root: PathBuf,
    pub(crate) artifact_root: PathBuf,
    pub(crate) expected_provenance: PathBuf,
    pub(crate) source_revision: String,
    pub(crate) repository: String,
    pub(crate) workflow_path: String,
    pub(crate) job_id: String,
    pub(crate) run_id: String,
    pub(crate) run_attempt: u32,
    pub(crate) toolchain: String,
}

pub(crate) fn build(request: &BundleRequest) -> Result<Vec<u8>, VerificationError> {
    let observation_root = loader::root(&request.observation_root)?;
    fs::create_dir_all(&request.artifact_root).map_err(|_| VerificationError::ArtifactRead)?;
    let input = InputProvenance {
        source: request.source_revision.clone(),
    };
    let ci = CiProvenance {
        repository: request.repository.clone(),
        workflow_path: request.workflow_path.clone(),
        job_id: request.job_id.clone(),
        run_id: request.run_id.clone(),
        run_attempt: request.run_attempt,
    };
    let policy_digest = loader::digest(POLICY_SOURCE.as_bytes());
    let mut artifacts = Vec::new();
    for (identity, required) in policy()? {
        let relative = super::artifact_path(&identity, &required)?;
        let environment = EnvironmentProvenance {
            toolchain: request.toolchain.clone(),
        };
        let mut observations = Vec::new();
        for case in required.cases {
            observations.push(Observation {
                case: case.clone(),
                events: vec![fragment(&observation_root, &identity, case.as_deref())?],
            });
        }
        let artifact = ObservationArtifact {
            observation_version: FORMAT_VERSION,
            verifier_policy_sha256: policy_digest.clone(),
            identity: identity.clone(),
            input: input.clone(),
            ci: ci.clone(),
            environment: environment.clone(),
            observations,
        };
        let bytes =
            serde_json::to_vec_pretty(&artifact).map_err(|_| VerificationError::InvalidJson)?;
        if bytes.len() > loader::MAX_FILE_BYTES {
            return Err(VerificationError::ArtifactSize);
        }
        write_new(request.artifact_root.join(&relative), &bytes)?;
        artifacts.push(ExpectedArtifact {
            identity,
            path: relative,
            sha256: loader::digest(&bytes),
            ci: ci.clone(),
            environment,
        });
    }
    let expected = ExpectedProvenance {
        provenance_version: FORMAT_VERSION,
        registry_sha256: loader::digest(REGISTRY.as_bytes()),
        verifier_policy_sha256: policy_digest,
        input,
        artifacts,
    };
    let bytes = serde_json::to_vec_pretty(&expected).map_err(|_| VerificationError::InvalidJson)?;
    write_new(request.expected_provenance.clone(), &bytes)?;
    // The producer verifies what it just wrote under the same runner identity
    // it recorded, so this leg proves the bundle is internally consistent. The
    // independent check is the separate `runtime-authority` job, which supplies
    // its own runner identity and therefore cannot be satisfied by a bundle
    // built somewhere else.
    super::verify(
        &request.workspace_root,
        &request.artifact_root,
        &request.expected_provenance,
        &super::RunnerIdentity {
            source_revision: request.source_revision.clone(),
            repository: request.repository.clone(),
            run_id: request.run_id.clone(),
            run_attempt: request.run_attempt,
        },
    )
}

// Kept out of `bundle.rs` so test churn does not move `POLICY_SOURCE`.
#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;

fn fragment(
    root: &std::path::Path,
    identity: &GateBackend,
    case: Option<&str>,
) -> Result<Value, VerificationError> {
    let backend = super::backend_name(identity.backend);
    let relative = match (runtime_gate(identity.gate)?, case) {
        (
            RuntimeAuthorityGate::ExecutionIdentity
            | RuntimeAuthorityGate::ExactRevisionRouting
            | RuntimeAuthorityGate::KeyedAcceptance,
            None,
        ) => {
            format!("start-authority/{backend}.json")
        },
        // Missing and draining revisions are separate policy cases recorded
        // by the same exact-route producer observation.
        (RuntimeAuthorityGate::ExactRevisionRouting, Some("missing" | "draining")) => {
            format!("start-authority/{backend}.json")
        },
        (RuntimeAuthorityGate::ClaimGenerationFencing, None) => {
            format!("claim-fencing/{backend}.json")
        },
        (RuntimeAuthorityGate::ClaimHandoff, None) => format!("claim-handoff/{backend}.json"),
        (RuntimeAuthorityGate::PersistenceConformance, Some("checkpoint-reconnect")) => {
            format!("checkpoint-reconnect/{backend}.json")
        },
        (
            RuntimeAuthorityGate::PersistenceConformance,
            Some(
                "owner-fencing" | "atomic-transition" | "publication-atomicity" | "lease-recovery",
            ),
        ) => format!("persistence-authority/{backend}.json"),
        (
            RuntimeAuthorityGate::PersistenceConformance,
            Some("tenant-isolation" | "keyed-acceptance" | "exact-revision-routing"),
        ) => format!("start-authority/{backend}.json"),
        (RuntimeAuthorityGate::PersistenceConformance, Some("claim-generation-fencing")) => {
            format!("claim-fencing/{backend}.json")
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("backend-reinitialization"))
            if identity.backend == Some(Backend::InMemory) =>
        {
            format!("checkpoint-reconnect/{backend}.json")
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("backend-reinitialization")) => {
            format!("ordered-migrations/{backend}.json")
        },
        (RuntimeAuthorityGate::PersistenceConformance, Some("remote-effects"))
        | (RuntimeAuthorityGate::RemoteEffects, None) => {
            format!("remote-effects/{backend}.json")
        },
        (RuntimeAuthorityGate::RequiredPostgresql, None) => {
            return merged_required_postgresql(root);
        },
        (RuntimeAuthorityGate::OrderedMigrations, Some("clean" | "previous-supported-version")) => {
            format!("ordered-migrations/{backend}.json")
        },
        (RuntimeAuthorityGate::ActivationDiagnostics, None) => {
            "activation-diagnostics/report.json".to_owned()
        },
        _ => return Err(VerificationError::InventoryMismatch),
    };
    read(root, &relative)
}

fn merged_required_postgresql(root: &std::path::Path) -> Result<Value, VerificationError> {
    let mut enabled = read(root, "required-postgresql/enabled.json")?;
    let mut disabled = read(root, "required-postgresql/disabled.json")?;
    for field in ["contract", "producer_version", "scenario_inventory_version"] {
        if enabled[field] != disabled[field] {
            return Err(VerificationError::SemanticObservation);
        }
    }
    enabled["scenarios"]
        .as_array_mut()
        .ok_or(VerificationError::SemanticObservation)?
        .append(
            disabled["scenarios"]
                .as_array_mut()
                .ok_or(VerificationError::SemanticObservation)?,
        );
    Ok(enabled)
}

fn read(root: &std::path::Path, relative: &str) -> Result<Value, VerificationError> {
    let path = root.join(relative);
    let bytes = loader::bounded_file(&path)?;
    let digest = loader::digest(&bytes);
    json::decode(&loader::artifact(root, relative, &digest)?)
}

fn write_new(path: PathBuf, bytes: &[u8]) -> Result<(), VerificationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| VerificationError::ArtifactRead)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| VerificationError::ArtifactPath)?;
    file.write_all(bytes)
        .map_err(|_| VerificationError::ArtifactRead)?;
    file.sync_all().map_err(|_| VerificationError::ArtifactRead)
}
