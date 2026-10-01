//! A journaled action through real activation, start admission and durable
//! turns: a default-contract (`Journaled`) stateless action submits effect
//! units on the resource handle of a fake payment gateway that counts every
//! provider call and deduplicates by the idempotency key it receives.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use nebula_action::{
    ActionContext, ActionContextExt, ActionError, GenericStatefulFactory, InstanceFactory,
    StatefulAction, StatelessAction,
};
use nebula_core::{ResourceKey, ScopeLevel, resource_key};
use nebula_resource::{
    Manager, RegistrationSpec, Resident, ResidentConfig, ResourceContext, SlotIdentity,
    call::{Cost, Effect, Operation, OperationCx, OperationError},
    error::Error as ResourceError,
    resource::{Provider as ResourceProvider, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};
use nebula_storage_port::dto::EffectOccurrenceRecord;
use serde::Deserialize;

use super::*;

/// The node that runs the journaled action.
pub(super) const NODE: &str = "charge";

/// A slot identity bound to credential `credential`.
pub(super) fn bound_to(credential: &str) -> SlotIdentity {
    SlotIdentity::from_bindings([("api", credential)])
}

/// A gate a provider call waits at.
#[derive(Debug, Default)]
pub(super) struct Gate {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}

/// One provider call as the gateway received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderCall {
    pub key: Option<String>,
    pub request: String,
}

/// The fake payment gateway: every call that reached it, keyed
/// deduplication, and scripted faults.
#[derive(Debug, Default)]
pub(super) struct Gateway {
    /// Calls that reached the gateway.
    pub calls: parking_lot::Mutex<Vec<ProviderCall>>,
    /// Receipts by idempotency key: a repeated key is applied once.
    applied: parking_lot::Mutex<HashMap<String, u64>>,
    /// The next call is applied and then never answers.
    pub hang_next: AtomicBool,
    /// The next call is applied and then waits for this gate.
    pub hold_next: parking_lot::Mutex<Option<Arc<Gate>>>,
    /// Calls to apply and then lose the answer of.
    pub lose_first: AtomicU32,
    /// Fired whenever a call reaches the gateway.
    pub entered: tokio::sync::Notify,
}

impl Gateway {
    /// Distinct effects the gateway applied.
    pub(super) fn applied(&self) -> usize {
        self.applied.lock().len()
    }

    /// Calls that reached the gateway.
    pub(super) fn call_count(&self) -> usize {
        self.calls.lock().len()
    }

    /// The keys of the calls that reached the gateway.
    pub(super) fn call_keys(&self) -> Vec<Option<String>> {
        self.calls
            .lock()
            .iter()
            .map(|call| call.key.clone())
            .collect()
    }

    async fn call(&self, key: Option<String>, request: &str) -> Result<u64, OperationError> {
        self.calls.lock().push(ProviderCall {
            key: key.clone(),
            request: request.to_owned(),
        });
        let receipt = {
            let mut applied = self.applied.lock();
            let next = u64::try_from(applied.len()).unwrap() + 1;
            let dedup = key.unwrap_or_else(|| format!("unkeyed-{next}"));
            *applied.entry(dedup).or_insert(next)
        };
        self.entered.notify_one();
        if self.hang_next.swap(false, Ordering::SeqCst) {
            return std::future::pending().await;
        }
        let hold = self.hold_next.lock().take();
        if let Some(gate) = hold {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        if self
            .lose_first
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(OperationError::interrupted("connection reset"));
        }
        Ok(receipt)
    }
}

/// The resource whose handles reach the gateway.
#[derive(Clone)]
pub(super) struct Payments(Arc<Gateway>);

#[async_trait::async_trait]
impl ResourceProvider for Payments {
    type Config = ();
    type Instance = Arc<Gateway>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.journal.payments")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("JournalPayments"),
            "",
        )
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<Arc<Gateway>, ResourceError> {
        Ok(Arc::clone(&self.0))
    }
}

nebula_resource::no_credential_slots!(Payments);

impl ResidentProvider for Payments {}

