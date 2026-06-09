//! `ActionFactory` — engine-side object-safe per-execution factory.
//!
//! The engine's
//! `ActionRegistry` keeps `Arc<dyn ActionFactory>` per `ActionKey`. On
//! each dispatch, the registry calls
//! [`instantiate`](ActionFactory::instantiate) with the current
//! [`NodeDefinition`](nebula_workflow::NodeDefinition) + an
//! [`ActionContext`](crate::ActionContext); the factory builds a fresh
//! [`ErasedAction`](crate::ErasedAction) ready for dispatch.
//!
//! The default `GenericStatelessFactory<A>` / `GenericStatefulFactory<A>` /
//! `GenericTriggerFactory<A>` / `GenericResourceFactory<A>` /
//! `GenericControlFactory<A>` types wrap any `A: Action + FromWorkflowNode`
//! into an [`ActionFactory`] by routing through
//! [`FromWorkflowNode::from_workflow_node`](crate::FromWorkflowNode::from_workflow_node)
//! and then erasing to the matching [`ErasedAction`] variant.

use std::{any::Any, future::Future, marker::PhantomData, pin::Pin, sync::OnceLock};

use async_trait::async_trait;
use nebula_workflow::NodeDefinition;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    action::Action,
    context::{ActionContext, TriggerContext},
    control::{ControlAction, ControlInput},
    erased::{
        ErasedAction, ErasedControl, ErasedResource, ErasedStateful, ErasedStateless, ErasedTrigger,
    },
    error::{ActionError, ValidationReason},
    from_workflow_node::FromWorkflowNode,
    metadata::ActionMetadata,
    resource::ResourceAction,
    result::ActionResult,
    stateful::StatefulAction,
    stateless::StatelessAction,
    trigger::{TriggerAction, TriggerEvent, TriggerEventOutcome},
};

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
pub trait ActionFactory: Send + Sync + 'static {
    /// Static metadata describing the action this factory produces.
    fn metadata(&self) -> &ActionMetadata;

    /// Build an [`ErasedAction`] for the given workflow node + context.
    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>>;
}

// ── Stateless ──────────────────────────────────────────────────────────────

/// Generic factory that produces [`ErasedAction::Stateless`] for any type
/// implementing [`StatelessAction`] + [`FromWorkflowNode`].
pub struct GenericStatelessFactory<A> {
    meta: OnceLock<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> Default for GenericStatelessFactory<A> {
    fn default() -> Self {
        Self {
            meta: OnceLock::new(),
            _phantom: PhantomData,
        }
    }
}

impl<A> GenericStatelessFactory<A> {
    /// Construct a new stateless factory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<A> ActionFactory for GenericStatelessFactory<A>
where
    A: StatelessAction + FromWorkflowNode<Error = ActionError>,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.meta.get_or_init(<A as Action>::metadata)
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let meta = self.metadata().clone();
            let inner = ErasedStatelessImpl::<A>::new(action, meta);
            Ok(ErasedAction::Stateless(Box::new(inner)))
        })
    }
}

struct ErasedStatelessImpl<A> {
    action: A,
    meta: ActionMetadata,
}

impl<A> ErasedStatelessImpl<A> {
    fn new(action: A, meta: ActionMetadata) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> ErasedStateless for ErasedStatelessImpl<A>
where
    A: StatelessAction,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    async fn dispatch(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input: <A as Action>::Input = serde_json::from_value(input).map_err(|e| {
            ActionError::validation(
                "input",
                ValidationReason::MalformedJson,
                Some(e.to_string()),
            )
        })?;

        let result = self.action.execute(typed_input, ctx).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── Stateful ───────────────────────────────────────────────────────────────

/// Generic factory that produces [`ErasedAction::Stateful`] for any type
/// implementing [`StatefulAction`] + [`FromWorkflowNode`].
pub struct GenericStatefulFactory<A> {
    meta: OnceLock<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> Default for GenericStatefulFactory<A> {
    fn default() -> Self {
        Self {
            meta: OnceLock::new(),
            _phantom: PhantomData,
        }
    }
}

impl<A> GenericStatefulFactory<A> {
    /// Construct a new stateful factory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<A> ActionFactory for GenericStatefulFactory<A>
where
    A: StatefulAction + FromWorkflowNode<Error = ActionError>,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
    A::State: Serialize + DeserializeOwned + Clone + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.meta.get_or_init(<A as Action>::metadata)
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let meta = self.metadata().clone();
            let inner = ErasedStatefulImpl::<A>::new(action, meta);
            Ok(ErasedAction::Stateful(Box::new(inner)))
        })
    }
}

struct ErasedStatefulImpl<A> {
    action: A,
    meta: ActionMetadata,
}

impl<A> ErasedStatefulImpl<A> {
    fn new(action: A, meta: ActionMetadata) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> ErasedStateful for ErasedStatefulImpl<A>
where
    A: StatefulAction,
    <A as Action>::Input: DeserializeOwned + Send + Sync,
    <A as Action>::Output: Serialize + Send + Sync,
    A::State: Serialize + DeserializeOwned + Clone + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    fn init_state(&self) -> Result<Value, ActionError> {
        serde_json::to_value(self.action.init_state())
            .map_err(|e| ActionError::fatal(format!("init_state serialization failed: {e}")))
    }

    fn migrate_state(&self, old: Value) -> Option<Value> {
        self.action
            .migrate_state(old)
            .and_then(|state| serde_json::to_value(state).ok())
    }

