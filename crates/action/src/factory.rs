//! `ActionFactory` — engine-side object-safe per-execution factory.
//!
//! The engine's
//! `ActionRegistry` keeps `Arc<dyn ActionFactory>` per `ActionKey`. On
//! each dispatch, the registry calls
//! [`instantiate`](ActionFactory::instantiate) with the current
//! [`NodeDefinition`](nebula_workflow::NodeDefinition) + an
//! [`ActionContext`](crate::ActionContext); the factory builds a fresh
//! [`ActionHandle`](crate::ActionHandle) ready for dispatch.
//!
//! The default `GenericStatelessFactory<A>` / `GenericStatefulFactory<A>` /
//! `GenericTriggerFactory<A>` / `GenericResourceFactory<A>` /
//! `GenericControlFactory<A>` types wrap any `A: Action + FromWorkflowNode`
//! into an [`ActionFactory`] by routing through
//! [`FromWorkflowNode::from_workflow_node`](crate::FromWorkflowNode::from_workflow_node)
//! and then erasing to the matching [`ActionHandle`] variant.

use std::{any::Any, future::Future, marker::PhantomData, pin::Pin, sync::Arc};

use async_trait::async_trait;
use nebula_core::Dependencies;
use nebula_workflow::NodeDefinition;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use futures::StreamExt as _;

use crate::{
    action::Action,
    context::{ActionContext, TriggerContext},
    control::ControlAction,
    effect::{
        ActionEffectContract, EffectPreparationContext, EffectPreparationError,
        PreparedRemoteEffect, RemoteEffectAction, RemoteEffectFactory,
    },
    error::ActionError,
    from_workflow_node::FromWorkflowNode,
    handle::{
        ActionHandle, ControlHandle, ResourceHandle, StatelessHandle, StreamHandle, TriggerHandle,
    },
    input::{ActionInput, ActionInputContract, PreparedActionInput},
    metadata::{ActionKind, ActionMetadata, ActionMetadataAdmissionError, ActionMetadataDraft},
    resource::ResourceAction,
    result::ActionResult,
    stateful::{StatefulAction, StatefulActionAdapter},
    stateless::StatelessAction,
    stream::StreamAction,
    trigger::{TriggerAction, TriggerEvent, TriggerEventOutcome},
};

fn admit_metadata<A: Action>(
    draft: ActionMetadataDraft,
    kind: ActionKind,
) -> Result<Arc<ActionMetadata>, ActionMetadataAdmissionError> {
    draft.admit_for::<A>(kind).map(Arc::new)
}

mod sealed {
    pub trait Sealed {}
}

/// Object-safe factory trait — engine registry stores `Arc<dyn ActionFactory>`.
///
/// `instantiate` returns a `Pin<Box<dyn Future<...>>>` so the trait remains
/// object-safe (vs `impl Future` which is not). The lifetime borrows
/// `node` and `ctx` for the duration of the future — typical engine
/// dispatch awaits the future to completion before moving on.
///
/// # Errors
///
/// Returns [`ActionError::Fatal`] if slot resolution fails or the factory
/// otherwise cannot construct an executable action for this dispatch.
pub trait ActionFactory: sealed::Sealed + Send + Sync + 'static {
    /// Static metadata describing the action this factory produces.
    fn metadata(&self) -> &Arc<ActionMetadata>;

    /// Declared resource and credential dependencies for the produced action.
    fn dependencies(&self) -> &Dependencies;

    /// Separate preparation capability for an explicitly declared remote effect.
    ///
    /// Its descriptor must exactly match metadata before any instantiation.
    /// Durable remote execution never calls the generic `instantiate` path.
    fn remote_effect_factory(&self) -> Option<&dyn RemoteEffectFactory> {
        None
    }

    /// Build an [`ActionHandle`] for the given workflow node + context.
    #[must_use = "the instantiated action handle must be dispatched, not discarded"]
    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>>;
}

// ── Stateless ──────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Stateless`] for any type
/// implementing [`StatelessAction`] + [`FromWorkflowNode`].
pub struct GenericStatelessFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericStatelessFactory<A> {}

