// budget-justified: the versioned policy parser, cross-field invariants, and schema lint form one atomic validation boundary.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

mod schema;

const REGISTRY_PATH: &str = "tools/xtask/gates/north-star-v1.toml";
const SCHEMA_PATH: &str = "tools/xtask/schemas/gate-evidence-v1.schema.json";
const CANONICAL_EVIDENCE: &str = include_str!("../../schemas/gate-evidence-v1.example.json");
const REGISTRY_VERSION: u16 = 1;
const SCHEMA_VERSION: u16 = 1;
const GATE_COUNT: usize = 22;

const ACCOUNTABLE_OWNER_ROLES: [&str; 12] = [
    "api-design-lead",
    "async-systems-lead",
    "systems-perf-lead",
    "qa-lead",
    "tooling-lead",
    "security-auditor",
    "product-steward",
    "observability-engineer",
    "release-lead",
    "chief-architect",
    "test-engineer",
    "docs-engineer",
];

#[derive(Debug, Serialize)]
pub(crate) struct ValidationSummary {
    registry_version: u16,
    schema_version: u16,
    gate_count: usize,
    status: &'static str,
}

impl ValidationSummary {
    pub(crate) fn to_json_line(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut output = serde_json::to_vec(self)?;
        output.push(b'\n');
        Ok(output)
    }
}

pub(crate) fn validate(workspace_root: &Path) -> Result<ValidationSummary, ValidationError> {
    let registry_path = workspace_root.join(REGISTRY_PATH);
    let schema_path = workspace_root.join(SCHEMA_PATH);
    let registry_source =
        fs::read_to_string(&registry_path).map_err(|source| ValidationError::Read {
            path: registry_path,
            source,
        })?;
    let schema_source =
        fs::read_to_string(&schema_path).map_err(|source| ValidationError::Read {
            path: schema_path,
            source,
        })?;

    validate_documents(&registry_source, &schema_source, workspace_root)
}

