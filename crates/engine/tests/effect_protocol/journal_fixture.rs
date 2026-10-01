//! A journaled action through real activation, start admission and durable
//! turns: a default-contract (`Journaled`) action — stateless by default,
//! or of another [`Kind`] — submits effect units on the resource handle of a fake payment gateway that counts every
//! provider call and deduplicates by the idempotency key it receives.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use nebula_action::{
    ActionContext, ActionContextExt, ActionError, ControlAction, ControlOutcome,
    GenericControlFactory, GenericStatefulFactory, InstanceFactory, StatefulAction,
    StatelessAction,
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
    /// The next call is throttled: the provider applies nothing.
    pub throttle_next: AtomicBool,
    /// Fired whenever a call reaches the gateway.
    pub entered: tokio::sync::Notify,
    /// What the stateful action consults at the start of every iteration —
    /// this gateway's own, so no state is shared between tests.
    pub iterations: IterationControls,
}

/// Per-iteration test controls of the stateful action, which it reads
/// through a free `Read` unit ([`Consult`]) at the start of every
/// iteration. A one-shot control fires on the first run that reaches its
/// iteration; the others apply on every run while set.
#[derive(Debug, Default)]
pub(super) struct IterationControls {
    /// Every iteration started, in order, across runs.
    pub started: parking_lot::Mutex<Vec<u32>>,
    /// One-shot: the iteration never gets past its start (a crash point).
    pub hold_at: parking_lot::Mutex<Option<u32>>,
    /// Fired when an iteration reached `hold_at`.
    pub held: tokio::sync::Notify,
    /// One-shot: the iteration's first call is applied and then waits at
    /// `call_gate`.
    pub hold_call_at: parking_lot::Mutex<Option<u32>>,
    /// The gate a `hold_call_at` call waits at.
    pub call_gate: Arc<Gate>,
    /// One-shot: the answer of the iteration's first call is lost.
    pub lose_at: parking_lot::Mutex<Option<u32>>,
    /// One-shot: the iteration's first call is throttled.
    pub throttle_at: parking_lot::Mutex<Option<u32>>,
    /// The iteration's units carry this request instead.
    pub override_at: parking_lot::Mutex<Option<(u32, String)>>,
    /// The iteration submits one more write after its units.
    pub extra_at: parking_lot::Mutex<Option<u32>>,
    /// The iteration skips its units.
    pub skip_at: parking_lot::Mutex<Option<u32>>,
    /// The iteration submits only its first so many units.
    pub keep_at: parking_lot::Mutex<Option<(u32, usize)>>,
    /// The action completes at the iteration, before its units.
    pub break_at: parking_lot::Mutex<Option<u32>>,
    /// The action fails (not retryable) at the iteration, before its units.
    pub fail_at: parking_lot::Mutex<Option<u32>>,
    /// The iteration submits its units without awaiting them.
    pub leak_at: parking_lot::Mutex<Option<u32>>,
    /// The iteration returns a timer `Wait` after its units.
    pub wait_at: parking_lot::Mutex<Option<u32>>,
}

/// What the stateful action does in one iteration, as the gateway's
/// [`IterationControls`] decide it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct IterationPlan {
    request_override: Option<String>,
    extra: bool,
    skip: bool,
    keep: Option<usize>,
    stop: bool,
    fail: bool,
    leak: bool,
    wait: bool,
}

/// Takes a one-shot control when it is set for `iteration`.
fn fires(control: &parking_lot::Mutex<Option<u32>>, iteration: u32) -> bool {
    let mut control = control.lock();
    if *control == Some(iteration) {
        *control = None;
        return true;
    }
    false
}

/// Whether a lasting control is set for `iteration`.
fn applies(control: &parking_lot::Mutex<Option<u32>>, iteration: u32) -> bool {
    *control.lock() == Some(iteration)
}

impl Gateway {
    /// The plan of the stateful action's `iteration`, arming the gateway's
    /// one-shot faults for the iteration's calls.
    async fn consult(&self, iteration: u32) -> IterationPlan {
        let controls = &self.iterations;
        controls.started.lock().push(iteration);
        if fires(&controls.hold_at, iteration) {
            controls.held.notify_one();
            std::future::pending::<()>().await;
        }
        if fires(&controls.hold_call_at, iteration) {
            *self.hold_next.lock() = Some(Arc::clone(&controls.call_gate));
        }
        if fires(&controls.lose_at, iteration) {
            self.lose_first.store(1, Ordering::SeqCst);
        }
        if fires(&controls.throttle_at, iteration) {
            self.throttle_next.store(true, Ordering::SeqCst);
        }
        IterationPlan {
            request_override: controls
                .override_at
                .lock()
                .as_ref()
                .filter(|(at, _)| *at == iteration)
                .map(|(_, request)| request.clone()),
            extra: applies(&controls.extra_at, iteration),
            skip: applies(&controls.skip_at, iteration),
            keep: controls
                .keep_at
                .lock()
                .filter(|(at, _)| *at == iteration)
                .map(|(_, keep)| keep),
            stop: applies(&controls.break_at, iteration),
            fail: applies(&controls.fail_at, iteration),
            leak: applies(&controls.leak_at, iteration),
            wait: applies(&controls.wait_at, iteration),
        }
    }

