use nebula_sdk::{
    integration::action::{
        EffectInvocationContext, EffectQueryContext, OperationCallId, OperationId,
    },
    integration::credential::{TestFailureCode, TestResult},
    prelude::{
        ActionBuilder, Error, PoolProvider, Pooled, Provider, RemoteDestinationGuarantee,
        ReleaseOutcome, RemoteEffectPolicy, ResourceContext, ResourceKey, TeardownCx, TeardownReason,
        WorkflowBuilder, action_key, no_credential_slots, resource_key,
    },
};

struct OwnedClient(String);

#[derive(Clone)]
struct ManualProvider;
no_credential_slots!(ManualProvider);
impl PoolProvider for ManualProvider {}

#[async_trait::async_trait]
impl Provider for ManualProvider {
    type Config = ();
    type Instance = OwnedClient;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.manual-provider")
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<OwnedClient, Error> {
        Ok(OwnedClient(String::from("owned connection")))
    }

    async fn destroy(&self, instance: OwnedClient, cx: TeardownCx) -> Result<(), Error> {
        let _remaining = cx.deadline.saturating_duration_since(std::time::Instant::now());
        let _is_shutdown = match cx.reason {
            TeardownReason::Shutdown => true,
            _ => false,
        };
        drop(instance.0);
        Ok(())
    }
}

fn invocation_identity(
    context: &dyn EffectInvocationContext,
) -> (OperationId, OperationCallId) {
    (context.operation_id(), context.call_id())
}

fn query_identity(context: &dyn EffectQueryContext) -> (OperationId, OperationCallId) {
    (context.operation_id(), context.call_id())
}

fn main() {
    let _resource_key = ManualProvider::key();
    let _invocation_identity: fn(&dyn EffectInvocationContext) -> (OperationId, OperationCallId) =
        invocation_identity;
    let _query_identity: fn(&dyn EffectQueryContext) -> (OperationId, OperationCallId) =
        query_identity;
    let release_completed = match ReleaseOutcome::Completed {
        ReleaseOutcome::Completed => true,
        ReleaseOutcome::Deferred => false,
        _ => false,
    };
    let metadata = ActionBuilder::new(action_key!("example.perimeter"), "Perimeter action")
        .with_description("Uses only the supported SDK authoring surface")
        .build();
    let workflow = WorkflowBuilder::new("public_perimeter")
        .add_node("invoke", "example", "perimeter")
        .build()
        .expect("the supported builder must accept one valid node");
    let result = TestResult::Failed {
        code: TestFailureCode::AuthenticationRejected,
    };
    let effect_policy = RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
        .maximum_invocations(1)
        .maximum_queries(0)
        .recovery_window(std::time::Duration::from_mins(1))
        .build()
        .expect("a bounded opaque effect policy is valid");

    assert_eq!(metadata.base.name, "Perimeter action");
    assert!(release_completed);
    assert_eq!(workflow.nodes.len(), 1);
    assert_eq!(effect_policy.max_invocations(), 1);
    assert_eq!(
        result.failure_code(),
        Some(TestFailureCode::AuthenticationRejected)
    );
}
