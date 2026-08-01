//! Authority-free immutable executable-plan contracts.

use core::fmt;
use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;
use nebula_core::{
    ActionKey, CredentialKey, ExecutablePlanRevisionId, NodeKey, PluginKey, PluginSetId, PortKey,
    ResourceKey, WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId,
};
use nebula_credential::Capabilities;
use nebula_schema::{
    Assignability, Field, FieldKey, FieldValue, FieldValues, InputSchema, OutputSchema, PathWalk,
    RequiredMode, Schema, SchemaKind, ValidSchema, explain_assignable, explain_field_assignable,
};
use semver::{BuildMetadata, Prerelease, Version};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const RECORD_VERSION_V1: u16 = 1;
pub(crate) const COMPILER_VERSION_GRAPH_V1: u16 = 1;
pub(crate) const CANONICAL_HASH_VERSION_V1: u16 = 1;
const EXECUTABLE_PLAN_GRAPH_V1_DOMAIN: &[u8] = b"nebula.executable-plan.graph.v1";
const VALUE_CANON_VERSION_GRAPH_V1: u16 = 1;
pub(crate) const SCHEMA_WIRE_VERSION_GRAPH_V1: u16 = 1;
const _: () = assert!(nebula_schema::VALUE_CANON_VERSION == VALUE_CANON_VERSION_GRAPH_V1);
const _: () = assert!(nebula_schema::SCHEMA_WIRE_VERSION == SCHEMA_WIRE_VERSION_GRAPH_V1);

/// One stable, secret-free activation diagnostic emitted by the plan compiler.
///
/// Values are constructed only inside this crate so the compiler can guarantee
/// that the five fields contain identifiers, versions, paths, or safe
/// sentinels rather than configuration or secret payloads.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActivationDiagnostic {
    code: String,
    path: String,
    expected: String,
    actual: String,
    remediation: String,
}

/// Longest a single diagnostic field may be, in bytes.
///
/// Diagnostics travel into logs and HTTP responses, and several fields embed
/// author-supplied text: a `path` names a node key, an `actual` reports the
/// contract that was observed. Neither is bounded by anything upstream, so a
/// workflow carrying a megabyte-long key would push a megabyte per diagnostic
/// into every log line and error body that mentions it. The bound is generous
/// enough that no honest identifier reaches it.
pub(crate) const MAX_DIAGNOSTIC_FIELD_BYTES: usize = 512;

/// Marker appended to a field that was cut to fit [`MAX_DIAGNOSTIC_FIELD_BYTES`].
///
/// A truncated value must be visibly truncated: a consumer comparing `actual`
/// against a known contract has to be able to tell "this differs" from "this is
/// the first 512 bytes of something that differs".
const TRUNCATION_MARKER: &str = "…";

/// Cut `field` to the byte bound, splitting only on a `char` boundary.
///
/// Slicing a `String` mid-codepoint panics, and these fields carry
/// author-supplied text, so the split point is walked back to the nearest
/// boundary rather than assumed.
fn bounded_field(field: String) -> String {
    if field.len() <= MAX_DIAGNOSTIC_FIELD_BYTES {
        return field;
    }
    let mut end = MAX_DIAGNOSTIC_FIELD_BYTES;
    while end > 0 && !field.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = String::with_capacity(end + TRUNCATION_MARKER.len());
    truncated.push_str(&field[..end]);
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

impl ActivationDiagnostic {
    pub(crate) fn new(
        code: impl Into<String>,
        path: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Option<Self> {
        // `code` is a stable machine-readable contract, so it is checked
        // against the bound rather than cut to fit: a truncated code would
        // silently name a different diagnostic. Compiler-authored codes are
        // short, so exceeding the bound is a construction bug, not input.
        let code = code.into();
        if code.len() > MAX_DIAGNOSTIC_FIELD_BYTES {
            return None;
        }

        let value = Self {
            code,
            path: bounded_field(path.into()),
            expected: bounded_field(expected.into()),
            actual: bounded_field(actual.into()),
            remediation: bounded_field(remediation.into()),
        };

        [
            value.code.as_str(),
            value.path.as_str(),
            value.expected.as_str(),
            value.actual.as_str(),
            value.remediation.as_str(),
        ]
        .iter()
        .all(|field| !field.trim().is_empty())
        .then_some(value)
    }

    /// Stable machine-readable diagnostic code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Canonical path to the incompatible workflow or registry element.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Secret-free description of the required contract.
    #[must_use]
    pub fn expected(&self) -> &str {
        &self.expected
    }

    /// Secret-free description of the observed contract or a safe sentinel.
    #[must_use]
    pub fn actual(&self) -> &str {
        &self.actual
    }

    /// Stable, actionable remediation guidance.
    #[must_use]
    pub fn remediation(&self) -> &str {
        &self.remediation
    }
}

impl fmt::Debug for ActivationDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationDiagnostic")
            .field("code", &self.code)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ActivationDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {}", self.code, self.path)
    }
}

/// A non-empty, canonically ordered set of plan-activation diagnostics.
#[derive(thiserror::Error)]
#[error("workflow plan compilation failed")]
pub struct PlanCompilationError {
    diagnostics: Box<[ActivationDiagnostic]>,
}

impl PlanCompilationError {
    pub(crate) fn new(mut diagnostics: Vec<ActivationDiagnostic>) -> Option<Self> {
        diagnostics.sort();
        diagnostics.dedup();
        (!diagnostics.is_empty()).then(|| Self {
            diagnostics: diagnostics.into_boxed_slice(),
        })
    }

    pub(crate) fn invalid_compiled_record() -> Self {
        Self {
            diagnostics: vec![ActivationDiagnostic {
                code: "PLUGIN_PLAN_GRAPH_V1:INVALID_COMPILED_RECORD".to_owned(),
                path: "/workflow".to_owned(),
                expected: "<canonical-graph-v1-plan>".to_owned(),
                actual: "<unsupported-variant>".to_owned(),
                remediation: "repair the reported workflow or component contract".to_owned(),
            }]
            .into_boxed_slice(),
        }
    }

    /// Canonically sorted, duplicate-free activation diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[ActivationDiagnostic] {
        &self.diagnostics
    }
}

impl fmt::Debug for PlanCompilationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanCompilationError")
            .field("diagnostic_count", &self.diagnostics.len())
            .finish()
    }
}

impl nebula_error::Classify for PlanCompilationError {
    fn category(&self) -> nebula_error::ErrorCategory {
        nebula_error::ErrorCategory::Validation
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new("PLUGIN_PLAN_COMPILE:INVALID_WORKFLOW")
    }
}

/// The workflow site that requires an abstract resource or credential binding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanBindingSite {
    /// An executable graph node.
    Node(NodeKey),
    /// A workflow trigger binding.
    Trigger(NodeKey),
}

/// The exact component contract required at an abstract binding site.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanBindingContract {
    /// A resource contract.
    #[non_exhaustive]
    Resource {
        /// Exact resource contract key.
        key: ResourceKey,
        /// Exact resource contract version.
        version: Version,
    },
    /// A credential contract.
    #[non_exhaustive]
    Credential {
        /// Exact credential contract key.
        key: CredentialKey,
        /// Exact credential contract version.
        version: Version,
        /// Capabilities the selected credential must provide.
        required_capabilities: Capabilities,
    },
}

/// One authority-free binding requirement compiled from workflow intent.
///
/// The selector remains an untrusted abstract author selector. It is not a
/// tenant-scoped resource or credential identifier and carries no authority.
#[derive(Clone, PartialEq, Eq)]
pub struct PlanBindingRequirement {
    site: PlanBindingSite,
    slot_key: String,
    selector: String,
    contract: PlanBindingContract,
    required: bool,
    lazy: bool,
}

impl PlanBindingRequirement {
    /// Node or trigger site that declares this slot.
    #[must_use]
    pub const fn site(&self) -> &PlanBindingSite {
        &self.site
    }

    /// Stable dependency slot key.
    #[must_use]
    pub fn slot_key(&self) -> &str {
        &self.slot_key
    }

    /// Abstract author selector to resolve under authenticated scope.
    #[must_use]
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// Exact resource or credential contract required by the slot.
    #[must_use]
    pub const fn contract(&self) -> &PlanBindingContract {
        &self.contract
    }

    /// Whether activation requires a matching binding.
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    /// Whether the runtime may resolve the binding lazily.
    #[must_use]
    pub const fn lazy(&self) -> bool {
        self.lazy
    }
}