/// One unit the action submits.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct UnitSpec {
    /// `Idempotent` (stable key), else an opaque `Write`.
    #[serde(default)]
    pub idempotent: bool,
    /// The logical request.
    pub request: String,
    /// The developer part of the idempotency key.
    #[serde(default)]
    pub key: Option<String>,
    /// Attempts the unit may be granted.
    #[serde(default = "one")]
    pub budget: u32,
}

const fn one() -> u32 {
    1
}

/// A write unit of `request`.
pub(super) fn write(request: &str) -> Value {
    json!({"request": request})
}

/// A charge: `Write` (opaque) or, with `IDEM`, `Idempotent` (stable key).
#[derive(Serialize, Deserialize)]
struct Charge<const IDEM: bool>(UnitSpec);

impl<const IDEM: bool> Operation<Payments> for Charge<IDEM> {
    type Output = u64;
    const KEY: &'static str = "test.charge";
    const EFFECT: Effect = if IDEM {
        Effect::Idempotent
    } else {
        Effect::Write
    };
    const KEY_WINDOW: std::time::Duration = std::time::Duration::from_hours(1);

    fn idempotency_key(&self) -> Option<String> {
        self.0.key.clone()
    }

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(self.0.budget).unwrap_or(NonZeroU32::MIN)
    }

    async fn run(self, cx: &mut OperationCx<'_, Payments>) -> Result<u64, OperationError> {
        let key = cx.idempotency_key().map(ToString::to_string);
        let request = self.0.request;
        cx.call(Cost::ONE, async move |gateway, ()| {
            gateway.call(key.clone(), &request).await
        })
        .await
    }
}

/// What a redeploy changed about the operation each unit submits.
#[derive(Debug, Clone, Copy)]
pub(super) enum Drift {
    /// The same `KEY` at another `VERSION`.
    Version,
    /// Another operation `KEY`.
    Operation,
}

/// [`Charge`]'s opaque write, redeployed at version 2.
#[derive(Serialize, Deserialize)]
struct ChargeV2(UnitSpec);

impl Operation<Payments> for ChargeV2 {
    type Output = u64;
    const KEY: &'static str = "test.charge";
    const VERSION: u32 = 2;
    const EFFECT: Effect = Effect::Write;

    async fn run(self, cx: &mut OperationCx<'_, Payments>) -> Result<u64, OperationError> {
        let request = self.0.request;
        cx.call(Cost::ONE, async move |gateway, ()| {
            gateway.call(None, &request).await
        })
        .await
    }
}

/// Another opaque write on the payments row.
#[derive(Serialize, Deserialize)]
struct Capture(UnitSpec);

impl Operation<Payments> for Capture {
    type Output = u64;
    const KEY: &'static str = "test.capture";
    const EFFECT: Effect = Effect::Write;

    async fn run(self, cx: &mut OperationCx<'_, Payments>) -> Result<u64, OperationError> {
        let request = self.0.request;
        cx.call(Cost::ONE, async move |gateway, ()| {
            gateway.call(None, &request).await
        })
        .await
    }
}

/// What the action does, as the execution input says.
#[derive(Debug, Deserialize)]
struct Script {
    units: Vec<UnitSpec>,
    /// Swallow unit errors (recording their kind) and return `Ok` anyway.
    #[serde(default)]
    swallow: bool,
    /// Submit every unit without waiting for it.
    #[serde(default)]
    leak: bool,
    /// Try a raw lease of the payments resource first.
    #[serde(default)]
    raw_lease: bool,
}

/// Test controls the action reads on every run.
#[derive(Debug, Default)]
pub(super) struct Controls {
    /// After its units, the action never returns (a crash point).
    pub hold_after_units: AtomicBool,
    /// Fired when the action reached its crash point.
    pub after_units: tokio::sync::Notify,
    /// Replaces every unit's request (a non-deterministic action).
    pub request_override: parking_lot::Mutex<Option<String>>,
    /// Replaces every unit's operation (a redeploy without an action
    /// version bump).
    pub drift: parking_lot::Mutex<Option<Drift>>,
    /// On the next dispatch only, the action first builds a write
    /// submission and drops it unpolled (a branch taken once).
    pub drop_unpolled_once: AtomicBool,
    /// The action skips every unit of its script (another branch).
    pub skip_units: AtomicBool,
    /// Retryable failures the action returns after its units.
    pub fail_after_units: AtomicU32,
    /// Dispatches of the action.
    pub dispatches: AtomicU32,
}

