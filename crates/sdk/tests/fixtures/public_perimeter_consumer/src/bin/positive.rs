use nebula_sdk::{
    integration::action::{
        CancellationToken, EffectInvocationContext, EffectPreparationContext, EffectQueryContext,
        ExecutionId, NodeKey, OperationCallId, OperationId, OrgId, WorkflowId, WorkspaceId,
    },
    integration::credential::{TestFailureCode, TestResult},
    prelude::{
        Action, ActionMetadataDraft, ActionResult, AuthoredValue, Deserialize, Error, Expression,
        PoolProvider, Pooled, ProgramSyntax, Provider, ReleaseOutcome, RemoteDestinationGuarantee,
        RemoteEffectPolicy, Resource, ResourceContext, ResourceKey, Schema, Serialize,
        TeardownCx, TeardownReason, TriggerHealthSnapshot, Value, WorkflowBuilder, metadata_name,
        no_credential_slots, resource_key,
    },
    simple_action,
};

#[derive(Debug, Deserialize, Schema)]
#[serde(crate = "nebula_sdk::serde")]
pub struct AuthorInput {
    message: String,
}

#[derive(Debug, Serialize, Schema)]
#[serde(crate = "nebula_sdk::serde")]
pub struct AuthorOutput {
    echoed: String,
}

simple_action! {
    name: EchoAction,
    key: "example.echo",
    input: AuthorInput,
    output: AuthorOutput,
    async fn execute(&self, input, _context) {
        Ok(ActionResult::success(AuthorOutput { echoed: input.message }))
    }
}

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
        let _remaining = cx
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        let _is_shutdown = match cx.reason {
            TeardownReason::Shutdown => true,
            _ => false,
        };
        drop(instance.0);
        Ok(())
    }
}

#[derive(Default, Clone, Resource)]
#[topology(Pooled)]
struct CatalogProvider;

impl PoolProvider for CatalogProvider {}

#[async_trait::async_trait]
impl Provider for CatalogProvider {
    type Config = ();
    type Instance = ();
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.catalog-provider")
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

fn invocation_identity(context: &dyn EffectInvocationContext) -> (OperationId, OperationCallId) {
    (context.operation_id(), context.call_id())
}

fn query_identity(context: &dyn EffectQueryContext) -> (OperationId, OperationCallId) {
    (context.operation_id(), context.call_id())
}

fn invocation_cancellation(context: &dyn EffectInvocationContext) -> &CancellationToken {
    context.cancellation()
}

fn assert_typed_action_contract<A>()
where
    A: Action<Input = AuthorInput, Output = AuthorOutput>,
{
}

fn main() {
    assert_typed_action_contract::<EchoAction>();
    let metadata: ActionMetadataDraft = EchoAction::metadata();
    let authored = nebula_sdk::params! { data; "message" => "hello" }
        .expect("literal authored input is valid");
    assert_eq!(authored.to_json(), nebula_sdk::json!({"message": "hello"}));
    let template = Expression::template("{{ 7 }}");
    assert_eq!(template.syntax(), ProgramSyntax::Template);
    template.parse().expect("valid template");
    let authored = AuthoredValue::Expression(template.clone());
    let wire = nebula_sdk::serde_json::to_vec(&authored).expect("secret-free authored wire");
    let authored: AuthoredValue =
        nebula_sdk::serde_json::from_slice(&wire).expect("valid authored wire");
    assert_eq!(authored, AuthoredValue::Expression(template));
    let catalog_contribution = CatalogProviderFactory::new().into_contribution();
    assert_eq!(catalog_contribution.key(), CatalogProvider::key());
    let _resource_key = ManualProvider::key();
    let _invocation_identity: fn(&dyn EffectInvocationContext) -> (OperationId, OperationCallId) =
        invocation_identity;
    let _query_identity: fn(&dyn EffectQueryContext) -> (OperationId, OperationCallId) =
        query_identity;
    let _invocation_cancellation: fn(&dyn EffectInvocationContext) -> &CancellationToken =
        invocation_cancellation;
    let _preparation_constructor: fn(
        ExecutionId,
        WorkflowId,
        NodeKey,
        OrgId,
        WorkspaceId,
    ) -> EffectPreparationContext = EffectPreparationContext::new;
    let release_completed = match ReleaseOutcome::Completed {
        ReleaseOutcome::Completed => true,
        ReleaseOutcome::Deferred => false,
        _ => false,
    };
    let manual_metadata = ActionMetadataDraft::new(
        nebula_sdk::prelude::action_key!("example.perimeter"),
        metadata_name!("Perimeter action"),
        "Uses only the supported SDK authoring surface",
    );
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

    let _: ActionMetadataDraft = metadata;
    let _: ActionMetadataDraft = manual_metadata;
    assert!(release_completed);
    assert_eq!(workflow.nodes.len(), 1);
    assert_eq!(effect_policy.max_invocations(), 1);
    assert_eq!(
        result.failure_code(),
        Some(TestFailureCode::AuthenticationRejected)
    );
    let _: Option<Value> = None;
    let _: Option<TriggerHealthSnapshot> = None;
}