impl<A: Action> GenericStatelessFactory<A> {
    /// Construct a new stateless factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Stateless)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericStatelessFactory<A>
where
    A: StatelessAction + FromWorkflowNode<Error = ActionError>,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let meta = Arc::clone(&self.meta);
            let inner = StatelessHandleImpl::<A>::new(action, meta);
            Ok(ActionHandle::Stateless(Box::new(inner)))
        })
    }
}

struct StatelessHandleImpl<A> {
    action: A,
    meta: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A> crate::handle::sealed::Stateless for StatelessHandleImpl<A> {}

impl<A> StatelessHandleImpl<A> {
    fn new(action: A, meta: Arc<ActionMetadata>) -> Self {
        let input_contract = ActionInputContract::new(meta.base().schema());
        Self {
            action,
            meta,
            input_contract,
        }
    }
}

#[async_trait]
impl<A> StatelessHandle for StatelessHandleImpl<A>
where
    A: StatelessAction,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    async fn dispatch(
        &self,
        input: PreparedActionInput,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input = input.into_typed::<A::Input>(&self.input_contract)?;

        let result = self.action.execute(typed_input, ctx).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── InstanceFactory (instance-backed stateless factory) ─────────────────────

/// Stateless [`ActionFactory`] backed by a pre-built action **instance** plus
/// caller-supplied [`ActionMetadataDraft`], instead of building the action from the
/// workflow node like [`GenericStatelessFactory`].
///
/// This is the `useValue` half of the dependency-injection dichotomy
/// (a pre-instantiated value) to [`GenericStatelessFactory`]'s `useFactory`
/// half (construct-on-demand): the same action instance is shared across every
/// dispatch, and the catalog metadata is supplied per registration.
///
/// Two properties distinguish it from the generic factory:
///
/// - **Caller metadata.** The metadata is supplied per registration, so one
///   action type can back many distinct catalog keys / port shapes. The generic
///   factory derives a single static [`Action::metadata`] from the type and so
///   binds one type to one key.
/// - **Shared instance.** It holds the action in an `Arc` and hands every
///   dispatch a handle over the *same* instance, so interior state (counters,
///   spies, caches) is shared across dispatches — matching a directly
///   constructed handler. The generic factory rebuilds a fresh action per
///   dispatch via [`FromWorkflowNode`].
///
/// The produced [`ActionHandle::Stateless`] dispatches through the same engine
/// path as any other factory — `InstanceFactory` is a first-class member of the
/// factory spine, not a wrapper around a separate dispatch path.
pub struct InstanceFactory<A> {
    action: Arc<A>,
    meta: Arc<ActionMetadata>,
}

impl<A> sealed::Sealed for InstanceFactory<A> {}

impl<A: Action> InstanceFactory<A> {
    /// Wrap a pre-built action instance with explicit metadata intent.
    ///
    /// The metadata's [`kind`](ActionMetadata::kind) is stamped to
    /// [`ActionKind::Stateless`] and `output_schema` is stamped from
    /// `<A::Output as HasSchema>::schema()` — the factory is the single writer
    /// of both fields — while every other field is preserved as the caller
    /// supplied it.
    ///
    /// # Errors
    /// Returns a typed catalog error if metadata or an associated schema is invalid.
    #[tracing::instrument(name = "action.metadata.admit", skip_all, err)]
    pub fn new(
        metadata: ActionMetadataDraft,
        action: A,
    ) -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            action: Arc::new(action),
            meta: admit_metadata::<A>(metadata, ActionKind::Stateless)?,
        })
    }
}

impl<A> ActionFactory for InstanceFactory<A>
where
    A: StatelessAction,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        let inner = InstanceStatelessHandle {
            action: Arc::clone(&self.action),
            meta: Arc::clone(&self.meta),
            input_contract: ActionInputContract::new(self.meta.base().schema()),
        };
        Box::pin(async move { Ok(ActionHandle::Stateless(Box::new(inner))) })
    }
}

