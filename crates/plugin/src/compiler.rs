//! Pure, authority-free Graph-v1 compilation from one frozen plugin registry.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use nebula_action::{
    ActionKind, CheckpointPolicy, FlowKind, InputPort, IsolationLevel, OutputPort,
};
use nebula_core::{
    ActionKey, CredentialKey, Dependencies, ExecutablePlanRevisionId, PluginKey, ResourceKey,
    SlotKind, WorkflowVersionId,
};
use nebula_credential::AuthPattern;
use nebula_schema::{Assignability, InputSchema, OutputSchema, explain_assignable};
use nebula_workflow::{
    Connection, ErrorStrategy, NodeDefinition, ParamValue, RetryConfig, SlotBinding,
    TriggerBinding, WorkflowDefinition,
};
use semver::{BuildMetadata, Version};

use crate::plan::{
    ActivationDiagnostic, CANONICAL_HASH_VERSION_V1, COMPILER_VERSION_GRAPH_V1,
    ExecutablePlanRevision, PlanCompilationError, RECORD_VERSION_V1, RecordedActionKindV1,
    RecordedActionV1, RecordedAuthPatternV1, RecordedBindingContractV1, RecordedBindingSiteV1,
    RecordedBindingV1, RecordedCheckpointPolicyV1, RecordedCheckpointingV1, RecordedConnectionV1,
    RecordedConverterV1, RecordedCredentialV1, RecordedDependenciesV1, RecordedDependencyV1,
    RecordedDurationV1, RecordedErrorStrategyV1, RecordedExecutablePlanRevisionV1,
    RecordedFlowKindV1, RecordedGraphContentV1, RecordedInputPortV1, RecordedIsolationV1,
    RecordedNodeV1, RecordedOutputPortV1, RecordedParameterV1, RecordedParameterValueV1,
    RecordedPlanManifestV1, RecordedPlanProfileV1, RecordedPluginV1, RecordedRateLimitV1,
    RecordedResourceV1, RecordedRetryV1, RecordedSchemaV1, RecordedSemverV1, RecordedSlotV1,
    RecordedTriggerV1, RecordedVariableV1, RecordedWorkflowConfigV1, RecordedWorkflowVersionV1,
    SCHEMA_WIRE_VERSION_GRAPH_V1, validate_node_parameters, validate_parameter,
    validate_parameter_contract, validate_reference_contract, validate_trigger_configuration,
};
use crate::resolved_plugin::{
    ActionContractSnapshot, CredentialContractSnapshot, ResourceContractSnapshot,
};
use crate::{FrozenPluginRegistry, ResolvedPlugin};

const DEFAULT_OUTPUT_PORT: &str = "out";

#[derive(Debug, Clone, Copy)]
enum DiagnosticCode {
    UnsupportedWorkflowSchema,
    DuplicateNode,
    DuplicateTrigger,
    MissingPlugin,
    MissingAction,
    ActionVersionMismatch,
    UnsupportedNodeKind,
    TriggerKindMismatch,
    DisabledNodeEdge,
    MissingConnectionEndpoint,
    DuplicateConnection,
    UnsupportedSourcePort,
    UnsupportedTargetPort,
    UnsupportedTagFilter,
    NodeTypeFilterRejected,
    SchemaIncompatible,
    InvalidParameterContract,
    InvalidTriggerConfiguration,
    InvalidReferenceContract,
    InvalidReferencePath,
    UnknownSlotOverride,
    SlotKindMismatch,
    EmptySelector,
    MissingResourceContract,
    MissingCredentialContract,
    DependencyTypeMismatch,
    UndeclaredPluginDependency,
    SupportCardinality,
    GraphCycle,
    UnsupportedContractProjection,
}

impl DiagnosticCode {
    const fn reason(self) -> &'static str {
        match self {
            Self::UnsupportedWorkflowSchema => "UNSUPPORTED_WORKFLOW_SCHEMA",
            Self::DuplicateNode => "DUPLICATE_NODE",
            Self::DuplicateTrigger => "DUPLICATE_TRIGGER",
            Self::MissingPlugin => "MISSING_PLUGIN",
            Self::MissingAction => "MISSING_ACTION",
            Self::ActionVersionMismatch => "ACTION_VERSION_MISMATCH",
            Self::UnsupportedNodeKind => "UNSUPPORTED_NODE_KIND",
            Self::TriggerKindMismatch => "TRIGGER_KIND_MISMATCH",
            Self::DisabledNodeEdge => "DISABLED_NODE_EDGE",
            Self::MissingConnectionEndpoint => "MISSING_CONNECTION_ENDPOINT",
            Self::DuplicateConnection => "DUPLICATE_CONNECTION",
            Self::UnsupportedSourcePort => "UNSUPPORTED_SOURCE_PORT",
            Self::UnsupportedTargetPort => "UNSUPPORTED_TARGET_PORT",
            Self::UnsupportedTagFilter => "UNSUPPORTED_TAG_FILTER",
            Self::NodeTypeFilterRejected => "NODE_TYPE_FILTER_REJECTED",
            Self::SchemaIncompatible => "SCHEMA_INCOMPATIBLE",
            Self::InvalidParameterContract => "INVALID_PARAMETER_CONTRACT",
            Self::InvalidTriggerConfiguration => "INVALID_TRIGGER_CONFIGURATION",
            Self::InvalidReferenceContract => "INVALID_REFERENCE_CONTRACT",
            Self::InvalidReferencePath => "INVALID_REFERENCE_PATH",
            Self::UnknownSlotOverride => "UNKNOWN_SLOT_OVERRIDE",
            Self::SlotKindMismatch => "SLOT_KIND_MISMATCH",
            Self::EmptySelector => "EMPTY_SELECTOR",
            Self::MissingResourceContract => "MISSING_RESOURCE_CONTRACT",
            Self::MissingCredentialContract => "MISSING_CREDENTIAL_CONTRACT",
            Self::DependencyTypeMismatch => "DEPENDENCY_TYPE_MISMATCH",
            Self::UndeclaredPluginDependency => "UNDECLARED_PLUGIN_DEPENDENCY",
            Self::SupportCardinality => "SUPPORT_CARDINALITY",
            Self::GraphCycle => "GRAPH_CYCLE",
            Self::UnsupportedContractProjection => "UNSUPPORTED_CONTRACT_PROJECTION",
        }
    }

    fn stable_code(self) -> String {
        format!("PLUGIN_PLAN_GRAPH_V1:{}", self.reason())
    }
}

