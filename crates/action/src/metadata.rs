//! Draft, admitted, and recorded action metadata.

use core::num::NonZeroU32;

use nebula_core::ActionKey;
use nebula_metadata::{
    BaseMetadata, Metadata, MetadataBuildError, MetadataDraft, MetadataReadmissionError,
    RecordedBaseMetadata,
};
use nebula_schema::ValidSchema;
use serde::{Deserialize, Serialize};

use crate::{
    effect::ActionEffectContract,
    port::{self, InputPort, OutputPort},
    validation::{ActionPackageValidationErrors, validate_action_package},
};

/// How isolated this action's execution should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IsolationLevel {
    /// No isolation. The action runs directly in the engine process.
    #[default]
    None,
    /// Capability-gated execution in the engine process.
    CapabilityGated,
}

/// The structural execution shape of an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActionKind {
    /// One-shot execution.
    Stateless,
    /// Iterative execution with runtime-owned state.
    Stateful,
    /// Execution producing a stream of chunks.
    Stream,
    /// Autonomous turn-based execution.
    Agent,
    /// Execution paused for external input.
    Interactive,
    /// Workflow control and routing.
    Control,
    /// External event source.
    Trigger,
    /// Scoped resource provider.
    Resource,
}

/// Requested checkpoint cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckpointPolicy {
    /// Defer to engine policy.
    #[default]
    Inherit,
    /// Checkpoint after completion.
    OnePass,
    /// Request a checkpoint after each step.
    Stepwise,
    /// Force a durable scheduler handoff.
    ForcedHandoff,
}

/// Compatibility failure between two admitted action definitions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A shared metadata compatibility rule failed.
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<ActionKey>),
    /// Ports changed without a major version bump.
    #[error("action ports changed without a major version bump")]
    PortsChangeWithoutMajorBump,
    /// Effect authority changed without a major version bump.
    #[error("action effect contract changed without a major version bump")]
    EffectContractChangeWithoutMajorBump,
    /// Output compatibility narrowed without a major version bump.
    #[error("action output schema narrowed without a major version bump")]
    OutputSchemaNarrowedWithoutMajorBump,
}