/// Factory for a pre-built typed remote-effect action.
///
/// This is the only constructor for a remote execution capability. Admission
/// derives schemas from `A`, stamps the stateless structural kind, verifies the
/// exact effect descriptor, and creates a private ingress identity retained by
/// this factory.
pub struct RemoteEffectInstanceFactory<A: Action> {
    action: A,
    meta: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A: Action> sealed::Sealed for RemoteEffectInstanceFactory<A> {}

impl<A> RemoteEffectInstanceFactory<A>
where
    A: RemoteEffectAction,
{
    /// Admit a typed remote-effect action and bind its ingress capability.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure for invalid metadata, a missing
    /// remote contract, or a descriptor mismatch.
    pub fn new(
        metadata: ActionMetadataDraft,
        action: A,
    ) -> Result<Self, ActionMetadataAdmissionError> {
        let meta = admit_metadata::<A>(metadata, ActionKind::Stateless)?;
        let ActionEffectContract::Remote(descriptor) = meta.effect_contract() else {
            return Err(ActionMetadataAdmissionError::RemoteEffectContractRequired);
        };
        if descriptor.as_ref() != action.descriptor() {
            return Err(ActionMetadataAdmissionError::RemoteEffectDescriptorMismatch);
        }
        let input_contract = ActionInputContract::new(meta.base().schema());
        Ok(Self {
            action,
            meta,
            input_contract,
        })
    }
}

impl<A> ActionFactory for RemoteEffectInstanceFactory<A>
where
    A: RemoteEffectAction,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn dependencies(&self) -> &Dependencies {
        A::dependencies()
    }

    fn remote_effect_factory(&self) -> Option<&dyn RemoteEffectFactory> {
        Some(self)
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async {
            Err(ActionError::fatal(
                "remote-effect actions require execution-owner dispatch",
            ))
        })
    }
}

impl<A: RemoteEffectAction> crate::effect::remote_effect_sealed::Sealed
    for RemoteEffectInstanceFactory<A>
{
}

#[async_trait]
impl<A> RemoteEffectFactory for RemoteEffectInstanceFactory<A>
where
    A: RemoteEffectAction,
{
    fn descriptor(&self) -> &crate::RemoteEffectDescriptor {
        self.action.descriptor()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    async fn prepare(
        &self,
        input: PreparedActionInput,
        context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        let input = input
            .into_typed::<A::Input>(&self.input_contract)
            .map_err(|_| EffectPreparationError::InvalidRequest)?;
        self.action.prepare(input, context).await
    }
}

struct InstanceStatelessHandle<A> {
    action: Arc<A>,
    meta: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A> crate::handle::sealed::Stateless for InstanceStatelessHandle<A> {}

#[async_trait]
impl<A> StatelessHandle for InstanceStatelessHandle<A>
where
    A: StatelessAction,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    async fn dispatch(
        &self,
        input: PreparedActionInput,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input = input.into_typed::<A::Input>(&self.input_contract)?;

        let result = self.action.execute(typed_input, ctx).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── Stateful ───────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Stateful`] for any type
/// implementing [`StatefulAction`] + [`FromWorkflowNode`].
pub struct GenericStatefulFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericStatefulFactory<A> {}

impl<A: Action> GenericStatefulFactory<A> {
    /// Construct a new stateful factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Stateful)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericStatefulFactory<A>
where
    A: StatefulAction + FromWorkflowNode<Error = ActionError>,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
    A::State: Serialize + DeserializeOwned + Clone + Send + Sync,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let inner = StatefulActionAdapter::with_metadata(action, Arc::clone(&self.meta));
            Ok(ActionHandle::Stateful(Box::new(inner)))
        })
    }
}

// ── Trigger ────────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Trigger`] for any type
/// implementing [`TriggerAction`] + [`FromWorkflowNode`].
pub struct GenericTriggerFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericTriggerFactory<A> {}

impl<A: Action> GenericTriggerFactory<A> {
    /// Construct a new trigger factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Trigger)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericTriggerFactory<A>
where
    A: TriggerAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
    <A as TriggerAction>::Error: Into<ActionError>,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let inner = TriggerHandleImpl::<A>::new(action, Arc::clone(&self.meta));
            Ok(ActionHandle::Trigger(Box::new(inner)))
        })
    }
}

struct TriggerHandleImpl<A> {
    action: A,
    meta: Arc<ActionMetadata>,
}