fn validate_documents(
    registry_source: &str,
    schema_source: &str,
    workspace_root: &Path,
) -> Result<ValidationSummary, ValidationError> {
    let registry: GateRegistry =
        toml::from_str(registry_source).map_err(ValidationError::RegistryParse)?;
    let schema: Value =
        serde_json::from_str(schema_source).map_err(ValidationError::SchemaParse)?;
    let canonical_evidence: Value =
        serde_json::from_str(CANONICAL_EVIDENCE).map_err(ValidationError::EvidenceParse)?;

    schema::validate(&schema)?;
    schema::validate_instance(&schema, &canonical_evidence)?;
    validate_registry(&registry, workspace_root)?;

    Ok(ValidationSummary {
        registry_version: registry.registry_version,
        schema_version: registry.evidence_schema_version,
        gate_count: registry.gates.len(),
        status: "valid",
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateRegistry {
    registry_version: u16,
    evidence_schema_version: u16,
    evidence_schema_path: String,
    duration_definitions: DurationDefinitions,
    gates: Vec<Gate>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurationDefinitions {
    focused_working_day: DurationDefinition,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurationDefinition {
    version: u16,
    minutes: u16,
}

struct ValidationContext<'a> {
    workspace_root: &'a Path,
    workflow_jobs: BTreeMap<String, BTreeSet<String>>,
    artifact_paths: BTreeSet<String>,
    focused_working_day_id: String,
    focused_working_day_minutes: u16,
    focused_working_day_is_referenced: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    id: String,
    statement: String,
    owner: String,
    state: GateState,
    state_reason: String,
    activation_checkpoint: ActivationCheckpoint,
    backends: Option<Vec<Backend>>,
    backend_non_applicability: Option<String>,
    required_ci: Vec<String>,
    evidence: Evidence,
    threshold: Threshold,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum GateState {
    Red,
    Partial,
    Missing,
    Passed,
}

impl GateState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Partial => "partial",
            Self::Missing => "missing",
            Self::Passed => "passed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
enum ActivationCheckpoint {
    #[serde(rename = "CP0")]
    Cp0,
    #[serde(rename = "CP1")]
    Cp1,
    #[serde(rename = "CP2")]
    Cp2,
    #[serde(rename = "CP3")]
    Cp3,
    #[serde(rename = "CP4")]
    Cp4,
    #[serde(rename = "CP5")]
    Cp5,
    #[serde(rename = "CP6")]
    Cp6,
}

impl ActivationCheckpoint {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cp0 => "CP0",
            Self::Cp1 => "CP1",
            Self::Cp2 => "CP2",
            Self::Cp3 => "CP3",
            Self::Cp4 => "CP4",
            Self::Cp5 => "CP5",
            Self::Cp6 => "CP6",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Ord, PartialOrd, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Backend {
    InMemory,
    Sqlite,
    Postgresql,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    kind: String,
    schema_version: u16,
    schema_path: String,
    artifact_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum Threshold {
    Zero {
        metrics: Vec<String>,
    },
    Exact {
        metric: String,
        expected: toml::Value,
    },
    All {
        metric: String,
        required_cases: Vec<String>,
    },
    RequiredSet {
        metric: String,
        required_values: Vec<String>,
    },
    PercentileDuration {
        duration_definition: String,
        p50_max_minutes: Option<u16>,
        p90_max_minutes: u16,
    },
    ConfiguredFairness {
        fairness_target_source: String,
        allowed_deviation_source: String,
        capacity_limit_source: String,
        maximum_capacity_overruns: u64,
    },
    GovernanceBlock {
        blocked: bool,
        required_approvals: Vec<String>,
    },
}

fn validate_registry(
    registry: &GateRegistry,
    workspace_root: &Path,
) -> Result<(), ValidationError> {
    if registry.registry_version != REGISTRY_VERSION {
        return invalid_registry(format!(
            "registry_version must be {REGISTRY_VERSION}, found {}",
            registry.registry_version
        ));
    }
    if registry.evidence_schema_version != SCHEMA_VERSION {
        return invalid_registry(format!(
            "evidence_schema_version must be {SCHEMA_VERSION}, found {}",
            registry.evidence_schema_version
        ));
    }
    if registry.evidence_schema_path != SCHEMA_PATH {
        return invalid_registry(format!("evidence_schema_path must be `{SCHEMA_PATH}`"));
    }
    let focused_working_day = &registry.duration_definitions.focused_working_day;
    if focused_working_day.version == 0 || focused_working_day.minutes == 0 {
        return invalid_registry(
            "focused working day must have a positive version and duration".to_owned(),
        );
    }
    if registry.gates.len() != GATE_COUNT {
        return invalid_registry(format!(
            "registry must contain {GATE_COUNT} gates, found {}",
            registry.gates.len()
        ));
    }

    let mut gate_ids = BTreeSet::new();
    let mut context = ValidationContext {
        workspace_root,
        workflow_jobs: BTreeMap::new(),
        artifact_paths: BTreeSet::new(),
        focused_working_day_id: format!("focused-working-day-v{}", focused_working_day.version),
        focused_working_day_minutes: focused_working_day.minutes,
        focused_working_day_is_referenced: false,
    };
    for (index, gate) in registry.gates.iter().enumerate() {
        if !gate_ids.insert(gate.id.as_str()) {
            return invalid_registry(format!("gate id `{}` is duplicated", gate.id));
        }
        validate_gate(gate, index, &mut context)?;
    }
    if !context.focused_working_day_is_referenced {
        return invalid_registry(format!(
            "duration definition `{}` is not referenced by a threshold",
            context.focused_working_day_id
        ));
    }
    Ok(())
}

fn validate_gate(
    gate: &Gate,
    index: usize,
    context: &mut ValidationContext<'_>,
) -> Result<(), ValidationError> {
    let expected_id = format!("NS{:02}", index + 1);
    if gate.id != expected_id {
        return invalid_registry(format!(
            "gate {} must be `{expected_id}`, found `{}`",
            index + 1,
            gate.id
        ));
    }
    validate_text(&gate.statement, &format!("{} statement", gate.id), 512)?;
    if !ACCOUNTABLE_OWNER_ROLES.contains(&gate.owner.as_str()) {
        return invalid_registry(format!(
            "{} owner `{}` is not an accountable lead role",
            gate.id, gate.owner
        ));
    }
    validate_text(
        gate.activation_checkpoint.as_str(),
        &format!("{} activation_checkpoint", gate.id),
        3,
    )?;
    validate_text(gate.state.as_str(), &format!("{} state", gate.id), 7)?;
    if matches!(gate.state, GateState::Passed) {
        return invalid_registry(format!(
            "{} state `passed` is unsupported because schema v1 cannot represent trustworthy promotion",
            gate.id
        ));
    }
    validate_text(
        &gate.state_reason,
        &format!("{} state_reason", gate.id),
        512,
    )?;
    validate_backend_applicability(gate)?;
    validate_evidence(gate, &mut context.artifact_paths)?;
    validate_threshold(
        gate,
        &context.focused_working_day_id,
        context.focused_working_day_minutes,
        &mut context.focused_working_day_is_referenced,
    )?;
    validate_ci_bindings(gate, context.workspace_root, &mut context.workflow_jobs)?;
    Ok(())
}

fn validate_backend_applicability(gate: &Gate) -> Result<(), ValidationError> {
    match (&gate.backends, &gate.backend_non_applicability) {
        (Some(backends), None) if !backends.is_empty() && backends.len() <= 3 => {
            let unique = backends.iter().copied().collect::<BTreeSet<_>>();
            if unique.len() != backends.len() {
                return invalid_registry(format!("{} backend set contains duplicates", gate.id));
            }
            Ok(())
        },
        (None, Some(reason)) => validate_text(
            reason,
            &format!("{} backend_non_applicability", gate.id),
            512,
        ),
        (Some(_), Some(_)) => invalid_registry(format!(
            "{} must define backends or backend_non_applicability, not both",
            gate.id
        )),
        (None | Some(_), None) => invalid_registry(format!(
            "{} must define a non-empty backend set or backend_non_applicability",
            gate.id
        )),
    }
}

fn validate_evidence(
    gate: &Gate,
    artifact_paths: &mut BTreeSet<String>,
) -> Result<(), ValidationError> {
    if gate.evidence.schema_version != SCHEMA_VERSION || gate.evidence.schema_path != SCHEMA_PATH {
        return invalid_registry(format!(
            "{} evidence must use schema v{SCHEMA_VERSION} at `{SCHEMA_PATH}`",
            gate.id
        ));
    }
    validate_kebab_identifier(&gate.evidence.kind, &format!("{} evidence kind", gate.id))?;
    validate_repo_path(
        &gate.evidence.artifact_path,
        &format!("{} evidence artifact_path", gate.id),
    )?;
    if !has_extension(&gate.evidence.artifact_path, "json") {
        return invalid_registry(format!(
            "{} evidence artifact_path must name a JSON result manifest",
            gate.id
        ));
    }
    if !artifact_paths.insert(gate.evidence.artifact_path.clone()) {
        return invalid_registry(format!("{} evidence artifact_path is duplicated", gate.id));
    }
    Ok(())
}

fn validate_threshold(
    gate: &Gate,
    focused_working_day_id: &str,
    focused_working_day_minutes: u16,
    focused_working_day_is_referenced: &mut bool,
) -> Result<(), ValidationError> {
    match &gate.threshold {
        Threshold::Zero { metrics } => validate_nonempty_unique(metrics, &gate.id, "metrics"),
        Threshold::Exact { metric, expected } => {
            validate_text(metric, &format!("{} threshold metric", gate.id), 128)?;
            match expected {
                toml::Value::Boolean(_) | toml::Value::Integer(_) => Ok(()),
                toml::Value::String(value) => {
                    validate_text(value, &format!("{} exact threshold value", gate.id), 128)
                },
                _ => invalid_registry(format!(
                    "{} exact threshold must use a boolean, integer, or bounded string",
                    gate.id
                )),
            }
        },
        Threshold::All {
            metric,
            required_cases,
        } => {
            validate_text(metric, &format!("{} threshold metric", gate.id), 128)?;
            validate_nonempty_unique(required_cases, &gate.id, "required_cases")
        },
        Threshold::RequiredSet {
            metric,
            required_values,
        } => {
            validate_text(metric, &format!("{} threshold metric", gate.id), 128)?;
            validate_nonempty_unique(required_values, &gate.id, "required_values")
        },
        Threshold::PercentileDuration {
            duration_definition,
            p50_max_minutes,
            p90_max_minutes,
        } => {
            validate_text(
                duration_definition,
                &format!("{} duration_definition", gate.id),
                128,
            )?;
            if *p90_max_minutes == 0
                || p50_max_minutes.is_some_and(|p50| p50 == 0 || p50 > *p90_max_minutes)
            {
                return invalid_registry(format!(
                    "{} percentile-duration limits are invalid",
                    gate.id
                ));
            }
            if !is_versioned_duration_reference(duration_definition) {
                return invalid_registry(format!(
                    "{} duration_definition must end with a positive `-vN` version",
                    gate.id
                ));
            }
            if duration_definition == focused_working_day_id {
                if *p90_max_minutes != focused_working_day_minutes {
                    return invalid_registry(format!(
                        "{} focused working-day p90 must equal its declared duration",
                        gate.id
                    ));
                }
                *focused_working_day_is_referenced = true;
            }
            Ok(())
        },
        Threshold::ConfiguredFairness {
            fairness_target_source,
            allowed_deviation_source,
            capacity_limit_source,
            maximum_capacity_overruns,
        } => {
            for (name, value) in [
                ("fairness_target_source", fairness_target_source),
                ("allowed_deviation_source", allowed_deviation_source),
                ("capacity_limit_source", capacity_limit_source),
            ] {
                validate_text(value, &format!("{} {name}", gate.id), 128)?;
            }
            if *maximum_capacity_overruns != 0 {
                return invalid_registry(
                    "configured-fairness maximum_capacity_overruns must be zero".to_owned(),
                );
            }
            Ok(())
        },
        Threshold::GovernanceBlock {
            blocked,
            required_approvals,
        } => {
            if !blocked {
                return invalid_registry("governance-block must be active".to_owned());
            }
            validate_nonempty_unique(required_approvals, &gate.id, "required_approvals")
        },
    }
}

fn is_versioned_duration_reference(reference: &str) -> bool {
    reference.rsplit_once("-v").is_some_and(|(name, version)| {
        !name.is_empty() && version.parse::<u16>().is_ok_and(|version| version > 0)
    })
}

fn validate_nonempty_unique(
    values: &[String],
    gate_id: &str,
    field: &str,
) -> Result<(), ValidationError> {
    if values.is_empty() || values.len() > 128 {
        return invalid_registry(format!(
            "{gate_id} threshold {field} must contain 1..=128 entries"
        ));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        validate_text(value, &format!("{gate_id} threshold {field} entry"), 128)?;
        if !unique.insert(value) {
            return invalid_registry(format!(
                "{gate_id} threshold {field} contains duplicate `{value}`"
            ));
        }
    }
    Ok(())
}

fn validate_ci_bindings(
    gate: &Gate,
    workspace_root: &Path,
    workflow_jobs: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), ValidationError> {
    if gate.required_ci.is_empty() || gate.required_ci.len() > 3 {
        return invalid_registry(format!(
            "{} required_ci must contain 1..=3 bindings",
            gate.id
        ));
    }
    let mut unique = BTreeSet::new();
    for binding in &gate.required_ci {
        if !unique.insert(binding) {
            return invalid_registry(format!(
                "{} required_ci contains duplicate `{binding}`",
                gate.id
            ));
        }
        let (workflow, job) =
            binding
                .split_once('#')
                .ok_or_else(|| ValidationError::InvalidRegistry {
                    detail: format!("{} CI binding `{binding}` must contain one `#`", gate.id),
                })?;
        if job.contains('#') {
            return invalid_registry(format!(
                "{} CI binding `{binding}` contains more than one `#`",
                gate.id
            ));
        }
        validate_workflow_path(workflow)?;
        validate_job_id(job)?;
        if !workflow_jobs.contains_key(workflow) {
            let path = workspace_root.join(workflow);
            let source = fs::read_to_string(&path)
                .map_err(|source| ValidationError::Read { path, source })?;
            let jobs = parse_workflow_jobs(&source, workflow)?;
            workflow_jobs.insert(workflow.to_owned(), jobs);
        }
        if !workflow_jobs
            .get(workflow)
            .is_some_and(|jobs| jobs.contains(job))
        {
            return invalid_registry(format!(
                "{} CI binding `{binding}` names a missing workflow job",
                gate.id
            ));
        }
    }
    Ok(())
}

fn validate_workflow_path(workflow: &str) -> Result<(), ValidationError> {
    validate_repo_path(workflow, "CI workflow path")?;
    if !workflow.starts_with(".github/workflows/")
        || !(has_extension(workflow, "yml") || has_extension(workflow, "yaml"))
    {
        return invalid_registry(format!(
            "CI workflow `{workflow}` must be a checked-in workflow YAML path"
        ));
    }
    Ok(())
}

fn has_extension(path: &str, expected: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn validate_kebab_identifier(value: &str, label: &str) -> Result<(), ValidationError> {
    validate_text(value, label, 64)?;
    if value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
    {
        Ok(())
    } else {
        invalid_registry(format!("{label} `{value}` must be a kebab-case identifier"))
    }
}

fn validate_job_id(job: &str) -> Result<(), ValidationError> {
    if job.is_empty()
        || job.len() > 128
        || !job
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return invalid_registry(format!("CI job id `{job}` is invalid"));
    }
    Ok(())
}

fn parse_workflow_jobs(
    source: &str,
    workflow_path: &str,
) -> Result<BTreeSet<String>, ValidationError> {
    let mut in_jobs = false;
    let mut jobs = BTreeSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !in_jobs {
            in_jobs = line == "jobs:";
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
        if indentation == 0 {
            break;
        }
        if indentation != 2 {
            continue;
        }
        let job = trimmed.strip_suffix(':').ok_or_else(|| {
            ValidationError::InvalidRegistry {
                detail: format!(
                    "workflow `{workflow_path}` has an invalid top-level job declaration `{trimmed}`"
                ),
            }
        })?;
        validate_job_id(job)?;
        jobs.insert(job.to_owned());
    }
    if !in_jobs || jobs.is_empty() {
        return invalid_registry(format!(
            "workflow `{workflow_path}` does not declare any jobs"
        ));
    }
    Ok(jobs)
}

fn validate_repo_path(path: &str, label: &str) -> Result<(), ValidationError> {
    validate_text(path, label, 512)?;
    if path.contains('\\')
        || Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return invalid_registry(format!("{label} `{path}` is not a safe repository path"));
    }
    Ok(())
}

fn validate_text(value: &str, label: &str, maximum: usize) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.len() > maximum {
        return invalid_registry(format!("{label} must contain 1..={maximum} bytes"));
    }
    Ok(())
}

fn invalid_registry<T>(detail: String) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidRegistry { detail })
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("cannot read North Star gate input `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("North Star gate registry TOML is invalid: {0}")]
    RegistryParse(#[source] toml::de::Error),
    #[error("North Star evidence schema JSON is invalid: {0}")]
    SchemaParse(#[source] serde_json::Error),
    #[error("canonical North Star evidence JSON is invalid: {0}")]
    EvidenceParse(#[source] serde_json::Error),
    #[error("North Star gate registry is invalid: {detail}")]
    InvalidRegistry { detail: String },
    #[error("North Star evidence schema is invalid: {detail}")]
    InvalidSchema { detail: String },
    #[error("canonical North Star evidence does not satisfy schema v1: {detail}")]
    InvalidEvidence { detail: String },
}

#[cfg(test)]
mod tests;