impl fmt::Debug for PlanBindingRequirement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanBindingRequirement")
            .field("site", &self.site)
            .field("slot_key", &self.slot_key)
            .field("contract", &self.contract)
            .field("required", &self.required)
            .field("lazy", &self.lazy)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RecordedPlanProfileV1 {
    #[serde(rename = "graph-v1")]
    GraphV1,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_field_names,
    reason = "workflow-qualified names are the stable Graph-v1 record contract"
)]
pub(crate) struct RecordedPlanManifestV1 {
    pub(crate) workflow_definition_schema_version: u32,
    pub(crate) workflow_id: WorkflowId,
    pub(crate) workflow_semantic_version: RecordedWorkflowVersionV1,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedWorkflowVersionV1 {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) patch: u32,
    pub(crate) pre: Option<String>,
    pub(crate) build: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedSemverV1 {
    pub(crate) major: u64,
    pub(crate) minor: u64,
    pub(crate) patch: u64,
    pub(crate) pre: String,
    pub(crate) build: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedPluginV1 {
    pub(crate) key: String,
    pub(crate) version: RecordedSemverV1,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedActionKindV1 {
    Stateless,
    Stateful,
    Control,
    Trigger,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedIsolationV1 {
    None,
    CapabilityGated,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedCheckpointPolicyV1 {
    Inherit,
    OnePass,
    Stepwise,
    ForcedHandoff,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RecordedInputPortV1 {
    Flow {
        key: String,
    },
    Support {
        key: String,
        required: bool,
        multi: bool,
        allowed_node_types: Option<Box<[String]>>,
        allowed_tags: Option<Box<[String]>>,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedFlowKindV1 {
    Main,
    Error,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RecordedOutputPortV1 {
    Flow {
        key: String,
        flow_kind: RecordedFlowKindV1,
    },
    Dynamic {
        key: String,
        source_field: String,
        label_field: Option<String>,
        include_fallback: bool,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedDependencyV1 {
    pub(crate) key: String,
    pub(crate) required: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RecordedSlotV1 {
    Resource {
        slot_key: String,
        default_selector: String,
        contract_key: String,
        required: bool,
        lazy: bool,
    },
    Credential {
        slot_key: String,
        default_selector: String,
        contract_key: String,
        required: bool,
        lazy: bool,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedDependenciesV1 {
    pub(crate) credentials: Box<[RecordedDependencyV1]>,
    pub(crate) resources: Box<[RecordedDependencyV1]>,
    pub(crate) slots: Box<[RecordedSlotV1]>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct RecordedSchemaV1 {
    pub(crate) schema_wire_version: u16,
    pub(crate) schema: ValidSchema,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct RecordedSchemaSerializeV1<'a> {
    schema_wire_version: u16,
    schema: &'a ValidSchema,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedSchemaDeserializeV1 {
    schema_wire_version: u16,
    schema: Value,
}

impl Serialize for RecordedSchemaV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        RecordedSchemaSerializeV1 {
            schema_wire_version: self.schema_wire_version,
            schema: &self.schema,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RecordedSchemaV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let recorded = RecordedSchemaDeserializeV1::deserialize(deserializer)?;
        let schema_wire = serde_json::to_string(&recorded.schema)
            .map_err(|_| serde::de::Error::custom("invalid Graph-v1 schema wire"))?;
        let schema = serde_json::from_str::<ValidSchema>(&schema_wire)
            .map_err(|_| serde::de::Error::custom("invalid Graph-v1 schema wire"))?;
        let normalized = serde_json::to_value(&schema)
            .map_err(|_| serde::de::Error::custom("invalid Graph-v1 schema wire"))?;
        let recorded_bytes = FieldValue::Literal(recorded.schema)
            .canonical_bytes()
            .map_err(|_| serde::de::Error::custom("invalid Graph-v1 schema wire"))?;
        let normalized_bytes = FieldValue::Literal(normalized)
            .canonical_bytes()
            .map_err(|_| serde::de::Error::custom("invalid Graph-v1 schema wire"))?;
        if recorded_bytes != normalized_bytes {
            return Err(serde::de::Error::custom("invalid Graph-v1 schema wire"));
        }
        Ok(Self {
            schema_wire_version: recorded.schema_wire_version,
            schema,
        })
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedActionV1 {
    pub(crate) key: String,
    pub(crate) plugin_key: String,
    pub(crate) version: RecordedSemverV1,
    pub(crate) kind: RecordedActionKindV1,
    pub(crate) isolation: RecordedIsolationV1,
    pub(crate) checkpoint_policy: RecordedCheckpointPolicyV1,
    pub(crate) max_concurrent: Option<u32>,
    pub(crate) inputs: Box<[RecordedInputPortV1]>,
    pub(crate) outputs: Box<[RecordedOutputPortV1]>,
    pub(crate) input_schema: RecordedSchemaV1,
    pub(crate) output_schema: RecordedSchemaV1,
    pub(crate) dependencies: RecordedDependenciesV1,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedResourceV1 {
    pub(crate) key: String,
    pub(crate) plugin_key: String,
    pub(crate) version: RecordedSemverV1,
    pub(crate) configuration_schema: RecordedSchemaV1,
    pub(crate) dependencies: RecordedDependenciesV1,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedAuthPatternV1 {
    NoAuth,
    SecretToken,
    IdentityPassword,
    OAuth2,
    KeyPair,
    Certificate,
    RequestSigning,
    ConnectionUri,
    InstanceIdentity,
    SharedSecret,
    Custom,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedCredentialV1 {
    pub(crate) key: String,
    pub(crate) plugin_key: String,
    pub(crate) version: RecordedSemverV1,
    pub(crate) pattern: RecordedAuthPatternV1,
    pub(crate) properties_schema: RecordedSchemaV1,
    pub(crate) capability_bits: u8,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedDurationV1 {
    pub(crate) seconds: u64,
    pub(crate) nanoseconds: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedRetryV1 {
    pub(crate) max_attempts: u32,
    pub(crate) initial_delay_ms: u64,
    pub(crate) max_delay_ms: u64,
    pub(crate) backoff_multiplier_bits: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedRateLimitV1 {
    pub(crate) max_requests: u32,
    pub(crate) window_seconds: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedNodeV1 {
    pub(crate) id: String,
    pub(crate) plugin_key: String,
    pub(crate) action_key: String,
    pub(crate) action_version: RecordedSemverV1,
    pub(crate) parameters: Box<[RecordedParameterV1]>,
    pub(crate) retry_policy: Option<RecordedRetryV1>,
    pub(crate) timeout: Option<RecordedDurationV1>,
    pub(crate) rate_limit: Option<RecordedRateLimitV1>,
    pub(crate) enabled: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedParameterV1 {
    pub(crate) key: String,
    pub(crate) value: RecordedParameterValueV1,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RecordedParameterValueV1 {
    Literal {
        value: Value,
    },
    Expression {
        expression: String,
    },
    Template {
        template: String,
    },
    Reference {
        node_key: String,
        output_path: String,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedConnectionV1 {
    pub(crate) from_node: String,
    pub(crate) from_port: String,
    pub(crate) to_node: String,
    pub(crate) to_port: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedVariableV1 {
    pub(crate) name: String,
    pub(crate) value: Value,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedErrorStrategyV1 {
    FailFast,
    ContinueOnError,
    IgnoreErrors,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedCheckpointingV1 {
    pub(crate) enabled: bool,
    pub(crate) interval: Option<RecordedDurationV1>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedWorkflowConfigV1 {
    pub(crate) timeout: Option<RecordedDurationV1>,
    pub(crate) max_parallel_nodes: u64,
    pub(crate) checkpointing: RecordedCheckpointingV1,
    pub(crate) retry_policy: Option<RecordedRetryV1>,
    pub(crate) error_strategy: RecordedErrorStrategyV1,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedTriggerV1 {
    pub(crate) id: String,
    pub(crate) plugin_key: String,
    pub(crate) action_key: String,
    pub(crate) action_version: RecordedSemverV1,
    pub(crate) configuration: Value,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedConverterV1 {
    pub(crate) key: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedGraphContentV1 {
    pub(crate) plugins: Box<[RecordedPluginV1]>,
    pub(crate) nodes: Box<[RecordedNodeV1]>,
    pub(crate) connections: Box<[RecordedConnectionV1]>,
    pub(crate) actions: Box<[RecordedActionV1]>,
    pub(crate) resources: Box<[RecordedResourceV1]>,
    pub(crate) credentials: Box<[RecordedCredentialV1]>,
    pub(crate) triggers: Box<[RecordedTriggerV1]>,
    pub(crate) variables: Box<[RecordedVariableV1]>,
    pub(crate) workflow_config: RecordedWorkflowConfigV1,
    pub(crate) converters: Box<[RecordedConverterV1]>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum RecordedBindingSiteV1 {
    Node(String),
    Trigger(String),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RecordedBindingContractV1 {
    Resource {
        key: String,
        version: RecordedSemverV1,
    },
    Credential {
        key: String,
        version: RecordedSemverV1,
        required_capability_bits: u8,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedBindingV1 {
    pub(crate) site: RecordedBindingSiteV1,
    pub(crate) slot_key: String,
    pub(crate) selector: String,
    pub(crate) contract: RecordedBindingContractV1,
    pub(crate) required: bool,
    pub(crate) lazy: bool,
}

/// Version-one persisted projection of an immutable executable plan.
///
/// Fields are private so deserialization never creates an integrity-checked plan. Use
/// [`ExecutablePlanRevision::try_from_recorded_v1`] to validate structure and
/// the claimed content identity.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedExecutablePlanRevisionV1 {
    pub(crate) record_version: u16,
    pub(crate) compiler_version: u16,
    pub(crate) canonical_hash_version: u16,
    pub(crate) profile: RecordedPlanProfileV1,
    pub(crate) claimed_id: ExecutablePlanRevisionId,
    pub(crate) workflow_version_id: WorkflowVersionId,
    pub(crate) plugin_set_id: PluginSetId,
    pub(crate) worker_flavor_revision_id: WorkerFlavorRevisionId,
    pub(crate) manifest: RecordedPlanManifestV1,
    pub(crate) content: RecordedGraphContentV1,
    pub(crate) bindings: Box<[RecordedBindingV1]>,
}

impl fmt::Debug for RecordedExecutablePlanRevisionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordedExecutablePlanRevisionV1")
            .field("claimed_id", &self.claimed_id)
            .field("workflow_version_id", &self.workflow_version_id)
            .field("plugin_set_id", &self.plugin_set_id)
            .field("worker_flavor_revision_id", &self.worker_flavor_revision_id)
            .field("binding_count", &self.bindings.len())
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct CanonicalExecutablePlanV1<'a> {
    record_version: u16,
    compiler_version: u16,
    canonical_hash_version: u16,
    profile: RecordedPlanProfileV1,
    workflow_version_id: WorkflowVersionId,
    plugin_set_id: PluginSetId,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    manifest: &'a RecordedPlanManifestV1,
    content: &'a RecordedGraphContentV1,
    bindings: &'a [RecordedBindingV1],
}

impl RecordedExecutablePlanRevisionV1 {
    pub(crate) fn recomputed_id(
        &self,
    ) -> Result<ExecutablePlanRevisionId, ExecutablePlanIntegrityError> {
        let canonical_input = CanonicalExecutablePlanV1 {
            record_version: self.record_version,
            compiler_version: self.compiler_version,
            canonical_hash_version: self.canonical_hash_version,
            profile: self.profile,
            workflow_version_id: self.workflow_version_id,
            plugin_set_id: self.plugin_set_id,
            worker_flavor_revision_id: self.worker_flavor_revision_id,
            manifest: &self.manifest,
            content: &self.content,
            bindings: &self.bindings,
        };
        let value = serde_json::to_value(canonical_input)
            .map_err(|_| ExecutablePlanIntegrityError::CanonicalEncoding)?;
        let canonical = FieldValue::Literal(value)
            .canonical_bytes()
            .map_err(|_| ExecutablePlanIntegrityError::CanonicalEncoding)?;

        let mut hasher = Sha256::new();
        hash_field(&mut hasher, 1, EXECUTABLE_PLAN_GRAPH_V1_DOMAIN);
        hash_field(&mut hasher, 2, &canonical);
        let digest: [u8; 32] = hasher.finalize().into();
        Ok(ExecutablePlanRevisionId::from_bytes(digest))
    }
}

fn hash_field(hasher: &mut Sha256, tag: u8, value: &[u8]) {
    hasher.update([tag]);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

/// Integrity failures while loading a recorded executable plan.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ExecutablePlanIntegrityError {
    /// The record or compiler format is not supported by Graph-v1.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_INTEGRITY:UNSUPPORTED_FORMAT"
    )]
    #[error("unsupported executable-plan record format")]
    UnsupportedFormat,

    /// A canonical section is empty, unsorted, duplicated, or otherwise malformed.
    #[classify(category = "validation", code = "PLUGIN_PLAN_INTEGRITY:NON_CANONICAL")]
    #[error("executable-plan record is not canonical in section '{section}'")]
    NonCanonical {
        /// Stable section path, never a payload value.
        section: &'static str,
    },

    /// Graph-v1 records cannot contain converters.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_INTEGRITY:CONVERTERS_UNSUPPORTED"
    )]
    #[error("Graph-v1 executable plans require an empty converter set")]
    ConvertersUnsupported,

    /// A recorded credential capability bit is not known to this format.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_INTEGRITY:UNKNOWN_CAPABILITY"
    )]
    #[error("executable-plan record contains unknown credential capability bits")]
    UnknownCredentialCapability,

    /// Canonical JSON encoding failed.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_INTEGRITY:CANONICAL_ENCODING"
    )]
    #[error("executable-plan record contains a value that cannot be canonically encoded")]
    CanonicalEncoding,

    /// The claimed revision identity differs from canonical record content.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_INTEGRITY:REVISION_ID_MISMATCH"
    )]
    #[error("executable-plan revision identity does not match its canonical content")]
    RevisionIdMismatch {
        /// Identity claimed by the record.
        claimed: ExecutablePlanRevisionId,
        /// Identity recomputed from canonical content.
        computed: ExecutablePlanRevisionId,
    },
}

/// Integrity-checked, immutable, authority-free executable plan revision.
#[derive(Clone)]
pub struct ExecutablePlanRevision {
    record: RecordedExecutablePlanRevisionV1,
    bindings: Box<[PlanBindingRequirement]>,
}

impl ExecutablePlanRevision {
    /// Validate a recorded Graph-v1 plan and create an immutable structural view.
    ///
    /// This check proves record canonicality and content identity only. It does
    /// not authenticate artifacts, authorize a tenant, retain the revision, or
    /// admit an execution.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutablePlanIntegrityError`] when format versions,
    /// collection canonicality, binding structure, canonical values, or the
    /// claimed revision identity are invalid.
    pub fn try_from_recorded_v1(
        record: RecordedExecutablePlanRevisionV1,
    ) -> Result<Self, ExecutablePlanIntegrityError> {
        validate_record(&record)?;
        let computed = record.recomputed_id()?;
        if record.claimed_id != computed {
            return Err(ExecutablePlanIntegrityError::RevisionIdMismatch {
                claimed: record.claimed_id,
                computed,
            });
        }

        let bindings = record
            .bindings
            .iter()
            .map(PlanBindingRequirement::try_from)
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        Ok(Self { record, bindings })
    }

    /// Canonical content identity of this plan revision.
    #[must_use]
    pub const fn id(&self) -> ExecutablePlanRevisionId {
        self.record.claimed_id
    }

    /// Exact workflow revision compiled into this plan.
    #[must_use]
    pub const fn workflow_version_id(&self) -> WorkflowVersionId {
        self.record.workflow_version_id
    }

    /// Exact logical plugin set used by compilation.
    #[must_use]
    pub const fn plugin_set_id(&self) -> PluginSetId {
        self.record.plugin_set_id
    }

    /// Exact frozen worker flavor required to execute this plan.
    #[must_use]
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        self.record.worker_flavor_revision_id
    }

    /// Canonically ordered abstract binding requirements.
    #[must_use]
    pub fn bindings(&self) -> &[PlanBindingRequirement] {
        &self.bindings
    }

    pub(crate) const fn recorded(&self) -> &RecordedExecutablePlanRevisionV1 {
        &self.record
    }
}

impl fmt::Debug for ExecutablePlanRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutablePlanRevision")
            .field("id", &self.id())
            .field("workflow_version_id", &self.workflow_version_id())
            .field("plugin_set_id", &self.plugin_set_id())
            .field(
                "worker_flavor_revision_id",
                &self.worker_flavor_revision_id(),
            )
            .field("binding_count", &self.bindings.len())
            .finish_non_exhaustive()
    }
}

impl TryFrom<RecordedExecutablePlanRevisionV1> for ExecutablePlanRevision {
    type Error = ExecutablePlanIntegrityError;

    fn try_from(record: RecordedExecutablePlanRevisionV1) -> Result<Self, Self::Error> {
        Self::try_from_recorded_v1(record)
    }
}

impl From<&ExecutablePlanRevision> for RecordedExecutablePlanRevisionV1 {
    fn from(plan: &ExecutablePlanRevision) -> Self {
        plan.record.clone()
    }
}

impl TryFrom<&RecordedBindingV1> for PlanBindingRequirement {
    type Error = ExecutablePlanIntegrityError;

    fn try_from(binding: &RecordedBindingV1) -> Result<Self, Self::Error> {
        let site = match &binding.site {
            RecordedBindingSiteV1::Node(node) => {
                PlanBindingSite::Node(node.parse().map_err(|_| {
                    ExecutablePlanIntegrityError::NonCanonical {
                        section: "bindings",
                    }
                })?)
            },
            RecordedBindingSiteV1::Trigger(trigger) => {
                PlanBindingSite::Trigger(trigger.parse().map_err(|_| {
                    ExecutablePlanIntegrityError::NonCanonical {
                        section: "bindings",
                    }
                })?)
            },
        };
        let contract = match &binding.contract {
            RecordedBindingContractV1::Resource { key, version } => PlanBindingContract::Resource {
                key: key
                    .parse()
                    .map_err(|_| ExecutablePlanIntegrityError::NonCanonical {
                        section: "bindings",
                    })?,
                version: version.try_into()?,
            },
            RecordedBindingContractV1::Credential {
                key,
                version,
                required_capability_bits,
            } => PlanBindingContract::Credential {
                key: key
                    .parse()
                    .map_err(|_| ExecutablePlanIntegrityError::NonCanonical {
                        section: "bindings",
                    })?,
                version: version.try_into()?,
                required_capabilities: Capabilities::from_bits(*required_capability_bits)
                    .ok_or(ExecutablePlanIntegrityError::UnknownCredentialCapability)?,
            },
        };
        Ok(Self {
            site,
            slot_key: binding.slot_key.clone(),
            selector: binding.selector.clone(),
            contract,
            required: binding.required,
            lazy: binding.lazy,
        })
    }
}

impl TryFrom<&RecordedSemverV1> for Version {
    type Error = ExecutablePlanIntegrityError;

    fn try_from(version: &RecordedSemverV1) -> Result<Self, Self::Error> {
        let pre = Prerelease::new(&version.pre).map_err(|_| {
            ExecutablePlanIntegrityError::NonCanonical {
                section: "bindings",
            }
        })?;
        let build = BuildMetadata::new(&version.build).map_err(|_| {
            ExecutablePlanIntegrityError::NonCanonical {
                section: "bindings",
            }
        })?;
        Ok(Self {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
            pre,
            build,
        })
    }
}

fn validate_record(
    record: &RecordedExecutablePlanRevisionV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    if record.record_version != RECORD_VERSION_V1
        || record.compiler_version != COMPILER_VERSION_GRAPH_V1
        || record.canonical_hash_version != CANONICAL_HASH_VERSION_V1
        || record.profile != RecordedPlanProfileV1::GraphV1
    {
        return Err(ExecutablePlanIntegrityError::UnsupportedFormat);
    }

    validate_manifest(&record.manifest)?;
    if !record.content.converters.is_empty() {
        return Err(ExecutablePlanIntegrityError::ConvertersUnsupported);
    }
    validate_workflow_config(&record.content.workflow_config)?;

    let plugins = validate_plugins(&record.content.plugins)?;
    let actions = validate_actions(&record.content.actions, &plugins)?;
    let resources = validate_resources(&record.content.resources, &plugins)?;
    let credentials = validate_credentials(&record.content.credentials, &plugins)?;
    let nodes = validate_nodes(&record.content.nodes, &actions)?;
    let triggers = validate_triggers(&record.content.triggers, &actions)?;
    validate_variables(&record.content.variables)?;
    validate_connections(&record.content.connections, &nodes, &actions)?;
    validate_component_closure(
        &plugins,
        &actions,
        &resources,
        &credentials,
        &nodes,
        &triggers,
    )?;
    validate_bindings(
        &record.bindings,
        &nodes,
        &triggers,
        &actions,
        &resources,
        &credentials,
    )?;
    Ok(())
}

fn noncanonical(section: &'static str) -> ExecutablePlanIntegrityError {
    ExecutablePlanIntegrityError::NonCanonical { section }
}

fn validate_manifest(
    manifest: &RecordedPlanManifestV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    if manifest.workflow_definition_schema_version != nebula_workflow::CURRENT_SCHEMA_VERSION {
        return Err(noncanonical("manifest.workflow_definition_schema_version"));
    }
    if let Some(pre) = manifest.workflow_semantic_version.pre.as_deref()
        && (pre.is_empty() || Prerelease::new(pre).is_err())
    {
        return Err(noncanonical("manifest.workflow_semantic_version"));
    }
    if let Some(build) = manifest.workflow_semantic_version.build.as_deref()
        && (build.is_empty() || BuildMetadata::new(build).is_err())
    {
        return Err(noncanonical("manifest.workflow_semantic_version"));
    }
    Ok(())
}

fn validate_semver(
    version: &RecordedSemverV1,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    if Prerelease::new(&version.pre).is_err() || BuildMetadata::new(&version.build).is_err() {
        return Err(noncanonical(section));
    }
    Ok(())
}

fn validate_plugin_semver(
    version: &RecordedSemverV1,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    validate_semver(version, section)?;
    if !version.build.is_empty() {
        return Err(noncanonical(section));
    }
    Ok(())
}

fn validate_duration(
    duration: &RecordedDurationV1,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    if duration.nanoseconds >= 1_000_000_000 {
        return Err(noncanonical(section));
    }
    Ok(())
}

fn validate_retry(
    retry: &RecordedRetryV1,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    let multiplier = f64::from_bits(retry.backoff_multiplier_bits);
    if retry.max_attempts == 0
        || retry.max_delay_ms < retry.initial_delay_ms
        || !multiplier.is_finite()
        || multiplier <= 0.0
        || (retry.initial_delay_ms == 0 && retry.max_attempts > 1)
    {
        return Err(noncanonical(section));
    }
    Ok(())
}

fn validate_workflow_config(
    config: &RecordedWorkflowConfigV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    if config.max_parallel_nodes == 0 {
        return Err(noncanonical("workflow_config.max_parallel_nodes"));
    }
    if let Some(timeout) = &config.timeout {
        validate_duration(timeout, "workflow_config.timeout")?;
    }
    if let Some(interval) = &config.checkpointing.interval {
        validate_duration(interval, "workflow_config.checkpointing.interval")?;
    }
    if let Some(retry) = &config.retry_policy {
        validate_retry(retry, "workflow_config.retry_policy")?;
    }
    Ok(())
}

fn validate_plugins(
    plugins: &[RecordedPluginV1],
) -> Result<HashMap<&str, &RecordedPluginV1>, ExecutablePlanIntegrityError> {
    if plugins.is_empty() || plugins.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(noncanonical("plugins"));
    }
    let mut by_key = HashMap::with_capacity(plugins.len());
    for plugin in plugins {
        if plugin.key.parse::<PluginKey>().is_err() {
            return Err(noncanonical("plugins.key"));
        }
        validate_plugin_semver(&plugin.version, "plugins.version")?;
        by_key.insert(plugin.key.as_str(), plugin);
    }
    Ok(by_key)
}

fn validate_schema(
    schema: &RecordedSchemaV1,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    if schema.schema_wire_version != SCHEMA_WIRE_VERSION_GRAPH_V1 {
        return Err(noncanonical(section));
    }
    for field in schema.schema.fields() {
        validate_schema_field(field, section)?;
    }
    Ok(())
}

fn validate_schema_field(
    field: &Field,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    if matches!(field, Field::Unknown(_)) {
        return Err(noncanonical(section));
    }
    if field_contains_secret(field) && field.default().is_some() {
        return Err(noncanonical(section));
    }
    match field {
        Field::Object(object) => {
            for child in &object.fields {
                validate_schema_field(child, section)?;
            }
        },
        Field::List(list) => {
            if let Some(item) = list.item.as_deref() {
                validate_schema_field(item, section)?;
            }
        },
        Field::Mode(mode) => {
            for variant in &mode.variants {
                validate_schema_field(&variant.field, section)?;
            }
        },
        _ => {},
    }
    Ok(())
}

fn field_contains_secret(field: &Field) -> bool {
    match field {
        Field::Secret(_) => true,
        Field::Object(object) => object.fields.iter().any(field_contains_secret),
        Field::List(list) => list.item.as_deref().is_some_and(field_contains_secret),
        Field::Mode(mode) => mode
            .variants
            .iter()
            .any(|variant| field_contains_secret(&variant.field)),
        _ => false,
    }
}

fn validate_dependencies(
    dependencies: &RecordedDependenciesV1,
    section: &'static str,
) -> Result<(), ExecutablePlanIntegrityError> {
    if dependencies
        .credentials
        .windows(2)
        .any(|pair| pair[0].key >= pair[1].key)
        || dependencies
            .credentials
            .iter()
            .any(|dependency| dependency.key.parse::<CredentialKey>().is_err())
        || dependencies
            .resources
            .windows(2)
            .any(|pair| pair[0].key >= pair[1].key)
        || dependencies
            .resources
            .iter()
            .any(|dependency| dependency.key.parse::<ResourceKey>().is_err())
    {
        return Err(noncanonical(section));
    }
    if dependencies.slots.windows(2).any(|pair| {
        slot_sort_key(&pair[0]) >= slot_sort_key(&pair[1])
            || slot_name(&pair[0]) == slot_name(&pair[1])
    }) {
        return Err(noncanonical(section));
    }
    for slot in &dependencies.slots {
        let (slot_key, default_selector, contract_key) = match slot {
            RecordedSlotV1::Resource {
                slot_key,
                default_selector,
                contract_key,
                ..
            } => {
                if contract_key.parse::<ResourceKey>().is_err() {
                    return Err(noncanonical(section));
                }
                (slot_key, default_selector, contract_key)
            },
            RecordedSlotV1::Credential {
                slot_key,
                default_selector,
                contract_key,
                ..
            } => {
                if contract_key.parse::<CredentialKey>().is_err() {
                    return Err(noncanonical(section));
                }
                (slot_key, default_selector, contract_key)
            },
        };
        if FieldKey::new(slot_key).is_err()
            || default_selector.trim().is_empty()
            || default_selector.trim() != default_selector
            || contract_key.trim().is_empty()
        {
            return Err(noncanonical(section));
        }
    }
    Ok(())
}

fn slot_sort_key(slot: &RecordedSlotV1) -> (&str, u8) {
    match slot {
        RecordedSlotV1::Resource { slot_key, .. } => (slot_key, 0),
        RecordedSlotV1::Credential { slot_key, .. } => (slot_key, 1),
    }
}

fn slot_name(slot: &RecordedSlotV1) -> &str {
    match slot {
        RecordedSlotV1::Resource { slot_key, .. } | RecordedSlotV1::Credential { slot_key, .. } => {
            slot_key
        },
    }
}

fn validate_ports(action: &RecordedActionV1) -> Result<(), ExecutablePlanIntegrityError> {
    if action.inputs.is_empty()
        || action.outputs.is_empty()
        || action
            .inputs
            .windows(2)
            .any(|pair| input_port_key(&pair[0]) >= input_port_key(&pair[1]))
        || action
            .outputs
            .windows(2)
            .any(|pair| output_port_key(&pair[0]) >= output_port_key(&pair[1]))
    {
        return Err(noncanonical("actions.ports"));
    }
    for input in &action.inputs {
        let key = input_port_key(input);
        if PortKey::try_from(key).is_err() {
            return Err(noncanonical("actions.inputs"));
        }
        if let RecordedInputPortV1::Support {
            allowed_node_types,
            allowed_tags,
            ..
        } = input
        {
            if let Some(values) = allowed_node_types.as_deref()
                && (values.is_empty()
                    || values.windows(2).any(|pair| pair[0] >= pair[1])
                    || values
                        .iter()
                        .any(|value| value.parse::<ActionKey>().is_err()))
            {
                return Err(noncanonical("actions.inputs"));
            }
            if let Some(values) = allowed_tags.as_deref()
                && (values.is_empty()
                    || values.windows(2).any(|pair| pair[0] >= pair[1])
                    || values
                        .iter()
                        .any(|value| value.is_empty() || value.trim() != value))
            {
                return Err(noncanonical("actions.inputs.allowed_tags"));
            }
        }
    }
    for output in &action.outputs {
        if PortKey::try_from(output_port_key(output)).is_err() {
            return Err(noncanonical("actions.outputs"));
        }
        if let RecordedOutputPortV1::Dynamic {
            source_field,
            label_field,
            ..
        } = output
            && (source_field.is_empty()
                || !is_canonical_reference_path(source_field)
                || label_field
                    .as_deref()
                    .is_some_and(|field| FieldKey::new(field).is_err()))
        {
            return Err(noncanonical("actions.outputs.dynamic"));
        }
    }
    Ok(())
}

fn input_port_key(port: &RecordedInputPortV1) -> &str {
    match port {
        RecordedInputPortV1::Flow { key } | RecordedInputPortV1::Support { key, .. } => key,
    }
}

fn output_port_key(port: &RecordedOutputPortV1) -> &str {
    match port {
        RecordedOutputPortV1::Flow { key, .. } | RecordedOutputPortV1::Dynamic { key, .. } => key,
    }
}

fn validate_actions<'a>(
    actions: &'a [RecordedActionV1],
    plugins: &HashMap<&str, &RecordedPluginV1>,
) -> Result<HashMap<&'a str, &'a RecordedActionV1>, ExecutablePlanIntegrityError> {
    if actions.is_empty() || actions.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(noncanonical("actions"));
    }
    let mut by_key = HashMap::with_capacity(actions.len());
    for action in actions {
        if action.key.parse::<ActionKey>().is_err()
            || action.plugin_key.parse::<PluginKey>().is_err()
            || !action.key.starts_with(&format!("{}.", action.plugin_key))
            || !plugins.contains_key(action.plugin_key.as_str())
            || action.max_concurrent == Some(0)
        {
            return Err(noncanonical("actions.identity"));
        }
        validate_semver(&action.version, "actions.version")?;
        validate_schema(&action.input_schema, "actions.input_schema")?;
        validate_schema(&action.output_schema, "actions.output_schema")?;
        validate_dependencies(&action.dependencies, "actions.dependencies")?;
        validate_ports(action)?;
        by_key.insert(action.key.as_str(), action);
    }
    Ok(by_key)
}

fn validate_resources<'a>(
    resources: &'a [RecordedResourceV1],
    plugins: &HashMap<&str, &RecordedPluginV1>,
) -> Result<HashMap<&'a str, &'a RecordedResourceV1>, ExecutablePlanIntegrityError> {
    if resources.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(noncanonical("resources"));
    }
    let mut by_key = HashMap::with_capacity(resources.len());
    for resource in resources {
        if resource.key.parse::<ResourceKey>().is_err()
            || resource.plugin_key.parse::<PluginKey>().is_err()
            || !resource
                .key
                .starts_with(&format!("{}.", resource.plugin_key))
            || !plugins.contains_key(resource.plugin_key.as_str())
        {
            return Err(noncanonical("resources.identity"));
        }
        validate_semver(&resource.version, "resources.version")?;
        validate_schema(
            &resource.configuration_schema,
            "resources.configuration_schema",
        )?;
        validate_dependencies(&resource.dependencies, "resources.dependencies")?;
        by_key.insert(resource.key.as_str(), resource);
    }
    Ok(by_key)
}

fn validate_credentials<'a>(
    credentials: &'a [RecordedCredentialV1],
    plugins: &HashMap<&str, &RecordedPluginV1>,
) -> Result<HashMap<&'a str, &'a RecordedCredentialV1>, ExecutablePlanIntegrityError> {
    if credentials
        .windows(2)
        .any(|pair| pair[0].key >= pair[1].key)
    {
        return Err(noncanonical("credentials"));
    }
    let mut by_key = HashMap::with_capacity(credentials.len());
    for credential in credentials {
        if credential.key.parse::<CredentialKey>().is_err()
            || credential.plugin_key.parse::<PluginKey>().is_err()
            || !credential
                .key
                .starts_with(&format!("{}.", credential.plugin_key))
            || !plugins.contains_key(credential.plugin_key.as_str())
        {
            return Err(noncanonical("credentials.identity"));
        }
        validate_semver(&credential.version, "credentials.version")?;
        validate_schema(
            &credential.properties_schema,
            "credentials.properties_schema",
        )?;
        if Capabilities::from_bits(credential.capability_bits).is_none() {
            return Err(ExecutablePlanIntegrityError::UnknownCredentialCapability);
        }
        by_key.insert(credential.key.as_str(), credential);
    }
    Ok(by_key)
}

pub(crate) fn validate_parameter(
    parameter: &RecordedParameterV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    if FieldKey::new(&parameter.key).is_err() {
        return Err(noncanonical("nodes.parameters"));
    }
    match &parameter.value {
        RecordedParameterValueV1::Literal { value } => {
            FieldValue::Literal(value.clone())
                .canonical_bytes()
                .map_err(|_| noncanonical("nodes.parameters"))?;
        },
        RecordedParameterValueV1::Expression { expression } => {
            if expression.trim().is_empty() {
                return Err(noncanonical("nodes.parameters"));
            }
        },
        RecordedParameterValueV1::Template { .. } => {},
        RecordedParameterValueV1::Reference {
            node_key,
            output_path,
        } => {
            if node_key.parse::<NodeKey>().is_err() || !is_canonical_reference_path(output_path) {
                return Err(noncanonical("nodes.parameters"));
            }
        },
    }
    Ok(())
}

fn is_canonical_reference_path(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    if path.starts_with('$') {
        return false;
    }
    path.split('.').all(|segment| {
        !segment.is_empty()
            && (!segment.bytes().all(|byte| byte.is_ascii_digit())
                || segment == "0"
                || !segment.starts_with('0'))
    })
}

pub(crate) fn validate_parameter_contract(
    parameter: &RecordedParameterV1,
    action: &RecordedActionV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    let key = FieldKey::new(&parameter.key).map_err(|_| noncanonical("nodes.parameters"))?;
    let field = action
        .input_schema
        .schema
        .find(&key)
        .ok_or_else(|| noncanonical("nodes.parameters.schema"))?;
    if let RecordedParameterValueV1::Literal { value } = &parameter.value
        && value_populates_secret(field, value)
    {
        return Err(noncanonical("nodes.parameters.secret"));
    }
    let Some(typed) = typed_parameter_value(&parameter.value)? else {
        return Ok(());
    };
    let one_field_schema = Schema::builder()
        .add(field.clone())
        .build()
        .map_err(|_| noncanonical("nodes.parameters.schema"))?;
    let mut values = FieldValues::new();
    values.set(key, typed);
    one_field_schema
        .validate(&values)
        .map_err(|_| noncanonical("nodes.parameters.schema"))?;
    Ok(())
}

fn typed_parameter_value(
    value: &RecordedParameterValueV1,
) -> Result<Option<FieldValue>, ExecutablePlanIntegrityError> {
    match value {
        RecordedParameterValueV1::Literal { value } => typed_literal(value.clone()).map(Some),
        RecordedParameterValueV1::Expression { expression } => Ok(Some(FieldValue::Expression(
            nebula_schema::Expression::new(expression.as_str()),
        ))),
        RecordedParameterValueV1::Template { template } => Ok(Some(FieldValue::Expression(
            nebula_schema::Expression::new(template.as_str()),
        ))),
        RecordedParameterValueV1::Reference { .. } => Ok(None),
    }
}

pub(crate) fn validate_node_parameters(
    parameters: &[RecordedParameterV1],
    action: &RecordedActionV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    if !action.input_schema.schema.root_rules().is_empty() {
        return Err(noncanonical("nodes.parameters.root_rules"));
    }

    let supplied = parameters
        .iter()
        .map(|parameter| parameter.key.as_str())
        .collect::<HashSet<_>>();
    let has_reference = parameters
        .iter()
        .any(|parameter| matches!(parameter.value, RecordedParameterValueV1::Reference { .. }));
    for field in action.input_schema.schema.fields() {
        match field.required() {
            RequiredMode::Always if !supplied.contains(field.key().as_str()) => {
                return Err(noncanonical("nodes.parameters.required"));
            },
            RequiredMode::When(_) if has_reference && !supplied.contains(field.key().as_str()) => {
                return Err(noncanonical("nodes.parameters.conditional_required"));
            },
            _ => {},
        }
    }

    if has_reference {
        return Ok(());
    }

    let mut values = FieldValues::new();
    for parameter in parameters {
        let key =
            FieldKey::new(&parameter.key).map_err(|_| noncanonical("nodes.parameters.schema"))?;
        let Some(value) = typed_parameter_value(&parameter.value)? else {
            return Err(noncanonical("nodes.parameters.schema"));
        };
        values.set(key, value);
    }
    if action
        .input_schema
        .schema
        .first_undeclared_path(&values)
        .is_some()
    {
        return Err(noncanonical("nodes.parameters.schema"));
    }
    action
        .input_schema
        .schema
        .validate(&values)
        .map(|_| ())
        .map_err(|_| noncanonical("nodes.parameters.schema"))
}

fn typed_literal(value: Value) -> Result<FieldValue, ExecutablePlanIntegrityError> {
    FieldValue::Literal(value.clone())
        .canonical_bytes()
        .map_err(|_| noncanonical("nodes.parameters.schema"))?;
    Ok(typed_literal_with_checked_depth(value))
}

fn typed_literal_with_checked_depth(value: Value) -> FieldValue {
    match value {
        Value::Object(map) => {
            let Some(parsed_keys): Option<Vec<FieldKey>> = map
                .keys()
                .map(|key| FieldKey::new(key.as_str()).ok())
                .collect()
            else {
                return FieldValue::Literal(Value::Object(map));
            };
            let mut values = IndexMap::with_capacity(map.len());
            for ((_, child), key) in map.into_iter().zip(parsed_keys) {
                values.insert(key, typed_literal_with_checked_depth(child));
            }
            FieldValue::Object(values)
        },
        Value::Array(items) => FieldValue::List(
            items
                .into_iter()
                .map(typed_literal_with_checked_depth)
                .collect(),
        ),
        scalar => FieldValue::Literal(scalar),
    }
}

fn value_populates_secret(field: &Field, value: &Value) -> bool {
    match field {
        Field::Secret(_) => true,
        Field::Object(object) => value.as_object().is_some_and(|values| {
            object.fields.iter().any(|child| {
                values
                    .get(child.key().as_str())
                    .is_some_and(|child_value| value_populates_secret(child, child_value))
            })
        }),
        Field::List(list) => list.item.as_deref().is_some_and(|item| {
            value.as_array().is_some_and(|values| {
                values
                    .iter()
                    .any(|child_value| value_populates_secret(item, child_value))
            })
        }),
        Field::Mode(mode) => value.as_object().is_some_and(|envelope| {
            let selected = envelope.get("mode").and_then(Value::as_str);
            let payload = envelope.get("value");
            mode.variants.iter().any(|variant| {
                selected == Some(variant.key.as_str())
                    && payload.is_some_and(|value| value_populates_secret(&variant.field, value))
            })
        }),
        _ => false,
    }
}

fn validate_nodes<'a>(
    nodes: &'a [RecordedNodeV1],
    actions: &HashMap<&str, &RecordedActionV1>,
) -> Result<HashMap<&'a str, &'a RecordedNodeV1>, ExecutablePlanIntegrityError> {
    if nodes.is_empty() || nodes.windows(2).any(|pair| pair[0].id >= pair[1].id) {
        return Err(noncanonical("nodes"));
    }
    let mut by_id = HashMap::with_capacity(nodes.len());
    for node in nodes {
        if node.id.parse::<NodeKey>().is_err()
            || node.plugin_key.parse::<PluginKey>().is_err()
            || node.action_key.parse::<ActionKey>().is_err()
            || !node.enabled
        {
            return Err(noncanonical("nodes.identity"));
        }
        validate_semver(&node.action_version, "nodes.action_version")?;
        let Some(action) = actions.get(node.action_key.as_str()) else {
            return Err(noncanonical("nodes.action"));
        };
        let plugin_matches = action.plugin_key == node.plugin_key;
        let version_matches = action.version == node.action_version;
        let contract_mismatch = !plugin_matches || !version_matches;
        if contract_mismatch || matches!(action.kind, RecordedActionKindV1::Trigger) {
            return Err(noncanonical("nodes.action"));
        }
        if node
            .parameters
            .windows(2)
            .any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(noncanonical("nodes.parameters"));
        }
        for parameter in &node.parameters {
            validate_parameter(parameter)?;
            validate_parameter_contract(parameter, action)?;
        }
        validate_node_parameters(&node.parameters, action)?;
        if let Some(retry) = &node.retry_policy {
            validate_retry(retry, "nodes.retry_policy")?;
        }
        if let Some(timeout) = &node.timeout {
            validate_duration(timeout, "nodes.timeout")?;
        }
        if node
            .rate_limit
            .as_ref()
            .is_some_and(|limit| limit.max_requests == 0 || limit.window_seconds == 0)
        {
            return Err(noncanonical("nodes.rate_limit"));
        }
        by_id.insert(node.id.as_str(), node);
    }
    Ok(by_id)
}

fn validate_triggers<'a>(
    triggers: &'a [RecordedTriggerV1],
    actions: &HashMap<&str, &RecordedActionV1>,
) -> Result<HashMap<&'a str, &'a RecordedTriggerV1>, ExecutablePlanIntegrityError> {
    if triggers.windows(2).any(|pair| pair[0].id >= pair[1].id) {
        return Err(noncanonical("triggers"));
    }
    let mut by_id = HashMap::with_capacity(triggers.len());
    for trigger in triggers {
        if trigger.id.parse::<NodeKey>().is_err()
            || trigger.plugin_key.parse::<PluginKey>().is_err()
            || trigger.action_key.parse::<ActionKey>().is_err()
        {
            return Err(noncanonical("triggers.identity"));
        }
        validate_semver(&trigger.action_version, "triggers.action_version")?;
        FieldValue::Literal(trigger.configuration.clone())
            .canonical_bytes()
            .map_err(|_| noncanonical("triggers.configuration"))?;
        let Some(action) = actions.get(trigger.action_key.as_str()) else {
            return Err(noncanonical("triggers.action"));
        };
        let plugin_matches = action.plugin_key == trigger.plugin_key;
        let version_matches = action.version == trigger.action_version;
        let contract_mismatch = !plugin_matches || !version_matches;
        if contract_mismatch || !matches!(action.kind, RecordedActionKindV1::Trigger) {
            return Err(noncanonical("triggers.action"));
        }
        validate_trigger_configuration(&trigger.configuration, action)?;
        by_id.insert(trigger.id.as_str(), trigger);
    }
    Ok(by_id)
}

pub(crate) fn validate_trigger_configuration(
    configuration: &Value,
    action: &RecordedActionV1,
) -> Result<(), ExecutablePlanIntegrityError> {
    let normalized = if configuration.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        configuration.clone()
    };
    let values = FieldValues::from_json(normalized)
        .map_err(|_| noncanonical("triggers.configuration.schema"))?;
    if action.input_schema.schema.kind() != SchemaKind::Record {
        if values.is_empty() {
            return Ok(());
        }
        return Err(noncanonical("triggers.configuration.schema"));
    }
    if action
        .input_schema
        .schema
        .first_undeclared_path(&values)
        .is_some()
    {
        return Err(noncanonical("triggers.configuration.schema"));
    }
    for field in action.input_schema.schema.fields() {
        if values
            .get(field.key())
            .is_some_and(|value| value_populates_secret(field, &value.to_json()))
        {
            return Err(noncanonical("triggers.configuration.secret"));
        }
    }
    action
        .input_schema
        .schema
        .validate(&values)
        .map_err(|_| noncanonical("triggers.configuration.schema"))?;
    Ok(())
}

fn validate_variables(
    variables: &[RecordedVariableV1],
) -> Result<(), ExecutablePlanIntegrityError> {
    if variables
        .windows(2)
        .any(|pair| pair[0].name >= pair[1].name)
        || variables
            .iter()
            .any(|variable| variable.name.trim().is_empty())
    {
        return Err(noncanonical("variables"));
    }
    for variable in variables {
        FieldValue::Literal(variable.value.clone())
            .canonical_bytes()
            .map_err(|_| noncanonical("variables"))?;
    }
    Ok(())
}

fn validate_connections(
    connections: &[RecordedConnectionV1],
    nodes: &HashMap<&str, &RecordedNodeV1>,
    actions: &HashMap<&str, &RecordedActionV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    if connections
        .windows(2)
        .any(|pair| connection_sort_key(&pair[0]) >= connection_sort_key(&pair[1]))
    {
        return Err(noncanonical("connections"));
    }
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for connection in connections {
        if connection.from_node.parse::<NodeKey>().is_err()
            || connection.to_node.parse::<NodeKey>().is_err()
            || PortKey::try_from(connection.from_port.as_str()).is_err()
            || connection
                .to_port
                .as_deref()
                .is_some_and(|port| PortKey::try_from(port).is_err())
            || connection.from_node == connection.to_node
        {
            return Err(noncanonical("connections.identity"));
        }
        let Some(source) = nodes.get(connection.from_node.as_str()) else {
            return Err(noncanonical("connections.from_node"));
        };
        let Some(target) = nodes.get(connection.to_node.as_str()) else {
            return Err(noncanonical("connections.to_node"));
        };
        let source_action = actions
            .get(source.action_key.as_str())
            .ok_or_else(|| noncanonical("connections.from_action"))?;
        let target_action = actions
            .get(target.action_key.as_str())
            .ok_or_else(|| noncanonical("connections.to_action"))?;
        let source_port = source_action
            .outputs
            .iter()
            .find(|port| output_port_key(port) == connection.from_port)
            .ok_or_else(|| noncanonical("connections.from_port"))?;
        if !matches!(
            source_port,
            RecordedOutputPortV1::Flow {
                key,
                flow_kind: RecordedFlowKindV1::Main,
            } if key == "out"
        ) {
            return Err(noncanonical("connections.from_port"));
        }
        match connection.to_port.as_deref() {
            Some(port) => {
                let target_port = target_action
                    .inputs
                    .iter()
                    .find(|input| input_port_key(input) == port)
                    .ok_or_else(|| noncanonical("connections.to_port"))?;
                let RecordedInputPortV1::Support {
                    allowed_node_types,
                    allowed_tags,
                    ..
                } = target_port
                else {
                    return Err(noncanonical("connections.to_port"));
                };
                if allowed_tags.is_some() {
                    return Err(noncanonical("connections.to_port.tag_filter"));
                }
                if allowed_node_types.as_deref().is_some_and(|allowed| {
                    !allowed.iter().any(|key| key == source.action_key.as_str())
                }) {
                    return Err(noncanonical("connections.to_port.filter"));
                }
            },
            None => {
                if target_action
                    .inputs
                    .iter()
                    .filter(|input| matches!(input, RecordedInputPortV1::Flow { .. }))
                    .count()
                    != 1
                {
                    return Err(noncanonical("connections.to_port"));
                }
            },
        }
        if connection.to_port.is_none() {
            let producer = OutputSchema::new(source_action.output_schema.schema.clone());
            let consumer = InputSchema::new(target_action.input_schema.schema.clone());
            if !matches!(explain_assignable(&producer, &consumer), Assignability::Yes) {
                return Err(noncanonical("connections.schema"));
            }
        }
        adjacency
            .entry(connection.from_node.as_str())
            .or_default()
            .push(connection.to_node.as_str());
    }
    validate_support_port_cardinality(connections, nodes, actions)?;
    if graph_has_cycle(nodes.keys().copied(), &adjacency) {
        return Err(noncanonical("connections.cycle"));
    }
    for node in nodes.values() {
        for parameter in &node.parameters {
            if let RecordedParameterValueV1::Reference {
                node_key,
                output_path,
            } = &parameter.value
            {
                let source = nodes
                    .get(node_key.as_str())
                    .ok_or_else(|| noncanonical("nodes.parameters.reference"))?;
                if !connections.iter().any(|connection| {
                    connection.from_node == *node_key && connection.to_node == node.id
                }) {
                    return Err(noncanonical("nodes.parameters.reference"));
                }
                validate_reference_contract(parameter, output_path, source, node, actions)?;
            }
        }
    }
    Ok(())
}

fn validate_support_port_cardinality(
    connections: &[RecordedConnectionV1],
    nodes: &HashMap<&str, &RecordedNodeV1>,
    actions: &HashMap<&str, &RecordedActionV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    for node in nodes.values() {
        let action = actions
            .get(node.action_key.as_str())
            .ok_or_else(|| noncanonical("connections.to_action"))?;
        for input in &action.inputs {
            let RecordedInputPortV1::Support {
                key,
                required,
                multi,
                ..
            } = input
            else {
                continue;
            };
            let count = connections
                .iter()
                .filter(|connection| {
                    connection.to_node == node.id
                        && connection.to_port.as_deref() == Some(key.as_str())
                })
                .count();
            if (*required && count == 0) || (!*multi && count > 1) {
                return Err(noncanonical("connections.support_cardinality"));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_reference_contract(
    parameter: &RecordedParameterV1,
    output_path: &str,
    source: &RecordedNodeV1,
    consumer: &RecordedNodeV1,
    actions: &HashMap<&str, &RecordedActionV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    let source_action = actions
        .get(source.action_key.as_str())
        .ok_or_else(|| noncanonical("nodes.parameters.reference"))?;
    let consumer_action = actions
        .get(consumer.action_key.as_str())
        .ok_or_else(|| noncanonical("nodes.parameters.reference"))?;
    let consumer_key =
        FieldKey::new(&parameter.key).map_err(|_| noncanonical("nodes.parameters.reference"))?;
    let consumer_field = consumer_action
        .input_schema
        .schema
        .find(&consumer_key)
        .ok_or_else(|| noncanonical("nodes.parameters.reference"))?;
    if output_path.is_empty() || output_path == "$" {
        let Field::Object(object) = consumer_field else {
            return Err(noncanonical("nodes.parameters.reference.root"));
        };
        if object.fields.is_empty() {
            return Err(noncanonical("nodes.parameters.reference.root"));
        }
        let consumer_schema = Schema::builder()
            .add_many(object.fields.clone())
            .build()
            .map_err(|_| noncanonical("nodes.parameters.reference.root"))?;
        let producer = OutputSchema::new(source_action.output_schema.schema.clone());
        let consumer = InputSchema::new(consumer_schema);
        if !matches!(explain_assignable(&producer, &consumer), Assignability::Yes) {
            return Err(noncanonical("nodes.parameters.reference.schema"));
        }
        return Ok(());
    }
    let producer_field = match source_action
        .output_schema
        .schema
        .walk_authored_path(output_path)
    {
        PathWalk::Resolved(field) => field,
        PathWalk::Unresolved(_) | PathWalk::Opaque => {
            return Err(noncanonical("nodes.parameters.reference.path"));
        },
        _ => return Err(noncanonical("nodes.parameters.reference.path")),
    };
    if !matches!(
        explain_field_assignable(producer_field, consumer_field),
        Assignability::Yes
    ) {
        return Err(noncanonical("nodes.parameters.reference.schema"));
    }
    Ok(())
}

fn connection_sort_key(connection: &RecordedConnectionV1) -> (&str, &str, &str, Option<&str>) {
    (
        connection.from_node.as_str(),
        connection.from_port.as_str(),
        connection.to_node.as_str(),
        connection.to_port.as_deref(),
    )
}

fn graph_has_cycle<'a>(
    nodes: impl Iterator<Item = &'a str>,
    adjacency: &HashMap<&'a str, Vec<&'a str>>,
) -> bool {
    let nodes = nodes.collect::<Vec<_>>();
    let node_set = nodes.iter().copied().collect::<HashSet<_>>();
    let mut indegree = nodes
        .iter()
        .copied()
        .map(|node| (node, 0_usize))
        .collect::<HashMap<_, _>>();
    for targets in adjacency.values() {
        for target in targets {
            if node_set.contains(target)
                && let Some(count) = indegree.get_mut(target)
            {
                *count = count.saturating_add(1);
            }
        }
    }

    let mut ready = indegree
        .iter()
        .filter_map(|(node, count)| (*count == 0).then_some(*node))
        .collect::<Vec<_>>();
    let mut visited = 0_usize;
    while let Some(node) = ready.pop() {
        visited = visited.saturating_add(1);
        if let Some(targets) = adjacency.get(node) {
            for target in targets {
                if let Some(count) = indegree.get_mut(target) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        ready.push(target);
                    }
                }
            }
        }
    }
    visited != nodes.len()
}

fn validate_component_closure(
    plugins: &HashMap<&str, &RecordedPluginV1>,
    actions: &HashMap<&str, &RecordedActionV1>,
    resources: &HashMap<&str, &RecordedResourceV1>,
    credentials: &HashMap<&str, &RecordedCredentialV1>,
    nodes: &HashMap<&str, &RecordedNodeV1>,
    triggers: &HashMap<&str, &RecordedTriggerV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    let used_actions = nodes
        .values()
        .map(|node| node.action_key.as_str())
        .chain(triggers.values().map(|trigger| trigger.action_key.as_str()))
        .collect::<BTreeSet<_>>();
    if used_actions.len() != actions.len() || actions.keys().any(|key| !used_actions.contains(key))
    {
        return Err(noncanonical("actions.unused"));
    }

    let mut used_resources = BTreeSet::new();
    let mut used_credentials = BTreeSet::new();
    let mut resource_stack = Vec::new();
    for action in actions.values() {
        collect_dependencies(
            &action.plugin_key,
            &action.dependencies,
            plugins,
            resources,
            credentials,
            &mut used_resources,
            &mut used_credentials,
            &mut resource_stack,
        )?;
    }
    while let Some(resource_key) = resource_stack.pop() {
        let resource = resources
            .get(resource_key)
            .ok_or_else(|| noncanonical("resources.dependencies"))?;
        collect_dependencies(
            &resource.plugin_key,
            &resource.dependencies,
            plugins,
            resources,
            credentials,
            &mut used_resources,
            &mut used_credentials,
            &mut resource_stack,
        )?;
    }
    if used_resources.len() != resources.len()
        || resources.keys().any(|key| !used_resources.contains(key))
        || used_credentials.len() != credentials.len()
        || credentials
            .keys()
            .any(|key| !used_credentials.contains(key))
    {
        return Err(noncanonical("components.unused"));
    }
    validate_resource_cycles(resources)?;

    let used_plugins = actions
        .values()
        .map(|action| action.plugin_key.as_str())
        .chain(
            resources
                .values()
                .map(|resource| resource.plugin_key.as_str()),
        )
        .chain(
            credentials
                .values()
                .map(|credential| credential.plugin_key.as_str()),
        )
        .collect::<BTreeSet<_>>();
    if used_plugins.len() != plugins.len() || plugins.keys().any(|key| !used_plugins.contains(key))
    {
        return Err(noncanonical("plugins.unused"));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "closure validation keeps the exact four typed record maps and two accumulated sets explicit"
)]
fn collect_dependencies<'a>(
    owner_plugin: &str,
    dependencies: &'a RecordedDependenciesV1,
    plugins: &HashMap<&str, &RecordedPluginV1>,
    resources: &HashMap<&'a str, &'a RecordedResourceV1>,
    credentials: &HashMap<&'a str, &'a RecordedCredentialV1>,
    used_resources: &mut BTreeSet<&'a str>,
    used_credentials: &mut BTreeSet<&'a str>,
    resource_stack: &mut Vec<&'a str>,
) -> Result<(), ExecutablePlanIntegrityError> {
    for dependency in &dependencies.resources {
        let resource = resources
            .get(dependency.key.as_str())
            .ok_or_else(|| noncanonical("resources.dependencies"))?;
        validate_cross_plugin_dependency(owner_plugin, &resource.plugin_key, plugins)?;
        if used_resources.insert(resource.key.as_str()) {
            resource_stack.push(resource.key.as_str());
        }
    }
    for dependency in &dependencies.credentials {
        let credential = credentials
            .get(dependency.key.as_str())
            .ok_or_else(|| noncanonical("credentials.dependencies"))?;
        validate_cross_plugin_dependency(owner_plugin, &credential.plugin_key, plugins)?;
        used_credentials.insert(credential.key.as_str());
    }
    for slot in &dependencies.slots {
        match slot {
            RecordedSlotV1::Resource { contract_key, .. } => {
                let resource = resources
                    .get(contract_key.as_str())
                    .ok_or_else(|| noncanonical("resources.slots"))?;
                validate_cross_plugin_dependency(owner_plugin, &resource.plugin_key, plugins)?;
                if used_resources.insert(resource.key.as_str()) {
                    resource_stack.push(resource.key.as_str());
                }
            },
            RecordedSlotV1::Credential { contract_key, .. } => {
                let credential = credentials
                    .get(contract_key.as_str())
                    .ok_or_else(|| noncanonical("credentials.slots"))?;
                validate_cross_plugin_dependency(owner_plugin, &credential.plugin_key, plugins)?;
                used_credentials.insert(credential.key.as_str());
            },
        }
    }
    Ok(())
}

fn validate_cross_plugin_dependency(
    owner_plugin: &str,
    provider_plugin: &str,
    plugins: &HashMap<&str, &RecordedPluginV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    if !plugins.contains_key(owner_plugin) || !plugins.contains_key(provider_plugin) {
        return Err(noncanonical("plugins.cross_dependency"));
    }
    Ok(())
}

fn validate_resource_cycles(
    resources: &HashMap<&str, &RecordedResourceV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    let adjacency = resources
        .values()
        .map(|resource| {
            (
                resource.key.as_str(),
                resource
                    .dependencies
                    .resources
                    .iter()
                    .map(|dependency| dependency.key.as_str())
                    .chain(resource.dependencies.slots.iter().filter_map(|slot| {
                        if let RecordedSlotV1::Resource { contract_key, .. } = slot {
                            Some(contract_key.as_str())
                        } else {
                            None
                        }
                    }))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    if graph_has_cycle(resources.keys().copied(), &adjacency) {
        return Err(noncanonical("resources.cycle"));
    }
    Ok(())
}

fn validate_bindings(
    bindings: &[RecordedBindingV1],
    nodes: &HashMap<&str, &RecordedNodeV1>,
    triggers: &HashMap<&str, &RecordedTriggerV1>,
    actions: &HashMap<&str, &RecordedActionV1>,
    resources: &HashMap<&str, &RecordedResourceV1>,
    credentials: &HashMap<&str, &RecordedCredentialV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    for binding in bindings {
        if binding.slot_key.trim().is_empty()
            || binding.selector.trim().is_empty()
            || binding.selector.trim() != binding.selector
        {
            return Err(noncanonical("bindings"));
        }
        let _validated = PlanBindingRequirement::try_from(binding)?;
        let action = match &binding.site {
            RecordedBindingSiteV1::Node(node) => nodes
                .get(node.as_str())
                .and_then(|node| actions.get(node.action_key.as_str())),
            RecordedBindingSiteV1::Trigger(trigger) => triggers
                .get(trigger.as_str())
                .and_then(|trigger| actions.get(trigger.action_key.as_str())),
        }
        .ok_or_else(|| noncanonical("bindings.site"))?;
        let slot = action
            .dependencies
            .slots
            .iter()
            .find(|slot| slot_name(slot) == binding.slot_key)
            .ok_or_else(|| noncanonical("bindings.slot"))?;
        validate_binding_contract(binding, slot, resources, credentials)?;
    }

    if bindings.windows(2).any(|pair| {
        binding_sort_key(&pair[0]) >= binding_sort_key(&pair[1])
            || binding_site_slot_key(&pair[0]) == binding_site_slot_key(&pair[1])
    }) {
        return Err(noncanonical("bindings"));
    }

    let declared_binding_count = nodes
        .values()
        .map(|node| {
            actions
                .get(node.action_key.as_str())
                .map_or(0, |action| action.dependencies.slots.len())
        })
        .chain(triggers.values().map(|trigger| {
            actions
                .get(trigger.action_key.as_str())
                .map_or(0, |action| action.dependencies.slots.len())
        }))
        .sum::<usize>();
    if declared_binding_count != bindings.len() {
        return Err(noncanonical("bindings.missing"));
    }
    Ok(())
}

fn validate_binding_contract(
    binding: &RecordedBindingV1,
    slot: &RecordedSlotV1,
    resources: &HashMap<&str, &RecordedResourceV1>,
    credentials: &HashMap<&str, &RecordedCredentialV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
    match (&binding.contract, slot) {
        (
            RecordedBindingContractV1::Resource { key, version },
            RecordedSlotV1::Resource {
                contract_key,
                required,
                lazy,
                ..
            },
        ) => {
            let resource = resources
                .get(key.as_str())
                .ok_or_else(|| noncanonical("bindings.contract"))?;
            if key != contract_key
                || &resource.version != version
                || binding.required != *required
                || binding.lazy != *lazy
            {
                return Err(noncanonical("bindings.contract"));
            }
        },
        (
            RecordedBindingContractV1::Credential {
                key,
                version,
                required_capability_bits,
            },
            RecordedSlotV1::Credential {
                contract_key,
                required,
                lazy,
                ..
            },
        ) => {
            let credential = credentials
                .get(key.as_str())
                .ok_or_else(|| noncanonical("bindings.contract"))?;
            if key != contract_key
                || &credential.version != version
                || credential.capability_bits != *required_capability_bits
                || binding.required != *required
                || binding.lazy != *lazy
            {
                return Err(noncanonical("bindings.contract"));
            }
        },
        _ => return Err(noncanonical("bindings.kind")),
    }
    Ok(())
}

fn binding_sort_key(binding: &RecordedBindingV1) -> (u8, &str, &str, u8) {
    let (site_tag, site_id) = match &binding.site {
        RecordedBindingSiteV1::Node(node) => (0, node.as_str()),
        RecordedBindingSiteV1::Trigger(trigger) => (1, trigger.as_str()),
    };
    let contract_tag = match binding.contract {
        RecordedBindingContractV1::Resource { .. } => 0,
        RecordedBindingContractV1::Credential { .. } => 1,
    };
    (site_tag, site_id, binding.slot_key.as_str(), contract_tag)
}

fn binding_site_slot_key(binding: &RecordedBindingV1) -> (u8, &str, &str) {
    let (site_tag, site_id) = match &binding.site {
        RecordedBindingSiteV1::Node(node) => (0, node.as_str()),
        RecordedBindingSiteV1::Trigger(trigger) => (1, trigger.as_str()),
    };
    (site_tag, site_id, binding.slot_key.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::{
        ExecutablePlanRevisionId, PluginSetId, WorkerFlavorRevisionId, WorkflowId,
        WorkflowVersionId,
    };
    use nebula_schema::{ModeField, ObjectField, Schema, SecretField, field_key};
    use serde_json::{Value, json};

    const SECRET_PAYLOAD: &str = "credential-value-that-must-not-leak";
    const ACTUAL_CONTRACT_DETAIL: &str = "registered-contract-v2";

    fn diagnostic(
        code: &str,
        path: &str,
        expected: &str,
        actual: &str,
        remediation: &str,
    ) -> ActivationDiagnostic {
        ActivationDiagnostic::new(code, path, expected, actual, remediation)
            .expect("the fixture uses non-empty diagnostic fields")
    }

    #[test]
    fn activation_diagnostics_reject_empty_fields_sort_dedupe_and_redact() {
        assert!(ActivationDiagnostic::new("", "graph.node", "expected", "actual", "fix").is_none());
        assert!(ActivationDiagnostic::new("E002", "graph.node", "expected", "", "fix").is_none());

        let later = diagnostic(
            "E002",
            "graph.node[2]",
            "registered action",
            ACTUAL_CONTRACT_DETAIL,
            "install the action",
        );
        let earlier = diagnostic(
            "E001",
            "graph.node[1]",
            "compatible schema",
            ACTUAL_CONTRACT_DETAIL,
            "update the parameter",
        );
        let error = PlanCompilationError::new(vec![later, earlier.clone(), earlier.clone()])
            .expect("the fixture has diagnostics");

        assert_eq!(
            error.diagnostics(),
            &[
                earlier.clone(),
                diagnostic(
                    "E002",
                    "graph.node[2]",
                    "registered action",
                    ACTUAL_CONTRACT_DETAIL,
                    "install the action",
                )
            ]
        );
        assert!(!format!("{error}").contains(ACTUAL_CONTRACT_DETAIL));
        assert!(!format!("{error:?}").contains(ACTUAL_CONTRACT_DETAIL));
        assert!(!format!("{earlier}").contains(ACTUAL_CONTRACT_DETAIL));
        assert!(!format!("{earlier:?}").contains(ACTUAL_CONTRACT_DETAIL));
        assert!(PlanCompilationError::new(Vec::new()).is_none());
    }

    /// Author-supplied text reaches `path` and `actual`, so an unbounded
    /// workflow value would push its whole length into every log line and error
    /// body that names the diagnostic.
    #[test]
    fn author_supplied_fields_are_bounded_and_visibly_truncated() {
        let overlong = "n".repeat(MAX_DIAGNOSTIC_FIELD_BYTES * 4);
        let bounded = diagnostic(
            "E010",
            &overlong,
            "a contract",
            &overlong,
            "shorten the key",
        );

        for field in [bounded.path(), bounded.actual()] {
            assert!(
                field.len() <= MAX_DIAGNOSTIC_FIELD_BYTES + TRUNCATION_MARKER.len(),
                "a diagnostic field must not carry an unbounded workflow value"
            );
            assert!(
                field.ends_with(TRUNCATION_MARKER),
                "a truncated value must be visibly truncated, so a consumer cannot \
                 mistake a prefix for the whole contract"
            );
        }
        assert_eq!(
            bounded.code(),
            "E010",
            "a short field is left exactly as-is"
        );
        assert_eq!(bounded.expected(), "a contract");
    }

    /// The bound splits on a `char` boundary: these fields carry author-supplied
    /// text, and slicing a `String` mid-codepoint panics.
    #[test]
    fn truncation_splits_on_a_char_boundary() {
        // Three bytes per char, so the bound lands mid-codepoint.
        let multibyte = "日".repeat(MAX_DIAGNOSTIC_FIELD_BYTES);
        let bounded = diagnostic("E011", &multibyte, "a contract", &multibyte, "shorten it");

        assert!(bounded.path().ends_with(TRUNCATION_MARKER));
        assert!(
            bounded.path().len() <= MAX_DIAGNOSTIC_FIELD_BYTES + TRUNCATION_MARKER.len(),
            "walking back to a boundary must not push the value over the bound"
        );
        assert!(
            bounded
                .path()
                .trim_end_matches(TRUNCATION_MARKER)
                .chars()
                .all(|character| character == '日'),
            "truncation must not produce a partial codepoint"
        );
    }

    /// A code is a stable machine-readable contract, so it is rejected rather
    /// than cut: a truncated code would silently name a different diagnostic.
    #[test]
    fn an_overlong_code_is_rejected_rather_than_truncated() {
        let overlong_code = "E".repeat(MAX_DIAGNOSTIC_FIELD_BYTES + 1);
        assert!(
            ActivationDiagnostic::new(&overlong_code, "/workflow", "a", "b", "fix").is_none(),
            "a code that does not fit its own contract is a construction bug"
        );
    }

    fn recorded_semver(major: u64, minor: u64, patch: u64) -> RecordedSemverV1 {
        RecordedSemverV1 {
            major,
            minor,
            patch,
            pre: String::new(),
            build: String::new(),
        }
    }

    fn recorded_schema(schema: ValidSchema) -> RecordedSchemaV1 {
        RecordedSchemaV1 {
            schema_wire_version: SCHEMA_WIRE_VERSION_GRAPH_V1,
            schema,
        }
    }

    fn empty_dependencies() -> RecordedDependenciesV1 {
        RecordedDependenciesV1 {
            credentials: Vec::new().into_boxed_slice(),
            resources: Vec::new().into_boxed_slice(),
            slots: Vec::new().into_boxed_slice(),
        }
    }

    fn minimal_action(dependencies: RecordedDependenciesV1) -> RecordedActionV1 {
        RecordedActionV1 {
            key: "demo.echo".into(),
            plugin_key: "demo".into(),
            version: recorded_semver(1, 0, 0),
            kind: RecordedActionKindV1::Stateless,
            isolation: RecordedIsolationV1::None,
            checkpoint_policy: RecordedCheckpointPolicyV1::Inherit,
            max_concurrent: None,
            inputs: vec![RecordedInputPortV1::Flow { key: "in".into() }].into_boxed_slice(),
            outputs: vec![RecordedOutputPortV1::Flow {
                key: "out".into(),
                flow_kind: RecordedFlowKindV1::Main,
            }]
            .into_boxed_slice(),
            input_schema: recorded_schema(ValidSchema::empty()),
            output_schema: recorded_schema(ValidSchema::empty()),
            dependencies,
        }
    }

    fn minimal_node(id: &str) -> RecordedNodeV1 {
        RecordedNodeV1 {
            id: id.into(),
            plugin_key: "demo".into(),
            action_key: "demo.echo".into(),
            action_version: recorded_semver(1, 0, 0),
            parameters: Vec::new().into_boxed_slice(),
            retry_policy: None,
            timeout: None,
            rate_limit: None,
            enabled: true,
        }
    }

    fn workflow_config() -> RecordedWorkflowConfigV1 {
        RecordedWorkflowConfigV1 {
            timeout: None,
            max_parallel_nodes: 1,
            checkpointing: RecordedCheckpointingV1 {
                enabled: true,
                interval: None,
            },
            retry_policy: None,
            error_strategy: RecordedErrorStrategyV1::FailFast,
        }
    }

    fn resource_binding(slot_key: &str, selector: &str) -> RecordedBindingV1 {
        RecordedBindingV1 {
            site: RecordedBindingSiteV1::Node("fetch".into()),
            slot_key: slot_key.into(),
            selector: selector.into(),
            contract: RecordedBindingContractV1::Resource {
                key: "demo.client".into(),
                version: recorded_semver(1, 0, 0),
            },
            required: true,
            lazy: false,
        }
    }

    fn credential_binding(
        slot_key: &str,
        selector: &str,
        capability_bits: u8,
    ) -> RecordedBindingV1 {
        RecordedBindingV1 {
            site: RecordedBindingSiteV1::Node("fetch".into()),
            slot_key: slot_key.into(),
            selector: selector.into(),
            contract: RecordedBindingContractV1::Credential {
                key: "demo.oauth".into(),
                version: recorded_semver(2, 1, 0),
                required_capability_bits: capability_bits,
            },
            required: true,
            lazy: true,
        }
    }

    fn reseal(record: &mut RecordedExecutablePlanRevisionV1) {
        record.claimed_id = record
            .recomputed_id()
            .expect("the fixture is canonical and hashable");
    }

    fn fixture_record() -> RecordedExecutablePlanRevisionV1 {
        let mut record = RecordedExecutablePlanRevisionV1 {
            record_version: RECORD_VERSION_V1,
            compiler_version: COMPILER_VERSION_GRAPH_V1,
            canonical_hash_version: CANONICAL_HASH_VERSION_V1,
            profile: RecordedPlanProfileV1::GraphV1,
            claimed_id: ExecutablePlanRevisionId::from_bytes([0; 32]),
            workflow_version_id: WorkflowVersionId::from_bytes([1; 16]),
            plugin_set_id: PluginSetId::from_bytes([2; 32]),
            worker_flavor_revision_id: WorkerFlavorRevisionId::from_bytes([3; 32]),
            manifest: RecordedPlanManifestV1 {
                workflow_definition_schema_version: nebula_workflow::CURRENT_SCHEMA_VERSION,
                workflow_id: WorkflowId::from_bytes([4; 16]),
                workflow_semantic_version: RecordedWorkflowVersionV1 {
                    major: 1,
                    minor: 2,
                    patch: 3,
                    pre: None,
                    build: None,
                },
            },
            content: RecordedGraphContentV1 {
                plugins: vec![RecordedPluginV1 {
                    key: "demo".into(),
                    version: recorded_semver(1, 0, 0),
                }]
                .into_boxed_slice(),
                nodes: vec![minimal_node("fetch")].into_boxed_slice(),
                connections: Vec::new().into_boxed_slice(),
                actions: vec![minimal_action(empty_dependencies())].into_boxed_slice(),
                resources: Vec::new().into_boxed_slice(),
                credentials: Vec::new().into_boxed_slice(),
                triggers: Vec::new().into_boxed_slice(),
                variables: Vec::new().into_boxed_slice(),
                workflow_config: workflow_config(),
                converters: Vec::new().into_boxed_slice(),
            },
            bindings: Vec::new().into_boxed_slice(),
        };
        reseal(&mut record);
        record
    }

    fn resource_binding_record(selector: &str) -> RecordedExecutablePlanRevisionV1 {
        let mut record = fixture_record();
        record.content.resources = vec![RecordedResourceV1 {
            key: "demo.client".into(),
            plugin_key: "demo".into(),
            version: recorded_semver(1, 0, 0),
            configuration_schema: recorded_schema(ValidSchema::empty()),
            dependencies: empty_dependencies(),
        }]
        .into_boxed_slice();
        record.content.actions[0].dependencies.slots = vec![RecordedSlotV1::Resource {
            slot_key: "client".into(),
            default_selector: "primary".into(),
            contract_key: "demo.client".into(),
            required: true,
            lazy: false,
        }]
        .into_boxed_slice();
        record.bindings = vec![resource_binding("client", selector)].into_boxed_slice();
        reseal(&mut record);
        record
    }

    fn credential_binding_record(
        selector: &str,
        required_capability_bits: u8,
    ) -> RecordedExecutablePlanRevisionV1 {
        let mut record = fixture_record();
        record.content.credentials = vec![RecordedCredentialV1 {
            key: "demo.oauth".into(),
            plugin_key: "demo".into(),
            version: recorded_semver(2, 1, 0),
            pattern: RecordedAuthPatternV1::OAuth2,
            properties_schema: recorded_schema(ValidSchema::empty()),
            capability_bits: Capabilities::REFRESHABLE.bits(),
        }]
        .into_boxed_slice();
        record.content.actions[0].dependencies.slots = vec![RecordedSlotV1::Credential {
            slot_key: "auth".into(),
            default_selector: "primary".into(),
            contract_key: "demo.oauth".into(),
            required: true,
            lazy: true,
        }]
        .into_boxed_slice();
        record.bindings = vec![credential_binding(
            "auth",
            selector,
            required_capability_bits,
        )]
        .into_boxed_slice();
        reseal(&mut record);
        record
    }

    fn object_with_secret_default(default: Value) -> ValidSchema {
        Schema::builder()
            .add(
                ObjectField::new(field_key!("auth"))
                    .add(SecretField::new(field_key!("token")))
                    .default(default),
            )
            .build()
            .expect("the object default is accepted by the general schema contract")
    }

    fn mode_with_secret_default(default: Value) -> ValidSchema {
        Schema::builder()
            .add(
                ModeField::new(field_key!("auth"))
                    .variant("token", "Token", SecretField::new(field_key!("token")))
                    .default(default),
            )
            .build()
            .expect("the mode default is accepted by the general schema contract")
    }

    #[test]
    fn checked_record_rejects_forged_id_and_unknown_fields() {
        let record = fixture_record();
        let mut forged = record.clone();
        forged.claimed_id = ExecutablePlanRevisionId::from_bytes([9; 32]);

        assert!(matches!(
            ExecutablePlanRevision::try_from_recorded_v1(forged),
            Err(ExecutablePlanIntegrityError::RevisionIdMismatch { .. })
        ));

        let mut encoded = serde_json::to_value(record).expect("the record serializes");
        encoded
            .as_object_mut()
            .expect("a record is a JSON object")
            .insert("future_field".into(), json!(true));
        let error = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(encoded)
            .expect_err("unknown top-level record fields must fail closed");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn minimal_typed_record_is_integrity_valid() {
        let record = fixture_record();
        let plan = ExecutablePlanRevision::try_from(record)
            .expect("the fixture is a fully closed Graph-v1 record");
        assert!(plan.bindings().is_empty());
    }

    #[test]
    fn graph_v1_hash_matches_literal_golden_and_independent_record_projection() {
        let record = fixture_record();
        assert_eq!(
            record.claimed_id.to_string(),
            "f1e5fa3021749835b3bea5848df1d517d405e5a26b95d5ec597f872fd1ae8f79"
        );

        let mut projected = serde_json::to_value(&record).expect("record serializes");
        projected
            .as_object_mut()
            .expect("record is an object")
            .remove("claimed_id");
        let canonical = FieldValue::Literal(projected)
            .canonical_bytes()
            .expect("record projection is canonical");
        let mut independent = Sha256::new();
        independent.update([1]);
        independent.update((EXECUTABLE_PLAN_GRAPH_V1_DOMAIN.len() as u64).to_be_bytes());
        independent.update(EXECUTABLE_PLAN_GRAPH_V1_DOMAIN);
        independent.update([2]);
        independent.update((canonical.len() as u64).to_be_bytes());
        independent.update(canonical);
        let digest: [u8; 32] = independent.finalize().into();
        assert_eq!(
            record.claimed_id,
            ExecutablePlanRevisionId::from_bytes(digest)
        );
    }

    #[test]
    fn json_object_order_is_invariant_but_array_order_and_included_fields_are_not() {
        let mut object_a_then_b = fixture_record();
        object_a_then_b.content.variables = vec![RecordedVariableV1 {
            name: "value".into(),
            value: serde_json::from_str(r#"{"a":1,"b":2}"#).expect("fixture JSON is valid"),
        }]
        .into_boxed_slice();
        reseal(&mut object_a_then_b);

        let mut object_b_then_a = fixture_record();
        object_b_then_a.content.variables = vec![RecordedVariableV1 {
            name: "value".into(),
            value: serde_json::from_str(r#"{"b":2,"a":1}"#).expect("fixture JSON is valid"),
        }]
        .into_boxed_slice();
        reseal(&mut object_b_then_a);
        assert_eq!(object_a_then_b.claimed_id, object_b_then_a.claimed_id);

        let mut array_a_then_b = fixture_record();
        array_a_then_b.content.variables = vec![RecordedVariableV1 {
            name: "value".into(),
            value: json!(["a", "b"]),
        }]
        .into_boxed_slice();
        reseal(&mut array_a_then_b);
        let mut array_b_then_a = fixture_record();
        array_b_then_a.content.variables = vec![RecordedVariableV1 {
            name: "value".into(),
            value: json!(["b", "a"]),
        }]
        .into_boxed_slice();
        reseal(&mut array_b_then_a);
        assert_ne!(array_a_then_b.claimed_id, array_b_then_a.claimed_id);

        let mut changed_schema_version = fixture_record();
        changed_schema_version
            .manifest
            .workflow_definition_schema_version += 1;
        reseal(&mut changed_schema_version);
        assert_ne!(
            fixture_record().claimed_id,
            changed_schema_version.claimed_id
        );
    }

    #[test]
    fn unsupported_versions_and_profile_fail_closed() {
        for mutate in [
            |record: &mut RecordedExecutablePlanRevisionV1| record.record_version += 1,
            |record: &mut RecordedExecutablePlanRevisionV1| record.compiler_version += 1,
            |record: &mut RecordedExecutablePlanRevisionV1| record.canonical_hash_version += 1,
        ] {
            let mut record = fixture_record();
            mutate(&mut record);
            assert!(matches!(
                ExecutablePlanRevision::try_from(record),
                Err(ExecutablePlanIntegrityError::UnsupportedFormat)
            ));
        }

        let mut encoded = serde_json::to_value(fixture_record()).expect("record serializes");
        encoded
            .as_object_mut()
            .expect("record is an object")
            .insert("profile".into(), json!("future-profile"));
        assert!(
            serde_json::from_value::<RecordedExecutablePlanRevisionV1>(encoded).is_err(),
            "an unknown execution profile must fail during record decoding"
        );
    }

    #[test]
    fn collection_and_converter_canonicality_fail_closed() {
        let mut unsorted = fixture_record();
        unsorted.content.nodes = vec![minimal_node("b"), minimal_node("a")].into_boxed_slice();
        assert!(matches!(
            ExecutablePlanRevision::try_from(unsorted),
            Err(ExecutablePlanIntegrityError::NonCanonical { section: "nodes" })
        ));

        let mut duplicate = fixture_record();
        duplicate.content.nodes = vec![minimal_node("a"), minimal_node("a")].into_boxed_slice();
        assert!(matches!(
            ExecutablePlanRevision::try_from(duplicate),
            Err(ExecutablePlanIntegrityError::NonCanonical { section: "nodes" })
        ));

        let mut converter = fixture_record();
        converter.content.converters = vec![RecordedConverterV1 {
            key: "implicit".into(),
        }]
        .into_boxed_slice();
        assert!(matches!(
            ExecutablePlanRevision::try_from(converter),
            Err(ExecutablePlanIntegrityError::ConvertersUnsupported)
        ));
    }

    #[test]
    fn explicit_literals_are_not_reclassified_as_expressions() {
        let mut literal = fixture_record();
        literal.content.actions[0].input_schema = recorded_schema(
            Schema::builder()
                .add(Field::string(field_key!("value")).no_expression())
                .build()
                .expect("fixture schema is valid"),
        );
        literal.content.nodes[0].parameters = vec![RecordedParameterV1 {
            key: "value".into(),
            value: RecordedParameterValueV1::Literal {
                value: json!("{{ $workflow.input }}"),
            },
        }]
        .into_boxed_slice();
        reseal(&mut literal);
        ExecutablePlanRevision::try_from(literal.clone())
            .expect("the tagged literal must remain a literal");

        literal.content.nodes[0].parameters[0].value = RecordedParameterValueV1::Expression {
            expression: "{{ $workflow.input }}".into(),
        };
        reseal(&mut literal);
        assert!(matches!(
            ExecutablePlanRevision::try_from(literal),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.schema"
            })
        ));
    }

    #[test]
    fn parameter_validation_rejects_invalid_nested_values_and_secret_literals() {
        let mut nested = fixture_record();
        nested.content.actions[0].input_schema = recorded_schema(
            Schema::builder()
                .add(
                    ObjectField::new(field_key!("config"))
                        .add(Field::string(field_key!("name")).required()),
                )
                .build()
                .expect("fixture schema is valid"),
        );
        nested.content.nodes[0].parameters = vec![RecordedParameterV1 {
            key: "config".into(),
            value: RecordedParameterValueV1::Literal { value: json!({}) },
        }]
        .into_boxed_slice();
        reseal(&mut nested);
        assert!(matches!(
            ExecutablePlanRevision::try_from(nested),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.schema"
            })
        ));

        let mut secret = fixture_record();
        secret.content.actions[0].input_schema = recorded_schema(
            Schema::builder()
                .add(
                    ObjectField::new(field_key!("auth")).add(SecretField::new(field_key!("token"))),
                )
                .build()
                .expect("fixture schema is valid"),
        );
        secret.content.nodes[0].parameters = vec![RecordedParameterV1 {
            key: "auth".into(),
            value: RecordedParameterValueV1::Literal {
                value: json!({"token": SECRET_PAYLOAD}),
            },
        }]
        .into_boxed_slice();
        reseal(&mut secret);
        let error = ExecutablePlanRevision::try_from(secret)
            .expect_err("an executable plan cannot persist credential material");
        assert!(matches!(
            error,
            ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.secret"
            }
        ));
        assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
    }

    #[test]
    fn node_parameter_set_cannot_bypass_required_fields_or_root_rules() {
        let mut missing = fixture_record();
        missing.content.actions[0].input_schema = recorded_schema(
            Schema::builder()
                .add(Field::string(field_key!("name")).required())
                .build()
                .expect("fixture schema is valid"),
        );
        reseal(&mut missing);
        assert!(matches!(
            ExecutablePlanRevision::try_from(missing),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.required"
            })
        ));

        let mut root_rules = fixture_record();
        root_rules.content.actions[0].input_schema = recorded_schema(
            Schema::builder()
                .add(Field::string(field_key!("name")))
                .root_rule(nebula_schema::Rule::predicate(
                    nebula_schema::Predicate::eq("name", json!("expected"))
                        .expect("fixture predicate is valid"),
                ))
                .build()
                .expect("fixture schema is valid"),
        );
        reseal(&mut root_rules);
        assert!(matches!(
            ExecutablePlanRevision::try_from(root_rules),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.root_rules"
            })
        ));
    }

    #[test]
    fn reference_paths_have_one_canonical_spelling() {
        for path in ["", "value", "items.0.name", "184467440737095516160"] {
            assert!(is_canonical_reference_path(path), "{path:?} is canonical");
        }
        for alias in [
            "$",
            "$.value",
            ".value",
            "value.",
            "value..name",
            "items.00",
        ] {
            assert!(
                !is_canonical_reference_path(alias),
                "{alias:?} must be normalized before persistence"
            );
        }
    }

    #[test]
    fn graph_cycle_check_is_iterative_for_deep_dags() {
        let nodes = (0..10_000)
            .map(|index| index.to_string())
            .collect::<Vec<_>>();
        let mut adjacency = HashMap::with_capacity(nodes.len());
        for pair in nodes.windows(2) {
            adjacency.insert(pair[0].as_str(), vec![pair[1].as_str()]);
        }
        assert!(!graph_has_cycle(
            nodes.iter().map(String::as_str),
            &adjacency
        ));
        adjacency
            .entry(nodes.last().expect("the fixture is not empty").as_str())
            .or_default()
            .push(nodes[0].as_str());
        assert!(graph_has_cycle(
            nodes.iter().map(String::as_str),
            &adjacency
        ));
    }

    #[test]
    fn only_replayable_connection_contracts_are_certified() {
        let mut dynamic_declaration = fixture_record();
        dynamic_declaration.content.actions[0].outputs = vec![
            RecordedOutputPortV1::Dynamic {
                key: "branch".into(),
                source_field: "route".into(),
                label_field: None,
                include_fallback: true,
            },
            RecordedOutputPortV1::Flow {
                key: "out".into(),
                flow_kind: RecordedFlowKindV1::Main,
            },
        ]
        .into_boxed_slice();
        reseal(&mut dynamic_declaration);
        ExecutablePlanRevision::try_from(dynamic_declaration.clone())
            .expect("an unused dynamic declaration does not claim routing semantics");

        dynamic_declaration.content.nodes =
            vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
        dynamic_declaration.content.connections = vec![RecordedConnectionV1 {
            from_node: "source".into(),
            from_port: "branch".into(),
            to_node: "target".into(),
            to_port: None,
        }]
        .into_boxed_slice();
        reseal(&mut dynamic_declaration);
        assert!(matches!(
            ExecutablePlanRevision::try_from(dynamic_declaration),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "connections.from_port"
            })
        ));

        let mut error_flow = fixture_record();
        error_flow.content.actions[0].outputs[0] = RecordedOutputPortV1::Flow {
            key: "out".into(),
            flow_kind: RecordedFlowKindV1::Error,
        };
        error_flow.content.nodes =
            vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
        error_flow.content.connections = vec![RecordedConnectionV1 {
            from_node: "source".into(),
            from_port: "out".into(),
            to_node: "target".into(),
            to_port: None,
        }]
        .into_boxed_slice();
        reseal(&mut error_flow);
        assert!(matches!(
            ExecutablePlanRevision::try_from(error_flow),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "connections.from_port"
            })
        ));
    }

    #[test]
    fn tag_filtered_support_ports_are_recordable_but_not_certifiable_edges() {
        let mut record = fixture_record();
        record.content.actions[0].inputs = vec![
            RecordedInputPortV1::Flow { key: "in".into() },
            RecordedInputPortV1::Support {
                key: "model".into(),
                required: false,
                multi: false,
                allowed_node_types: None,
                allowed_tags: Some(vec!["llm".into()].into_boxed_slice()),
            },
        ]
        .into_boxed_slice();
        reseal(&mut record);
        ExecutablePlanRevision::try_from(record.clone())
            .expect("unused tag-filter declarations remain an exact contract fact");

        record.content.nodes =
            vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
        record.content.connections = vec![RecordedConnectionV1 {
            from_node: "source".into(),
            from_port: "out".into(),
            to_node: "target".into(),
            to_port: Some("model".into()),
        }]
        .into_boxed_slice();
        reseal(&mut record);
        assert!(matches!(
            ExecutablePlanRevision::try_from(record),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "connections.to_port.tag_filter"
            })
        ));
    }

    #[test]
    fn binding_integrity_rejects_unknown_bits_and_duplicate_site_slot() {
        let unknown_bits = credential_binding_record("primary", 0b1000_0000);
        assert!(matches!(
            ExecutablePlanRevision::try_from(unknown_bits),
            Err(ExecutablePlanIntegrityError::UnknownCredentialCapability)
        ));

        let mut duplicate_site_slot = resource_binding_record("primary");
        duplicate_site_slot.bindings = vec![
            resource_binding("client", "primary"),
            resource_binding("client", "secondary"),
        ]
        .into_boxed_slice();
        assert!(matches!(
            ExecutablePlanRevision::try_from(duplicate_site_slot),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "bindings"
            })
        ));
    }

    #[test]
    fn typed_schema_rejects_unknown_fields_and_any_secret_bearing_default() {
        let mut unknown_wire = serde_json::to_value(fixture_record()).expect("record serializes");
        let schema = unknown_wire
            .pointer_mut("/content/actions/0/input_schema/schema")
            .expect("fixture action input schema exists");
        schema.as_object_mut().expect("schema is an object").insert(
            "fields".into(),
            json!([{
                "type": "future_secret",
                "key": "future",
                "payload": SECRET_PAYLOAD
            }]),
        );
        let mut unknown = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(unknown_wire)
            .expect("unknown schema field kinds are preserved by the schema wire");
        reseal(&mut unknown);
        let error = ExecutablePlanRevision::try_from(unknown)
            .expect_err("Graph-v1 must reject opaque future schema fields");
        assert!(matches!(
            error,
            ExecutablePlanIntegrityError::NonCanonical {
                section: "actions.input_schema"
            }
        ));
        assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));

        for secret_schema in [
            object_with_secret_default(json!({"token": SECRET_PAYLOAD})),
            object_with_secret_default(Value::Null),
            mode_with_secret_default(json!({"mode": "token", "value": SECRET_PAYLOAD})),
            mode_with_secret_default(Value::Null),
        ] {
            let mut record = fixture_record();
            record.content.actions[0].input_schema = recorded_schema(secret_schema);
            reseal(&mut record);
            assert!(matches!(
                ExecutablePlanRevision::try_from(record),
                Err(ExecutablePlanIntegrityError::NonCanonical {
                    section: "actions.input_schema"
                })
            ));
        }
    }

    #[test]
    fn malformed_schema_decode_is_secret_free_and_fail_closed() {
        let mut encoded = serde_json::to_value(fixture_record()).expect("record serializes");
        let schema = encoded
            .pointer_mut("/content/actions/0/input_schema/schema")
            .expect("fixture action input schema exists");
        *schema = json!({
            "fields": [{
                "type": "string",
                "key": format!("{SECRET_PAYLOAD} invalid")
            }]
        });

        let error = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(encoded)
            .expect_err("a malformed typed schema must fail during decoding");
        assert!(error.to_string().contains("invalid Graph-v1 schema wire"));
        assert!(!error.to_string().contains(SECRET_PAYLOAD));
        assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
    }

    #[test]
    fn exact_component_build_metadata_is_kept_but_plugin_build_metadata_is_rejected() {
        let mut component = fixture_record();
        component.content.actions[0].version.build = "linux.1".into();
        component.content.nodes[0].action_version.build = "linux.1".into();
        reseal(&mut component);
        ExecutablePlanRevision::try_from(component)
            .expect("exact component versions retain valid build metadata");

        let mut plugin = fixture_record();
        plugin.content.plugins[0].version.build = "linux.1".into();
        reseal(&mut plugin);
        assert!(matches!(
            ExecutablePlanRevision::try_from(plugin),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "plugins.version"
            })
        ));
    }

    #[test]
    fn empty_reference_path_means_whole_output_and_requires_a_durable_edge() {
        let mut record = fixture_record();
        record.content.actions[0].input_schema = recorded_schema(
            Schema::builder()
                .add(
                    ObjectField::new(field_key!("payload")).add(Field::string(field_key!("value"))),
                )
                .build()
                .expect("fixture consumer schema is valid"),
        );
        record.content.actions[0].output_schema = recorded_schema(
            Schema::builder()
                .add(Field::string(field_key!("value")))
                .build()
                .expect("fixture producer schema is valid"),
        );
        record.content.actions[0].inputs = vec![
            RecordedInputPortV1::Flow { key: "in".into() },
            RecordedInputPortV1::Support {
                key: "support".into(),
                required: false,
                multi: false,
                allowed_node_types: None,
                allowed_tags: None,
            },
        ]
        .into_boxed_slice();
        record.content.nodes =
            vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
        record.content.nodes[1].parameters = vec![RecordedParameterV1 {
            key: "payload".into(),
            value: RecordedParameterValueV1::Reference {
                node_key: "source".into(),
                output_path: String::new(),
            },
        }]
        .into_boxed_slice();
        record.content.connections = vec![RecordedConnectionV1 {
            from_node: "source".into(),
            from_port: "out".into(),
            to_node: "target".into(),
            to_port: Some("support".into()),
        }]
        .into_boxed_slice();
        reseal(&mut record);
        ExecutablePlanRevision::try_from(record)
            .expect("an empty reference path selects the producer's whole output");
    }

    #[test]
    fn component_closure_and_binding_references_fail_closed() {
        let mut unused = fixture_record();
        unused.content.credentials = vec![RecordedCredentialV1 {
            key: "demo.oauth".into(),
            plugin_key: "demo".into(),
            version: recorded_semver(1, 0, 0),
            pattern: RecordedAuthPatternV1::OAuth2,
            properties_schema: recorded_schema(ValidSchema::empty()),
            capability_bits: Capabilities::REFRESHABLE.bits(),
        }]
        .into_boxed_slice();
        reseal(&mut unused);
        assert!(matches!(
            ExecutablePlanRevision::try_from(unused),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "components.unused"
            })
        ));

        let mut dangling = resource_binding_record("primary");
        dangling.bindings[0].site = RecordedBindingSiteV1::Node("missing".into());
        reseal(&mut dangling);
        assert!(matches!(
            ExecutablePlanRevision::try_from(dangling),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "bindings.site"
            })
        ));
    }

    #[test]
    fn nested_unknown_fields_fail_closed() {
        let record = resource_binding_record("primary");
        let mut encoded = serde_json::to_value(record).expect("record serializes");
        encoded
            .get_mut("bindings")
            .and_then(Value::as_array_mut)
            .and_then(|bindings| bindings.first_mut())
            .and_then(Value::as_object_mut)
            .expect("fixture binding is an object")
            .insert("future_field".into(), json!(true));
        let wire = serde_json::to_string(&encoded).expect("mutated record serializes");
        let error = serde_json::from_str::<RecordedExecutablePlanRevisionV1>(&wire)
            .expect_err("unknown nested record fields must fail closed");
        assert!(
            error.to_string().contains("unknown field"),
            "unexpected nested decode error: {error}"
        );
    }

    #[test]
    fn checked_plan_roundtrips_record_and_redacts_debug_surfaces() {
        let record = credential_binding_record(SECRET_PAYLOAD, Capabilities::REFRESHABLE.bits());
        assert!(!format!("{record:?}").contains(SECRET_PAYLOAD));

        let wire_value = serde_json::to_value(&record).expect("record serializes");
        let decoded = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(wire_value)
            .expect("record deserializes from an owned JSON value");
        let plan = ExecutablePlanRevision::try_from(decoded).expect("record is integrity-valid");
        assert_eq!(plan.bindings().len(), 1);
        assert_eq!(plan.bindings()[0].selector(), SECRET_PAYLOAD);
        assert!(!format!("{:?}", plan.bindings()[0]).contains(SECRET_PAYLOAD));
        assert!(!format!("{plan:?}").contains(SECRET_PAYLOAD));

        let projected = RecordedExecutablePlanRevisionV1::from(&plan);
        let reloaded =
            ExecutablePlanRevision::try_from(projected).expect("roundtrip remains valid");
        assert_eq!(reloaded.id(), plan.id());
        assert_eq!(reloaded.workflow_version_id(), plan.workflow_version_id());
        assert_eq!(reloaded.plugin_set_id(), plan.plugin_set_id());
        assert_eq!(
            reloaded.worker_flavor_revision_id(),
            plan.worker_flavor_revision_id()
        );
    }
}