/// Runs `script` on `ctx`'s payments handle, as the action body.
async fn run_script(
    controls: &Controls,
    script: Script,
    ctx: &(impl ActionContext + ?Sized),
) -> Result<Value, ActionError> {
    controls.dispatches.fetch_add(1, Ordering::SeqCst);
    let raw_lease = if script.raw_lease {
        let refused = ctx
            .acquire_resource_by_id::<Payments>(Payments::key().as_str())
            .await
            .is_err();
        Some(refused)
    } else {
        None
    };
    let handle = ctx.resource_handle_by_id::<Payments>(Payments::key().as_str())?;
    if controls.drop_unpolled_once.swap(false, Ordering::SeqCst) {
        drop(handle.submit(Charge::<false>(UnitSpec {
            idempotent: false,
            request: "never-polled".to_owned(),
            key: None,
            budget: 1,
        })));
    }
    let mut receipts = Vec::new();
    let units = if controls.skip_units.load(Ordering::SeqCst) {
        Vec::new()
    } else {
        script.units
    };
    for mut spec in units {
        if let Some(request) = controls.request_override.lock().clone() {
            spec.request = request;
        }
        let drift = *controls.drift.lock();
        let unit = match drift {
            Some(Drift::Version) => handle.submit(ChargeV2(spec)),
            Some(Drift::Operation) => handle.submit(Capture(spec)),
            None if spec.idempotent => handle.submit(Charge::<true>(spec)),
            None => handle.submit(Charge::<false>(spec)),
        };
        if script.leak {
            drop(tokio::spawn(unit));
            continue;
        }
        match unit.await {
            Ok(receipt) => receipts.push(json!(receipt)),
            Err(error) if script.swallow => receipts.push(json!({
                "kind": error.kind().to_string(),
                "sent": error.sent().as_str(),
                "detail": error.detail(),
            })),
            Err(error) => return Err(error.into()),
        }
    }
    if controls.hold_after_units.load(Ordering::SeqCst) {
        controls.after_units.notify_one();
        std::future::pending::<()>().await;
    }
    if controls
        .fail_after_units
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(1)
        })
        .is_ok()
    {
        return Err(ActionError::retryable("downstream step failed"));
    }
    Ok(json!({ "receipts": receipts, "raw_lease_refused": raw_lease }))
}

/// The journaled stateless action.
struct ChargeAction {
    controls: Arc<Controls>,
}

impl Action for ChargeAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("journal.charge"),
            nebula_action::metadata_name!("Charge"),
            "Journaled test action",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ChargeAction {
    async fn execute(
        &self,
        input: Value,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let script: Script = serde_json::from_value(input)
            .map_err(|error| ActionError::fatal(format!("bad script: {error}")))?;
        run_script(&self.controls, script, ctx)
            .await
            .map(ActionResult::success)
    }
}

/// The controls of the stateful action, which the generic stateful factory
/// builds per dispatch.
static STATEFUL_CONTROLS: OnceLock<Arc<Controls>> = OnceLock::new();

/// The journaled stateful action: one iteration running the script.
#[derive(nebula_action::Action)]
#[action(
    key = "journal.charge",
    name = "Charge",
    description = "Journaled stateful test action",
    input = Value,
    output = Value
)]
struct StatefulCharge;

impl StatefulAction for StatefulCharge {
    type State = u32;

    fn init_state(&self) -> u32 {
        0
    }

    async fn execute(
        &self,
        input: &Value,
        state: &mut u32,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        *state += 1;
        let script: Script = serde_json::from_value(input.clone())
            .map_err(|error| ActionError::fatal(format!("bad script: {error}")))?;
        let controls = STATEFUL_CONTROLS.get_or_init(Arc::default);
        let output = run_script(controls, script, ctx).await?;
        Ok(ActionResult::Break {
            output: nebula_action::ActionOutput::Value(output),
            reason: nebula_action::BreakReason::Completed,
        })
    }
}

/// Which action kind the fixture's node runs.
#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Stateless,
    Stateful,
}