/// Failure while admitting an authored action definition.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ActionMetadataAdmissionError {
    /// An associated Rust type could not produce a valid schema.
    #[error(transparent)]
    Schema(#[from] MetadataBuildError),
    /// Action-specific metadata declarations are structurally invalid.
    #[error(transparent)]
    Package(#[from] ActionPackageValidationErrors),
    /// A remote-effect factory was built from metadata without a remote contract.
    #[error("remote-effect factory requires a remote effect contract")]
    RemoteEffectContractRequired,
    /// The typed remote adapter descriptor differs from authored metadata.
    #[error("remote-effect adapter descriptor does not match admitted metadata")]
    RemoteEffectDescriptorMismatch,
}

/// Failure while re-admitting recorded action metadata.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ActionMetadataReadmissionError {
    /// Shared metadata evidence differs from the fresh definition.
    #[error(transparent)]
    Base(#[from] MetadataReadmissionError),
    /// Action-specific recorded evidence differs from the fresh definition.
    #[error("recorded action metadata does not match the fresh definition")]
    DefinitionMismatch,
}

/// Author-owned action metadata before schemas and structural kind are bound.
///
/// This type intentionally has no input-schema, output-schema, or action-kind
/// setters. A factory derives both schemas from [`Action::Input`](crate::Action::Input)
/// and [`Action::Output`](crate::Action::Output), stamps its structural kind,
/// and consumes the draft through the crate-private admission boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "an action metadata draft must be admitted by a factory"]
pub struct ActionMetadataDraft {
    base: MetadataDraft<ActionKey>,
    inputs: Vec<InputPort>,
    outputs: Vec<OutputPort>,
    isolation_level: IsolationLevel,
    checkpoint_policy: CheckpointPolicy,
    effect_contract: ActionEffectContract,
    max_concurrent: Option<NonZeroU32>,
}

impl ActionMetadataDraft {
    /// Construct a draft after checking a dynamic display name.
    ///
    /// # Errors
    ///
    /// Returns a metadata error when the display name is invalid.
    pub fn try_new(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, nebula_metadata::MetadataError> {
        Ok(Self::new(
            key,
            nebula_metadata::MetadataName::try_from(name.into())?,
            description,
        ))
    }

    /// Construct an action definition draft.
    pub fn new(
        key: ActionKey,
        name: nebula_metadata::MetadataName,
        description: impl Into<String>,
    ) -> Self {
        Self {
            base: MetadataDraft::new(key, name, description),
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            isolation_level: IsolationLevel::None,
            checkpoint_policy: CheckpointPolicy::Inherit,
            effect_contract: ActionEffectContract::Undeclared,
            max_concurrent: None,
        }
    }

    /// Set the complete interface version.
    pub fn with_version(mut self, version: nebula_metadata::MetadataVersion) -> Self {
        self.base = self.base.with_version(version);
        self
    }

    /// Retain a macro-checked full SemVer literal for fallible factory admission.
    ///
    /// Macro expansion validates this literal before emitting it. Admission
    /// still checks it, so generated library code needs no panic or fallback.
    #[doc(hidden)]
    pub fn with_version_literal(mut self, version: &'static str) -> Self {
        self.base = self.base.with_version_literal(version);
        self
    }

    /// Set a catalog icon.
    pub fn with_icon(mut self, icon: nebula_metadata::Icon) -> Self {
        self.base = self.base.with_icon(icon);
        self
    }

    /// Set an inline icon identifier.
    pub fn with_inline_icon(mut self, name: impl Into<String>) -> Self {
        self.base = self.base.with_inline_icon(name);
        self
    }

    /// Set a URL-backed icon.
    pub fn with_url_icon(mut self, url: impl Into<String>) -> Self {
        self.base = self.base.with_url_icon(url);
        self
    }

    /// Set the documentation URL.
    pub fn with_documentation_url(mut self, url: impl Into<String>) -> Self {
        self.base = self.base.with_documentation_url(url);
        self
    }

    /// Replace the catalog categories.
    pub fn with_categories(
        mut self,
        categories: impl IntoIterator<Item = nebula_metadata::CatalogCategoryKey>,
    ) -> Self {
        self.base = self.base.with_categories(categories);
        self
    }

    /// Append a typed catalog link.
    pub fn add_link(mut self, link: nebula_metadata::CatalogLink) -> Self {
        self.base = self.base.add_link(link);
        self
    }

    /// Replace the catalog tags.
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.base = self.base.with_tags(tags);
        self
    }

    /// Append one catalog tag.
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.base = self.base.add_tag(tag);
        self
    }

    /// Mark the definition experimental.
    pub fn mark_experimental(mut self) -> Self {
        self.base = self.base.mark_experimental();
        self
    }

    /// Mark the definition beta.
    pub fn mark_beta(mut self) -> Self {
        self.base = self.base.mark_beta();
        self
    }

    /// Mark the definition stable.
    pub fn mark_stable(mut self) -> Self {
        self.base = self.base.mark_stable();
        self
    }

    /// Attach a deprecation notice.
    pub fn with_deprecation(mut self, notice: nebula_metadata::DeprecationNotice) -> Self {
        self.base = self.base.with_deprecation(notice);
        self
    }

    /// Replace the input-port declaration.
    pub fn with_inputs(mut self, inputs: Vec<InputPort>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Append one input port.
    pub fn add_input(mut self, input: InputPort) -> Self {
        self.inputs.push(input);
        self
    }

    /// Replace the output-port declaration.
    pub fn with_outputs(mut self, outputs: Vec<OutputPort>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Append one output port.
    pub fn add_output(mut self, output: OutputPort) -> Self {
        self.outputs.push(output);
        self
    }

    /// Select the in-process isolation policy.
    pub fn with_isolation_level(mut self, isolation_level: IsolationLevel) -> Self {
        self.isolation_level = isolation_level;
        self
    }

    /// Select the requested checkpoint cadence.
    pub fn with_checkpoint_policy(mut self, checkpoint_policy: CheckpointPolicy) -> Self {
        self.checkpoint_policy = checkpoint_policy;
        self
    }

    /// Declare external-effect authority.
    pub fn with_effect_contract(mut self, effect_contract: ActionEffectContract) -> Self {
        self.effect_contract = effect_contract;
        self
    }

    /// Set the per-action concurrency hint.
    pub fn with_max_concurrent(mut self, max_concurrent: NonZeroU32) -> Self {
        self.max_concurrent = Some(max_concurrent);
        self
    }

    /// Derive both schemas, stamp the factory's kind, and validate the package.
    ///
    /// This is the sole transition from authored intent to executable metadata.
    /// Factory and adapter constructors call it exactly once and retain the
    /// returned immutable value.
    ///
    /// # Errors
    ///
    /// Returns a typed schema-construction or package-validation failure.
    #[tracing::instrument(name = "action.metadata.admit", skip_all, fields(?kind), err)]
    pub(crate) fn admit_for<A: crate::Action>(
        self,
        kind: ActionKind,
    ) -> Result<ActionMetadata, ActionMetadataAdmissionError> {
        let input_schema =
            nebula_schema::schema_of::<A::Input>().map_err(MetadataBuildError::from)?;
        let output_schema =
            nebula_schema::schema_of::<A::Output>().map_err(MetadataBuildError::from)?;
        let metadata = ActionMetadata {
            base: self.base.bind_schema(input_schema)?,
            inputs: self.inputs.into_boxed_slice(),
            outputs: self.outputs.into_boxed_slice(),
            isolation_level: self.isolation_level,
            kind,
            checkpoint_policy: self.checkpoint_policy,
            effect_contract: self.effect_contract,
            max_concurrent: self.max_concurrent,
            output_schema,
        };
        nebula_metadata::check_json_record(&metadata).map_err(MetadataBuildError::from)?;
        validate_action_package(&metadata)?;
        Ok(metadata)
    }
}

/// Immutable action metadata admitted against concrete Rust input/output types.
///
/// ```compile_fail
/// fn requires_deserialize<T: serde::de::DeserializeOwned>() {}
/// requires_deserialize::<nebula_action::ActionMetadata>();
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionMetadata {
    base: BaseMetadata<ActionKey>,
    inputs: Box<[InputPort]>,
    outputs: Box<[OutputPort]>,
    isolation_level: IsolationLevel,
    kind: ActionKind,
    checkpoint_policy: CheckpointPolicy,
    effect_contract: ActionEffectContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_concurrent: Option<NonZeroU32>,
    output_schema: ValidSchema,
}

impl Metadata for ActionMetadata {
    type Key = ActionKey;

    fn base(&self) -> &BaseMetadata<ActionKey> {
        &self.base
    }
}

impl ActionMetadata {
    /// Shared admitted metadata and canonical input schema.
    #[must_use]
    pub fn base(&self) -> &BaseMetadata<ActionKey> {
        &self.base
    }

    /// Declared input ports.
    #[must_use]
    pub fn inputs(&self) -> &[InputPort] {
        &self.inputs
    }

    /// Declared output ports.
    #[must_use]
    pub fn outputs(&self) -> &[OutputPort] {
        &self.outputs
    }

    /// In-process isolation policy.
    #[must_use]
    pub const fn isolation_level(&self) -> IsolationLevel {
        self.isolation_level
    }

    /// Factory-stamped structural kind.
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        self.kind
    }

    /// Requested checkpoint cadence.
    #[must_use]
    pub const fn checkpoint_policy(&self) -> CheckpointPolicy {
        self.checkpoint_policy
    }

    /// Declared external-effect authority.
    #[must_use]
    pub const fn effect_contract(&self) -> &ActionEffectContract {
        &self.effect_contract
    }

    /// Per-action concurrency hint.
    #[must_use]
    pub const fn max_concurrent(&self) -> Option<NonZeroU32> {
        self.max_concurrent
    }

    /// Canonical output schema derived from the Rust output type.
    #[must_use]
    pub const fn output_schema(&self) -> &ValidSchema {
        &self.output_schema
    }

    /// Output schema with producer polarity.
    #[must_use]
    pub fn typed_output_schema(&self) -> nebula_schema::OutputSchema {
        nebula_schema::OutputSchema::new(self.output_schema.clone())
    }

    /// Validate revision compatibility with an earlier admitted definition.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;
        let same_major = self.base.version().major == previous.base.version().major;

        if same_major && self.effect_contract != previous.effect_contract {
            return Err(MetadataCompatibilityError::EffectContractChangeWithoutMajorBump);
        }
        if same_major && (self.inputs != previous.inputs || self.outputs != previous.outputs) {
            return Err(MetadataCompatibilityError::PortsChangeWithoutMajorBump);
        }
        if same_major
            && matches!(
                self.typed_output_schema()
                    .explain_successor_of(&previous.typed_output_schema()),
                nebula_schema::Assignability::No(_)
            )
        {
            return Err(MetadataCompatibilityError::OutputSchemaNarrowedWithoutMajorBump);
        }
        Ok(())
    }
}

