use std::{
    any::TypeId,
    assert_matches,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ExecutionId, WorkflowId, node_key, resource_key};
use nebula_workflow::NodeDefinition;
use serde_json::Value;

use super::*;
use crate::{
    StatelessAction,
    action::Action,
    effect::{
        ActionEffectContract, EffectContractError, RemoteDestinationGuarantee,
        RemoteEffectDeclarationError, RemoteEffectDescriptor, RemoteEffectPolicy,
    },
    result::ActionResult,
    testing::TestContextBuilder,
};

// ── InvalidMetadataAction ────────────────────────────────────────────────
//
// A stateless action whose `metadata()` returns deliberately invalid
// action-specific data (empty description). The factory must refuse to
// instantiate it with `ActionError::Fatal` — fail-closed per FIX 3.

struct InvalidMetadataAction;

impl crate::Component for InvalidMetadataAction {
    type Kind = crate::kind::Action;
    type Config = Value;

    const KEY: &'static str = "test.invalid_meta";

    fn metadata() -> crate::Metadata {
        // Blank names are rejected at construction; an empty description
        // still exercises the action-package admission guard.
        crate::Metadata::new(crate::metadata_name!("Invalid metadata"), "")
    }
}

impl Action for InvalidMetadataAction {
    type Output = Value;
}

impl StatelessAction for InvalidMetadataAction {
    async fn execute(
        &self,
        _input: Value,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(Value::Null))
    }
}

impl FromWorkflowNode for InvalidMetadataAction {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

#[test]
fn construction_rejects_invalid_metadata() {
    let Err(error) = GenericStatelessFactory::<InvalidMetadataAction>::new() else {
        panic!("invalid metadata cannot produce a factory");
    };
    let ActionAdmissionError::Package(errors) = error else {
        panic!("empty description must be a package admission error");
    };
    assert_matches!(
        errors.errors(),
        [crate::ActionPackageValidationError::EmptyMetadataField {
            field: "description"
        }]
    );
}

#[test]
fn invalid_metadata_never_reaches_instantiation() {
    assert!(GenericStatelessFactory::<InvalidMetadataAction>::new().is_err());
}

// ── ValidMetadataAction ──────────────────────────────────────────────────
//
// A production-shaped stateless action with a non-empty name, description,
// and the default input+output ports from `crate::Metadata::new`. Proves
// that `check_metadata_or_fatal` does NOT reject valid metadata — the
// positive counterpart to the two fatal-path tests above.

struct FactoryDependency;

impl crate::Component for FactoryDependency {
    type Kind = crate::kind::Resource;
    type Config = ();

    const KEY: &'static str = "factory_dependency";

    fn metadata() -> crate::Metadata {
        crate::Metadata::new(crate::metadata_name!("Factory dependency"), "")
    }
}

/// Stands in for a resource handle: the value a slot on the resource holds.
struct FactoryDependencyHandle;

impl crate::SlotValue for FactoryDependencyHandle {
    type Target = FactoryDependency;
}

struct ValidMetadataAction;

impl crate::Component for ValidMetadataAction {
    type Kind = crate::kind::Action;
    type Config = Value;

    const KEY: &'static str = "test.valid_meta";

    fn metadata() -> crate::Metadata {
        crate::Metadata::new(
            crate::metadata_name!("Valid Metadata Action"),
            "A production-shaped action fixture used to prove the metadata gate passes valid registrations.",
        )
    }

    fn dependencies() -> crate::Dependencies {
        crate::Dependencies::none().slot(crate::Slot::of::<FactoryDependencyHandle>("dependency"))
    }
}

impl Action for ValidMetadataAction {
    type Output = Value;
}

impl StatelessAction for ValidMetadataAction {
    async fn execute(
        &self,
        _input: Value,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(Value::Null))
    }
}

