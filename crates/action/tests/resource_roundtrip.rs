//! Integration test for [`ResourceAction`] single-type roundtrip.
//!
//! Proves that a `ResourceAction` with a non-trivial `Resource` type
//! (a struct that implements neither `Default` nor `Clone`) can be
//! boxed by `ResourceActionAdapter::configure` and successfully
//! downcast back by `ResourceActionAdapter::cleanup`. Before the A4
//! fix, this test would have compiled against the old trait with
//! `type Config = PoolConfig; type Instance = PoolHandle;` and failed
//! at runtime with `ActionError::Fatal("resource instance downcast
//! failed")` on every `cleanup` call.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use nebula_action::{
    Action, ActionError, ActionMetadata, ResourceAction, ResourceActionAdapter, ResourceHandler,
    TestContextBuilder, testing::TestActionContext as ActionContext,
};
use nebula_core::DeclaresDependencies;

/// Handle to a fictitious pool. Non-trivial enough to expose any
/// downcast mismatch as a test failure rather than a spurious pass.
#[derive(Debug)]
struct PoolHandle {
    id: u32,
    cleanup_observed: Arc<AtomicBool>,
}

impl Drop for PoolHandle {
    fn drop(&mut self) {
        self.cleanup_observed.store(true, Ordering::SeqCst);
    }
}

struct PoolAction {
    meta: ActionMetadata,
    cleanup_observed: Arc<AtomicBool>,
    cleanup_ran: Arc<AtomicBool>,
}

impl DeclaresDependencies for PoolAction {}
impl Action for PoolAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl ResourceAction for PoolAction {
    type Resource = PoolHandle;

    async fn configure(
        &self,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<PoolHandle, ActionError> {
        Ok(PoolHandle {
            id: 42,
            cleanup_observed: self.cleanup_observed.clone(),
        })
    }

    async fn cleanup(
        &self,
        resource: PoolHandle,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<(), ActionError> {
        // Assert we got the same handle back that configure produced.
        // If the adapter ever confused `Config` and `Instance` types
        // again, this test fails with a downcast Fatal BEFORE reaching
        // this body.
        assert_eq!(resource.id, 42);
        self.cleanup_ran.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn resource_action_configure_cleanup_roundtrip() {
    let cleanup_observed = Arc::new(AtomicBool::new(false));
    let cleanup_ran = Arc::new(AtomicBool::new(false));

    let action = PoolAction {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.resource.pool"),
            "Pool",
            "Typed pool resource",
        ),
        cleanup_observed: cleanup_observed.clone(),
        cleanup_ran: cleanup_ran.clone(),
    };
    let adapter = ResourceActionAdapter::new(action);
    let handler: Arc<dyn ResourceHandler> = Arc::new(adapter);
    let ctx: ActionContext = TestContextBuilder::minimal().build();

    // configure: boxes the typed PoolHandle as dyn Any
    let boxed = handler
        .configure(serde_json::json!({}), &ctx)
        .await
        .expect("configure succeeds");

    // cleanup: downcasts back and must not hit the Fatal invariant check
    handler
        .cleanup(boxed, &ctx)
        .await
        .expect("cleanup succeeds without downcast mismatch");

    assert!(
        cleanup_ran.load(Ordering::SeqCst),
        "cleanup body must have executed — downcast succeeded"
    );
    assert!(
        cleanup_observed.load(Ordering::SeqCst),
        "PoolHandle::Drop must have fired after cleanup consumed it"
    );
}
