//! Admitted and recorded action definitions.

use core::num::NonZeroU32;

use nebula_core::ActionKey;
use nebula_metadata::{
    BaseDefinition, Definition, Metadata, MetadataBuildError, MetadataReadmissionError,
    RecordedBaseDefinition,
};
use nebula_schema::ValidSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Action,
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

/// Historical wire tags retained for exact evidence round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecordedCheckpointPolicy {
    Inherit,
    OnePass,
    Stepwise,
    ForcedHandoff,
}

/// Compatibility failure between two admitted action definitions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ActionCompatibilityError {
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
pub enum ActionAdmissionError {
    /// An associated Rust type could not produce a valid schema.
    #[error(transparent)]
    Schema(#[from] MetadataBuildError),
    /// Action-specific metadata declarations are structurally invalid.
    #[error(transparent)]
    Package(#[from] ActionPackageValidationErrors),
    /// A remote action declared a non-default [`Action::EFFECT_CONTRACT`].
    ///
    /// Its contract is its
    /// [`RemoteEffectAction::descriptor`](crate::RemoteEffectAction::descriptor),
    /// stamped by [`RemoteEffectInstanceFactory`](crate::RemoteEffectInstanceFactory).
    #[error("a remote-effect action must not declare an effect contract")]
    EffectContractOnRemoteAction,
    /// A remote action's descriptor could not be built.
    #[error(transparent)]
    RemoteEffectDeclaration(#[from] crate::effect::RemoteEffectDeclarationError),
}

/// Failure while re-admitting recorded action metadata.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ActionReadmissionError {
    /// Shared metadata evidence differs from the fresh definition.
    #[error(transparent)]
    Base(#[from] MetadataReadmissionError),
    /// Action-specific recorded evidence differs from the fresh definition.
    #[error("recorded action metadata does not match the fresh definition")]
    DefinitionMismatch,
}

/// The action-specific half of an authored action, read from its [`Action`]
/// impl: ports and execution policy. The shared half is its
/// [`Component::metadata`](crate::Component::metadata).
///
/// Crate-private: authors declare these on the trait, and tests build one
/// directly to admit several shapes for one action type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionInterface {
    pub(crate) inputs: Vec<InputPort>,
    pub(crate) outputs: Vec<OutputPort>,
    pub(crate) isolation_level: IsolationLevel,
    pub(crate) effect_contract: ActionEffectContract,
    pub(crate) max_concurrent: Option<NonZeroU32>,
}

impl ActionInterface {
    /// The interface `A` declares on its [`Action`] impl.
    pub(crate) fn of<A: Action>() -> Self {
        Self {
            inputs: A::inputs(),
            outputs: A::outputs(),
            isolation_level: A::ISOLATION_LEVEL,
            effect_contract: A::EFFECT_CONTRACT,
            max_concurrent: A::MAX_CONCURRENT,
        }
    }
}

impl Default for ActionInterface {
    fn default() -> Self {
        Self {
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            isolation_level: IsolationLevel::None,
            effect_contract: ActionEffectContract::default(),
            max_concurrent: None,
        }
    }
}

impl ActionDefinition {
    /// Admit `A`: stamp [`Component::KEY`](crate::Component::KEY), derive both schemas, stamp the
    /// factory's kind, and validate the package.
    ///
    /// This is the sole transition from authored intent to executable metadata.
    /// Factory and adapter constructors call it exactly once and retain the
    /// returned immutable value.
    ///
    /// # Errors
    ///
    /// Returns a typed schema-construction or package-validation failure.
    #[tracing::instrument(name = "action.metadata.admit", skip_all, fields(?kind), err)]
    pub(crate) fn admit_for<A: Action>(kind: ActionKind) -> Result<Self, ActionAdmissionError> {
        Self::admit_with::<A>(A::metadata(), ActionInterface::of::<A>(), kind)
    }

    /// Admit a remote-effect action: stamp the stateless kind and its
    /// [`descriptor`](crate::RemoteEffectAction::descriptor) as the effect
    /// contract.
    ///
    /// # Errors
    ///
    /// Returns [`ActionAdmissionError::EffectContractOnRemoteAction`] when
    /// the action declares a non-default [`Action::EFFECT_CONTRACT`], or a
    /// schema or package failure.
    pub(crate) fn admit_remote_for<A: Action>(
        descriptor: crate::RemoteEffectDescriptor,
    ) -> Result<Self, ActionAdmissionError> {
        let mut interface = ActionInterface::of::<A>();
        if interface.effect_contract != ActionEffectContract::default() {
            return Err(ActionAdmissionError::EffectContractOnRemoteAction);
        }
        interface.effect_contract = ActionEffectContract::Remote(Box::new(descriptor));
        Self::admit_with::<A>(A::metadata(), interface, ActionKind::Stateless)
    }

    /// Admit `A`'s schemas and key with explicit metadata and interface.
    pub(crate) fn admit_with<A: Action>(
        metadata: Metadata,
        interface: ActionInterface,
        kind: ActionKind,
    ) -> Result<Self, ActionAdmissionError> {
        let key = crate::key_of::<A>();
        let input_schema =
            nebula_schema::schema_of::<A::Config>().map_err(MetadataBuildError::from)?;
        let output_schema =
            nebula_schema::schema_of::<A::Output>().map_err(MetadataBuildError::from)?;
        output_schema
            .ensure_current_semantics()
            .map_err(MetadataBuildError::from)?;
        let definition = Self {
            base: metadata.bind(key, input_schema)?,
            inputs: interface.inputs.into_boxed_slice(),
            outputs: interface.outputs.into_boxed_slice(),
            isolation_level: interface.isolation_level,
            kind,
            checkpoint_policy: RecordedCheckpointPolicy::Inherit,
            effect_contract: interface.effect_contract,
            max_concurrent: interface.max_concurrent,
            output_schema,
        };
        nebula_metadata::check_json_record(&definition).map_err(MetadataBuildError::from)?;
        validate_action_package(&definition)?;
        Ok(definition)
    }
}

/// Immutable action metadata admitted against concrete Rust input/output types.
///
/// ```compile_fail
/// fn requires_deserialize<T: serde::de::DeserializeOwned>() {}
/// requires_deserialize::<nebula_action::ActionDefinition>();
/// ```
///
/// Per-action checkpoint cadence is not an executable metadata capability.
///
/// ```compile_fail
/// fn inspect(metadata: &nebula_action::ActionDefinition) {
///     let _ = metadata.checkpoint_policy();
/// }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionDefinition {
    base: BaseDefinition<ActionKey>,
    inputs: Box<[InputPort]>,
    outputs: Box<[OutputPort]>,
    isolation_level: IsolationLevel,
    kind: ActionKind,
    checkpoint_policy: RecordedCheckpointPolicy,
    effect_contract: ActionEffectContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_concurrent: Option<NonZeroU32>,
    output_schema: ValidSchema,
}

impl Definition for ActionDefinition {
    type Key = ActionKey;

