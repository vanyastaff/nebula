//! Authority-free immutable executable-plan contracts.

use core::fmt;
use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;
use nebula_core::{
    ActionKey, CredentialKey, ExecutablePlanRevisionId, NodeKey, PluginKey, PluginSetId, PortKey,
    ResourceKey, WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId,
};
use nebula_credential::Capabilities;
use nebula_error::ActivationDiagnostic;
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
pub(crate) const COMPILER_VERSION_GRAPH_V3: u16 = 3;
pub(crate) const CANONICAL_HASH_VERSION_V1: u16 = 1;
pub(crate) const CANONICAL_HASH_VERSION_V2: u16 = 2;
pub(crate) const DEFAULT_OUTPUT_PORT: &str = "out";
pub(crate) const INTRINSIC_ERROR_PORT: &str = "error";
const EXECUTABLE_PLAN_GRAPH_V1_DOMAIN: &[u8] = b"nebula.executable-plan.graph.v1";
const EXECUTABLE_PLAN_GRAPH_V2_DOMAIN: &[u8] = b"nebula.executable-plan.graph.v2";
const VALUE_CANON_VERSION_GRAPH_V1: u16 = 1;
pub(crate) const SCHEMA_WIRE_VERSION_GRAPH_V1: u16 = 1;
const _: () = assert!(nebula_schema::VALUE_CANON_VERSION == VALUE_CANON_VERSION_GRAPH_V1);
const _: () = assert!(nebula_schema::SCHEMA_WIRE_VERSION == SCHEMA_WIRE_VERSION_GRAPH_V1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanEpoch {
    GraphV1,
    GraphV3,
}

impl PlanEpoch {
    pub(crate) const CURRENT: Self = Self::GraphV3;

    const fn from_record(compiler_version: u16, canonical_hash_version: u16) -> Option<Self> {
        match (compiler_version, canonical_hash_version) {
            (COMPILER_VERSION_GRAPH_V1, CANONICAL_HASH_VERSION_V1) => Some(Self::GraphV1),
            (COMPILER_VERSION_GRAPH_V3, CANONICAL_HASH_VERSION_V2) => Some(Self::GraphV3),
            _ => None,
        }
    }

    pub(crate) const fn compiler_version(self) -> u16 {
        match self {
            Self::GraphV1 => COMPILER_VERSION_GRAPH_V1,
            Self::GraphV3 => COMPILER_VERSION_GRAPH_V3,
        }
    }

    pub(crate) const fn canonical_hash_version(self) -> u16 {
        match self {
            Self::GraphV1 => CANONICAL_HASH_VERSION_V1,
            Self::GraphV3 => CANONICAL_HASH_VERSION_V2,
        }
    }

    const fn records_effect_contract(self) -> bool {
        matches!(self, Self::GraphV3)
    }

    const fn supports_intrinsic_error_port(self) -> bool {
        matches!(self, Self::GraphV3)
    }
}

/// A non-empty, canonically ordered set of plan-activation diagnostics.
#[derive(thiserror::Error)]
#[error("workflow plan compilation failed")]
pub struct PlanCompilationError {
    diagnostics: Box<[ActivationDiagnostic]>,
}

impl PlanCompilationError {
    pub(crate) fn new(diagnostics: Vec<ActivationDiagnostic>) -> Option<Self> {
        let diagnostics = nebula_error::canonical_diagnostics(diagnostics);
        (!diagnostics.is_empty()).then(|| Self {
            diagnostics: diagnostics.into_boxed_slice(),
        })
    }

    pub(crate) fn invalid_compiled_record() -> Self {
        let diagnostic = ActivationDiagnostic::new(
            "PLUGIN_PLAN_GRAPH_V1:INVALID_COMPILED_RECORD",
            "/workflow",
            "<canonical-graph-v1-plan>",
            "<unsupported-variant>",
            "repair the reported workflow or component contract",
        )
        .expect("the compiled-record diagnostic has five non-empty constant fields");
        Self {
            diagnostics: vec![diagnostic].into_boxed_slice(),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effect_contract: Option<crate::plan_effect::RecordedActionEffectV1>,
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
        let domain = match self.canonical_hash_version {
            CANONICAL_HASH_VERSION_V1 => EXECUTABLE_PLAN_GRAPH_V1_DOMAIN,
            CANONICAL_HASH_VERSION_V2 => EXECUTABLE_PLAN_GRAPH_V2_DOMAIN,
            _ => return Err(ExecutablePlanIntegrityError::UnsupportedFormat),
        };
        hash_field(&mut hasher, 1, domain);
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

impl nebula_error::ActivationDiagnostics for ExecutablePlanIntegrityError {
    fn activation_diagnostics(&self) -> Vec<ActivationDiagnostic> {
        let (code, path, expected, actual, remediation) = match self {
            Self::UnsupportedFormat => (
                "PLUGIN_PLAN_INTEGRITY:UNSUPPORTED_FORMAT",
                "/plan/record_format".to_owned(),
                "graph_v1_json".to_owned(),
                "<unsupported-format>".to_owned(),
                "recompile the workflow with a runtime that writes Graph-v1 plans",
            ),
            // The section is a stable path, never a payload value: a
            // non-canonical section can hold parameter defaults.
            Self::NonCanonical { section } => (
                "PLUGIN_PLAN_INTEGRITY:NON_CANONICAL",
                format!("/plan/{section}"),
                "a canonically ordered, duplicate-free section".to_owned(),
                "a section that is empty, unsorted, or duplicated".to_owned(),
                "recompile the workflow: the record was not produced by a canonical compiler",
            ),
            Self::ConvertersUnsupported => (
                "PLUGIN_PLAN_INTEGRITY:CONVERTERS_UNSUPPORTED",
                "/plan/converters".to_owned(),
                "an empty converter set".to_owned(),
                "a non-empty converter set".to_owned(),
                "recompile the workflow: Graph-v1 plans cannot carry converters",
            ),
            Self::UnknownCredentialCapability => (
                "PLUGIN_PLAN_INTEGRITY:UNKNOWN_CAPABILITY",
                "/plan/bindings/required_capability_bits".to_owned(),
                "capability bits this runtime defines".to_owned(),
                "<unknown-capability-bits>".to_owned(),
                "run a worker whose credential contract defines every recorded capability",
            ),
            Self::CanonicalEncoding => (
                "PLUGIN_PLAN_INTEGRITY:CANONICAL_ENCODING",
                "/plan".to_owned(),
                "a canonically encodable record".to_owned(),
                "<uncanonicalizable-value>".to_owned(),
                "recompile the workflow: a recorded value cannot be canonically encoded",
            ),
            // Both identities are content digests, so reporting them leaks
            // nothing about the plan they address.
            Self::RevisionIdMismatch { claimed, computed } => (
                "PLUGIN_PLAN_INTEGRITY:REVISION_ID_MISMATCH",
                "/plan/id".to_owned(),
                computed.to_string(),
                claimed.to_string(),
                "discard the record: its claimed identity does not match its own content",
            ),
        };

        vec![
            ActivationDiagnostic::new(code, &path, expected, actual, remediation).unwrap_or_else(
                || {
                    ActivationDiagnostic::new(
                        code,
                        "/plan",
                        "<contract>",
                        "<unavailable>",
                        "recompile the workflow against a canonical compiler",
                    )
                    .unwrap_or_else(|| {
                        unreachable!("the fallback diagnostic uses non-empty constants")
                    })
                },
            ),
        ]
    }
}

impl nebula_error::ActivationDiagnostics for PlanCompilationError {
    fn activation_diagnostics(&self) -> Vec<ActivationDiagnostic> {
        // Already canonical: `new` sorts and dedups on construction.
        self.diagnostics.to_vec()
    }
}

/// Integrity-checked, immutable, authority-free executable plan revision.
#[derive(Clone)]
pub struct ExecutablePlanRevision {
    record: RecordedExecutablePlanRevisionV1,
    bindings: Box<[PlanBindingRequirement]>,
}

/// Effect declaration lookup in an exact plan without conflating old data and bad keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanActionEffectContract {
    /// The plan records the complete checked declaration.
    Declared(nebula_action::effect::ActionEffectContract),
    /// The action exists in a readable legacy plan without an effect declaration.
    LegacyUndeclared,
    /// The requested action key is absent from the exact plan.
    UnknownAction,
}

impl ExecutablePlanRevision {
    /// Project recorded node execution semantics without a registry or compiler lookup.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ExecutionGraphProjectionError`] if a recorded value cannot
    /// be represented on this runtime platform. This grants no tenant authority.
    #[tracing::instrument(skip_all, fields(plan_revision_id = %self.id()), err)]
    pub fn execution_graph(
        &self,
    ) -> Result<crate::ExecutableGraph, crate::ExecutionGraphProjectionError> {
        crate::ExecutableGraph::project(self)
    }

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

    /// Workflow identity recorded by compilation into this exact plan.
    #[must_use]
    pub const fn workflow_id(&self) -> WorkflowId {
        self.record.manifest.workflow_id
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

    /// Read the checked static effect declaration for an exact action key.
    ///
    /// # Errors
    /// Returns a bounded integrity error if a recorded declaration is unsupported.
    pub fn action_effect_contract(
        &self,
        action_key: &ActionKey,
    ) -> Result<PlanActionEffectContract, ExecutablePlanIntegrityError> {
        let Some(action) = self
            .record
            .content
            .actions
            .iter()
            .find(|action| action.key == action_key.as_str())
        else {
            return Ok(PlanActionEffectContract::UnknownAction);
        };
        let Some(effect) = action.effect_contract.as_ref() else {
            return Ok(PlanActionEffectContract::LegacyUndeclared);
        };
        effect
            .checked_contract()
            .map(PlanActionEffectContract::Declared)
            .map_err(|_| noncanonical("actions.effects"))
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
        || record.profile != RecordedPlanProfileV1::GraphV1
    {
        return Err(ExecutablePlanIntegrityError::UnsupportedFormat);
    }
    let Some(epoch) =
        PlanEpoch::from_record(record.compiler_version, record.canonical_hash_version)
    else {
        return Err(ExecutablePlanIntegrityError::UnsupportedFormat);
    };
    for action in &record.content.actions {
        match (&action.effect_contract, epoch.records_effect_contract()) {
            (None, false) => {},
            (Some(contract), true) => {
                let declared = contract
                    .checked_contract()
                    .map_err(|_| noncanonical("actions.effects"))?;
                if matches!(
                    declared,
                    nebula_action::effect::ActionEffectContract::Remote(_)
                ) && action.kind != RecordedActionKindV1::Stateless
                {
                    return Err(noncanonical("actions.effects"));
                }
            },
            _ => return Err(noncanonical("actions.effects")),
        }
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
    validate_connections(&record.content.connections, &nodes, &actions, epoch)?;
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
    epoch: PlanEpoch,
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
        let intrinsic_error =
            epoch.supports_intrinsic_error_port() && connection.from_port == INTRINSIC_ERROR_PORT;
        if intrinsic_error && connection.to_port.is_some() {
            return Err(noncanonical("connections.to_port"));
        }
        if !intrinsic_error {
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
                } if key == DEFAULT_OUTPUT_PORT
            ) {
                return Err(noncanonical("connections.from_port"));
            }
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
            let producer = OutputSchema::new(if intrinsic_error {
                nebula_schema::schema_of::<nebula_workflow::ErrorPortPayload>()
            } else {
                source_action.output_schema.schema.clone()
            });
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
                validate_reference_contract(
                    parameter,
                    output_path,
                    source,
                    node,
                    actions,
                    connections,
                    epoch,
                )?;
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
    connections: &[RecordedConnectionV1],
    epoch: PlanEpoch,
) -> Result<(), ExecutablePlanIntegrityError> {
    let source_action = actions
        .get(source.action_key.as_str())
        .ok_or_else(|| noncanonical("nodes.parameters.reference"))?;
    let mut found = false;
    for edge in connections
        .iter()
        .filter(|edge| edge.from_node == source.id && edge.to_node == consumer.id)
    {
        found = true;
        let producer_schema =
            if epoch.supports_intrinsic_error_port() && edge.from_port == INTRINSIC_ERROR_PORT {
                nebula_schema::schema_of::<nebula_workflow::ErrorPortPayload>()
            } else {
                source_action.output_schema.schema.clone()
            };
        validate_reference_schema(parameter, output_path, &producer_schema, consumer, actions)?;
    }
    if !found {
        return Err(noncanonical("nodes.parameters.reference"));
    }
    Ok(())
}

fn validate_reference_schema(
    parameter: &RecordedParameterV1,
    output_path: &str,
    producer_schema: &ValidSchema,
    consumer: &RecordedNodeV1,
    actions: &HashMap<&str, &RecordedActionV1>,
) -> Result<(), ExecutablePlanIntegrityError> {
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
        let producer = OutputSchema::new(producer_schema.clone());
        let consumer = InputSchema::new(consumer_schema);
        if !matches!(explain_assignable(&producer, &consumer), Assignability::Yes) {
            return Err(noncanonical("nodes.parameters.reference.schema"));
        }
        return Ok(());
    }
    let producer_field = match producer_schema.walk_authored_path(output_path) {
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
#[path = "plan_record_contract_tests.rs"]
mod plan_record_contract_tests;
