use nebula_sdk::{
    integration::action::{
        EffectInvocationContext, EffectQueryContext, OperationCallId, OperationId,
    },
    integration::credential::{TestFailureCode, TestResult},
    prelude::{
        ActionBuilder, RemoteDestinationGuarantee, RemoteEffectPolicy, WorkflowBuilder, action_key,
    },
};

fn invocation_identity(
    context: &dyn EffectInvocationContext,
) -> (OperationId, OperationCallId) {
    (context.operation_id(), context.call_id())
}

fn query_identity(context: &dyn EffectQueryContext) -> (OperationId, OperationCallId) {
    (context.operation_id(), context.call_id())
}

fn main() {
    let _invocation_identity: fn(&dyn EffectInvocationContext) -> (OperationId, OperationCallId) =
        invocation_identity;
    let _query_identity: fn(&dyn EffectQueryContext) -> (OperationId, OperationCallId) =
        query_identity;
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
    assert_eq!(workflow.nodes.len(), 1);
    assert_eq!(effect_policy.max_invocations(), 1);
    assert_eq!(
        result.failure_code(),
        Some(TestFailureCode::AuthenticationRejected)
    );
}
