//! Structural evidence admission is separate from semantic gate qualification.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    Backend, GateRegistry, Threshold,
    external_registry::{ExternalGateId, RuntimeAuthorityGate},
};

mod bundle;
mod json;
mod loader;
mod semantic;
#[cfg(test)]
mod tests;

pub(crate) use bundle::{BundleRequest, build as build_bundle};

const REGISTRY: &str = include_str!("../../../gates/north-star-v1.toml");
fn runtime_gate(id: ExternalGateId) -> Result<RuntimeAuthorityGate, VerificationError> {
    RuntimeAuthorityGate::try_from(id).map_err(|()| VerificationError::InventoryMismatch)
}
const FORMAT_VERSION: u16 = 1;
const POLICY_SOURCE: &str = concat!(
    "nebula-runtime-authority-structural-policy-v1\0",
    include_str!("../backend.rs"),
    "\0",
    include_str!("../external_registry.rs"),
    "\0",
    include_str!("mod.rs"),
    "\0",
    // The artifact-to-observation mapping is policy-bearing and therefore part
    // of the verifier policy digest.
    include_str!("bundle.rs"),
    "\0",
    include_str!("loader.rs"),
    "\0",
    include_str!("json.rs"),
    "\0",
    include_str!("semantic/mod.rs"),
    "\0",
    include_str!("semantic/activation_diagnostics.rs"),
    "\0",
    include_str!("semantic/checkpoint_reconnect.rs"),
    "\0",
    include_str!("semantic/claim_fencing.rs"),
    "\0",
    include_str!("semantic/claim_handoff.rs"),
    "\0",
    include_str!("semantic/ordered_migrations.rs"),
    "\0",
    include_str!("semantic/persistence_authority.rs"),
    "\0",
    include_str!("semantic/remote_effects.rs"),
    "\0",
    include_str!("semantic/required_postgresql.rs"),
    "\0",
    include_str!("semantic/start_authority.rs")
);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerificationError {
    #[error("runtime authority artifact cannot be read")]
    ArtifactRead,
    #[error(
        "runtime authority artifact path must name a regular file without links inside its immutable root"
    )]
    ArtifactPath,
    #[error("runtime authority artifact exceeds its byte limit")]
    ArtifactSize,
    #[error("runtime authority artifact SHA-256 does not match the provenance manifest")]
    ArtifactDigest,
    #[error("runtime authority artifact JSON is invalid, ambiguous, or exceeds structural limits")]
    InvalidJson,
    #[error("runtime authority evidence or provenance version is unsupported")]
    UnsupportedVersion,
    #[error(
        "runtime authority provenance manifest must be a separate file outside the artifact root"
    )]
    ProvenancePath,
    #[error("runtime authority evidence provenance does not match the trusted expectation")]
    ProvenanceMismatch,
    #[error(
        "runtime authority required gate/backend/case inventory is missing, duplicated, or unexpected"
    )]
    InventoryMismatch,
    #[error("runtime authority observation is skipped, erroneous, or empty")]
    InvalidObservation,
    #[error("runtime authority repository policy does not match the verifier's compiled registry")]
    PolicyMismatch,
    #[error("runtime authority observation does not satisfy its executable semantic policy")]
    SemanticObservation,
    #[error(
        "runtime authority observation for gate {gate}, backend {backend}, case {case} does not satisfy its executable semantic policy"
    )]
    SemanticArtifact {
        gate: String,
        backend: String,
        case: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct GateBackend {
    gate: ExternalGateId,
    #[serde(deserialize_with = "required_option")]
    backend: Option<Backend>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InputProvenance {
    #[serde(rename = "source_revision")]
    source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CiProvenance {
    repository: String,
    workflow_path: String,
    job_id: String,
    run_id: String,
    run_attempt: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EnvironmentProvenance {
    toolchain: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExpectedArtifact {
    identity: GateBackend,
    path: String,
    sha256: String,
    ci: CiProvenance,
    environment: EnvironmentProvenance,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExpectedProvenance {
    provenance_version: u16,
    registry_sha256: String,
    verifier_policy_sha256: String,
    input: InputProvenance,
    artifacts: Vec<ExpectedArtifact>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservationArtifact {
    observation_version: u16,
    verifier_policy_sha256: String,
    identity: GateBackend,
    input: InputProvenance,
    ci: CiProvenance,
    environment: EnvironmentProvenance,
    observations: Vec<Observation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    #[serde(deserialize_with = "required_option")]
    case: Option<String>,
    events: Vec<serde_json::Value>,
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

struct RequiredArtifact {
    cases: BTreeSet<Option<String>>,
    ci_jobs: BTreeSet<String>,
    artifact_stem: String,
}

fn policy() -> Result<BTreeMap<GateBackend, RequiredArtifact>, VerificationError> {
    let registry: GateRegistry =
        toml::from_str(REGISTRY).map_err(|_| VerificationError::PolicyMismatch)?;
    let mut required = BTreeMap::new();
    let mut gates = BTreeSet::new();
    for gate in registry
        .gates
        .into_iter()
        .filter(|gate| runtime_gate(gate.id).is_ok())
    {
        gates.insert(gate.id);
        let runtime_gate = runtime_gate(gate.id)?;
        validate_runtime_threshold(runtime_gate, &gate.threshold)?;
        let artifact_stem = gate.evidence.artifact_name;
        let cases = match gate.threshold {
            Threshold::All { required_cases, .. } => required_cases.into_iter().map(Some).collect(),
            _ => BTreeSet::from([None]),
        };
        let backends = gate.backends.map_or_else(
            || vec![None],
            |values| values.into_iter().map(Some).collect(),
        );
        for backend in backends {
            required.insert(
                GateBackend {
                    gate: gate.id,
                    backend,
                },
                RequiredArtifact {
                    cases: cases.clone(),
                    ci_jobs: gate.required_ci.iter().cloned().collect(),
                    artifact_stem: artifact_stem.clone(),
                },
            );
        }
    }
    if gates.len() != RuntimeAuthorityGate::ALL.len() {
        return Err(VerificationError::PolicyMismatch);
    }
    Ok(required)
}

fn validate_runtime_threshold(
    gate: RuntimeAuthorityGate,
    threshold: &Threshold,
) -> Result<(), VerificationError> {
    use RuntimeAuthorityGate as Gate;

    let matches_policy = match (gate, threshold) {
        (
            Gate::ExecutionIdentity,
            Threshold::RequiredSet {
                metric,
                required_values,
            },
        ) => {
            metric == "non_terminal_execution_identity_fields"
                && values_match(
                    required_values,
                    &["bundle_revision", "workflow_revision", "flavor_revision"],
                )
        },
        (
            Gate::ExactRevisionRouting,
            Threshold::All {
                metric,
                required_cases,
            },
        ) => {
            metric == "fail_closed_revision_cases"
                && values_match(required_cases, &["missing", "draining"])
        },
        (Gate::ClaimGenerationFencing, Threshold::Zero { metrics }) => {
            values_match(metrics, &["stale_generation_mutation_count"])
        },
        (Gate::KeyedAcceptance, Threshold::Exact { metric, expected }) => {
            metric == "durable_drive_identities_per_ambiguous_acceptance"
                && expected.as_integer() == Some(1)
        },
        (Gate::ClaimHandoff, Threshold::Zero { metrics }) => values_match(
            metrics,
            &[
                "claim_reclaimed_while_action_blocked",
                "claim_exhausted_while_action_blocked",
                "competing_claim_count",
            ],
        ),
        (
            Gate::PersistenceConformance,
            Threshold::All {
                metric,
                required_cases,
            },
        ) => {
            metric == "persistence_conformance_case_pass"
                && values_match(
                    required_cases,
                    &[
                        "checkpoint-reconnect",
                        "owner-fencing",
                        "atomic-transition",
                        "tenant-isolation",
                        "keyed-acceptance",
                        "publication-atomicity",
                        "lease-recovery",
                        "claim-generation-fencing",
                        "exact-revision-routing",
                        "backend-reinitialization",
                        "remote-effects",
                    ],
                )
        },
        (Gate::RequiredPostgresql, Threshold::Exact { metric, expected }) => {
            metric == "required_ci_result_when_postgresql_is_absent"
                && expected.as_str() == Some("failure")
        },
        (
            Gate::OrderedMigrations,
            Threshold::All {
                metric,
                required_cases,
            },
        ) => {
            metric == "ordered_migration_fixture_pass"
                && values_match(required_cases, &["clean", "previous-supported-version"])
        },
        (
            Gate::ActivationDiagnostics,
            Threshold::RequiredSet {
                metric,
                required_values,
            },
        ) => {
            metric == "activation_diagnostic_fields"
                && values_match(
                    required_values,
                    &["code", "path", "expected", "actual", "remediation"],
                )
        },
        (Gate::RemoteEffects, Threshold::Zero { metrics }) => values_match(
            metrics,
            &[
                "stale_committed_effect_count",
                "duplicate_committed_effect_count",
            ],
        ),
        _ => false,
    };
    if matches_policy {
        Ok(())
    } else {
        Err(VerificationError::PolicyMismatch)
    }
}

fn values_match(actual: &[String], expected: &[&str]) -> bool {
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    actual == expected
}

/// Identity of the job running the verifier.
///
/// Supplied by the trusted runner on the command line and never read from the
/// artifact tree. Recording provenance and never comparing it to anything left
/// a hand-authored bundle indistinguishable from one CI produced: the values
/// were shape-checked, so any well-formed revision and run id passed.
pub(crate) struct RunnerIdentity {
    pub(crate) source_revision: String,
    pub(crate) repository: String,
    pub(crate) run_id: String,
    pub(crate) run_attempt: u32,
}

#[derive(Debug, Serialize)]
struct VerificationSummary {
    status: &'static str,
    contract: &'static str,
    effective_states: Vec<EffectiveGateState>,
}

#[derive(Debug, Serialize)]
struct EffectiveGateState {
    gate: ExternalGateId,
    state: &'static str,
}

impl VerificationSummary {
    fn from_verified_observations(
        verified: &BTreeMap<GateBackend, BTreeSet<Option<String>>>,
        required: &BTreeMap<GateBackend, RequiredArtifact>,
    ) -> Result<Self, VerificationError> {
        let mut effective_states = Vec::with_capacity(RuntimeAuthorityGate::ALL.len());
        for gate in RuntimeAuthorityGate::ALL {
            let external = ExternalGateId::from(gate);
            let expected = required
                .iter()
                .filter(|(identity, _)| identity.gate == external)
                .collect::<Vec<_>>();
            if expected.is_empty()
                || expected
                    .iter()
                    .any(|(identity, artifact)| verified.get(*identity) != Some(&artifact.cases))
            {
                return Err(VerificationError::InventoryMismatch);
            }
            effective_states.push(EffectiveGateState {
                gate: external,
                // Runtime evidence establishes only the observed contract.
                // Release-level `passed` also requires independent policy
                // provenance, so a complete verified inventory advances to
                // `partial` and no farther.
                state: "partial",
            });
        }
        Ok(Self {
            status: "verified",
            contract: "runtime-authority",
            effective_states,
        })
    }

    fn to_json_line(&self) -> Result<Vec<u8>, VerificationError> {
        let mut output = serde_json::to_vec(self).map_err(|_| VerificationError::InvalidJson)?;
        output.push(b'\n');
        Ok(output)
    }
}

impl RunnerIdentity {
    /// Reject provenance that names a different revision, repository, or run.
    fn admit(&self, input: &InputProvenance, ci: &CiProvenance) -> Result<(), VerificationError> {
        let recorded = (
            input.source.as_str(),
            ci.repository.as_str(),
            ci.run_id.as_str(),
            ci.run_attempt,
        );
        let running = (
            self.source_revision.as_str(),
            self.repository.as_str(),
            self.run_id.as_str(),
            self.run_attempt,
        );
        if recorded == running {
            Ok(())
        } else {
            Err(VerificationError::ProvenanceMismatch)
        }
    }
}

/// Admit trusted artifacts and recompute every runtime-authority predicate.
pub(crate) fn verify(
    workspace: &Path,
    artifact_root: &Path,
    expected_path: &Path,
    runner: &RunnerIdentity,
) -> Result<Vec<u8>, VerificationError> {
    let expected: ExpectedProvenance = json::decode(&loader::bounded_file(expected_path)?)?;
    let root = loader::root(artifact_root)?;
    let expected_canonical = expected_path
        .canonicalize()
        .map_err(|_| VerificationError::ArtifactRead)?;
    if expected_canonical.starts_with(&root) {
        return Err(VerificationError::ProvenancePath);
    }
    if loader::bounded_file(&workspace.join(super::REGISTRY_PATH))? != REGISTRY.as_bytes() {
        return Err(VerificationError::PolicyMismatch);
    }
    super::validate(workspace).map_err(|_| VerificationError::PolicyMismatch)?;
    admit_artifacts(&root, &expected, runner)?;
    let verified = verify_semantics(workspace, &root, &expected)?;
    let required = policy()?;
    VerificationSummary::from_verified_observations(&verified, &required)?.to_json_line()
}

fn verify_semantics(
    workspace: &Path,
    root: &Path,
    expected: &ExpectedProvenance,
) -> Result<BTreeMap<GateBackend, BTreeSet<Option<String>>>, VerificationError> {
    let mut verified = BTreeMap::<GateBackend, BTreeSet<Option<String>>>::new();
    for entry in &expected.artifacts {
        let bytes = loader::artifact(root, &entry.path, &entry.sha256)?;
        let artifact: ObservationArtifact = json::decode(&bytes)?;
        for observation in &artifact.observations {
            semantic::verify(
                workspace,
                &artifact.identity,
                observation.case.as_deref(),
                &observation.events,
            )
            .map_err(|_| VerificationError::SemanticArtifact {
                gate: artifact.identity.gate.to_string(),
                backend: backend_name(artifact.identity.backend).to_owned(),
                case: observation.case.as_deref().unwrap_or("default").to_owned(),
            })?;
            verified
                .entry(artifact.identity.clone())
                .or_default()
                .insert(observation.case.clone());
        }
    }
    Ok(verified)
}

fn backend_name(backend: Option<Backend>) -> &'static str {
    backend.map_or("independent", <&'static str>::from)
}

fn admit_artifacts(
    root: &Path,
    expected: &ExpectedProvenance,
    runner: &RunnerIdentity,
) -> Result<(), VerificationError> {
    if expected.provenance_version != FORMAT_VERSION {
        return Err(VerificationError::UnsupportedVersion);
    }
    if expected.registry_sha256 != loader::digest(REGISTRY.as_bytes())
        || expected.verifier_policy_sha256 != loader::digest(POLICY_SOURCE.as_bytes())
    {
        return Err(VerificationError::PolicyMismatch);
    }
    validate_input(&expected.input)?;
    let required = policy()?;
    if expected.artifacts.len() != required.len() {
        return Err(VerificationError::InventoryMismatch);
    }
    let mut identities = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for entry in &expected.artifacts {
        let policy = required
            .get(&entry.identity)
            .ok_or(VerificationError::InventoryMismatch)?;
        if entry.path != artifact_path(&entry.identity, policy)?
            || !identities.insert(&entry.identity)
            || !paths.insert(&entry.path)
        {
            return Err(VerificationError::InventoryMismatch);
        }
        validate_ci(&entry.ci, policy)?;
        runner.admit(&expected.input, &entry.ci)?;
        validate_environment(&entry.environment, entry.identity.backend)?;
        let bytes = loader::artifact(root, &entry.path, &entry.sha256)?;
        let artifact: ObservationArtifact = json::decode(&bytes)?;
        validate_artifact(&artifact, entry, &expected.input, policy)?;
    }
    Ok(())
}

fn artifact_path(
    identity: &GateBackend,
    policy: &RequiredArtifact,
) -> Result<String, VerificationError> {
    let backend = backend_name(identity.backend);
    if policy
        .artifact_stem
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Ok(format!("{}/{backend}.json", policy.artifact_stem))
    } else {
        Err(VerificationError::PolicyMismatch)
    }
}

fn validate_input(input: &InputProvenance) -> Result<(), VerificationError> {
    if !loader::is_digest(&input.source, 40) {
        return Err(VerificationError::ProvenanceMismatch);
    }
    Ok(())
}

fn bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

/// The one job that runs the producers and assembles the bundle.
///
/// Membership in a gate's whole `required_ci` set is not a producer check:
/// every runtime gate also lists the `#tests` aggregation job, which runs no
/// tests at all, so an artifact could legitimately claim to have come from
/// there. The recorded job must be the producer, and the producer must still
/// be one of the gate's required jobs so the registry and this constant cannot
/// silently disagree.
const PRODUCING_CI_JOB: &str = ".github/workflows/test-matrix.yml#postgres-conformance";

fn validate_ci(ci: &CiProvenance, policy: &RequiredArtifact) -> Result<(), VerificationError> {
    let recorded = format!("{}#{}", ci.workflow_path, ci.job_id);
    if !bounded_identity(&ci.repository)
        || ci.repository.split('/').count() != 2
        || ci.repository.split('/').any(str::is_empty)
        || ci.run_attempt == 0
        || ci.run_id.is_empty()
        || ci.run_id.len() > 32
        || !ci.run_id.bytes().all(|byte| byte.is_ascii_digit())
        || recorded != PRODUCING_CI_JOB
        || !policy.ci_jobs.contains(&recorded)
    {
        return Err(VerificationError::ProvenanceMismatch);
    }
    Ok(())
}

fn validate_environment(
    environment: &EnvironmentProvenance,
    _backend: Option<Backend>,
) -> Result<(), VerificationError> {
    let bounded_description = |value: &str| {
        !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
    };
    if !bounded_description(&environment.toolchain) {
        return Err(VerificationError::ProvenanceMismatch);
    }
    Ok(())
}

fn validate_artifact(
    artifact: &ObservationArtifact,
    expected: &ExpectedArtifact,
    input: &InputProvenance,
    policy: &RequiredArtifact,
) -> Result<(), VerificationError> {
    if artifact.observation_version != FORMAT_VERSION {
        return Err(VerificationError::UnsupportedVersion);
    }
    if artifact.verifier_policy_sha256 != loader::digest(POLICY_SOURCE.as_bytes())
        || artifact.identity != expected.identity
        || artifact.input != *input
        || artifact.ci != expected.ci
        || artifact.environment != expected.environment
    {
        return Err(VerificationError::ProvenanceMismatch);
    }
    let mut cases = BTreeSet::new();
    for observation in &artifact.observations {
        if !cases.insert(observation.case.clone()) || !policy.cases.contains(&observation.case) {
            return Err(VerificationError::InventoryMismatch);
        }
        if observation.events.is_empty() {
            return Err(VerificationError::InvalidObservation);
        }
    }
    if cases != policy.cases {
        return Err(VerificationError::InventoryMismatch);
    }
    Ok(())
}
