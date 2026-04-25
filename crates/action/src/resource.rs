//! [`ResourceAction`] trait, [`ResourceHandler`] dyn contract, and adapter.
//!
//! A resource action runs `configure` before the downstream subtree and
//! `cleanup` when the scope ends. The produced resource is visible only
//! to the downstream branch, unlike `ctx.resource()` from the global
//! registry.

use std::{any::Any, fmt, future::Future, pin::Pin};

use serde_json::Value;

use crate::{action::Action, context::ActionContext, error::ActionError, metadata::ActionMetadata};

// ── Core trait ──────────────────────────────────────────────────────────────

/// Resource action: graph-level dependency injection.
///
/// The engine runs `configure` before downstream nodes; the resulting
/// resource is scoped to the branch. When the scope ends, the engine
/// calls `cleanup` with the same resource. Use for connection pools,
/// caches, or other resources visible only to the downstream subtree
/// (unlike `ctx.resource()` from the global registry).
///
/// A single associated type `Resource` is used for both the `configure`
/// return and the `cleanup` parameter. Earlier iterations split these
/// into `Config` (returned) and `Instance` (consumed by cleanup), but
/// the adapter could not safely bridge the two: it boxed `Config` and
/// downcast to `Instance`, so any impl where they differed failed at
/// runtime with `ActionError::Fatal`. Zero production impls ever used
/// distinct types, so the split was removed rather than patched.
pub trait ResourceAction: Action {
    /// Resource produced by `configure`, consumed by `cleanup`.
    type Resource: Send + Sync + 'static;

    /// Build the resource for this scope; engine runs this before downstream nodes.
    fn configure(
        &self,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<Self::Resource, ActionError>> + Send;

    /// Clean up the resource when the scope ends (e.g. drop pool, close connections).
    fn cleanup(
        &self,
        resource: Self::Resource,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}

// ── Handler trait ───────────────────────────────────────────────────────────

/// Type alias for the dyn-safe future returned by
/// [`ResourceHandler::configure`] — a boxed `Box<dyn Any + Send + Sync>`
/// carrying the configured resource instance.
pub type ResourceConfigureFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, ActionError>> + Send + 'a>>;

/// Type alias for the dyn-safe future returned by [`ResourceHandler::cleanup`].
pub type ResourceCleanupFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>;

/// Resource handler — configure/cleanup lifecycle for graph-scoped resources.
///
/// The engine runs `configure` before downstream nodes; the resulting instance
/// is scoped to the branch. When the scope ends, `cleanup` is called.
///
/// # Errors
///
/// Returns [`ActionError`] on configuration or cleanup failure.
pub trait ResourceHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Build the resource for this scope.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the resource cannot be configured.
    fn configure<'life0, 'life1, 'a>(
        &'life0 self,
        config: Value,
        ctx: &'life1 dyn ActionContext,
    ) -> ResourceConfigureFuture<'a>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a;

    /// Clean up the resource instance when the scope ends.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if cleanup fails.
    fn cleanup<'life0, 'life1, 'a>(
        &'life0 self,
        instance: Box<dyn Any + Send + Sync>,
        ctx: &'life1 dyn ActionContext,
    ) -> ResourceCleanupFuture<'a>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a;
}

// ── Adapter ─────────────────────────────────────────────────────────────────

/// Wraps a [`ResourceAction`] as a [`dyn ResourceHandler`].
///
/// Bridges the typed `configure`/`cleanup` lifecycle to the JSON-erased handler
/// trait. The `configure` result is boxed as `Box<dyn Any + Send + Sync>`;
/// `cleanup` downcasts it back to the typed `Resource`.
///
/// # Example
///
/// ```rust,ignore
/// let handler: Arc<dyn ResourceHandler> = Arc::new(ResourceActionAdapter::new(my_resource));
/// ```
pub struct ResourceActionAdapter<A> {
    action: A,
}

impl<A> ResourceActionAdapter<A> {
    /// Wrap a typed resource action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

impl<A> ResourceHandler for ResourceActionAdapter<A>
where
    A: ResourceAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    /// Configure the resource by delegating to the typed action.
    ///
    /// The `_config` parameter is reserved for future use; the typed
    /// [`ResourceAction::configure`] obtains its configuration from context.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the resource cannot be configured.
    fn configure<'life0, 'life1, 'a>(
        &'life0 self,
        _config: Value,
        ctx: &'life1 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            let resource = self.action.configure(ctx).await?;
            let boxed: Box<dyn Any + Send + Sync> = Box::new(resource);
            Ok(boxed)
        })
    }

    /// Clean up the resource by downcasting and delegating.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] if the downcast invariant is violated
    /// (engine bug — the box we returned from `configure` was routed to a
    /// different adapter), or propagates errors from the underlying action.
    fn cleanup<'life0, 'life1, 'a>(
        &'life0 self,
        resource: Box<dyn Any + Send + Sync>,
        ctx: &'life1 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            // The downcast is an engine-level invariant check: we box
            // `A::Resource` in `configure` above and the engine routes the
            // same box back here. A mismatch is an engine bug, not a user
            // footgun — there is no `Config`/`Instance` split to bridge.
            let typed = resource.downcast::<A::Resource>().map_err(|_| {
                ActionError::fatal(format!(
                    "ResourceActionAdapter: downcast invariant violated for {}",
                    std::any::type_name::<A::Resource>()
                ))
            })?;
            self.action.cleanup(*typed, ctx).await
        })
    }
}

impl<A: Action> fmt::Debug for ResourceActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceActionAdapter")
            .field("action", &self.action.metadata().base.key)
            .finish_non_exhaustive()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::DeclaresDependencies;

    use super::*;
    use crate::testing::{TestActionContext, TestContextBuilder};

    struct MockResourceAction {
        meta: ActionMetadata,
    }

    impl MockResourceAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.resource_action"),
                    "MockResource",
                    "Creates a string pool",
                ),
            }
        }
    }

    impl DeclaresDependencies for MockResourceAction {}

    impl Action for MockResourceAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl ResourceAction for MockResourceAction {
        type Resource = String;

        async fn configure(
            &self,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<String, ActionError> {
            Ok("pool-default".to_owned())
        }

        async fn cleanup(
            &self,
            _resource: String,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<(), ActionError> {
            Ok(())
        }
    }

    fn make_ctx() -> TestActionContext {
        TestContextBuilder::new().build()
    }

    #[test]
    fn resource_adapter_is_dyn_compatible() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let _: Arc<dyn ResourceHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn resource_adapter_configure_returns_boxed_instance() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let instance = handler
            .configure(serde_json::json!({}), &ctx)
            .await
            .unwrap();
        let typed = instance.downcast::<String>().unwrap();
        assert_eq!(*typed, "pool-default");
    }

    #[tokio::test]
    async fn resource_adapter_cleanup_receives_typed_instance() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let instance: Box<dyn Any + Send + Sync> = Box::new("pool-default".to_owned());
        handler.cleanup(instance, &ctx).await.unwrap();
    }

    #[tokio::test]
    async fn resource_adapter_cleanup_fails_on_wrong_type() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let wrong_instance: Box<dyn Any + Send + Sync> = Box::new(42u32);
        let err = handler.cleanup(wrong_instance, &ctx).await.unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    #[test]
    fn resource_adapter_into_inner_returns_action() {
        let adapter = ResourceActionAdapter::new(MockResourceAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().base.key,
            nebula_core::action_key!("test.resource_action")
        );
    }
}