    /// The iterations the stateful action started, across runs.
    pub(super) fn started(&self) -> Vec<u32> {
        self.iterations.started.lock().clone()
    }
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
        if self.throttle_next.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            return Err(OperationError::throttled(None));
        }
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

/// The stateful action's look at its iteration's controls: a free `Read`,
/// never journaled.
#[derive(Serialize, Deserialize)]
struct Consult {
    iteration: u32,
}

impl Operation<Payments> for Consult {
    type Output = IterationPlan;
    const KEY: &'static str = "test.consult";
    const EFFECT: Effect = Effect::Read;

    async fn run(
        self,
        cx: &mut OperationCx<'_, Payments>,
    ) -> Result<IterationPlan, OperationError> {
        let iteration = self.iteration;
        cx.call(Cost::FREE, async move |gateway, ()| {
            Ok(gateway.consult(iteration).await)
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
    /// Retryable failures the action returns before its units.
    pub fail_before_units: AtomicU32,
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
    let handle = ctx.resource_handle_by_id::<Payments>(Payments::key().as_str())?;
    if controls.drop_unpolled_once.swap(false, Ordering::SeqCst) {
        drop(handle.submit(Charge::<false>(UnitSpec {
            idempotent: false,
            request: "never-polled".to_owned(),
            key: None,
            budget: 1,
        })));
    }
    if controls
        .fail_before_units
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(1)
        })
        .is_ok()
    {
        return Err(ActionError::retryable("upstream step failed"));
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
    Ok(json!({ "receipts": receipts }))
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

/// What the stateful action does, as the execution input says: the units
/// of each iteration, in order.
#[derive(Debug, Deserialize)]
struct IterationScript {
    iterations: Vec<Vec<UnitSpec>>,
    /// Swallow unit errors (recording their kind) and go on anyway.
    #[serde(default)]
    swallow: bool,
    /// Before its units, the first iteration submits this many writes of
    /// distinct requests (a node over its journaled slot cap).
    #[serde(default)]
    fill: u32,
    /// Submit every unit of an iteration and await them together
    /// (`join_all`), instead of one after the other.
    #[serde(default)]
    concurrent: bool,
}

/// The stateful action's state: the next iteration and every receipt so
/// far.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct IterationState {
    next: u32,
    receipts: Vec<Value>,
}

/// The journaled stateful action: one iteration per entry of its script,
/// each consulting the gateway's [`IterationControls`] first. Its controls
/// live on the fixture's own gateway, so concurrent tests share nothing.
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
    type State = IterationState;

    fn init_state(&self) -> IterationState {
        IterationState::default()
    }

    async fn execute(
        &self,
        input: &Value,
        state: &mut IterationState,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let script: IterationScript = serde_json::from_value(input.clone())
            .map_err(|error| ActionError::fatal(format!("bad script: {error}")))?;
        let handle = ctx.resource_handle_by_id::<Payments>(Payments::key().as_str())?;
        let iteration = state.next;
        let plan = handle.submit(Consult { iteration }).await?;
        if plan.fail {
            return Err(ActionError::fatal("the iteration failed"));
        }
        if plan.stop {
            return Ok(ActionResult::Break {
                output: nebula_action::ActionOutput::Value(json!({ "receipts": state.receipts })),
                reason: nebula_action::BreakReason::Completed,
            });
        }
        let mut units = if plan.skip {
            Vec::new()
        } else {
            script
                .iterations
                .get(usize::try_from(iteration).unwrap())
                .cloned()
                .unwrap_or_default()
        };
        if let Some(keep) = plan.keep {
            units.truncate(keep);
        }
        if iteration == 0 {
            let fill = (0..script.fill).map(|n| UnitSpec {
                idempotent: false,
                request: format!("fill-{n}"),
                key: None,
                budget: 1,
            });
            units.splice(0..0, fill);
        }
        if plan.extra {
            units.push(UnitSpec {
                idempotent: false,
                request: format!("extra-{iteration}"),
                key: None,
                budget: 1,
            });
        }
        let submit = |mut spec: UnitSpec| {
            if let Some(request) = &plan.request_override {
                spec.request.clone_from(request);
            }
            if spec.idempotent {
                handle.submit(Charge::<true>(spec))
            } else {
                handle.submit(Charge::<false>(spec))
            }
        };
        let settled = if script.concurrent {
            futures::future::join_all(units.into_iter().map(submit)).await
        } else {
            let mut settled = Vec::new();
            for spec in units {
                let unit = submit(spec);
                if plan.leak {
                    drop(tokio::spawn(unit));
                    continue;
                }
                let result = unit.await;
                let failed = result.is_err();
                settled.push(result);
                if failed && !script.swallow {
                    break;
                }
            }
            settled
        };
        for result in settled {
            match result {
                Ok(receipt) => state.receipts.push(json!(receipt)),
                Err(error) if script.swallow => state.receipts.push(json!({
                    "kind": error.kind().to_string(),
                    "sent": error.sent().as_str(),
                    "detail": error.detail(),
                })),
                Err(error) => return Err(error.into()),
            }
        }
        state.next += 1;
        let output = nebula_action::ActionOutput::Value(json!({ "receipts": state.receipts }));
        if plan.wait {
            return Ok(ActionResult::Wait {
                condition: nebula_action::WaitCondition::Duration {
                    duration: std::time::Duration::from_millis(50),
                },
                timeout: None,
                partial_output: Some(output),
            });
        }
        if usize::try_from(state.next).unwrap() >= script.iterations.len() {
            return Ok(ActionResult::Break {
                output,
                reason: nebula_action::BreakReason::Completed,
            });
        }
        Ok(ActionResult::Continue {
            output,
            progress: None,
            delay: None,
        })
    }
}

/// A control action of the default (`Journaled`) contract that tries the
/// script's units anyway and passes their receipts on. It runs with fresh
/// default controls on every evaluation — no state shared between tests.
#[derive(nebula_action::Action)]
#[action(
    key = "journal.charge",
    name = "Charge",
    description = "Default-contract control test action",
    input = Value,
    output = Value
)]
struct ControlCharge;

