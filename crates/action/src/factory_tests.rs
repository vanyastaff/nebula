use std::{
    any::TypeId,
    assert_matches,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{
    Dependencies, ExecutionId, ResourceRequirement, WorkflowId, action_key, node_key, resource_key,
};
use nebula_workflow::NodeDefinition;
use serde_json::Value;

use super::*;
use crate::{
    StatelessAction,
    action::Action,
    effect::{RemoteDestinationGuarantee, RemoteEffectDescriptor, RemoteEffectPolicy},
    result::ActionResult,
    testing::TestContextBuilder,
};

// ── InvalidMetadataAction ────────────────────────────────────────────────
//
// A stateless action whose `metadata()` returns deliberately invalid
// action-specific data (empty description). The factory must refuse to
// instantiate it with `ActionError::Fatal` — fail-closed per FIX 3.

struct InvalidMetadataAction;

impl Action for InvalidMetadataAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        // Blank names are rejected at construction; an empty description
        // still exercises the action-package admission guard.
        ActionMetadataDraft::new(
            action_key!("test.invalid_meta"),
            crate::metadata_name!("Invalid metadata"),
            "",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPS: OnceLock<Dependencies> = OnceLock::new();
        DEPS.get_or_init(Dependencies::new)
    }
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
    let ActionMetadataAdmissionError::Package(errors) = error else {
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
// and the default input+output ports from `crate::ActionMetadataDraft::new`. Proves
// that `check_metadata_or_fatal` does NOT reject valid metadata — the
// positive counterpart to the two fatal-path tests above.

struct FactoryDependency;
struct ValidMetadataAction;

impl Action for ValidMetadataAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.valid_meta"),
            crate::metadata_name!("Valid Metadata Action"),
            "A production-shaped action fixture used to prove the metadata gate passes valid registrations.",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPS: OnceLock<Dependencies> = OnceLock::new();
        DEPS.get_or_init(|| {
            Dependencies::new().resource(ResourceRequirement::new(
                resource_key!("factory_dependency"),
                TypeId::of::<FactoryDependency>(),
                std::any::type_name::<FactoryDependency>(),
            ))
        })
    }
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
fn factory_exposes_exact_action_dependencies() {
    let factory = GenericStatelessFactory::<ValidMetadataAction>::new()
        .expect("valid test catalog definition");
    let projected_dependencies = factory.dependencies();
    let [resource_requirement] = projected_dependencies.resources() else {
        panic!("factory must expose the action's one declared resource dependency");
    };

    assert_eq!(
        resource_requirement.key,
        resource_key!("factory_dependency")
    );
    assert_eq!(
        resource_requirement.type_id,
        TypeId::of::<FactoryDependency>()
    );
    assert_eq!(
        resource_requirement.type_name,
        std::any::type_name::<FactoryDependency>()
    );
    assert!(resource_requirement.required);
    assert_eq!(resource_requirement.purpose, None);
    assert!(projected_dependencies.credentials().is_empty());
    assert!(projected_dependencies.slot_fields().is_empty());
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
    let factory_metadata = factory.metadata();

    let handle = factory
        .instantiate(&node, &ctx)
        .await
        .expect("handle construction must succeed");

    assert!(Arc::ptr_eq(factory_metadata, handle.metadata()));
}

struct RemoteValueProbe {
    descriptor: RemoteEffectDescriptor,
    preparations: Arc<AtomicUsize>,
}

impl Action for RemoteValueProbe {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        unreachable!("instance-backed remote admission receives an explicit draft")
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl RemoteEffectAction for RemoteValueProbe {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        &self.descriptor
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
    descriptor: RemoteEffectDescriptor,
    preparations: Arc<AtomicUsize>,
}

impl Action for RemoteBoolProbe {
    type Input = bool;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        unreachable!("instance-backed remote admission receives an explicit draft")
    }

    fn dependencies() -> &'static Dependencies {
        RemoteValueProbe::dependencies()
    }
}

impl RemoteEffectAction for RemoteBoolProbe {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        &self.descriptor
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

fn remote_descriptor() -> RemoteEffectDescriptor {
    let policy = RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
        .maximum_invocations(1)
        .maximum_queries(0)
        .recovery_window(Duration::from_mins(1))
        .build()
        .expect("fixture policy is valid");
    RemoteEffectDescriptor::new("test.remote", 1, policy).expect("fixture descriptor is valid")
}

fn remote_draft(key: &'static str, descriptor: &RemoteEffectDescriptor) -> ActionMetadataDraft {
    ActionMetadataDraft::new(
        nebula_core::ActionKey::new(key).expect("fixture key is valid"),
        crate::metadata_name!("Remote probe"),
        "Remote input contract probe",
    )
    .with_effect_contract(ActionEffectContract::Remote(Box::new(descriptor.clone())))
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
    let descriptor = remote_descriptor();
    let right_preparations = Arc::new(AtomicUsize::new(0));
    let left = RemoteEffectInstanceFactory::new(
        remote_draft("test.remote.left", &descriptor),
        RemoteValueProbe {
            descriptor: descriptor.clone(),
            preparations: Arc::new(AtomicUsize::new(0)),
        },
    )
    .expect("left factory admission succeeds");
    let right = RemoteEffectInstanceFactory::new(
        remote_draft("test.remote.right", &descriptor),
        RemoteValueProbe {
            descriptor,
            preparations: Arc::clone(&right_preparations),
        },
    )
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
    let descriptor = remote_descriptor();
    let right_preparations = Arc::new(AtomicUsize::new(0));
    let left = RemoteEffectInstanceFactory::new(
        remote_draft("test.remote.value", &descriptor),
        RemoteValueProbe {
            descriptor: descriptor.clone(),
            preparations: Arc::new(AtomicUsize::new(0)),
        },
    )
    .expect("value factory admission succeeds");
    let right = RemoteEffectInstanceFactory::new(
        remote_draft("test.remote.bool", &descriptor),
        RemoteBoolProbe {
            descriptor,
            preparations: Arc::clone(&right_preparations),
        },
    )
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