    fn base(&self) -> &BaseDefinition<ActionKey> {
        &self.base
    }
}

impl ActionDefinition {
    /// Shared admitted metadata and canonical input schema.
    #[must_use]
    pub fn base(&self) -> &BaseDefinition<ActionKey> {
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
    pub fn validate_compatibility(&self, previous: &Self) -> Result<(), ActionCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;
        let same_major = self.base.version().major == previous.base.version().major;

        if same_major && self.effect_contract != previous.effect_contract {
            return Err(ActionCompatibilityError::EffectContractChangeWithoutMajorBump);
        }
        if same_major && (self.inputs != previous.inputs || self.outputs != previous.outputs) {
            return Err(ActionCompatibilityError::PortsChangeWithoutMajorBump);
        }
        if same_major
            && matches!(
                self.typed_output_schema()
                    .explain_successor_of(&previous.typed_output_schema()),
                nebula_schema::Assignability::No(_)
            )
        {
            return Err(ActionCompatibilityError::OutputSchemaNarrowedWithoutMajorBump);
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
pub struct RecordedActionDefinition {
    base: RecordedBaseDefinition<ActionKey>,
    inputs: Box<[InputPort]>,
    outputs: Box<[OutputPort]>,
    isolation_level: IsolationLevel,
    kind: ActionKind,
    checkpoint_policy: RecordedCheckpointPolicy,
    effect_contract: ActionEffectContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_concurrent: Option<NonZeroU32>,
    output_schema: ValidSchema,
}

impl<'de> Deserialize<'de> for RecordedActionDefinition {
    #[tracing::instrument(name = "action.metadata.decode_recorded", skip_all)]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            base: RecordedBaseDefinition<ActionKey>,
            inputs: Box<[InputPort]>,
            outputs: Box<[OutputPort]>,
            isolation_level: IsolationLevel,
            kind: ActionKind,
            checkpoint_policy: RecordedCheckpointPolicy,
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

impl RecordedActionDefinition {
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
        fresh_definition: &ActionDefinition,
    ) -> Result<ActionDefinition, ActionReadmissionError> {
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
            Err(ActionReadmissionError::DefinitionMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use nebula_core::Dependencies;
    use nebula_schema::{HasSchema, PropertyCollector, Schema, field_key};

    use super::*;
    use crate::{Action, Component};

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

    impl Component for ExampleAction {
        type Kind = crate::kind::Action;
        type Config = ExampleInput;

        const KEY: &'static str = "test.example";

        fn metadata() -> Metadata {
            Metadata::new(crate::metadata_name!("Example"), "Example action")
        }
    }

    impl Action for ExampleAction {
        type Output = ExampleOutput;

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    fn admitted() -> ActionDefinition {
        ActionDefinition::admit_for::<ExampleAction>(ActionKind::Control).expect("valid definition")
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
                .properties()
                .iter()
                .any(|field| field.key().as_str() == "value")
        );
        assert!(
            metadata
                .output_schema()
                .properties()
                .iter()
                .any(|field| field.key().as_str() == "result")
        );
    }

    #[test]
    fn recorded_metadata_requires_exact_fresh_definition() {
        let fresh = admitted();
        let encoded = serde_json::to_vec(&fresh).expect("serialize admitted metadata");
        let recorded: RecordedActionDefinition =
            serde_json::from_slice(&encoded).expect("decode recorded evidence");
        assert_eq!(recorded.readmit_against(&fresh).unwrap(), fresh);

        let different = ActionDefinition::admit_with::<ExampleAction>(
            Metadata::new(crate::metadata_name!("Example"), "Changed description"),
            ActionInterface::default(),
            ActionKind::Control,
        )
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

    #[test]
    fn action_readmission_requires_input_and_output_policy_equality() {
        let current = admitted();
        for pointer in ["/base/schema", "/output_schema"] {
            let mut wire = serde_json::to_value(&current).unwrap();
            wire.pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("policy_version");
            let recorded: RecordedActionDefinition = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(&recorded).unwrap(), wire);
            assert!(recorded.readmit_against(&current).is_err());
        }
    }
}