    async fn dispatch(
        &self,
        input: &Value,
        state: &mut Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input: <A as Action>::Input =
            serde_json::from_value(input.clone()).map_err(|e| {
                ActionError::validation(
                    "input",
                    ValidationReason::MalformedJson,
                    Some(e.to_string()),
                )
            })?;

        let mut typed_state: A::State = match serde_json::from_value::<A::State>(state.clone()) {
            Ok(s) => s,
            Err(e) => self.action.migrate_state(state.clone()).ok_or_else(|| {
                ActionError::validation(
                    "state",
                    ValidationReason::StateDeserialization,
                    Some(e.to_string()),
                )
            })?,
        };

        let action_result = self
            .action
            .execute(typed_input, &mut typed_state, ctx)
            .await;

        match (serde_json::to_value(&typed_state), &action_result) {
            (Ok(new_state), _) => *state = new_state,
            (Err(ser_err), Ok(_)) => {
                return Err(ActionError::fatal(format!(
                    "state serialization failed: {ser_err}"
                )));
            },
            (Err(_), Err(_)) => {
                // On error path, propagate original error; checkpoint lost.
            },
        }

        let result = action_result?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── Trigger ────────────────────────────────────────────────────────────────

/// Generic factory that produces [`ErasedAction::Trigger`] for any type
/// implementing [`TriggerAction`] + [`FromWorkflowNode`].
pub struct GenericTriggerFactory<A> {
    meta: OnceLock<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> Default for GenericTriggerFactory<A> {
    fn default() -> Self {
        Self {
            meta: OnceLock::new(),
            _phantom: PhantomData,
        }
    }
}

impl<A> GenericTriggerFactory<A> {
    /// Construct a new trigger factory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<A> ActionFactory for GenericTriggerFactory<A>
where
    A: TriggerAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
    <A as TriggerAction>::Error: Into<ActionError>,
{
    fn metadata(&self) -> &ActionMetadata {
        self.meta.get_or_init(<A as Action>::metadata)
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let meta = self.metadata().clone();
            let inner = ErasedTriggerImpl::<A>::new(action, meta);
            Ok(ErasedAction::Trigger(Box::new(inner)))
        })
    }
}

struct ErasedTriggerImpl<A> {
    action: A,
    meta: ActionMetadata,
}

impl<A> ErasedTriggerImpl<A> {
    fn new(action: A, meta: ActionMetadata) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> ErasedTrigger for ErasedTriggerImpl<A>
where
    A: TriggerAction + Send + Sync + 'static,
    <A as TriggerAction>::Error: Into<ActionError>,
{
    fn metadata(&self) -> &ActionMetadata {
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

/// Generic factory that produces [`ErasedAction::Resource`] for any type
/// implementing [`ResourceAction`] + [`FromWorkflowNode`].
pub struct GenericResourceFactory<A> {
    meta: OnceLock<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> Default for GenericResourceFactory<A> {
    fn default() -> Self {
        Self {
            meta: OnceLock::new(),
            _phantom: PhantomData,
        }
    }
}

impl<A> GenericResourceFactory<A> {
    /// Construct a new resource factory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<A> ActionFactory for GenericResourceFactory<A>
where
    A: ResourceAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.meta.get_or_init(<A as Action>::metadata)
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let meta = self.metadata().clone();
            let inner = ErasedResourceImpl::<A>::new(action, meta);
            Ok(ErasedAction::Resource(Box::new(inner)))
        })
    }
}

struct ErasedResourceImpl<A> {
    action: A,
    meta: ActionMetadata,
}

impl<A> ErasedResourceImpl<A> {
    fn new(action: A, meta: ActionMetadata) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> ErasedResource for ErasedResourceImpl<A>
where
    A: ResourceAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
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
                "ErasedResourceImpl: downcast invariant violated for {}",
                std::any::type_name::<A::Resource>()
            ))
        })?;
        self.action.cleanup(*typed, ctx).await
    }
}

// ── Control ────────────────────────────────────────────────────────────────

/// Generic factory that produces [`ErasedAction::Control`] for any type
/// implementing [`ControlAction`] + [`FromWorkflowNode`].
pub struct GenericControlFactory<A> {
    meta: OnceLock<ActionMetadata>,
    _phantom: PhantomData<fn() -> A>,
}

impl<A> Default for GenericControlFactory<A> {
    fn default() -> Self {
        Self {
            meta: OnceLock::new(),
            _phantom: PhantomData,
        }
    }
}

impl<A> GenericControlFactory<A> {
    /// Construct a new control factory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<A> ActionFactory for GenericControlFactory<A>
where
    A: ControlAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.meta.get_or_init(<A as Action>::metadata)
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>> {
        Box::pin(async move {
            let action = A::from_workflow_node(node, ctx).await?;
            let meta = self.metadata().clone();
            let inner = ErasedControlImpl::<A>::new(action, meta);
            Ok(ErasedAction::Control(Box::new(inner)))
        })
    }
}

struct ErasedControlImpl<A> {
    action: A,
    meta: ActionMetadata,
}

impl<A> ErasedControlImpl<A> {
    fn new(action: A, meta: ActionMetadata) -> Self {
        Self { action, meta }
    }
}

#[async_trait]
impl<A> ErasedControl for ErasedControlImpl<A>
where
    A: ControlAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    async fn dispatch(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let outcome = self
            .action
            .evaluate(ControlInput::from_value(input), ctx)
            .await?;
        Ok(outcome.into())
    }
}