#[derive(Debug, Clone, Copy)]
enum DiagnosticValue<'a> {
    CurrentWorkflowSchema,
    WorkflowSchema(u32),
    RegisteredPlugin,
    RegisteredAction,
    RegisteredResource,
    RegisteredCredential,
    ExactVersion(&'a Version),
    Plugin(&'a PluginKey),
    Action(&'a ActionKey),
    Resource(&'a ResourceKey),
    Credential(&'a CredentialKey),
    Missing,
    UniqueNode,
    UniqueTrigger,
    GraphNodeKind,
    TriggerKind,
    StatelessKind,
    StatefulKind,
    ControlKind,
    ResourceKind,
    AgentKind,
    InteractiveKind,
    StreamKind,
    MainOutput,
    DefaultFlowInput,
    SupportInput,
    CompatibleSchema,
    IncompatibleSchema,
    ValidParameterContract,
    ValidTriggerConfiguration,
    ValidReferenceContract,
    CanonicalReference,
    ResourceSlot,
    CredentialSlot,
    NonEmptySelector,
    MatchingLocalType,
    DeclaredPluginDependency,
    ValidCardinality,
    AcyclicGraph,
    CanonicalPlan,
    UnsupportedVariant,
    DisabledNode,
    Duplicate,
}

impl DiagnosticValue<'_> {
    fn render(self) -> String {
        match self {
            Self::CurrentWorkflowSchema => nebula_workflow::CURRENT_SCHEMA_VERSION.to_string(),
            Self::WorkflowSchema(version) => version.to_string(),
            Self::RegisteredPlugin => "<registered-plugin>".to_owned(),
            Self::RegisteredAction => "<registered-action>".to_owned(),
            Self::RegisteredResource => "<registered-resource>".to_owned(),
            Self::RegisteredCredential => "<registered-credential>".to_owned(),
            Self::ExactVersion(version) => version.to_string(),
            Self::Plugin(key) => key.to_string(),
            Self::Action(key) => key.to_string(),
            Self::Resource(key) => key.to_string(),
            Self::Credential(key) => key.to_string(),
            Self::Missing => "<missing>".to_owned(),
            Self::UniqueNode => "<unique-node-id>".to_owned(),
            Self::UniqueTrigger => "<unique-trigger-id>".to_owned(),
            Self::GraphNodeKind => "stateless|stateful|control".to_owned(),
            Self::TriggerKind => "trigger".to_owned(),
            Self::StatelessKind => "stateless".to_owned(),
            Self::StatefulKind => "stateful".to_owned(),
            Self::ControlKind => "control".to_owned(),
            Self::ResourceKind => "resource".to_owned(),
            Self::AgentKind => "agent".to_owned(),
            Self::InteractiveKind => "interactive".to_owned(),
            Self::StreamKind => "stream".to_owned(),
            Self::MainOutput => "main:out".to_owned(),
            Self::DefaultFlowInput => "<default-flow-input>".to_owned(),
            Self::SupportInput => "<support-input>".to_owned(),
            Self::CompatibleSchema => "<assignability-yes>".to_owned(),
            Self::IncompatibleSchema => "<assignability-no-or-unknown>".to_owned(),
            Self::ValidParameterContract => "<valid-parameter-contract>".to_owned(),
            Self::ValidTriggerConfiguration => "<valid-trigger-configuration>".to_owned(),
            Self::ValidReferenceContract => "<valid-reference-contract>".to_owned(),
            Self::CanonicalReference => "<root-or-canonical-dotted-path>".to_owned(),
            Self::ResourceSlot => "resource".to_owned(),
            Self::CredentialSlot => "credential".to_owned(),
            Self::NonEmptySelector => "<non-empty-selector>".to_owned(),
            Self::MatchingLocalType => "<matching-local-type>".to_owned(),
            Self::DeclaredPluginDependency => "<declared-plugin-dependency>".to_owned(),
            Self::ValidCardinality => "<valid-support-cardinality>".to_owned(),
            Self::AcyclicGraph => "<acyclic-graph>".to_owned(),
            Self::CanonicalPlan => "<canonical-graph-v1-plan>".to_owned(),
            Self::UnsupportedVariant => "<unsupported-variant>".to_owned(),
            Self::DisabledNode => "<disabled-node>".to_owned(),
            Self::Duplicate => "<duplicate>".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Remediation {
    UpgradeWorkflowSchema,
    UseUniqueIdentity,
    RegisterPlugin,
    RegisterAction,
    PinFrozenVersion,
    SelectGraphKind,
    MoveTriggerBinding,
    RemoveDisabledEdge,
    RepairConnection,
    UseMainOutput,
    UseDefaultOrSupportInput,
    RemoveTagFilter,
    SelectAllowedNodeType,
    AlignSchemas,
    AlignParameterContract,
    AlignTriggerConfiguration,
    AlignReferenceContract,
    CanonicalizeReference,
    DeclareMatchingSlot,
    SelectMatchingSlotKind,
    ProvideSelector,
    RegisterResource,
    RegisterCredential,
    AlignDependencyType,
    DeclarePluginDependency,
    RepairSupportCardinality,
    RemoveCycle,
    UpgradeCompilerProfile,
}

impl Remediation {
    const fn text(self) -> &'static str {
        match self {
            Self::UpgradeWorkflowSchema => "migrate the workflow to the current schema version",
            Self::UseUniqueIdentity => "use a unique stable identifier",
            Self::RegisterPlugin => "freeze a registry containing the referenced plugin",
            Self::RegisterAction => "register the exact namespaced action in the referenced plugin",
            Self::PinFrozenVersion => {
                "pin the exact interface version present in the frozen registry"
            },
            Self::SelectGraphKind => "use a Graph-v1 action kind or another ratified profile",
            Self::MoveTriggerBinding => "declare trigger actions through trigger_bindings",
            Self::RemoveDisabledEdge => "remove the edge or enable and compile the referenced node",
            Self::RepairConnection => "connect two enabled nodes with a unique supported edge",
            Self::UseMainOutput => "connect from the main out port in Graph-v1",
            Self::UseDefaultOrSupportInput => {
                "use the default flow input or a supported support port"
            },
            Self::RemoveTagFilter => "remove the tag-filtered edge until tag authority is ratified",
            Self::SelectAllowedNodeType => "connect an action allowed by the support-port contract",
            Self::AlignSchemas => "align producer output and consumer input schemas",
            Self::AlignParameterContract => {
                "align the parameter kind and value with the action input schema"
            },
            Self::AlignTriggerConfiguration => {
                "align the trigger configuration with the trigger action schema"
            },
            Self::AlignReferenceContract => {
                "connect a compatible source and use a resolvable output path"
            },
            Self::CanonicalizeReference => "use $, $.field, or a canonical bare dotted path",
            Self::DeclareMatchingSlot => {
                "remove the override or declare the matching dependency slot"
            },
            Self::SelectMatchingSlotKind => {
                "use a selector override matching the declared slot kind"
            },
            Self::ProvideSelector => "provide a non-empty abstract selector",
            Self::RegisterResource => "register the exact resource contract in the frozen registry",
            Self::RegisterCredential => {
                "register the exact credential contract in the frozen registry"
            },
            Self::AlignDependencyType => {
                "declare the same local concrete type as the registered contract"
            },
            Self::DeclarePluginDependency => {
                "declare the provider plugin and compatible version requirement"
            },
            Self::RepairSupportCardinality => "satisfy required and multi support-port cardinality",
            Self::RemoveCycle => "remove a graph or resource dependency cycle",
            Self::UpgradeCompilerProfile => {
                "use a compiler format that explicitly supports this contract"
            },
        }
    }
}

#[derive(Default)]
struct Diagnostics {
    values: Vec<ActivationDiagnostic>,
}

impl Diagnostics {
    fn push(
        &mut self,
        code: DiagnosticCode,
        path: JsonPointer,
        expected: DiagnosticValue<'_>,
        actual: DiagnosticValue<'_>,
        remediation: Remediation,
    ) {
        if let Some(diagnostic) = ActivationDiagnostic::new(
            code.stable_code(),
            path.into_string(),
            expected.render(),
            actual.render(),
            remediation.text(),
        ) {
            self.values.push(diagnostic);
        }
    }

    fn into_error(self) -> Option<PlanCompilationError> {
        PlanCompilationError::new(self.values)
    }
}

fn unsupported_contract_projection(path: JsonPointer) -> PlanCompilationError {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(
        DiagnosticCode::UnsupportedContractProjection,
        path,
        DiagnosticValue::CanonicalPlan,
        DiagnosticValue::UnsupportedVariant,
        Remediation::UpgradeCompilerProfile,
    );
    match diagnostics.into_error() {
        Some(error) => error,
        None => PlanCompilationError::invalid_compiled_record(),
    }
}

#[derive(Debug, Clone)]
struct JsonPointer(String);

impl JsonPointer {
    fn root(segment: &str) -> Self {
        let mut value = Self(String::new());
        value.push(segment);
        value
    }

    fn child(mut self, segment: &str) -> Self {
        self.push(segment);
        self
    }

    fn push(&mut self, segment: &str) {
        self.0.push('/');
        for character in segment.chars() {
            match character {
                '~' => self.0.push_str("~0"),
                '/' => self.0.push_str("~1"),
                other => self.0.push(other),
            }
        }
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ContractProjectionError {
    ActionKind,
    Isolation,
    CheckpointPolicy,
    InputPort,
    OutputPort,
    AuthPattern,
    /// A connection filter was present but empty, i.e. "accept nothing".
    EmptyConnectionFilter,
}

/// Projects the exact snapshotted action contract used by compilation and
/// registry compatibility checks.
pub(crate) fn project_action_contract(
    plugin: &ResolvedPlugin,
    key: &ActionKey,
) -> Result<Option<RecordedActionV1>, ContractProjectionError> {
    plugin
        .action_contract(key)
        .map(|snapshot| project_action_snapshot(plugin.key(), snapshot))
        .transpose()
}

/// Projects the exact snapshotted resource contract used by compilation and
/// registry compatibility checks.
pub(crate) fn project_resource_contract(
    plugin: &ResolvedPlugin,
    key: &ResourceKey,
) -> Result<Option<RecordedResourceV1>, ContractProjectionError> {
    plugin
        .resource_contract(key)
        .map(|snapshot| project_resource_snapshot(plugin.key(), snapshot))
        .transpose()
}

/// Projects the exact snapshotted credential contract used by compilation and
/// registry compatibility checks.
pub(crate) fn project_credential_contract(
    plugin: &ResolvedPlugin,
    key: &CredentialKey,
) -> Result<Option<RecordedCredentialV1>, ContractProjectionError> {
    plugin
        .credential_contract(key)
        .map(|snapshot| project_credential_snapshot(plugin.key(), snapshot))
        .transpose()
}

fn project_action_snapshot(
    plugin_key: &PluginKey,
    snapshot: &ActionContractSnapshot,
) -> Result<RecordedActionV1, ContractProjectionError> {
    let metadata = snapshot.metadata();
    let mut inputs = metadata
        .inputs
        .iter()
        .map(project_input_port)
        .collect::<Result<Vec<_>, _>>()?;
    inputs.sort_by(|left, right| input_port_key(left).cmp(input_port_key(right)));
    let mut outputs = metadata
        .outputs
        .iter()
        .map(project_output_port)
        .collect::<Result<Vec<_>, _>>()?;
    outputs.sort_by(|left, right| output_port_key(left).cmp(output_port_key(right)));

    Ok(RecordedActionV1 {
        key: metadata.base.key.to_string(),
        plugin_key: plugin_key.to_string(),
        version: record_semver(&metadata.base.version),
        kind: project_action_kind(metadata.kind)?,
        isolation: project_isolation(metadata.isolation_level)?,
        checkpoint_policy: project_checkpoint_policy(metadata.checkpoint_policy)?,
        max_concurrent: metadata.max_concurrent.map(core::num::NonZeroU32::get),
        inputs: inputs.into_boxed_slice(),
        outputs: outputs.into_boxed_slice(),
        input_schema: record_schema(metadata.base.schema.clone()),
        output_schema: record_schema(metadata.output_schema.clone()),
        dependencies: project_dependencies(snapshot.dependencies()),
    })
}

fn project_resource_snapshot(
    plugin_key: &PluginKey,
    snapshot: &ResourceContractSnapshot,
) -> Result<RecordedResourceV1, ContractProjectionError> {
    let metadata = snapshot.metadata();
    Ok(RecordedResourceV1 {
        key: metadata.base.key.to_string(),
        plugin_key: plugin_key.to_string(),
        version: record_semver(&metadata.base.version),
        configuration_schema: record_schema(metadata.base.schema.clone()),
        dependencies: project_dependencies(snapshot.dependencies()),
    })
}

fn project_credential_snapshot(
    plugin_key: &PluginKey,
    snapshot: &CredentialContractSnapshot,
) -> Result<RecordedCredentialV1, ContractProjectionError> {
    let metadata = snapshot.metadata();
    Ok(RecordedCredentialV1 {
        key: snapshot.projected_key().to_string(),
        plugin_key: plugin_key.to_string(),
        version: record_semver(&metadata.base.version),
        pattern: project_auth_pattern(metadata.pattern)?,
        properties_schema: record_schema(metadata.base.schema.clone()),
        capability_bits: snapshot.capabilities().bits(),
    })
}

fn project_action_kind(kind: ActionKind) -> Result<RecordedActionKindV1, ContractProjectionError> {
    match kind {
        ActionKind::Stateless => Ok(RecordedActionKindV1::Stateless),
        ActionKind::Stateful => Ok(RecordedActionKindV1::Stateful),
        ActionKind::Control => Ok(RecordedActionKindV1::Control),
        ActionKind::Trigger => Ok(RecordedActionKindV1::Trigger),
        ActionKind::Stream | ActionKind::Agent | ActionKind::Interactive | ActionKind::Resource => {
            Err(ContractProjectionError::ActionKind)
        },
        _ => Err(ContractProjectionError::ActionKind),
    }
}

fn project_isolation(
    isolation: IsolationLevel,
) -> Result<RecordedIsolationV1, ContractProjectionError> {
    match isolation {
        IsolationLevel::None => Ok(RecordedIsolationV1::None),
        IsolationLevel::CapabilityGated => Ok(RecordedIsolationV1::CapabilityGated),
        _ => Err(ContractProjectionError::Isolation),
    }
}

fn project_checkpoint_policy(
    policy: CheckpointPolicy,
) -> Result<RecordedCheckpointPolicyV1, ContractProjectionError> {
    match policy {
        CheckpointPolicy::Inherit => Ok(RecordedCheckpointPolicyV1::Inherit),
        CheckpointPolicy::OnePass => Ok(RecordedCheckpointPolicyV1::OnePass),
        CheckpointPolicy::Stepwise => Ok(RecordedCheckpointPolicyV1::Stepwise),
        CheckpointPolicy::ForcedHandoff => Ok(RecordedCheckpointPolicyV1::ForcedHandoff),
        _ => Err(ContractProjectionError::CheckpointPolicy),
    }
}

fn project_input_port(port: &InputPort) -> Result<RecordedInputPortV1, ContractProjectionError> {
    match port {
        InputPort::Flow { key } => Ok(RecordedInputPortV1::Flow {
            key: key.to_string(),
        }),
        InputPort::Support(support) => Ok(RecordedInputPortV1::Support {
            key: support.key.to_string(),
            required: support.required,
            multi: support.multi,
            allowed_node_types: canonical_optional_strings(
                support.filter.allowed_node_types.as_deref(),
            )?,
            allowed_tags: canonical_optional_strings(support.filter.allowed_tags.as_deref())?,
        }),
        _ => Err(ContractProjectionError::InputPort),
    }
}

fn project_output_port(port: &OutputPort) -> Result<RecordedOutputPortV1, ContractProjectionError> {
    match port {
        OutputPort::Flow { key, kind } => Ok(RecordedOutputPortV1::Flow {
            key: key.to_string(),
            flow_kind: match kind {
                FlowKind::Main => RecordedFlowKindV1::Main,
                FlowKind::Error => RecordedFlowKindV1::Error,
                _ => return Err(ContractProjectionError::OutputPort),
            },
        }),
        OutputPort::Dynamic(dynamic) => Ok(RecordedOutputPortV1::Dynamic {
            key: dynamic.key.to_string(),
            source_field: dynamic.source_field.clone(),
            label_field: dynamic.label_field.clone(),
            include_fallback: dynamic.include_fallback,
        }),
        _ => Err(ContractProjectionError::OutputPort),
    }
}

fn project_auth_pattern(
    pattern: AuthPattern,
) -> Result<RecordedAuthPatternV1, ContractProjectionError> {
    match pattern {
        AuthPattern::NoAuth => Ok(RecordedAuthPatternV1::NoAuth),
        AuthPattern::SecretToken => Ok(RecordedAuthPatternV1::SecretToken),
        AuthPattern::IdentityPassword => Ok(RecordedAuthPatternV1::IdentityPassword),
        AuthPattern::OAuth2 => Ok(RecordedAuthPatternV1::OAuth2),
        AuthPattern::KeyPair => Ok(RecordedAuthPatternV1::KeyPair),
        AuthPattern::Certificate => Ok(RecordedAuthPatternV1::Certificate),
        AuthPattern::RequestSigning => Ok(RecordedAuthPatternV1::RequestSigning),
        AuthPattern::ConnectionUri => Ok(RecordedAuthPatternV1::ConnectionUri),
        AuthPattern::InstanceIdentity => Ok(RecordedAuthPatternV1::InstanceIdentity),
        AuthPattern::SharedSecret => Ok(RecordedAuthPatternV1::SharedSecret),
        AuthPattern::Custom => Ok(RecordedAuthPatternV1::Custom),
        _ => Err(ContractProjectionError::AuthPattern),
    }
}

/// Canonicalize one optional connection-filter list.
///
/// `None` means "unfiltered"; a present list means "only these". An explicitly
/// empty list therefore says "accept nothing" — a meaning the recorded wire
/// format cannot carry, because the plan validator rejects `Some([])` as
/// noncanonical. Collapsing it to `None` would silently invert a deny-all
/// filter into allow-all and admit every source node onto the port, so an
/// empty filter is refused at projection instead.
/// Choose the candidate with the lowest key, independent of iteration order.
///
/// Provider lookups walk `FrozenPluginRegistry::iter`, whose order is
/// documented as unspecified because it is a `HashMap` with a per-process
/// `RandomState` seed. Two plugins can legitimately namespace-own one
/// component key, and the winner's `plugin_key` is recorded into the canonical
/// bytes hashed into `ExecutablePlanRevisionId` — so "first one the iterator
/// yields" made a single registry and workflow compile to different revision
/// IDs on different replicas, and an exact-revision load then reported a plan
/// missing that had just been installed. A total order over the keys removes
/// the process dependence entirely.
fn lowest_keyed_provider<Key: Ord, Value>(
    candidates: impl Iterator<Item = (Key, Value)>,
) -> Option<Value> {
    candidates
        .min_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, value)| value)
}

fn canonical_optional_strings(
    values: Option<&[String]>,
) -> Result<Option<Box<[String]>>, ContractProjectionError> {
    let Some(values) = values else {
        return Ok(None);
    };
    if values.is_empty() {
        return Err(ContractProjectionError::EmptyConnectionFilter);
    }
    let mut values = values.to_vec();
    values.sort();
    values.dedup();
    Ok(Some(values.into_boxed_slice()))
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

fn project_dependencies(dependencies: &Dependencies) -> RecordedDependenciesV1 {
    let mut credentials = dependencies
        .credentials()
        .iter()
        .map(|dependency| RecordedDependencyV1 {
            key: dependency.key.to_string(),
            required: dependency.required,
        })
        .collect::<Vec<_>>();
    credentials.sort_by(|left, right| left.key.cmp(&right.key));

    let mut resources = dependencies
        .resources()
        .iter()
        .map(|dependency| RecordedDependencyV1 {
            key: dependency.key.to_string(),
            required: dependency.required,
        })
        .collect::<Vec<_>>();
    resources.sort_by(|left, right| left.key.cmp(&right.key));

    let mut slots = dependencies
        .slot_fields()
        .iter()
        .map(|slot| match &slot.kind {
            SlotKind::Resource { key, .. } => RecordedSlotV1::Resource {
                slot_key: slot.slot_key.to_owned(),
                default_selector: slot.default_id.trim().to_owned(),
                contract_key: key.to_string(),
                required: slot.required,
                lazy: slot.lazy,
            },
            SlotKind::Credential { key, .. } => RecordedSlotV1::Credential {
                slot_key: slot.slot_key.to_owned(),
                default_selector: slot.default_id.trim().to_owned(),
                contract_key: key.to_string(),
                required: slot.required,
                lazy: slot.lazy,
            },
        })
        .collect::<Vec<_>>();
    slots.sort_by(|left, right| slot_sort_key(left).cmp(&slot_sort_key(right)));

    RecordedDependenciesV1 {
        credentials: credentials.into_boxed_slice(),
        resources: resources.into_boxed_slice(),
        slots: slots.into_boxed_slice(),
    }
}

fn slot_sort_key(slot: &RecordedSlotV1) -> (&str, u8) {
    match slot {
        RecordedSlotV1::Resource { slot_key, .. } => (slot_key, 0),
        RecordedSlotV1::Credential { slot_key, .. } => (slot_key, 1),
    }
}

fn record_schema(schema: nebula_schema::ValidSchema) -> RecordedSchemaV1 {
    RecordedSchemaV1 {
        schema_wire_version: SCHEMA_WIRE_VERSION_GRAPH_V1,
        schema,
    }
}

fn record_semver(version: &Version) -> RecordedSemverV1 {
    RecordedSemverV1 {
        major: version.major,
        minor: version.minor,
        patch: version.patch,
        pre: version.pre.to_string(),
        build: version.build.to_string(),
    }
}

fn record_plugin_semver(version: &Version) -> RecordedSemverV1 {
    let mut version = version.clone();
    version.build = BuildMetadata::EMPTY;
    record_semver(&version)
}

fn record_duration(duration: Duration) -> RecordedDurationV1 {
    RecordedDurationV1 {
        seconds: duration.as_secs(),
        nanoseconds: duration.subsec_nanos(),
    }
}

fn record_retry(retry: &RetryConfig) -> RecordedRetryV1 {
    RecordedRetryV1 {
        max_attempts: retry.max_attempts,
        initial_delay_ms: retry.initial_delay_ms,
        max_delay_ms: retry.max_delay_ms,
        backoff_multiplier_bits: retry.backoff_multiplier.to_bits(),
    }
}

fn record_error_strategy(strategy: ErrorStrategy) -> Option<RecordedErrorStrategyV1> {
    match strategy {
        ErrorStrategy::FailFast => Some(RecordedErrorStrategyV1::FailFast),
        ErrorStrategy::ContinueOnError => Some(RecordedErrorStrategyV1::ContinueOnError),
        ErrorStrategy::IgnoreErrors => Some(RecordedErrorStrategyV1::IgnoreErrors),
        _ => None,
    }
}

fn qualify_action_key(plugin_key: &PluginKey, authored: &ActionKey) -> Option<ActionKey> {
    let prefix = format!("{}.", plugin_key.as_str());
    if authored.as_str().starts_with(&prefix) {
        return Some(authored.clone());
    }
    ActionKey::new(format!("{prefix}{}", authored.as_str())).ok()
}

fn normalize_reference_path(path: &str) -> Option<String> {
    let normalized = match path {
        "$" => "",
        value if value.starts_with("$.") => &value[2..],
        value if value.starts_with('$') => return None,
        value => value,
    };
    if normalized.is_empty() {
        return Some(String::new());
    }
    normalized
        .split('.')
        .all(|segment| {
            !segment.is_empty()
                && (!segment.bytes().all(|byte| byte.is_ascii_digit())
                    || segment == "0"
                    || !segment.starts_with('0'))
        })
        .then(|| normalized.to_owned())
}

struct GraphCompiler<'a> {
    registry: &'a FrozenPluginRegistry,
    workflow_version_id: WorkflowVersionId,
    workflow: &'a WorkflowDefinition,
    diagnostics: Diagnostics,
    actions: BTreeMap<String, RecordedActionV1>,
    action_dependencies: BTreeMap<String, Dependencies>,
    resources: BTreeMap<String, RecordedResourceV1>,
    credentials: BTreeMap<String, RecordedCredentialV1>,
    plugins: BTreeMap<String, RecordedPluginV1>,
    nodes: BTreeMap<String, RecordedNodeV1>,
    triggers: BTreeMap<String, RecordedTriggerV1>,
    bindings: Vec<RecordedBindingV1>,
}

impl<'a> GraphCompiler<'a> {
    fn new(
        registry: &'a FrozenPluginRegistry,
        workflow_version_id: WorkflowVersionId,
        workflow: &'a WorkflowDefinition,
    ) -> Self {
        Self {
            registry,
            workflow_version_id,
            workflow,
            diagnostics: Diagnostics::default(),
            actions: BTreeMap::new(),
            action_dependencies: BTreeMap::new(),
            resources: BTreeMap::new(),
            credentials: BTreeMap::new(),
            plugins: BTreeMap::new(),
            nodes: BTreeMap::new(),
            triggers: BTreeMap::new(),
            bindings: Vec::new(),
        }
    }

    fn compile(mut self) -> Result<ExecutablePlanRevision, PlanCompilationError> {
        self.validate_workflow_header();
        self.compile_nodes();
        self.compile_triggers();
        let connections = self.compile_connections();
        self.validate_references(&connections);
        self.close_dependencies();
        self.validate_resource_cycles();

        if let Some(error) = std::mem::take(&mut self.diagnostics).into_error() {
            return Err(error);
        }

        let Some(error_strategy) = record_error_strategy(self.workflow.config.error_strategy)
        else {
            return Err(unsupported_contract_projection(
                JsonPointer::root("config").child("error_strategy"),
            ));
        };
        let max_parallel_nodes =
            u64::try_from(self.workflow.config.max_parallel_nodes).map_err(|_| {
                unsupported_contract_projection(
                    JsonPointer::root("config").child("max_parallel_nodes"),
                )
            })?;
        let mut variables = self
            .workflow
            .variables
            .iter()
            .map(|(name, value)| RecordedVariableV1 {
                name: name.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        variables.sort_by(|left, right| left.name.cmp(&right.name));
        let workflow_config = RecordedWorkflowConfigV1 {
            timeout: self.workflow.config.timeout.map(record_duration),
            max_parallel_nodes,
            checkpointing: RecordedCheckpointingV1 {
                enabled: self.workflow.config.checkpointing.enabled,
                interval: self
                    .workflow
                    .config
                    .checkpointing
                    .interval
                    .map(record_duration),
            },
            retry_policy: self.workflow.config.retry_policy.as_ref().map(record_retry),
            error_strategy,
        };
        self.bindings
            .sort_by(|left, right| binding_sort_key(left).cmp(&binding_sort_key(right)));
        let mut record = RecordedExecutablePlanRevisionV1 {
            record_version: RECORD_VERSION_V1,
            compiler_version: COMPILER_VERSION_GRAPH_V1,
            canonical_hash_version: CANONICAL_HASH_VERSION_V1,
            profile: RecordedPlanProfileV1::GraphV1,
            claimed_id: ExecutablePlanRevisionId::from_bytes([0; 32]),
            workflow_version_id: self.workflow_version_id,
            plugin_set_id: self.registry.plugin_set().id(),
            worker_flavor_revision_id: self.registry.revision().id(),
            manifest: RecordedPlanManifestV1 {
                workflow_definition_schema_version: self.workflow.schema_version,
                workflow_id: self.workflow.id,
                workflow_semantic_version: RecordedWorkflowVersionV1 {
                    major: self.workflow.version.major,
                    minor: self.workflow.version.minor,
                    patch: self.workflow.version.patch,
                    pre: self.workflow.version.pre.clone(),
                    build: self.workflow.version.build.clone(),
                },
            },
            content: RecordedGraphContentV1 {
                plugins: self
                    .plugins
                    .into_values()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                nodes: self
                    .nodes
                    .into_values()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                connections: connections.into_boxed_slice(),
                actions: self
                    .actions
                    .into_values()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                resources: self
                    .resources
                    .into_values()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                credentials: self
                    .credentials
                    .into_values()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                triggers: self
                    .triggers
                    .into_values()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                variables: variables.into_boxed_slice(),
                workflow_config,
                converters: Vec::<RecordedConverterV1>::new().into_boxed_slice(),
            },
            bindings: self.bindings.into_boxed_slice(),
        };
        let computed = match record.recomputed_id() {
            Ok(computed) => computed,
            Err(_) => return Err(PlanCompilationError::invalid_compiled_record()),
        };
        record.claimed_id = computed;
        ExecutablePlanRevision::try_from_recorded_v1(record)
            .map_err(|_| PlanCompilationError::invalid_compiled_record())
    }

    fn validate_workflow_header(&mut self) {
        if self.workflow.schema_version != nebula_workflow::CURRENT_SCHEMA_VERSION {
            self.diagnostics.push(
                DiagnosticCode::UnsupportedWorkflowSchema,
                JsonPointer::root("schema_version"),
                DiagnosticValue::CurrentWorkflowSchema,
                DiagnosticValue::WorkflowSchema(self.workflow.schema_version),
                Remediation::UpgradeWorkflowSchema,
            );
        }

        let mut node_ids = HashSet::new();
        for node in &self.workflow.nodes {
            if !node_ids.insert(node.id.clone()) {
                self.diagnostics.push(
                    DiagnosticCode::DuplicateNode,
                    JsonPointer::root("nodes").child(node.id.as_str()),
                    DiagnosticValue::UniqueNode,
                    DiagnosticValue::Duplicate,
                    Remediation::UseUniqueIdentity,
                );
            }
        }
        let mut trigger_ids = HashSet::new();
        for trigger in &self.workflow.trigger_bindings {
            if !trigger_ids.insert(trigger.id.clone()) {
                self.diagnostics.push(
                    DiagnosticCode::DuplicateTrigger,
                    JsonPointer::root("trigger_bindings").child(trigger.id.as_str()),
                    DiagnosticValue::UniqueTrigger,
                    DiagnosticValue::Duplicate,
                    Remediation::UseUniqueIdentity,
                );
            }
        }
    }

    fn compile_nodes(&mut self) {
        let mut nodes = self.workflow.nodes.iter().collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        for node in nodes {
            if !node.enabled {
                continue;
            }
            let Some((full_key, action)) =
                self.resolve_action(&node.plugin_key, &node.action_key, false, node.id.as_str())
            else {
                continue;
            };
            if !matches!(
                action.kind,
                RecordedActionKindV1::Stateless
                    | RecordedActionKindV1::Stateful
                    | RecordedActionKindV1::Control
            ) {
                self.diagnostics.push(
                    DiagnosticCode::UnsupportedNodeKind,
                    JsonPointer::root("nodes")
                        .child(node.id.as_str())
                        .child("action_key"),
                    DiagnosticValue::GraphNodeKind,
                    recorded_action_kind_value(&action.kind),
                    Remediation::SelectGraphKind,
                );
                continue;
            }
            if let Some(pin) = node.interface_version.as_ref()
                && action.version != record_semver(pin)
            {
                let actual_version = semver_from_record(&action.version);
                self.diagnostics.push(
                    DiagnosticCode::ActionVersionMismatch,
                    JsonPointer::root("nodes")
                        .child(node.id.as_str())
                        .child("interface_version"),
                    DiagnosticValue::ExactVersion(pin),
                    actual_version
                        .as_ref()
                        .map_or(DiagnosticValue::Missing, DiagnosticValue::ExactVersion),
                    Remediation::PinFrozenVersion,
                );
                continue;
            }
            let parameters = self.compile_parameters(node, &action);
            if parameters.len() != node.parameters.len() {
                continue;
            }
            if validate_node_parameters(&parameters, &action).is_err() {
                self.diagnostics.push(
                    DiagnosticCode::InvalidParameterContract,
                    JsonPointer::root("nodes")
                        .child(node.id.as_str())
                        .child("parameters"),
                    DiagnosticValue::ValidParameterContract,
                    DiagnosticValue::UnsupportedVariant,
                    Remediation::AlignParameterContract,
                );
                continue;
            }
            let Some(dependencies) = self.action_dependencies.get(full_key.as_str()).cloned()
            else {
                self.unsupported_projection(
                    JsonPointer::root("nodes")
                        .child(node.id.as_str())
                        .child("dependencies"),
                );
                continue;
            };
            self.compile_bindings_for_node(node, &dependencies);
            self.nodes.insert(
                node.id.to_string(),
                RecordedNodeV1 {
                    id: node.id.to_string(),
                    plugin_key: node.plugin_key.to_string(),
                    action_key: full_key.to_string(),
                    action_version: action.version.clone(),
                    parameters: parameters.into_boxed_slice(),
                    retry_policy: node.retry_policy.as_ref().map(record_retry),
                    timeout: node.timeout.map(record_duration),
                    rate_limit: node.rate_limit.as_ref().map(|limit| RecordedRateLimitV1 {
                        max_requests: limit.max_requests,
                        window_seconds: limit.window_secs,
                    }),
                    enabled: true,
                },
            );
        }
    }

    fn compile_triggers(&mut self) {
        let mut triggers = self.workflow.trigger_bindings.iter().collect::<Vec<_>>();
        triggers.sort_by(|left, right| left.id.cmp(&right.id));
        for trigger in triggers {
            let Some((full_key, action)) = self.resolve_action(
                &trigger.plugin_key,
                &trigger.action_key,
                true,
                trigger.id.as_str(),
            ) else {
                continue;
            };
            if !matches!(action.kind, RecordedActionKindV1::Trigger) {
                self.diagnostics.push(
                    DiagnosticCode::TriggerKindMismatch,
                    JsonPointer::root("trigger_bindings")
                        .child(trigger.id.as_str())
                        .child("action_key"),
                    DiagnosticValue::TriggerKind,
                    recorded_action_kind_value(&action.kind),
                    Remediation::MoveTriggerBinding,
                );
                continue;
            }
            if let Some(pin) = trigger.interface_version.as_ref()
                && action.version != record_semver(pin)
            {
                let actual_version = semver_from_record(&action.version);
                self.diagnostics.push(
                    DiagnosticCode::ActionVersionMismatch,
                    JsonPointer::root("trigger_bindings")
                        .child(trigger.id.as_str())
                        .child("interface_version"),
                    DiagnosticValue::ExactVersion(pin),
                    actual_version
                        .as_ref()
                        .map_or(DiagnosticValue::Missing, DiagnosticValue::ExactVersion),
                    Remediation::PinFrozenVersion,
                );
                continue;
            }
            let configuration = if trigger.config.is_null() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                trigger.config.clone()
            };
            if validate_trigger_configuration(&configuration, &action).is_err() {
                self.diagnostics.push(
                    DiagnosticCode::InvalidTriggerConfiguration,
                    JsonPointer::root("trigger_bindings")
                        .child(trigger.id.as_str())
                        .child("config"),
                    DiagnosticValue::ValidTriggerConfiguration,
                    DiagnosticValue::UnsupportedVariant,
                    Remediation::AlignTriggerConfiguration,
                );
                continue;
            }
            let Some(dependencies) = self.action_dependencies.get(full_key.as_str()).cloned()
            else {
                self.unsupported_projection(
                    JsonPointer::root("trigger_bindings")
                        .child(trigger.id.as_str())
                        .child("dependencies"),
                );
                continue;
            };
            self.compile_bindings_for_trigger(trigger, &dependencies);
            self.triggers.insert(
                trigger.id.to_string(),
                RecordedTriggerV1 {
                    id: trigger.id.to_string(),
                    plugin_key: trigger.plugin_key.to_string(),
                    action_key: full_key.to_string(),
                    action_version: action.version.clone(),
                    configuration,
                },
            );
        }
    }

    fn resolve_action(
        &mut self,
        plugin_key: &PluginKey,
        authored_key: &ActionKey,
        trigger: bool,
        site_id: &str,
    ) -> Option<(ActionKey, RecordedActionV1)> {
        let path_root = if trigger { "trigger_bindings" } else { "nodes" };
        let Some(plugin) = self.registry.get(plugin_key) else {
            self.diagnostics.push(
                DiagnosticCode::MissingPlugin,
                JsonPointer::root(path_root)
                    .child(site_id)
                    .child("plugin_key"),
                DiagnosticValue::RegisteredPlugin,
                DiagnosticValue::Plugin(plugin_key),
                Remediation::RegisterPlugin,
            );
            return None;
        };
        let Some(full_key) = qualify_action_key(plugin_key, authored_key) else {
            self.diagnostics.push(
                DiagnosticCode::MissingAction,
                JsonPointer::root(path_root)
                    .child(site_id)
                    .child("action_key"),
                DiagnosticValue::RegisteredAction,
                DiagnosticValue::Action(authored_key),
                Remediation::RegisterAction,
            );
            return None;
        };
        let Some(snapshot) = plugin.action_contract(&full_key) else {
            self.diagnostics.push(
                DiagnosticCode::MissingAction,
                JsonPointer::root(path_root)
                    .child(site_id)
                    .child("action_key"),
                DiagnosticValue::RegisteredAction,
                DiagnosticValue::Action(&full_key),
                Remediation::RegisterAction,
            );
            return None;
        };
        let kind = snapshot.metadata().kind;
        if trigger && !matches!(kind, ActionKind::Trigger) {
            self.diagnostics.push(
                DiagnosticCode::TriggerKindMismatch,
                JsonPointer::root(path_root)
                    .child(site_id)
                    .child("action_key"),
                DiagnosticValue::TriggerKind,
                action_kind_value(kind),
                Remediation::MoveTriggerBinding,
            );
            return None;
        }
        if !trigger
            && !matches!(
                kind,
                ActionKind::Stateless | ActionKind::Stateful | ActionKind::Control
            )
        {
            self.diagnostics.push(
                DiagnosticCode::UnsupportedNodeKind,
                JsonPointer::root(path_root)
                    .child(site_id)
                    .child("action_key"),
                DiagnosticValue::GraphNodeKind,
                action_kind_value(kind),
                if matches!(kind, ActionKind::Trigger) {
                    Remediation::MoveTriggerBinding
                } else {
                    Remediation::SelectGraphKind
                },
            );
            return None;
        }
        let projection = project_action_contract(plugin.as_ref(), &full_key);
        let snapshot_dependencies = snapshot.dependencies().clone();
        let action = match projection {
            Ok(Some(action)) => action,
            Ok(None) => {
                self.unsupported_projection(
                    JsonPointer::root(path_root)
                        .child(site_id)
                        .child("action_key"),
                );
                return None;
            },
            Err(_) => {
                self.diagnostics.push(
                    DiagnosticCode::UnsupportedContractProjection,
                    JsonPointer::root(path_root)
                        .child(site_id)
                        .child("action_key"),
                    DiagnosticValue::GraphNodeKind,
                    DiagnosticValue::UnsupportedVariant,
                    Remediation::UpgradeCompilerProfile,
                );
                return None;
            },
        };
        self.record_plugin(plugin.as_ref());
        self.action_dependencies
            .entry(full_key.to_string())
            .or_insert(snapshot_dependencies);
        self.actions
            .entry(full_key.to_string())
            .or_insert_with(|| action.clone());
        Some((full_key, action))
    }

    fn compile_parameters(
        &mut self,
        node: &NodeDefinition,
        action: &RecordedActionV1,
    ) -> Vec<RecordedParameterV1> {
        let mut parameters = node.parameters.iter().collect::<Vec<_>>();
        parameters.sort_by_key(|(key, _)| *key);
        parameters
            .into_iter()
            .filter_map(|(key, value)| {
                let value = match value {
                    ParamValue::Literal { value } => RecordedParameterValueV1::Literal {
                        value: value.clone(),
                    },
                    ParamValue::Expression { expr } => RecordedParameterValueV1::Expression {
                        expression: expr.clone(),
                    },
                    ParamValue::Template { template } => RecordedParameterValueV1::Template {
                        template: template.clone(),
                    },
                    ParamValue::Reference {
                        node_key,
                        output_path,
                    } => {
                        let Some(output_path) = normalize_reference_path(output_path) else {
                            self.diagnostics.push(
                                DiagnosticCode::InvalidReferencePath,
                                JsonPointer::root("nodes")
                                    .child(node.id.as_str())
                                    .child("parameters")
                                    .child(key),
                                DiagnosticValue::CanonicalReference,
                                DiagnosticValue::UnsupportedVariant,
                                Remediation::CanonicalizeReference,
                            );
                            return None;
                        };
                        RecordedParameterValueV1::Reference {
                            node_key: node_key.to_string(),
                            output_path,
                        }
                    },
                    _ => {
                        self.diagnostics.push(
                            DiagnosticCode::UnsupportedContractProjection,
                            JsonPointer::root("nodes")
                                .child(node.id.as_str())
                                .child("parameters")
                                .child(key),
                            DiagnosticValue::CanonicalPlan,
                            DiagnosticValue::UnsupportedVariant,
                            Remediation::UpgradeCompilerProfile,
                        );
                        return None;
                    },
                };
                let parameter = RecordedParameterV1 {
                    key: key.clone(),
                    value,
                };
                if validate_parameter(&parameter).is_err()
                    || validate_parameter_contract(&parameter, action).is_err()
                {
                    self.diagnostics.push(
                        DiagnosticCode::InvalidParameterContract,
                        JsonPointer::root("nodes")
                            .child(node.id.as_str())
                            .child("parameters")
                            .child(key),
                        DiagnosticValue::ValidParameterContract,
                        DiagnosticValue::UnsupportedVariant,
                        Remediation::AlignParameterContract,
                    );
                    return None;
                }
                Some(parameter)
            })
            .collect()
    }

    fn compile_bindings_for_node(&mut self, node: &NodeDefinition, dependencies: &Dependencies) {
        let declared = dependencies
            .slot_fields()
            .iter()
            .map(|slot| slot.slot_key)
            .collect::<BTreeSet<_>>();
        let mut overrides = node.slot_bindings.iter().collect::<Vec<_>>();
        overrides.sort_by_key(|(key, _)| *key);
        for (slot_key, _) in overrides {
            if !declared.contains(slot_key.as_str()) {
                self.diagnostics.push(
                    DiagnosticCode::UnknownSlotOverride,
                    JsonPointer::root("nodes")
                        .child(node.id.as_str())
                        .child("slot_bindings")
                        .child(slot_key),
                    DiagnosticValue::ResourceSlot,
                    DiagnosticValue::Missing,
                    Remediation::DeclareMatchingSlot,
                );
            }
        }
        for slot in dependencies.slot_fields() {
            let override_value = node.slot_bindings.get(slot.slot_key);
            let selector = match (&slot.kind, override_value) {
                (SlotKind::Resource { .. }, Some(SlotBinding::ResourceId(value)))
                | (SlotKind::Credential { .. }, Some(SlotBinding::CredentialId(value))) => {
                    value.as_str()
                },
                (SlotKind::Resource { .. }, Some(SlotBinding::CredentialId(_))) => {
                    self.diagnostics.push(
                        DiagnosticCode::SlotKindMismatch,
                        JsonPointer::root("nodes")
                            .child(node.id.as_str())
                            .child("slot_bindings")
                            .child(slot.slot_key),
                        DiagnosticValue::ResourceSlot,
                        DiagnosticValue::CredentialSlot,
                        Remediation::SelectMatchingSlotKind,
                    );
                    continue;
                },
                (SlotKind::Credential { .. }, Some(SlotBinding::ResourceId(_))) => {
                    self.diagnostics.push(
                        DiagnosticCode::SlotKindMismatch,
                        JsonPointer::root("nodes")
                            .child(node.id.as_str())
                            .child("slot_bindings")
                            .child(slot.slot_key),
                        DiagnosticValue::CredentialSlot,
                        DiagnosticValue::ResourceSlot,
                        Remediation::SelectMatchingSlotKind,
                    );
                    continue;
                },
                (_, None) => slot.default_id,
            };
            self.compile_binding(
                RecordedBindingSiteV1::Node(node.id.to_string()),
                node.plugin_key.clone(),
                slot,
                selector,
                JsonPointer::root("nodes")
                    .child(node.id.as_str())
                    .child("slot_bindings")
                    .child(slot.slot_key),
            );
        }
    }

    fn compile_bindings_for_trigger(
        &mut self,
        trigger: &TriggerBinding,
        dependencies: &Dependencies,
    ) {
        for slot in dependencies.slot_fields() {
            self.compile_binding(
                RecordedBindingSiteV1::Trigger(trigger.id.to_string()),
                trigger.plugin_key.clone(),
                slot,
                slot.default_id,
                JsonPointer::root("trigger_bindings")
                    .child(trigger.id.as_str())
                    .child("bindings")
                    .child(slot.slot_key),
            );
        }
    }

    fn compile_binding(
        &mut self,
        site: RecordedBindingSiteV1,
        owner_plugin: PluginKey,
        slot: &nebula_core::SlotField,
        selector: &str,
        path: JsonPointer,
    ) {
        if selector.trim().is_empty() {
            self.diagnostics.push(
                DiagnosticCode::EmptySelector,
                path,
                DiagnosticValue::NonEmptySelector,
                DiagnosticValue::Missing,
                Remediation::ProvideSelector,
            );
            return;
        }
        let selector = selector.trim();
        let contract = match &slot.kind {
            SlotKind::Resource { type_id, key, .. } => {
                let Some((provider, snapshot)) = self.find_resource(key) else {
                    self.diagnostics.push(
                        DiagnosticCode::MissingResourceContract,
                        path,
                        DiagnosticValue::RegisteredResource,
                        DiagnosticValue::Resource(key),
                        Remediation::RegisterResource,
                    );
                    return;
                };
                if snapshot.type_id() != *type_id {
                    self.dependency_type_mismatch(path, DiagnosticValue::Resource(key));
                    return;
                }
                let provider_key = provider.key().clone();
                let version = snapshot.metadata().base.version.clone();
                if !self.validate_plugin_edge(&owner_plugin, &provider_key, path) {
                    return;
                }
                RecordedBindingContractV1::Resource {
                    key: key.to_string(),
                    version: record_semver(&version),
                }
            },
            SlotKind::Credential { type_id, key, .. } => {
                let Some((provider, snapshot)) = self.find_credential(key) else {
                    self.diagnostics.push(
                        DiagnosticCode::MissingCredentialContract,
                        path,
                        DiagnosticValue::RegisteredCredential,
                        DiagnosticValue::Credential(key),
                        Remediation::RegisterCredential,
                    );
                    return;
                };
                if snapshot.type_id() != *type_id {
                    self.dependency_type_mismatch(path, DiagnosticValue::Credential(key));
                    return;
                }
                let provider_key = provider.key().clone();
                let version = snapshot.metadata().base.version.clone();
                let capabilities = snapshot.capabilities().bits();
                if !self.validate_plugin_edge(&owner_plugin, &provider_key, path) {
                    return;
                }
                RecordedBindingContractV1::Credential {
                    key: key.to_string(),
                    version: record_semver(&version),
                    required_capability_bits: capabilities,
                }
            },
        };
        self.bindings.push(RecordedBindingV1 {
            site,
            slot_key: slot.slot_key.to_owned(),
            selector: selector.to_owned(),
            contract,
            required: slot.required,
            lazy: slot.lazy,
        });
    }

    fn compile_connections(&mut self) -> Vec<RecordedConnectionV1> {
        let node_states = self
            .workflow
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.enabled))
            .collect::<HashMap<_, _>>();
        let mut records = Vec::new();
        let mut seen = BTreeSet::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
        let mut connections = self.workflow.connections.iter().collect::<Vec<_>>();
        connections.sort_by(|left, right| {
            authored_connection_key(left).cmp(&authored_connection_key(right))
        });
        for connection in connections {
            let path = connection_path(connection);
            let from_state = node_states.get(&connection.from_node).copied();
            let to_state = node_states.get(&connection.to_node).copied();
            if from_state.is_none() || to_state.is_none() {
                self.diagnostics.push(
                    DiagnosticCode::MissingConnectionEndpoint,
                    path,
                    DiagnosticValue::UniqueNode,
                    DiagnosticValue::Missing,
                    Remediation::RepairConnection,
                );
                continue;
            }
            if from_state == Some(false) || to_state == Some(false) {
                self.diagnostics.push(
                    DiagnosticCode::DisabledNodeEdge,
                    path,
                    DiagnosticValue::UniqueNode,
                    DiagnosticValue::DisabledNode,
                    Remediation::RemoveDisabledEdge,
                );
                continue;
            }
            let Some(source) = self.nodes.get(connection.from_node.as_str()) else {
                continue;
            };
            let Some(target) = self.nodes.get(connection.to_node.as_str()) else {
                continue;
            };
            let Some(source_action) = self.actions.get(&source.action_key) else {
                continue;
            };
            let Some(target_action) = self.actions.get(&target.action_key) else {
                continue;
            };
            let from_port = connection.effective_from_port().to_string();
            let Some(source_port) = source_action
                .outputs
                .iter()
                .find(|port| output_port_key(port) == from_port)
            else {
                self.diagnostics.push(
                    DiagnosticCode::UnsupportedSourcePort,
                    path.clone().child("from_port"),
                    DiagnosticValue::MainOutput,
                    DiagnosticValue::Missing,
                    Remediation::UseMainOutput,
                );
                continue;
            };
            if !matches!(
                source_port,
                RecordedOutputPortV1::Flow {
                    key,
                    flow_kind: RecordedFlowKindV1::Main,
                } if key == DEFAULT_OUTPUT_PORT
            ) {
                self.diagnostics.push(
                    DiagnosticCode::UnsupportedSourcePort,
                    path.clone().child("from_port"),
                    DiagnosticValue::MainOutput,
                    DiagnosticValue::UnsupportedVariant,
                    Remediation::UseMainOutput,
                );
                continue;
            }
            let to_port = connection.to_port.as_ref().map(ToString::to_string);
            match to_port.as_deref() {
                None => {
                    let flow_count = target_action
                        .inputs
                        .iter()
                        .filter(|port| matches!(port, RecordedInputPortV1::Flow { .. }))
                        .count();
                    if flow_count != 1 {
                        self.diagnostics.push(
                            DiagnosticCode::UnsupportedTargetPort,
                            path.clone().child("to_port"),
                            DiagnosticValue::DefaultFlowInput,
                            DiagnosticValue::UnsupportedVariant,
                            Remediation::UseDefaultOrSupportInput,
                        );
                        continue;
                    }
                    let producer = OutputSchema::new(source_action.output_schema.schema.clone());
                    let consumer = InputSchema::new(target_action.input_schema.schema.clone());
                    if !matches!(explain_assignable(&producer, &consumer), Assignability::Yes) {
                        self.diagnostics.push(
                            DiagnosticCode::SchemaIncompatible,
                            path.clone(),
                            DiagnosticValue::CompatibleSchema,
                            DiagnosticValue::IncompatibleSchema,
                            Remediation::AlignSchemas,
                        );
                        continue;
                    }
                },
                Some(port) => {
                    let Some(target_port) = target_action
                        .inputs
                        .iter()
                        .find(|candidate| input_port_key(candidate) == port)
                    else {
                        self.diagnostics.push(
                            DiagnosticCode::UnsupportedTargetPort,
                            path.clone().child("to_port"),
                            DiagnosticValue::SupportInput,
                            DiagnosticValue::Missing,
                            Remediation::UseDefaultOrSupportInput,
                        );
                        continue;
                    };
                    let RecordedInputPortV1::Support {
                        allowed_node_types,
                        allowed_tags,
                        ..
                    } = target_port
                    else {
                        self.diagnostics.push(
                            DiagnosticCode::UnsupportedTargetPort,
                            path.clone().child("to_port"),
                            DiagnosticValue::SupportInput,
                            DiagnosticValue::UnsupportedVariant,
                            Remediation::UseDefaultOrSupportInput,
                        );
                        continue;
                    };
                    if allowed_tags.is_some() {
                        self.diagnostics.push(
                            DiagnosticCode::UnsupportedTagFilter,
                            path.clone().child("to_port"),
                            DiagnosticValue::SupportInput,
                            DiagnosticValue::UnsupportedVariant,
                            Remediation::RemoveTagFilter,
                        );
                        continue;
                    }
                    if allowed_node_types
                        .as_deref()
                        .is_some_and(|allowed| !allowed.iter().any(|key| key == &source.action_key))
                    {
                        self.diagnostics.push(
                            DiagnosticCode::NodeTypeFilterRejected,
                            path.clone().child("to_port"),
                            DiagnosticValue::SupportInput,
                            DiagnosticValue::UnsupportedVariant,
                            Remediation::SelectAllowedNodeType,
                        );
                        continue;
                    }
                },
            }

            let record = RecordedConnectionV1 {
                from_node: connection.from_node.to_string(),
                from_port,
                to_node: connection.to_node.to_string(),
                to_port,
            };
            let key = connection_record_key(&record);
            if !seen.insert(key) {
                self.diagnostics.push(
                    DiagnosticCode::DuplicateConnection,
                    path,
                    DiagnosticValue::UniqueNode,
                    DiagnosticValue::Duplicate,
                    Remediation::RepairConnection,
                );
                continue;
            }
            adjacency
                .entry(record.from_node.clone())
                .or_default()
                .push(record.to_node.clone());
            records.push(record);
        }
        records.sort_by_key(connection_record_key);
        self.validate_support_cardinality(&records);
        if graph_has_cycle(self.nodes.keys(), &adjacency) {
            self.diagnostics.push(
                DiagnosticCode::GraphCycle,
                JsonPointer::root("connections"),
                DiagnosticValue::AcyclicGraph,
                DiagnosticValue::UnsupportedVariant,
                Remediation::RemoveCycle,
            );
        }
        records
    }

    fn validate_support_cardinality(&mut self, connections: &[RecordedConnectionV1]) {
        for node in self.nodes.values() {
            let Some(action) = self.actions.get(&node.action_key) else {
                continue;
            };
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
                    self.diagnostics.push(
                        DiagnosticCode::SupportCardinality,
                        JsonPointer::root("nodes")
                            .child(node.id.as_str())
                            .child("inputs")
                            .child(key),
                        DiagnosticValue::ValidCardinality,
                        DiagnosticValue::UnsupportedVariant,
                        Remediation::RepairSupportCardinality,
                    );
                }
            }
        }
    }

    fn validate_references(&mut self, connections: &[RecordedConnectionV1]) {
        let actions = self
            .actions
            .values()
            .map(|action| (action.key.as_str(), action))
            .collect::<HashMap<_, _>>();
        let mut failures = Vec::new();
        for consumer in self.nodes.values() {
            for parameter in &consumer.parameters {
                let RecordedParameterValueV1::Reference {
                    node_key,
                    output_path,
                } = &parameter.value
                else {
                    continue;
                };
                let path = JsonPointer::root("nodes")
                    .child(consumer.id.as_str())
                    .child("parameters")
                    .child(parameter.key.as_str());
                let Some(source) = self.nodes.get(node_key) else {
                    failures.push((path, DiagnosticValue::Missing));
                    continue;
                };
                if !connections.iter().any(|connection| {
                    connection.from_node == *node_key && connection.to_node == consumer.id
                }) {
                    failures.push((path, DiagnosticValue::Missing));
                    continue;
                }
                if validate_reference_contract(parameter, output_path, source, consumer, &actions)
                    .is_err()
                {
                    failures.push((path, DiagnosticValue::IncompatibleSchema));
                }
            }
        }
        for (path, actual) in failures {
            self.diagnostics.push(
                DiagnosticCode::InvalidReferenceContract,
                path,
                DiagnosticValue::ValidReferenceContract,
                actual,
                Remediation::AlignReferenceContract,
            );
        }
    }

    fn close_dependencies(&mut self) {
        let mut queue = self
            .actions
            .values()
            .filter_map(|action| {
                self.action_dependencies
                    .get(&action.key)
                    .cloned()
                    .map(|dependencies| (action.plugin_key.clone(), dependencies))
            })
            .collect::<Vec<_>>();
        let mut processed_resources = BTreeSet::new();
        while let Some((owner, dependencies)) = queue.pop() {
            let Ok(owner_plugin_key) = owner.parse::<PluginKey>() else {
                continue;
            };
            for dependency in dependencies.resources() {
                let Some((provider_key, record, nested)) = self.resolve_resource_dependency(
                    &owner_plugin_key,
                    &dependency.key,
                    dependency.type_id,
                    JsonPointer::root("components")
                        .child("resources")
                        .child(dependency.key.as_str()),
                ) else {
                    continue;
                };
                let inserted = self.resources.insert(record.key.clone(), record).is_none();
                if inserted && processed_resources.insert(dependency.key.to_string()) {
                    queue.push((provider_key.to_string(), nested));
                }
            }
            for dependency in dependencies.credentials() {
                if let Some(record) = self.resolve_credential_dependency(
                    &owner_plugin_key,
                    &dependency.key,
                    dependency.type_id,
                    JsonPointer::root("components")
                        .child("credentials")
                        .child(dependency.key.as_str()),
                ) {
                    self.credentials.entry(record.key.clone()).or_insert(record);
                }
            }
            for slot in dependencies.slot_fields() {
                match &slot.kind {
                    SlotKind::Resource { type_id, key, .. } => {
                        if let Some((provider_key, record, nested)) = self
                            .resolve_resource_dependency(
                                &owner_plugin_key,
                                key,
                                *type_id,
                                JsonPointer::root("components")
                                    .child("resources")
                                    .child(key.as_str()),
                            )
                        {
                            let inserted =
                                self.resources.insert(record.key.clone(), record).is_none();
                            if inserted && processed_resources.insert(key.to_string()) {
                                queue.push((provider_key.to_string(), nested));
                            }
                        }
                    },
                    SlotKind::Credential { type_id, key, .. } => {
                        if let Some(record) = self.resolve_credential_dependency(
                            &owner_plugin_key,
                            key,
                            *type_id,
                            JsonPointer::root("components")
                                .child("credentials")
                                .child(key.as_str()),
                        ) {
                            self.credentials.entry(record.key.clone()).or_insert(record);
                        }
                    },
                }
            }
        }
    }

    fn resolve_resource_dependency(
        &mut self,
        owner_plugin: &PluginKey,
        key: &ResourceKey,
        expected_type: TypeId,
        path: JsonPointer,
    ) -> Option<(PluginKey, RecordedResourceV1, Dependencies)> {
        let (provider_key, type_id, dependencies, projection) = {
            let Some((provider, snapshot)) = self.find_resource(key) else {
                self.diagnostics.push(
                    DiagnosticCode::MissingResourceContract,
                    path,
                    DiagnosticValue::RegisteredResource,
                    DiagnosticValue::Resource(key),
                    Remediation::RegisterResource,
                );
                return None;
            };
            (
                provider.key().clone(),
                snapshot.type_id(),
                snapshot.dependencies().clone(),
                project_resource_contract(provider, key),
            )
        };
        if type_id != expected_type {
            self.dependency_type_mismatch(path, DiagnosticValue::Resource(key));
            return None;
        }
        if !self.validate_plugin_edge(owner_plugin, &provider_key, path.clone()) {
            return None;
        }
        let record = match projection {
            Ok(Some(record)) => record,
            Ok(None) => {
                self.diagnostics.push(
                    DiagnosticCode::MissingResourceContract,
                    path,
                    DiagnosticValue::RegisteredResource,
                    DiagnosticValue::Resource(key),
                    Remediation::RegisterResource,
                );
                return None;
            },
            Err(_) => {
                self.unsupported_projection(path);
                return None;
            },
        };
        if let Some(provider) = self.plugin(&provider_key) {
            let plugin_record = plugin_record(provider);
            self.plugins
                .entry(provider_key.to_string())
                .or_insert(plugin_record);
        }
        Some((provider_key, record, dependencies))
    }

    fn resolve_credential_dependency(
        &mut self,
        owner_plugin: &PluginKey,
        key: &CredentialKey,
        expected_type: TypeId,
        path: JsonPointer,
    ) -> Option<RecordedCredentialV1> {
        let (provider_key, type_id, projection) = {
            let Some((provider, snapshot)) = self.find_credential(key) else {
                self.diagnostics.push(
                    DiagnosticCode::MissingCredentialContract,
                    path,
                    DiagnosticValue::RegisteredCredential,
                    DiagnosticValue::Credential(key),
                    Remediation::RegisterCredential,
                );
                return None;
            };
            (
                provider.key().clone(),
                snapshot.type_id(),
                project_credential_contract(provider, key),
            )
        };
        if type_id != expected_type {
            self.dependency_type_mismatch(path, DiagnosticValue::Credential(key));
            return None;
        }
        if !self.validate_plugin_edge(owner_plugin, &provider_key, path.clone()) {
            return None;
        }
        let record = match projection {
            Ok(Some(record)) => record,
            Ok(None) => {
                self.diagnostics.push(
                    DiagnosticCode::MissingCredentialContract,
                    path,
                    DiagnosticValue::RegisteredCredential,
                    DiagnosticValue::Credential(key),
                    Remediation::RegisterCredential,
                );
                return None;
            },
            Err(_) => {
                self.unsupported_projection(path);
                return None;
            },
        };
        if let Some(provider) = self.plugin(&provider_key) {
            let plugin_record = plugin_record(provider);
            self.plugins
                .entry(provider_key.to_string())
                .or_insert(plugin_record);
        }
        Some(record)
    }

    fn dependency_type_mismatch(&mut self, path: JsonPointer, actual: DiagnosticValue<'_>) {
        self.diagnostics.push(
            DiagnosticCode::DependencyTypeMismatch,
            path,
            DiagnosticValue::MatchingLocalType,
            actual,
            Remediation::AlignDependencyType,
        );
    }

    fn unsupported_projection(&mut self, path: JsonPointer) {
        self.diagnostics.push(
            DiagnosticCode::UnsupportedContractProjection,
            path,
            DiagnosticValue::CanonicalPlan,
            DiagnosticValue::UnsupportedVariant,
            Remediation::UpgradeCompilerProfile,
        );
    }

    fn validate_plugin_edge(
        &mut self,
        owner_key: &PluginKey,
        provider_key: &PluginKey,
        path: JsonPointer,
    ) -> bool {
        if owner_key == provider_key {
            return true;
        }
        let Some(owner) = self.plugin(owner_key) else {
            return false;
        };
        let declared = owner.manifest().dependencies().iter().any(|dependency| {
            dependency.key() == provider_key
                && self
                    .plugin(provider_key)
                    .is_some_and(|provider| dependency.req().matches(provider.version()))
        });
        if !declared {
            self.diagnostics.push(
                DiagnosticCode::UndeclaredPluginDependency,
                path,
                DiagnosticValue::DeclaredPluginDependency,
                DiagnosticValue::Plugin(provider_key),
                Remediation::DeclarePluginDependency,
            );
        }
        declared
    }

    fn validate_resource_cycles(&mut self) {
        let adjacency =
            self.resources
                .values()
                .map(|resource| {
                    let targets =
                        resource
                            .dependencies
                            .resources
                            .iter()
                            .map(|dependency| dependency.key.clone())
                            .chain(resource.dependencies.slots.iter().filter_map(
                                |slot| match slot {
                                    RecordedSlotV1::Resource { contract_key, .. } => {
                                        Some(contract_key.clone())
                                    },
                                    RecordedSlotV1::Credential { .. } => None,
                                },
                            ))
                            .collect::<Vec<_>>();
                    (resource.key.clone(), targets)
                })
                .collect::<HashMap<_, _>>();
        if graph_has_cycle(self.resources.keys(), &adjacency) {
            self.diagnostics.push(
                DiagnosticCode::GraphCycle,
                JsonPointer::root("components").child("resources"),
                DiagnosticValue::AcyclicGraph,
                DiagnosticValue::UnsupportedVariant,
                Remediation::RemoveCycle,
            );
        }
    }

    fn plugin(&self, key: &PluginKey) -> Option<&ResolvedPlugin> {
        self.registry
            .iter()
            .find_map(|(candidate, plugin)| (candidate == key).then_some(plugin.as_ref()))
    }

    /// Locate the plugin providing `key`, choosing the provider deterministically.
    ///
    /// `FrozenPluginRegistry::iter` documents its order as unspecified — it is
    /// a `HashMap` whose `RandomState` is seeded per process. Two plugins can
    /// legitimately namespace-own one component key (`PluginKey` permits `.`,
    /// so `acme` and `acme.storage` both prefix `acme.storage.bucket`), and the
    /// chosen provider's key is recorded into the canonical bytes hashed into
    /// `ExecutablePlanRevisionId`. Taking whichever provider the iterator
    /// happened to yield first therefore let one registry and one workflow
    /// compile to *different* revision IDs on two replicas, after which an
    /// exact-revision load reports a plan missing that was just installed.
    /// Lowest plugin key wins: a total order that is stable across processes.
    fn find_resource(
        &self,
        key: &ResourceKey,
    ) -> Option<(&ResolvedPlugin, &ResourceContractSnapshot)> {
        lowest_keyed_provider(self.registry.iter().filter_map(|(plugin_key, plugin)| {
            plugin
                .resource_contract(key)
                .map(|snapshot| (plugin_key.as_str(), (plugin.as_ref(), snapshot)))
        }))
    }

    /// Locate the plugin providing `key`, choosing deterministically.
    ///
    /// Same reasoning as [`Self::find_resource`]: the winner's key reaches the
    /// content-addressed revision identity, so it must not depend on hash order.
    fn find_credential(
        &self,
        key: &CredentialKey,
    ) -> Option<(&ResolvedPlugin, &CredentialContractSnapshot)> {
        lowest_keyed_provider(self.registry.iter().filter_map(|(plugin_key, plugin)| {
            plugin
                .credential_contract(key)
                .map(|snapshot| (plugin_key.as_str(), (plugin.as_ref(), snapshot)))
        }))
    }

    fn record_plugin(&mut self, plugin: &ResolvedPlugin) {
        self.plugins
            .entry(plugin.key().to_string())
            .or_insert_with(|| plugin_record(plugin));
    }
}

fn plugin_record(plugin: &ResolvedPlugin) -> RecordedPluginV1 {
    RecordedPluginV1 {
        key: plugin.key().to_string(),
        version: record_plugin_semver(plugin.version()),
    }
}

fn recorded_action_kind_value(kind: &RecordedActionKindV1) -> DiagnosticValue<'static> {
    match kind {
        RecordedActionKindV1::Stateless => DiagnosticValue::StatelessKind,
        RecordedActionKindV1::Stateful => DiagnosticValue::StatefulKind,
        RecordedActionKindV1::Control => DiagnosticValue::ControlKind,
        RecordedActionKindV1::Trigger => DiagnosticValue::TriggerKind,
    }
}

fn action_kind_value(kind: ActionKind) -> DiagnosticValue<'static> {
    match kind {
        ActionKind::Stateless => DiagnosticValue::StatelessKind,
        ActionKind::Stateful => DiagnosticValue::StatefulKind,
        ActionKind::Control => DiagnosticValue::ControlKind,
        ActionKind::Trigger => DiagnosticValue::TriggerKind,
        ActionKind::Resource => DiagnosticValue::ResourceKind,
        ActionKind::Agent => DiagnosticValue::AgentKind,
        ActionKind::Interactive => DiagnosticValue::InteractiveKind,
        ActionKind::Stream => DiagnosticValue::StreamKind,
        _ => DiagnosticValue::UnsupportedVariant,
    }
}