impl<A> crate::handle::sealed::Trigger for TriggerHandleImpl<A> {}

impl<A> TriggerHandleImpl<A> {
    fn new(action: A, meta: Arc<ActionMetadata>) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> TriggerHandle for TriggerHandleImpl<A>
where
    A: TriggerAction + Send + Sync + 'static,
    <A as TriggerAction>::Error: Into<ActionError>,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    async fn start(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError> {
        self.action.start(ctx).await.map_err(Into::into)
    }

    async fn stop(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError> {
        self.action.stop(ctx).await.map_err(Into::into)
    }

    fn accepts_events(&self) -> bool {
        self.action.accepts_events()
    }

    async fn handle_event(
        &self,
        event: TriggerEvent,
        ctx: &dyn TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        // The trigger receives a typed payload; downcast at the boundary.
        let (_id, _received_at, typed_event) = event
            .downcast::<<A::Source as crate::trigger::TriggerSource>::Event>()
            .map_err(|original| {
                ActionError::fatal(format!(
                    "trigger event payload type mismatch: expected {}, got {}",
                    std::any::type_name::<<A::Source as crate::trigger::TriggerSource>::Event>(),
                    original.payload_type_name(),
                ))
            })?;
        self.action
            .handle(ctx, typed_event)
            .await
            .map_err(Into::into)
    }
}

// ── Resource ───────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Resource`] for any type
/// implementing [`ResourceAction`] + [`FromWorkflowNode`].
pub struct GenericResourceFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericResourceFactory<A> {}

impl<A: Action> GenericResourceFactory<A> {
    /// Construct a new resource factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Resource)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericResourceFactory<A>
where
    A: ResourceAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let inner = ResourceHandleImpl::<A>::new(action, Arc::clone(&self.meta));
            Ok(ActionHandle::Resource(Box::new(inner)))
        })
    }
}

struct ResourceHandleImpl<A> {
    action: A,
    meta: Arc<ActionMetadata>,
}

impl<A> crate::handle::sealed::Resource for ResourceHandleImpl<A> {}

impl<A> ResourceHandleImpl<A> {
    fn new(action: A, meta: Arc<ActionMetadata>) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> ResourceHandle for ResourceHandleImpl<A>
where
    A: ResourceAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    async fn configure(
        &self,
        _config: Value,
        ctx: &dyn ActionContext,
    ) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        let resource = self.action.configure(ctx).await?;
        let boxed: Box<dyn Any + Send + Sync> = Box::new(resource);
        Ok(boxed)
    }

    async fn cleanup(
        &self,
        instance: Box<dyn Any + Send + Sync>,
        ctx: &dyn ActionContext,
    ) -> Result<(), ActionError> {
        let typed = instance.downcast::<A::Resource>().map_err(|_| {
            ActionError::fatal(format!(
                "ResourceHandleImpl: downcast invariant violated for {}",
                std::any::type_name::<A::Resource>()
            ))
        })?;
        self.action.cleanup(*typed, ctx).await
    }
}

// ── Control ────────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Control`] for any type
/// implementing [`ControlAction`] + [`FromWorkflowNode`].
pub struct GenericControlFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericControlFactory<A> {}

impl<A: Action> GenericControlFactory<A> {
    /// Construct a new control factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Control)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericControlFactory<A>
where
    A: ControlAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let inner = ControlHandleImpl::<A>::new(action, Arc::clone(&self.meta));
            Ok(ActionHandle::Control(Box::new(inner)))
        })
    }
}

struct ControlHandleImpl<A> {
    action: A,
    meta: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A> crate::handle::sealed::Control for ControlHandleImpl<A> {}

impl<A> ControlHandleImpl<A> {
    fn new(action: A, meta: Arc<ActionMetadata>) -> Self {
        let input_contract = ActionInputContract::new(meta.base().schema());
        Self {
            action,
            meta,
            input_contract,
        }
    }
}

#[async_trait]
impl<A> ControlHandle for ControlHandleImpl<A>
where
    A: ControlAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    async fn dispatch(
        &self,
        input: PreparedActionInput,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let input = input.into_typed::<A::Input>(&self.input_contract)?;
        let outcome: ActionResult<A::Output> = self.action.evaluate(input, ctx).await?.into();
        outcome.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|_| ActionError::fatal("control output cannot be serialized as declared"))
        })
    }
}