/// Deserialized action metadata evidence awaiting explicit re-admission.
///
/// Direct serde decoding checks structure but cannot bound parser allocation.
/// Raw byte and reader callers must use the bounded constructors or an
/// externally bounded transport.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordedActionMetadata {
    base: RecordedBaseMetadata<ActionKey>,
    inputs: Box<[InputPort]>,
    outputs: Box<[OutputPort]>,
    isolation_level: IsolationLevel,
    kind: ActionKind,
    checkpoint_policy: CheckpointPolicy,
    effect_contract: ActionEffectContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_concurrent: Option<NonZeroU32>,
    output_schema: ValidSchema,
}

impl<'de> Deserialize<'de> for RecordedActionMetadata {
    #[tracing::instrument(name = "action.metadata.decode_recorded", skip_all)]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            base: RecordedBaseMetadata<ActionKey>,
            inputs: Box<[InputPort]>,
            outputs: Box<[OutputPort]>,
            isolation_level: IsolationLevel,
            kind: ActionKind,
            checkpoint_policy: CheckpointPolicy,
            effect_contract: ActionEffectContract,
            #[serde(default)]
            max_concurrent: Option<NonZeroU32>,
            output_schema: ValidSchema,
        }

        let fields: Fields = nebula_metadata::deserialize_metadata_object(deserializer)
            .map_err(|_| serde::de::Error::custom("invalid recorded action metadata"))?;
        let recorded = Self {
            base: fields.base,
            inputs: fields.inputs,
            outputs: fields.outputs,
            isolation_level: fields.isolation_level,
            kind: fields.kind,
            checkpoint_policy: fields.checkpoint_policy,
            effect_contract: fields.effect_contract,
            max_concurrent: fields.max_concurrent,
            output_schema: fields.output_schema,
        };
        nebula_metadata::check_json_record(&recorded)
            .map_err(<D::Error as serde::de::Error>::custom)?;
        Ok(recorded)
    }
}

