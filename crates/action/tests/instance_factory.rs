//! Integration tests for [`InstanceFactory`] — the instance-backed stateless
//! [`ActionFactory`].
//!
//! `InstanceFactory` is the migration vehicle for the ADR-0098 D0 spine collapse:
//! it lets a registry hold a pre-built action instance plus per-registration
//! metadata on the surviving factory spine, where the generic factory (which
//! ties one type to one static key and rebuilds the action per dispatch) cannot.
//! These tests pin the two properties that distinguish it from
//! `GenericStatelessFactory`: caller-supplied metadata, and a shared instance.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionHandle, ActionKind, ActionMetadata,
    ActionResult, InstanceFactory, StatelessAction, TestContextBuilder,
};
use nebula_core::{Dependencies, action_key, node_key};
use nebula_workflow::NodeDefinition;

/// Stateless action that counts every execution through a shared `Arc`, so a
/// test can observe whether dispatches reuse one instance or rebuild it.
struct CountingEcho {
    hits: Arc<AtomicUsize>,
}

impl Action for CountingEcho {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        // A distinct type-level key, so a test can prove the factory does NOT
        // fall back to this when caller metadata is supplied.
        ActionMetadata::new(
            action_key!("builtin.counting_echo"),
            "CountingEcho",
            "type-level metadata",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for CountingEcho {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

#[tokio::test]
async fn instance_factory_uses_caller_metadata_not_type_metadata() {
    let factory = InstanceFactory::new(
        ActionMetadata::new(
            action_key!("tenant.custom_echo"),
            "Custom Echo",
            "per-registration metadata",
        ),
        CountingEcho {
            hits: Arc::new(AtomicUsize::new(0)),
        },
    );

    // Caller metadata wins over the type's own `Action::metadata()` key — this
    // is why one action type can back many distinct catalog keys.
    assert_eq!(
        factory.metadata().base.key,
        action_key!("tenant.custom_echo")
    );
    assert_ne!(
        factory.metadata().base.key,
        <CountingEcho as Action>::metadata().base.key,
        "InstanceFactory must not fall back to the type's static metadata key"
    );
    // The factory is the single writer of the kind for the handle it produces.
    assert_eq!(factory.metadata().kind, ActionKind::Stateless);
}

#[tokio::test]
async fn instance_factory_shares_one_instance_across_dispatches() {
    let hits = Arc::new(AtomicUsize::new(0));
    let factory = InstanceFactory::new(
        ActionMetadata::new(action_key!("tenant.custom_echo"), "Custom Echo", ""),
        CountingEcho {
            hits: Arc::clone(&hits),
        },
    );

    let node = NodeDefinition::new(
        node_key!("n"),
        "custom_echo",
        "tenant",
        "tenant.custom_echo",
    )
    .expect("node definition builds");
    let ctx = TestContextBuilder::new().build();

    // Two independent instantiate→dispatch cycles must hit the SAME underlying
    // instance: the shared counter accumulates. A generic factory would rebuild
    // a fresh action per dispatch and the count would not carry across cycles.
    for expected in 1..=2u64 {
        let handle = factory
            .instantiate(&node, &ctx)
            .await
            .expect("instantiate succeeds");
        let ActionHandle::Stateless(stateless) = handle else {
            panic!("InstanceFactory must produce ActionHandle::Stateless");
        };

        let result = stateless
            .dispatch(serde_json::json!({ "n": expected }), &ctx)
            .await
            .expect("dispatch succeeds");

        // Assert the echoed value (correctness), not merely that it succeeded.
        match result {
            ActionResult::Success { output } => {
                assert_eq!(
                    output.as_value(),
                    Some(&serde_json::json!({ "n": expected }))
                );
            },
            _ => panic!("expected ActionResult::Success"),
        }

        assert_eq!(
            hits.load(Ordering::SeqCst),
            expected as usize,
            "shared instance: the counter must accumulate across dispatch cycles"
        );
    }
}