impl ControlAction for ControlCharge {
    async fn evaluate(
        &self,
        input: Value,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<Value>, ActionError> {
        let script: Script = serde_json::from_value(input)
            .map_err(|error| ActionError::fatal(format!("bad script: {error}")))?;
        let output = run_script(&Controls::default(), script, ctx).await?;
        Ok(ControlOutcome::Pass { output })
    }
}

/// The control action of a read-only contract, like the built-in control
/// actions (If, Switch, Filter).
#[derive(nebula_action::Action)]
#[action(
    key = "journal.charge",
    name = "Charge",
    description = "Read-only control test action",
    input = Value,
    output = Value,
    read_only
)]
struct ReadOnlyControlCharge;

impl ControlAction for ReadOnlyControlCharge {
    async fn evaluate(
        &self,
        input: Value,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome<Value>, ActionError> {
        ControlCharge.evaluate(input, ctx).await
    }
}

/// Which action kind the fixture's node runs. (An agent action cannot be
/// compiled into a durable plan; `resource_integration` covers its handles.)
#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Stateless,
    Stateful,
    /// A control action of the default (`Journaled`) contract.
    Control,
    /// A control action of the `ReadOnly` contract.
    ReadOnlyControl,
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
        Kind::Control => Arc::new(GenericControlFactory::<ControlCharge>::new().expect("admitted")),
        Kind::ReadOnlyControl => {
            Arc::new(GenericControlFactory::<ReadOnlyControlCharge>::new().expect("admitted"))
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

    pub(super) async fn build_with(
        ports: Ports,
        kind: Kind,
        retry: Option<nebula_workflow::RetryConfig>,
        strategy: nebula_workflow::ErrorStrategy,
    ) -> Self {
        let gateway = Arc::new(Gateway::default());
        // A stateful action reads its controls from the fixture's gateway; a
        // control action runs with its own fresh controls.
        let controls = Arc::new(Controls::default());
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
        self.start_input(script_input(json!({ "units": units }), extra))
            .await
    }

    /// Admits an execution of the stateful action running `iterations`
    /// (each a list of JSON unit specs) with `extra` script flags.
    pub(super) async fn start_iterations(
        &self,
        iterations: &[&[Value]],
        extra: Value,
    ) -> nebula_core::ExecutionId {
        self.start_input(script_input(json!({ "iterations": iterations }), extra))
            .await
    }

    async fn start_input(&self, input: Value) -> nebula_core::ExecutionId {
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

    /// Runs the fixture's workflow on `input` once, in process, on an
    /// engine without execution stores: no operation ledger, so no effect
    /// journal.
    pub(super) async fn run_storeless(&self, input: Value) -> nebula_engine::ExecutionResult {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_factory(
            self.frozen
                .resolve_action(&action_key!("journal.charge"))
                .expect("the fixture's action"),
        );
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                registry,
                Arc::new(InProcessRunner::new()),
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .unwrap(),
        );
        let engine = WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_resource_manager(Arc::clone(&self.manager));
        engine.record_resource_slot_identity(
            ScopeLevel::Global,
            Payments::key(),
            self.identity.lock().clone(),
        );
        // The registry resolves the node's action by its full key.
        let node = NodeDefinition::new(node_key!("charge"), "Charge", "journal", "journal.charge")
            .unwrap();
        let definition = WorkflowBuilder::new("Storeless journaled effects")
            .add_node(node)
            .build()
            .unwrap();
        tokio::time::timeout(
            HANG_GUARD,
            engine.execute_workflow(
                &nebula_engine::store_seam::single_tenant_scope(),
                &definition,
                input,
                ExecutionBudget::default(),
            ),
        )
        .await
        .expect("the run finishes")
        .expect("workflow execution")
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

/// A script `input` extended with the `extra` flags.
fn script_input(mut input: Value, extra: Value) -> Value {
    if let (Some(input), Value::Object(extra)) = (input.as_object_mut(), extra) {
        input.extend(extra);
    }
    input
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