impl FromWorkflowNode for ValidMetadataAction {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

#[test]
fn the_definition_carries_the_action_slots() {
    let factory = GenericStatelessFactory::<ValidMetadataAction>::new()
        .expect("valid test catalog definition");
    let [slot] = factory.definition().dependencies().slots() else {
        panic!("the definition must carry the action's one declared slot");
    };

    assert_eq!(slot.key(), "dependency");
    assert!(slot.is_required());
    assert_eq!(slot.purpose(), None);
    let nebula_component::SlotTarget::Resource(target) = slot.target() else {
        panic!("the slot points at a resource");
    };
    assert_eq!(target.key(), &resource_key!("factory_dependency"));
    assert_eq!(target.type_id(), TypeId::of::<FactoryDependency>());
    assert_eq!(
        target.type_name(),
        std::any::type_name::<FactoryDependency>()
    );
}

/// A factory with valid metadata (non-empty name, description, default
/// ports) must succeed — `check_metadata_or_fatal` must not block valid
/// registrations (positive counterpart to the two fatal-path tests).
#[tokio::test]
async fn instantiate_succeeds_with_valid_metadata() {
    let factory = GenericStatelessFactory::<ValidMetadataAction>::new()
        .expect("valid test catalog definition");
    let node = NodeDefinition::new(
        node_key!("test_node"),
        "Test Node",
        "nebula.test",
        "test.valid_meta",
    )
    .expect("stub node key is valid");
    let ctx = TestContextBuilder::new().build();

    let result = factory.instantiate(&node, &ctx).await;

    assert!(
        result.is_ok(),
        "valid metadata must not produce a Fatal error; got: {result:?}",
    );
    assert!(
        matches!(result.unwrap(), ActionHandle::Stateless(_)),
        "valid metadata action should produce a Stateless handle",
    );
}

#[tokio::test]
async fn instantiated_handle_shares_factory_metadata_allocation() {
    let factory = GenericStatelessFactory::<ValidMetadataAction>::new()
        .expect("valid test catalog definition");
    let node = NodeDefinition::new(
        node_key!("test_node"),
        "Test Node",
        "nebula.test",
        "test.valid_meta",
    )
    .expect("stub node key is valid");
    let ctx = TestContextBuilder::new().build();
    let factory_metadata = factory.definition();

    let handle = factory
        .instantiate(&node, &ctx)
        .await
        .expect("handle construction must succeed");

    assert!(Arc::ptr_eq(factory_metadata, handle.definition()));
}

struct RemoteValueProbe {
    preparations: Arc<AtomicUsize>,
}

impl crate::Component for RemoteValueProbe {
    type Kind = crate::kind::Action;
    type Config = Value;

    const KEY: &'static str = "test.remote.value";

    fn metadata() -> crate::Metadata {
        remote_metadata()
    }
}

impl Action for RemoteValueProbe {
    type Output = Value;
}

impl RemoteEffectAction for RemoteValueProbe {
    fn descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
        remote_descriptor()
    }

    async fn prepare(
        &self,
        _input: Value,
        _context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        self.preparations.fetch_add(1, Ordering::SeqCst);
        Err(EffectPreparationError::Unavailable)
    }
}

struct RemoteBoolProbe {
    preparations: Arc<AtomicUsize>,
}

impl crate::Component for RemoteBoolProbe {
    type Kind = crate::kind::Action;
    type Config = bool;

    const KEY: &'static str = "test.remote.bool";

    fn metadata() -> crate::Metadata {
        remote_metadata()
    }
}

impl Action for RemoteBoolProbe {
    type Output = Value;
}

impl RemoteEffectAction for RemoteBoolProbe {
    fn descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
        remote_descriptor()
    }

    async fn prepare(
        &self,
        _input: bool,
        _context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        self.preparations.fetch_add(1, Ordering::SeqCst);
        Err(EffectPreparationError::Unavailable)
    }
}

fn remote_descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
    let policy = RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
        .maximum_invocations(1)
        .maximum_queries(0)
        .recovery_window(Duration::from_mins(1))
        .build()?;
    Ok(RemoteEffectDescriptor::new("test.remote", 1, policy)?)
}

fn remote_metadata() -> crate::Metadata {
    crate::Metadata::new(
        crate::metadata_name!("Remote probe"),
        "Remote input contract probe",
    )
}

fn effect_context() -> EffectPreparationContext {
    EffectPreparationContext::new(
        ExecutionId::new(),
        WorkflowId::new(),
        node_key!("remote"),
        nebula_core::OrgId::new(),
        nebula_core::WorkspaceId::new(),
    )
}