/// A frozen plugin of the fixture's action.
fn frozen_plugin(kind: Kind, controls: &Arc<Controls>) -> Arc<FrozenPluginRegistry> {
    let factory: Arc<dyn ActionFactory> = match kind {
        Kind::Stateless => Arc::new(
            InstanceFactory::new(
                ChargeAction::metadata(),
                ChargeAction {
                    controls: Arc::clone(controls),
                },
            )
            .expect("action metadata is admitted"),
        ),
        Kind::Stateful => {
            Arc::new(GenericStatefulFactory::<StatefulCharge>::new().expect("admitted"))
        },
    };
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(
            ResolvedPlugin::from(ProviderPlugin {
                manifest: PluginManifest::builder("journal", "Journal")
                    .build()
                    .unwrap(),
                factory,
            })
            .unwrap(),
        ))
        .unwrap();
    Arc::new(
        plugins
            .freeze(
                ArtifactSetDigest::from_bytes([0x7a; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap(),
    )
}

pub(super) struct JournalFixture {
    pub ports: Ports,
    pub frozen: Arc<FrozenPluginRegistry>,
    pub definition: WorkflowDefinition,
    pub scope: Scope,
    pub gateway: Arc<Gateway>,
    pub controls: Arc<Controls>,
    pub manager: Arc<Manager>,
    /// The credential identity the engine resolves the payments row under.
    pub identity: parking_lot::Mutex<SlotIdentity>,
}

impl JournalFixture {
    pub(super) async fn new(ports: Ports) -> Self {
        Self::build(ports, Kind::Stateless, None).await
    }

    /// A fixture whose node retries a retryable failure under `retry`.
    pub(super) async fn with_retry(ports: Ports, retry: nebula_workflow::RetryConfig) -> Self {
        Self::build(ports, Kind::Stateless, Some(retry)).await
    }

    /// A fixture whose workflow fails nodes under `strategy`.
    pub(super) async fn with_error_strategy(
        ports: Ports,
        strategy: nebula_workflow::ErrorStrategy,
    ) -> Self {
        Self::build_with(ports, Kind::Stateless, None, strategy).await
    }

    pub(super) async fn build(
        ports: Ports,
        kind: Kind,
        retry: Option<nebula_workflow::RetryConfig>,
    ) -> Self {
        Self::build_with(
            ports,
            kind,
            retry,
            nebula_workflow::ErrorStrategy::default(),
        )
        .await
    }

    async fn build_with(
        ports: Ports,
        kind: Kind,
        retry: Option<nebula_workflow::RetryConfig>,
        strategy: nebula_workflow::ErrorStrategy,
    ) -> Self {
        let gateway = Arc::new(Gateway::default());
        let controls = match kind {
            Kind::Stateless => Arc::new(Controls::default()),
            Kind::Stateful => Arc::clone(STATEFUL_CONTROLS.get_or_init(Arc::default)),
        };
        let manager = Arc::new(Manager::new());
        for identity in [
            SlotIdentity::Unbound,
            bound_to("cred-a"),
            bound_to("cred-b"),
        ] {
            manager
                .register(RegistrationSpec {
                    resource: Payments(Arc::clone(&gateway)),
                    config: (),
                    scope: ScopeLevel::Global,
                    slot_identity: identity,
                    topology: Resident::<Payments>::new(ResidentConfig::default()),
                    recovery_gate: None,
                    rate_limit: None,
                })
                .expect("register the payments row");
        }
        let frozen = frozen_plugin(kind, &controls);
        let mut node =
            NodeDefinition::new(node_key!("charge"), "Charge", "journal", "charge").unwrap();
        node.retry_policy = retry;
        let mut definition = WorkflowBuilder::new("Journaled effects")
            .add_node(node)
            .build()
            .unwrap();
        definition.config.error_strategy = strategy;
        let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
        ports
            .workflows
            .workflow
            .create(
                &scope,
                WorkflowRecord {
                    id: definition.id.to_string(),
                    scope: scope.clone(),
                    version: 1,
                    slug: "journaled-effects".into(),
                    deleted: false,
                },
            )
            .await
            .unwrap();
        WorkflowActivationService::new(
            ports.workflows.workflow.clone(),
            ports.workflows.versions.clone(),
            frozen.clone(),
            PlanFlavorRevisionInstaller::new(ports.writer.clone()),
            Arc::new(SystemClock),
        )
        .activate(&scope, definition.id, 1, definition.clone())
        .await
        .unwrap_or_else(|error| panic!("fixture activation failed: {error}"));
        Self {
            ports,
            frozen,
            definition,
            scope,
            gateway,
            controls,
            manager,
            identity: parking_lot::Mutex::new(SlotIdentity::Unbound),
        }
    }

    /// Admits an execution running `units` (JSON unit specs) with `extra`
    /// script flags.
    pub(super) async fn start(&self, units: &[Value], extra: Value) -> nebula_core::ExecutionId {
        let mut input = json!({ "units": units });
        if let (Some(input), Value::Object(extra)) = (input.as_object_mut(), extra) {
            input.extend(extra);
        }
        WorkflowStartService::new(
            self.ports.workflows.clone(),
            self.ports.stores.execution.clone(),
            self.ports.starts.clone(),
            PlanFlavorRevisionLoader::new(self.ports.catalog.clone()),
            self.frozen.clone(),
            Arc::new(SystemClock),
            ExecutionBudget::default(),
        )
        .unwrap()
        .start(
            &self.scope,
            self.definition.id,
            Some(input),
            // Every start is a new execution, never an idempotent replay.
            Some(&nebula_core::ExecutionId::new().to_string()),
            None,
        )
        .await
        .unwrap()
        .state()
        .execution_id
    }

    /// A fresh engine over the fixture's ports, resource manager and current
    /// credential identity.
    pub(super) fn engine(&self) -> WorkflowEngine {
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                Arc::new(ActionRegistry::new()),
                Arc::new(InProcessRunner::new()),
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .unwrap(),
        );
        let engine = WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_resource_manager(Arc::clone(&self.manager))
            .with_execution_stores(self.ports.stores.clone())
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(self.ports.catalog.clone())),
                self.frozen.clone(),
                self.ports.starts.clone(),
            );
        engine.record_resource_slot_identity(
            ScopeLevel::Global,
            Payments::key(),
            self.identity.lock().clone(),
        );
        engine
    }

    /// Runs one turn of `execution`.
    pub(super) async fn run(
        &self,
        execution: nebula_core::ExecutionId,
    ) -> Result<nebula_engine::ExecutionResult, nebula_engine::EngineError> {
        tokio::time::timeout(
            HANG_GUARD,
            self.engine().resume_execution(&self.scope, execution),
        )
        .await
        .expect("the turn finishes")
    }

    /// Runs a turn of `execution` until `entered` fires, then kills it the
    /// way a crashed process would.
    pub(super) async fn crash_at(
        &self,
        execution: nebula_core::ExecutionId,
        entered: &tokio::sync::Notify,
    ) {
        let engine = self.engine();
        let scope = self.scope.clone();
        let turn = tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
        tokio::time::timeout(HANG_GUARD, entered.notified())
            .await
            .expect("the turn reaches the crash point");
        turn.abort();
        assert!(turn.await.unwrap_err().is_cancelled());
    }

    /// Every slot the node prepared, in preparation order.
    pub(super) async fn slots(
        &self,
        execution: nebula_core::ExecutionId,
    ) -> Vec<EffectOccurrenceRecord> {
        self.ports
            .ledger
            .read_occurrences(&self.scope, &execution.to_string(), NODE)
            .await
            .unwrap()
    }
}

/// The phase of `slot`.
pub(super) fn phase(slot: &EffectOccurrenceRecord) -> EffectPhase {
    slot.record().protocol().unwrap().phase()
}

/// The provider key recorded for `slot`.
pub(super) fn recorded_key(slot: &EffectOccurrenceRecord) -> String {
    slot.record()
        .operation()
        .provider_key()
        .expect("a journaled effect records its provider key")
        .to_string()
}

/// The durable error record of the failed node.
pub(super) fn node_error(result: &nebula_engine::ExecutionResult) -> &str {
    result
        .node_errors
        .get(&node_key!("charge"))
        .map(String::as_str)
        .expect("the node failed with a durable error record")
}

/// The receipts the node returned.
pub(super) fn receipts(result: &nebula_engine::ExecutionResult) -> Value {
    result.node_outputs[&node_key!("charge")]["receipts"].clone()
}