impl RecordedActionMetadata {
    /// Decode recorded evidence within the selected whole-envelope byte limit.
    ///
    /// # Errors
    /// Returns a payload-free error for oversized input or invalid wire records.
    pub fn from_slice(
        bytes: &[u8],
        limits: nebula_metadata::MetadataDecodeLimits,
    ) -> Result<Self, nebula_metadata::MetadataDecodeError> {
        nebula_metadata::decode_json_slice(bytes, limits)
    }

    /// Read recorded evidence with bounded buffering before JSON parsing.
    ///
    /// The caller owns the reader's framing and I/O deadline.
    ///
    /// # Errors
    /// Returns a payload-free read, size, or record error.
    pub fn from_reader(
        reader: impl std::io::Read,
        limits: nebula_metadata::MetadataDecodeLimits,
    ) -> Result<Self, nebula_metadata::MetadataDecodeError> {
        nebula_metadata::decode_json_reader(reader, limits)
    }

    /// Re-admit recorded evidence only when it exactly matches a fresh definition.
    ///
    /// The returned value is cloned exclusively from `fresh_definition`; no
    /// deserialized field becomes executable metadata.
    ///
    /// # Errors
    ///
    /// Returns a typed mismatch when shared or action-specific evidence differs.
    #[tracing::instrument(name = "action.metadata.readmit_recorded", skip_all, err)]
    pub fn readmit_against(
        &self,
        fresh_definition: &ActionMetadata,
    ) -> Result<ActionMetadata, ActionMetadataReadmissionError> {
        self.base.readmit_against(&fresh_definition.base)?;
        if self.inputs.as_ref() == fresh_definition.inputs()
            && self.outputs.as_ref() == fresh_definition.outputs()
            && self.isolation_level == fresh_definition.isolation_level
            && self.kind == fresh_definition.kind
            && self.checkpoint_policy == fresh_definition.checkpoint_policy
            && self.effect_contract == fresh_definition.effect_contract
            && self.max_concurrent == fresh_definition.max_concurrent
            && self.output_schema == fresh_definition.output_schema
        {
            Ok(fresh_definition.clone())
        } else {
            tracing::warn!(
                error_code = "ACTION:RECORDED_METADATA_MISMATCH",
                "recorded action metadata rejected"
            );
            Err(ActionMetadataReadmissionError::DefinitionMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use nebula_core::{Dependencies, action_key};
    use nebula_schema::{FieldCollector, HasSchema, Schema, field_key};

    use super::*;
    use crate::Action;

    #[derive(Debug, serde::Deserialize)]
    struct ExampleInput {
        value: String,
    }

    impl HasSchema for ExampleInput {
        fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
            Schema::builder()
                .string(field_key!("value"), nebula_schema::StringBuilder::required)
                .build()
        }
    }

    #[derive(Debug, serde::Serialize)]
    struct ExampleOutput {
        result: String,
    }

    impl HasSchema for ExampleOutput {
        fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
            Schema::builder()
                .string(field_key!("result"), nebula_schema::StringBuilder::required)
                .build()
        }
    }

    struct ExampleAction;

    impl Action for ExampleAction {
        type Input = ExampleInput;
        type Output = ExampleOutput;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("test.example"),
                crate::metadata_name!("Example"),
                "Example action",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    fn admitted() -> ActionMetadata {
        ExampleAction::metadata()
            .admit_for::<ExampleAction>(ActionKind::Control)
            .expect("valid definition")
    }

    #[test]
    fn admission_derives_both_schemas_and_stamps_kind() {
        let input = ExampleInput {
            value: "input".to_owned(),
        };
        assert_eq!(input.value, "input");
        let metadata = admitted();
        assert_eq!(metadata.kind(), ActionKind::Control);
        assert!(
            metadata
                .base()
                .schema()
                .fields()
                .iter()
                .any(|field| field.key().as_str() == "value")
        );
        assert!(
            metadata
                .output_schema()
                .fields()
                .iter()
                .any(|field| field.key().as_str() == "result")
        );
    }

    #[test]
    fn recorded_metadata_requires_exact_fresh_definition() {
        let fresh = admitted();
        let encoded = serde_json::to_vec(&fresh).expect("serialize admitted metadata");
        let recorded: RecordedActionMetadata =
            serde_json::from_slice(&encoded).expect("decode recorded evidence");
        assert_eq!(recorded.readmit_against(&fresh).unwrap(), fresh);

        let different = ActionMetadataDraft::new(
            action_key!("test.example"),
            crate::metadata_name!("Example"),
            "Changed description",
        )
        .admit_for::<ExampleAction>(ActionKind::Control)
        .expect("valid changed definition");
        assert!(recorded.readmit_against(&different).is_err());
    }

    #[test]
    fn admitted_wire_nests_shared_metadata_with_required_wire_version() {
        let encoded = serde_json::to_value(admitted()).expect("serialize admitted metadata");
        assert!(encoded.get("key").is_none());
        assert_eq!(encoded["base"]["metadata_wire_version"], 2);
        assert_eq!(
            encoded["base"]
                .get("key")
                .and_then(serde_json::Value::as_str),
            Some("test.example")
        );
        assert!(encoded["base"].get("schema").is_some());
        assert!(encoded.get("output_schema").is_some());
    }
}