#[tokio::test]
async fn equal_schema_remote_contract_rejects_foreign_factory_token() {
    let right_preparations = Arc::new(AtomicUsize::new(0));
    let left = RemoteEffectInstanceFactory::new(RemoteValueProbe {
        preparations: Arc::new(AtomicUsize::new(0)),
    })
    .expect("left factory admission succeeds");
    let right = RemoteEffectInstanceFactory::new(RemoteValueProbe {
        preparations: Arc::clone(&right_preparations),
    })
    .expect("right factory admission succeeds");
    let prepared = RemoteEffectFactory::prepare_input(
        &left,
        ActionInput::Raw(serde_json::json!({"value": 1})),
    )
    .expect("left contract accepts its input");

    let error = RemoteEffectFactory::prepare(&right, prepared, &effect_context())
        .await
        .expect_err("equal schemas do not confer factory identity");

    assert_eq!(error, EffectPreparationError::InvalidRequest);
    assert_eq!(right_preparations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn different_schema_remote_contract_rejects_foreign_factory_token() {
    let right_preparations = Arc::new(AtomicUsize::new(0));
    let left = RemoteEffectInstanceFactory::new(RemoteValueProbe {
        preparations: Arc::new(AtomicUsize::new(0)),
    })
    .expect("value factory admission succeeds");
    let right = RemoteEffectInstanceFactory::new(RemoteBoolProbe {
        preparations: Arc::clone(&right_preparations),
    })
    .expect("bool factory admission succeeds");
    let prepared =
        RemoteEffectFactory::prepare_input(&left, ActionInput::Raw(serde_json::json!(true)))
            .expect("value factory accepts boolean JSON as a JSON value");

    let error = RemoteEffectFactory::prepare(&right, prepared, &effect_context())
        .await
        .expect_err("schema mismatch must fail before remote preparation");

    assert_eq!(error, EffectPreparationError::InvalidRequest);
    assert_eq!(right_preparations.load(Ordering::SeqCst), 0);
}

/// Remote fixture that declares an effect contract of its own.
///
/// Only `Journaled` and `ReadOnly` can be declared: `EFFECT_CONTRACT` is a
/// constant and a remote contract holds a boxed descriptor, so a remote
/// contract declared on the type is unrepresentable.
struct RemoteDeclaringReadOnly;

impl crate::Component for RemoteDeclaringReadOnly {
    type Kind = crate::kind::Action;
    type Config = Value;

    const KEY: &'static str = "test.remote.declared";

    fn metadata() -> crate::Metadata {
        remote_metadata()
    }
}

impl Action for RemoteDeclaringReadOnly {
    type Output = Value;

    const EFFECT_CONTRACT: ActionEffectContract = ActionEffectContract::ReadOnly;
}

impl RemoteEffectAction for RemoteDeclaringReadOnly {
    fn descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
        remote_descriptor()
    }

    async fn prepare(
        &self,
        _input: Value,
        _context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        Err(EffectPreparationError::Unavailable)
    }
}

/// Remote fixture whose descriptor is invalid.
struct RemoteInvalidDescriptor;

impl crate::Component for RemoteInvalidDescriptor {
    type Kind = crate::kind::Action;
    type Config = Value;

    const KEY: &'static str = "test.remote.invalid";

    fn metadata() -> crate::Metadata {
        remote_metadata()
    }
}

impl Action for RemoteInvalidDescriptor {
    type Output = Value;
}

impl RemoteEffectAction for RemoteInvalidDescriptor {
    fn descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
        let policy = RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
            .maximum_invocations(1)
            .maximum_queries(0)
            .recovery_window(Duration::from_mins(1))
            .build()?;
        Ok(RemoteEffectDescriptor::new("", 1, policy)?)
    }

    async fn prepare(
        &self,
        _input: Value,
        _context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        Err(EffectPreparationError::Unavailable)
    }
}

#[test]
fn remote_factory_stamps_the_type_descriptor_as_the_effect_contract() {
    let factory = RemoteEffectInstanceFactory::new(RemoteValueProbe {
        preparations: Arc::new(AtomicUsize::new(0)),
    })
    .expect("remote fixture admits");
    let descriptor = remote_descriptor().expect("fixture descriptor is valid");

    assert_eq!(
        ActionFactory::definition(&factory).effect_contract(),
        &ActionEffectContract::Remote(Box::new(descriptor.clone()))
    );
    assert_eq!(RemoteEffectFactory::descriptor(&factory), &descriptor);
    assert_eq!(
        ActionFactory::definition(&factory).kind(),
        ActionKind::Stateless
    );
}

#[test]
fn remote_factory_refuses_an_action_that_declares_an_effect_contract() {
    assert_matches!(
        RemoteEffectInstanceFactory::new(RemoteDeclaringReadOnly).map(drop),
        Err(ActionAdmissionError::EffectContractOnRemoteAction)
    );
}

#[test]
fn remote_factory_refuses_an_invalid_descriptor() {
    assert_matches!(
        RemoteEffectInstanceFactory::new(RemoteInvalidDescriptor).map(drop),
        Err(ActionAdmissionError::RemoteEffectDeclaration(
            RemoteEffectDeclarationError::Contract(EffectContractError::InvalidIdentifier)
        ))
    );
}