fn semver_from_record(recorded: &RecordedSemverV1) -> Option<Version> {
    let mut version = Version::new(recorded.major, recorded.minor, recorded.patch);
    version.pre = recorded.pre.parse().ok()?;
    version.build = recorded.build.parse().ok()?;
    Some(version)
}

fn authored_connection_key(connection: &Connection) -> (String, String, String, Option<String>) {
    (
        connection.from_node.to_string(),
        connection.effective_from_port().to_string(),
        connection.to_node.to_string(),
        connection.to_port.as_ref().map(ToString::to_string),
    )
}

fn connection_record_key(
    connection: &RecordedConnectionV1,
) -> (String, String, String, Option<String>) {
    (
        connection.from_node.clone(),
        connection.from_port.clone(),
        connection.to_node.clone(),
        connection.to_port.clone(),
    )
}

fn connection_path(connection: &Connection) -> JsonPointer {
    JsonPointer::root("connections")
        .child(connection.from_node.as_str())
        .child(connection.effective_from_port().as_str())
        .child(connection.to_node.as_str())
        .child(
            connection
                .to_port
                .as_ref()
                .map_or("<default>", nebula_core::PortKey::as_str),
        )
}

fn graph_has_cycle<'a>(
    nodes: impl Iterator<Item = &'a String>,
    adjacency: &HashMap<String, Vec<String>>,
) -> bool {
    let nodes = nodes.map(String::as_str).collect::<Vec<_>>();
    let node_set = nodes.iter().copied().collect::<HashSet<_>>();
    let mut indegree = nodes
        .iter()
        .copied()
        .map(|node| (node, 0_usize))
        .collect::<HashMap<_, _>>();
    for targets in adjacency.values() {
        for target in targets {
            if node_set.contains(target.as_str())
                && let Some(count) = indegree.get_mut(target.as_str())
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
                if let Some(count) = indegree.get_mut(target.as_str()) {
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

fn binding_sort_key(binding: &RecordedBindingV1) -> (u8, &str, &str, u8) {
    let (site_tag, site) = match &binding.site {
        RecordedBindingSiteV1::Node(node) => (0, node.as_str()),
        RecordedBindingSiteV1::Trigger(trigger) => (1, trigger.as_str()),
    };
    let contract_tag = match binding.contract {
        RecordedBindingContractV1::Resource { .. } => 0,
        RecordedBindingContractV1::Credential { .. } => 1,
    };
    (site_tag, site, binding.slot_key.as_str(), contract_tag)
}

impl FrozenPluginRegistry {
    /// Compile an immutable authority-free Graph-v1 plan against this exact
    /// frozen registry.
    ///
    /// Omitted action interface pins are resolved once from the frozen
    /// snapshot and the resulting exact version is recorded. The compiler
    /// never resolves tenant authority, concrete resource or credential IDs,
    /// performs I/O, or mutates runtime state.
    ///
    /// ```
    /// # use nebula_core::WorkflowVersionId;
    /// # use nebula_plugin::{
    /// #     ExecutablePlanRevision, FrozenPluginRegistry, RecordedExecutablePlanRevisionV1,
    /// # };
    /// # use nebula_workflow::WorkflowDefinition;
    /// # fn checked_roundtrip(
    /// #     registry: &FrozenPluginRegistry,
    /// #     workflow_version_id: WorkflowVersionId,
    /// #     workflow: &WorkflowDefinition,
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// let plan = registry.compile_graph_v1(workflow_version_id, workflow)?;
    /// assert_eq!(plan.workflow_version_id(), workflow_version_id);
    /// assert_eq!(plan.plugin_set_id(), registry.plugin_set().id());
    /// assert_eq!(
    ///     plan.worker_flavor_revision_id(),
    ///     registry.revision().id()
    /// );
    /// let _abstract_bindings = plan.bindings();
    ///
    /// let recorded = RecordedExecutablePlanRevisionV1::from(&plan);
    /// let encoded = serde_json::to_vec(&recorded)?;
    /// let decoded: RecordedExecutablePlanRevisionV1 = serde_json::from_slice(&encoded)?;
    /// let loaded = ExecutablePlanRevision::try_from(decoded)?;
    /// assert_eq!(loaded.id(), plan.id());
    /// loaded.validate_against(registry)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`PlanCompilationError`] with a non-empty, canonically sorted
    /// set of secret-free activation diagnostics when the workflow is not an
    /// exact Graph-v1 contract for this frozen registry.
    #[tracing::instrument(
        skip(self, workflow),
        fields(
            workflow_version_id = %workflow_version_id,
            plugin_set_id = %self.plugin_set().id(),
            worker_flavor_revision_id = %self.revision().id(),
            profile = "graph_v1",
            outcome = tracing::field::Empty,
            error_code = tracing::field::Empty,
            diagnostic_count = tracing::field::Empty,
            executable_plan_revision_id = tracing::field::Empty,
        )
    )]
    pub fn compile_graph_v1(
        &self,
        workflow_version_id: WorkflowVersionId,
        workflow: &WorkflowDefinition,
    ) -> Result<ExecutablePlanRevision, PlanCompilationError> {
        let result = GraphCompiler::new(self, workflow_version_id, workflow).compile();
        let span = tracing::Span::current();
        match &result {
            Ok(plan) => {
                span.record("outcome", "success");
                span.record(
                    "executable_plan_revision_id",
                    tracing::field::display(plan.id()),
                );
                span.record("diagnostic_count", 0_u64);
            },
            Err(error) => {
                span.record("outcome", "error");
                span.record("error_code", "PLUGIN_PLAN_COMPILE:INVALID_WORKFLOW");
                span.record("diagnostic_count", error.diagnostics().len() as u64);
            },
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use nebula_action::{ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata};
    use nebula_core::{
        ArtifactSetDigest, Dependencies, PluginKey, WorkflowId, WorkflowVersionId, node_key,
    };
    use nebula_metadata::PluginManifest;
    use nebula_schema::{Field, ObjectField, Schema, SecretField, ValidSchema, field_key};
    use nebula_workflow::{NodeDefinition, ParamValue, WorkflowBuilder};
    use serde_json::json;

    use super::*;

    const SECRET_PAYLOAD: &str = "compiler-secret-that-must-not-leak";

    /// Red→green proof that provider selection does not depend on hash order.
    ///
    /// `PluginKey` permits `.`, so `acme` and `acme.storage` can both
    /// namespace-own `acme.storage.bucket`. The winner's key is hashed into the
    /// content-addressed revision id, so picking whichever provider the
    /// registry's `HashMap` yielded first made two replicas compile the same
    /// registry and workflow to different `ExecutablePlanRevisionId`s.
    #[test]
    fn provider_selection_is_lowest_key_regardless_of_iteration_order() {
        let forward =
            lowest_keyed_provider([("acme.storage", "namespaced"), ("acme", "root")].into_iter());
        let reverse =
            lowest_keyed_provider([("acme", "root"), ("acme.storage", "namespaced")].into_iter());

        assert_eq!(
            forward, reverse,
            "the same provider set must resolve identically whatever order it is walked in"
        );
        assert_eq!(
            forward,
            Some("root"),
            "the lowest plugin key is the deterministic winner"
        );
        assert_eq!(
            lowest_keyed_provider(std::iter::empty::<(&str, &str)>()),
            None,
            "no provider still means no provider"
        );
    }

    /// Red→green proof that a deny-all connection filter is never inverted.
    ///
    /// `None` is "unfiltered" and a present list is "only these", so an
    /// explicitly empty list means "accept nothing". Canonicalizing it to
    /// `None` admitted every source node onto the port — the exact opposite of
    /// what the plugin declared — and produced a record the plan validator
    /// rejects as noncanonical anyway.
    #[test]
    fn empty_connection_filter_is_refused_not_collapsed_to_unfiltered() {
        let empty: &[String] = &[];
        assert!(
            matches!(
                canonical_optional_strings(Some(empty)),
                Err(ContractProjectionError::EmptyConnectionFilter)
            ),
            "an explicitly empty filter must be refused, never read as unfiltered"
        );

        assert!(
            matches!(canonical_optional_strings(None), Ok(None)),
            "an absent filter is genuinely unfiltered"
        );

        let projected =
            canonical_optional_strings(Some(&["b".to_owned(), "a".to_owned(), "b".to_owned()]))
                .expect("a non-empty filter must project")
                .expect("a present filter must stay present");
        assert_eq!(
            &*projected,
            ["a".to_owned(), "b".to_owned()],
            "a present filter is sorted and deduplicated, and stays present"
        );
    }

    struct TestActionFactory {
        metadata: ActionMetadata,
        dependencies: Dependencies,
    }

    impl ActionFactory for TestActionFactory {
        fn metadata(&self) -> &ActionMetadata {
            &self.metadata
        }

        fn dependencies(&self) -> &Dependencies {
            &self.dependencies
        }

        fn instantiate<'a>(
            &'a self,
            _node: &'a NodeDefinition,
            _context: &'a dyn ActionContext,
        ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
            Box::pin(async {
                Err(ActionError::fatal(
                    "the pure compiler test factory is never instantiated",
                ))
            })
        }
    }

    struct TestPlugin {
        manifest: PluginManifest,
        actions: Vec<Arc<dyn ActionFactory>>,
    }

    impl std::fmt::Debug for TestPlugin {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("TestPlugin")
                .field("key", self.manifest.key())
                .finish()
        }
    }

    impl crate::Plugin for TestPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }

        fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
            self.actions.clone()
        }
    }

    fn action_metadata(
        local_key: &str,
        kind: ActionKind,
        input_schema: ValidSchema,
        output_schema: ValidSchema,
    ) -> ActionMetadata {
        ActionMetadata::new(
            ActionKey::new(format!("demo.{local_key}")).expect("fixture action key is valid"),
            local_key,
            "compiler contract fixture",
        )
        .with_kind(kind)
        .with_schema(input_schema)
        .with_output_schema(output_schema)
    }

    fn frozen(actions: Vec<ActionMetadata>) -> FrozenPluginRegistry {
        let plugin = TestPlugin {
            manifest: PluginManifest::builder("demo", "Demo")
                .build()
                .expect("fixture manifest is valid"),
            actions: actions
                .into_iter()
                .map(|metadata| {
                    Arc::new(TestActionFactory {
                        metadata,
                        dependencies: Dependencies::new(),
                    }) as Arc<dyn ActionFactory>
                })
                .collect(),
        };
        let resolved =
            Arc::new(ResolvedPlugin::from(plugin).expect("fixture plugin contracts resolve"));
        let mut registry = crate::PluginRegistry::new();
        registry
            .register(resolved)
            .expect("fixture plugin registers once");
        registry
            .freeze(
                ArtifactSetDigest::from_bytes([0x61; 32]),
                "1.0.0"
                    .parse()
                    .expect("fixture runtime contract version is valid"),
            )
            .expect("fixture registry freezes")
    }

    fn one_node_workflow(node: NodeDefinition) -> WorkflowDefinition {
        WorkflowBuilder::new("Compiler contract")
            .id(WorkflowId::from_bytes([0x62; 16]))
            .add_node(node)
            .build()
            .expect("fixture workflow is structurally valid")
    }

    fn compile_error(
        registry: &FrozenPluginRegistry,
        workflow: &WorkflowDefinition,
    ) -> PlanCompilationError {
        registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x63; 16]), workflow)
            .expect_err("fixture must fail Graph-v1 compilation")
    }

    #[test]
    fn reference_paths_normalize_only_ratified_aliases() {
        assert_eq!(normalize_reference_path("$").as_deref(), Some(""));
        assert_eq!(
            normalize_reference_path("$.payload.0").as_deref(),
            Some("payload.0")
        );
        assert_eq!(
            normalize_reference_path("payload.0").as_deref(),
            Some("payload.0")
        );
        assert!(normalize_reference_path("$payload").is_none());
        assert!(normalize_reference_path("payload..name").is_none());
        assert!(normalize_reference_path("payload.00").is_none());
    }

    #[test]
    fn json_pointer_escapes_dynamic_identifier_segments() {
        assert_eq!(
            JsonPointer::root("nodes")
                .child("a/b~c")
                .child("parameters")
                .into_string(),
            "/nodes/a~1b~0c/parameters"
        );
    }

    #[test]
    fn diagnostics_are_stably_sorted_and_payload_free() {
        let mut diagnostics = Diagnostics::default();
        let plugin = PluginKey::new("sample").unwrap();
        diagnostics.push(
            DiagnosticCode::MissingPlugin,
            JsonPointer::root("nodes").child("b"),
            DiagnosticValue::RegisteredPlugin,
            DiagnosticValue::Plugin(&plugin),
            Remediation::RegisterPlugin,
        );
        diagnostics.push(
            DiagnosticCode::MissingPlugin,
            JsonPointer::root("nodes").child("a"),
            DiagnosticValue::RegisteredPlugin,
            DiagnosticValue::Plugin(&plugin),
            Remediation::RegisterPlugin,
        );
        let error = diagnostics.into_error().unwrap();
        assert_eq!(error.diagnostics().len(), 2);
        assert_eq!(error.diagnostics()[0].path(), "/nodes/a");
        assert_eq!(
            error.diagnostics()[0].code(),
            "PLUGIN_PLAN_GRAPH_V1:MISSING_PLUGIN"
        );
        assert!(!format!("{error:?}").contains("credential-value"));
    }

    #[test]
    fn tagged_literal_is_not_reclassified_but_expression_is_rejected() {
        let input_schema = Schema::builder()
            .add(Field::string(field_key!("value")).no_expression())
            .build()
            .expect("fixture schema is valid");
        let registry = frozen(vec![action_metadata(
            "literal",
            ActionKind::Stateless,
            input_schema,
            ValidSchema::empty(),
        )]);
        let literal = NodeDefinition::new(node_key!("run"), "Run", "demo", "literal")
            .expect("fixture node is valid")
            .with_parameter("value", ParamValue::literal(json!("{{ $workflow.input }}")));
        registry
            .compile_graph_v1(
                WorkflowVersionId::from_bytes([0x64; 16]),
                &one_node_workflow(literal),
            )
            .expect("the explicitly tagged literal remains a literal");

        let expression = NodeDefinition::new(node_key!("run"), "Run", "demo", "literal")
            .expect("fixture node is valid")
            .with_parameter("value", ParamValue::expression("{{ $workflow.input }}"));
        let error = compile_error(&registry, &one_node_workflow(expression));
        assert!(error.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
                && diagnostic.path() == "/nodes/run/parameters/value"
        }));
    }

    #[test]
    fn nested_secret_parameter_is_rejected_without_payload_disclosure() {
        let input_schema = Schema::builder()
            .add(ObjectField::new(field_key!("auth")).add(SecretField::new(field_key!("token"))))
            .build()
            .expect("fixture schema is valid");
        let registry = frozen(vec![action_metadata(
            "secret",
            ActionKind::Stateless,
            input_schema,
            ValidSchema::empty(),
        )]);
        let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "secret")
            .expect("fixture node is valid")
            .with_parameter(
                "auth",
                ParamValue::literal(json!({"token": SECRET_PAYLOAD})),
            );
        let error = compile_error(&registry, &one_node_workflow(node));
        let diagnostic = error
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_PARAMETER_CONTRACT"
                    && diagnostic.path() == "/nodes/run/parameters/auth"
            })
            .expect("secret parameter has an exact safe diagnostic");
        for field in [
            diagnostic.code(),
            diagnostic.path(),
            diagnostic.expected(),
            diagnostic.actual(),
            diagnostic.remediation(),
        ] {
            assert!(!field.contains(SECRET_PAYLOAD));
        }
        assert!(!error.to_string().contains(SECRET_PAYLOAD));
        assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
    }

    #[test]
    fn trigger_secret_configuration_is_rejected_at_exact_path() {
        let trigger_schema = Schema::builder()
            .add(SecretField::new(field_key!("token")))
            .build()
            .expect("fixture schema is valid");
        let registry = frozen(vec![
            action_metadata(
                "run",
                ActionKind::Stateless,
                ValidSchema::empty(),
                ValidSchema::empty(),
            ),
            action_metadata(
                "start",
                ActionKind::Trigger,
                trigger_schema,
                ValidSchema::empty(),
            ),
        ]);
        let node = NodeDefinition::new(node_key!("run"), "Run", "demo", "run")
            .expect("fixture node is valid");
        let workflow = WorkflowBuilder::new("Compiler trigger contract")
            .id(WorkflowId::from_bytes([0x65; 16]))
            .add_node(node)
            .add_trigger(
                node_key!("hook"),
                PluginKey::new("demo").expect("fixture plugin key is valid"),
                ActionKey::new("start").expect("fixture action key is valid"),
                json!({"token": SECRET_PAYLOAD}),
            )
            .build()
            .expect("fixture workflow is structurally valid");
        let error = compile_error(&registry, &workflow);
        assert!(error.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_TRIGGER_CONFIGURATION"
                && diagnostic.path() == "/trigger_bindings/hook/config"
        }));
        assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
    }

    #[test]
    fn reference_alias_is_normalized_and_bad_contract_has_exact_path() {
        let value_schema = Schema::builder()
            .add(Field::string(field_key!("value")))
            .build()
            .expect("fixture schema is valid");
        let registry = frozen(vec![
            action_metadata(
                "source",
                ActionKind::Stateless,
                ValidSchema::empty(),
                value_schema.clone(),
            ),
            action_metadata(
                "target",
                ActionKind::Stateless,
                value_schema,
                ValidSchema::empty(),
            ),
        ]);
        let source = NodeDefinition::new(node_key!("source"), "Source", "demo", "source")
            .expect("fixture source node is valid");
        let target = NodeDefinition::new(node_key!("target"), "Target", "demo", "target")
            .expect("fixture target node is valid")
            .with_parameter(
                "value",
                ParamValue::reference(node_key!("source"), "$.value"),
            );
        let workflow = WorkflowBuilder::new("Compiler reference contract")
            .id(WorkflowId::from_bytes([0x66; 16]))
            .add_node(source.clone())
            .add_node(target)
            .connect(node_key!("source"), node_key!("target"))
            .build()
            .expect("fixture workflow is structurally valid");
        let plan = registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x67; 16]), &workflow)
            .expect("the ratified reference alias compiles");
        let recorded = RecordedExecutablePlanRevisionV1::from(&plan);
        let target = recorded
            .content
            .nodes
            .iter()
            .find(|node| node.id == "target")
            .expect("target node is recorded");
        assert!(matches!(
            &target.parameters[0].value,
            RecordedParameterValueV1::Reference { output_path, .. } if output_path == "value"
        ));

        let bad_target = NodeDefinition::new(node_key!("target"), "Target", "demo", "target")
            .expect("fixture target node is valid")
            .with_parameter(
                "value",
                ParamValue::reference(node_key!("source"), "$.missing"),
            );
        let bad_workflow = WorkflowBuilder::new("Compiler bad reference contract")
            .id(WorkflowId::from_bytes([0x68; 16]))
            .add_node(source)
            .add_node(bad_target)
            .connect(node_key!("source"), node_key!("target"))
            .build()
            .expect("fixture workflow is structurally valid");
        let error = compile_error(&registry, &bad_workflow);
        assert!(error.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "PLUGIN_PLAN_GRAPH_V1:INVALID_REFERENCE_CONTRACT"
                && diagnostic.path() == "/nodes/target/parameters/value"
        }));
    }
}