// ── Stream ─────────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Stream`] for any type
/// implementing [`StreamAction`] + [`FromWorkflowNode`].
///
/// The factory stamps [`ActionKind::Stream`] as the single writer of the kind
/// on the stored metadata. The `StreamHandleImpl` adapter drives the chunk
/// stream fully in-process and delivers a single folded value — identical to
/// stateless from the engine's perspective.
pub struct GenericStreamFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericStreamFactory<A> {}

impl<A: Action> GenericStreamFactory<A> {
    /// Construct a new stream factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Stream)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericStreamFactory<A>
where
    A: StreamAction + FromWorkflowNode<Error = ActionError>,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let inner = StreamHandleImpl::<A>::new(action, Arc::clone(&self.meta));
            Ok(ActionHandle::Stream(Box::new(inner)))
        })
    }
}

struct StreamHandleImpl<A> {
    action: A,
    meta: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A> crate::handle::sealed::Stream for StreamHandleImpl<A> {}

impl<A> StreamHandleImpl<A> {
    fn new(action: A, meta: Arc<ActionMetadata>) -> Self {
        let input_contract = ActionInputContract::new(meta.base().schema());
        Self {
            action,
            meta,
            input_contract,
        }
    }
}

#[async_trait]
impl<A> StreamHandle for StreamHandleImpl<A>
where
    A: StreamAction,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    #[tracing::instrument(
        name = "stream_handle.dispatch",
        skip_all,
        fields(
            action.key = %self.meta.base().key().as_str(),
            action.kind = "stream",
        )
    )]
    async fn dispatch(
        &self,
        input: PreparedActionInput,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input = input.into_typed::<A::Input>(&self.input_contract)?;

        let chunk_stream = self.action.open_stream(typed_input, ctx);
        tokio::pin!(chunk_stream);

        let mut accumulator = self.action.init();

        while let Some(chunk_result) = chunk_stream.next().await {
            // D-4: first Err short-circuits with no partial output.
            let chunk = chunk_result?;
            accumulator = self.action.fold(accumulator, chunk);
        }

        let output_value = serde_json::to_value(accumulator)
            .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))?;

        Ok(ActionResult::success(output_value))
    }
}

// ── Agent ──────────────────────────────────────────────────────────────────

/// Generic factory that produces [`ActionHandle::Agent`] for any type
/// implementing [`crate::agent::AgentAction`] + [`FromWorkflowNode`].
///
/// The factory stamps [`ActionKind::Agent`] as the single writer of the kind
/// on the stored metadata. The adapter inside the handle drives the turn loop
/// in the engine, deserializing/serializing turn state on each call to `step`.
pub struct GenericAgentFactory<A> {
    meta: Arc<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> sealed::Sealed for GenericAgentFactory<A> {}

impl<A: Action> GenericAgentFactory<A> {
    /// Construct a new agent factory.
    ///
    /// # Errors
    ///
    /// Returns a typed admission failure when associated schemas or authored
    /// package declarations are invalid.
    pub fn new() -> Result<Self, ActionMetadataAdmissionError> {
        Ok(Self {
            meta: admit_metadata::<A>(A::metadata(), ActionKind::Agent)?,
            _phantom: PhantomData,
        })
    }
}

impl<A> ActionFactory for GenericAgentFactory<A>
where
    A: crate::agent::AgentAction + FromWorkflowNode<Error = ActionError>,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
    A::Turn: Serialize + DeserializeOwned + Clone + Send + Sync,
{
    fn dependencies(&self) -> &Dependencies {
        <A as Action>::dependencies()
    }

    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let adapter = crate::agent::AgentActionAdapter::<A>::with_metadata(
                action,
                Arc::clone(&self.meta),
            );
            Ok(ActionHandle::Agent(Box::new(adapter)))
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        Dependencies, ExecutionId, ResourceRequirement, WorkflowId, action_key, node_key,
        resource_key,
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
}
