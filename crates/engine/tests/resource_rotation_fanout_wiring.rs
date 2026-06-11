#![cfg(feature = "rotation")]

//! End-to-end: the production rotation fan-out **wiring** (
//! §Deferred "Rotation fan-out is implemented but unwired"; closes #679 /
//! #680 / #681 prerequisites).
//!
//! Unlike `rotation_resource_fanout.rs` (which calls
//! `ResourceFanoutIndex::dispatch_*` directly), this exercises the REAL
//! engine path: a [`ResourceFanoutDriver`] spawned over real
//! `nebula-eventbus` buses, driven by **emitting the exact events the
//! credential-runtime composition root emits in production**
//! ([`CredentialEvent::Refreshed`] / [`CredentialEvent::Revoked`] on
//! `EventBus<CredentialEvent>`, [`LeaseEvent::LeaseRevoked`] on
//! `EventBus<LeaseEvent>` — the `EventMetricObserver` shape, ).
//! Nothing here calls `dispatch_refresh` / `dispatch_revoke` itself; the
//! driver's bus subscription does, exactly as in production.
//!
//! Wired path under test:
//! `EventBus → ResourceFanoutDriver subscriber → ResourceFanoutIndex
//!  ::dispatch_{refresh,revoke} → Manager::{refresh_slot_for,
//!  taint_slot_for + drain_and_revoke} → resource on_credential_* hook`.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialEvent, CredentialId, LeaseEvent};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox,
    WorkflowEngine,
    credential::rotation::{ResourceFanoutDriver, ResourceFanoutIndex},
};
use nebula_eventbus::EventBus;
use nebula_metrics::MetricsRegistry;
use nebula_resource::{
    AcquireOptions, Manager, Provider, RegistrationSpec, ResidentConfig, ResourceConfig,
    ResourceContext, SlotIdentity,
    error::Error as ResourceError,
    resource::ResourceMetadata,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use tokio_util::sync::CancellationToken;

// ── Test resource recording every rotation/revoke hook delivery ──────

#[derive(Debug)]
struct HookError(&'static str);
impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for HookError {}
impl From<HookError> for ResourceError {
    fn from(e: HookError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Hook returns `Ok` immediately.
    Ok,
    /// Hook never completes — models a wedged resource so the
    /// per-resource timeout must fire (`timed_out`) without ever
    /// un-tainting the row (the #681 invariant, end-to-end).
    Hang,
}

#[derive(Clone, Default)]
struct Recorder {
    refresh: Arc<AtomicUsize>,
    revoke: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct Recording {
    behaviour: Behaviour,
    rec: Recorder,
}

impl Provider for Recording {
    type Config = NoCfg;
    type Instance = ();

    fn key() -> ResourceKey {
        resource_key!("fanout-wiring-rec")
    }

    async fn create(&self, _c: &NoCfg, _x: &ResourceContext) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn on_credential_refresh(&self, _s: &str, _r: &()) -> Result<(), ResourceError> {
        self.rec.refresh.fetch_add(1, Ordering::SeqCst);
        drive(self.behaviour).await
    }

    async fn on_credential_revoke(&self, _s: &str, _r: &()) -> Result<(), ResourceError> {
        self.rec.revoke.fetch_add(1, Ordering::SeqCst);
        drive(self.behaviour).await
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl nebula_resource::HasCredentialSlots for Recording {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl Resident for Recording {
    fn is_alive_sync(&self, _r: &()) -> bool {
        true
    }
}

async fn drive(b: Behaviour) -> Result<(), ResourceError> {
    match b {
        Behaviour::Ok => Ok(()),
        Behaviour::Hang => {
            std::future::pending::<()>().await;
            // guard-justified: `std::future::pending()` never resolves,
            // so this line is statically unreachable (the wedged arm).
            unreachable!()
        },
    }
}

#[derive(Clone)]
struct NoCfg;
nebula_schema::impl_empty_has_schema!(NoCfg);
impl ResourceConfig for NoCfg {
    fn validate(&self) -> Result<(), ResourceError> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

// ── Harness ─────────────────────────────────────────────────────────

/// Spin up a real `Manager` + bound resident row under one
/// `(key, scope)` keyed by `slot_identity == cid.bits`, a real
/// `EventBus<CredentialEvent>` + `EventBus<LeaseEvent>`, and the
/// production [`ResourceFanoutDriver`] spawned over them. Returns the
/// pieces a test drives the wired path with.
struct Wired {
    cred_bus: Arc<EventBus<CredentialEvent>>,
    lease_bus: Arc<EventBus<LeaseEvent>>,
    mgr: Arc<Manager>,
    cid: CredentialId,
    org: OrgId,
    slot_identity: SlotIdentity,
    rec: Recorder,
    // Held: dropping aborts the driver task.
    _driver: ResourceFanoutDriver,
}

async fn wire(behaviour: Behaviour) -> Wired {
    let rec = Recorder::default();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let mgr = Arc::new(Manager::new());
    let index = Arc::new(ResourceFanoutIndex::new());
    let cid = CredentialId::new();
    // The collision-free structural identity of the single resolved row,
    // derived from the same `(slot, credential)` binding the resource
    // would resolve — used at register, acquire, and bind.
    let slot_identity = SlotIdentity::from_bindings([("db", "wired-cred")]);

    mgr.register(RegistrationSpec {
        resource: Recording {
            behaviour,
            rec: rec.clone(),
        },
        config: NoCfg,
        scope: scope.clone(),
        slot_identity: slot_identity.clone(),
        topology: TopologyRuntime::Resident(ResidentRuntime::<Recording>::new(
            ResidentConfig::default(),
        )),
        acquire: Manager::erased_acquire_resident_for::<Recording>(),
        recovery_gate: None,
    })
    .expect("register resolved-credential row");

    // Resident materializes its shared runtime lazily on first acquire —
    // warm it so the rotation/revoke hook has a live `&Runtime`.
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let _g = mgr
        .acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &slot_identity,
        )
        .await
        .expect("warm resident runtime");
    drop(_g);

    // The bind seam: this is what the resource-activation path records
    // when a credential resolves into a `#[credential]` slot. We bind
    // directly here (the production registrar bind path is covered by
    // the registrar unit tests) so this test isolates the *driver*
    // wiring: bus event → driver → fan-out → hook.
    index.bind(
        cid,
        Recording::key(),
        scope.clone(),
        "db",
        slot_identity.clone(),
    );

    let cred_bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let lease_bus = Arc::new(EventBus::<LeaseEvent>::new(16));
    let driver = ResourceFanoutDriver::spawn(
        Arc::clone(&index),
        Arc::clone(&mgr),
        Arc::clone(&cred_bus),
        Some(Arc::clone(&lease_bus)),
    );

    Wired {
        cred_bus,
        lease_bus,
        mgr,
        cid,
        org,
        slot_identity,
        rec,
        _driver: driver,
    }
}

/// Poll `cond` up to ~2s (yielding) — the driver runs on its own task,
/// so a bus emission is observed asynchronously. Fails loudly on
/// timeout rather than hanging the runner.
async fn eventually(label: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("condition `{label}` not reached within ~2s — driver wiring did not fire");
}

// ── Tests ───────────────────────────────────────────────────────────

/// A `CredentialEvent::Refreshed` on the credential bus (exactly what
/// `EventMetricObserver::on_refresh` emits after a refresh CAS-persists
/// fresh material) must drive `dispatch_refresh` through the spawned
/// driver and deliver `on_credential_refresh` to the bound resource.
#[tokio::test]
async fn refreshed_event_drives_fanout_to_resource_hook() {
    let w = wire(Behaviour::Ok).await;

    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });

    eventually("refresh hook delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        0,
        "a Refreshed event must not drive the revoke hook"
    );
}

/// A `CredentialEvent::Revoked` (the facade-level revoke signal)
/// must drive `dispatch_revoke` → taint → drain → `on_credential_revoke`.
#[tokio::test]
async fn credential_revoked_event_drives_revoke_fanout() {
    let w = wire(Behaviour::Ok).await;

    w.cred_bus.emit(CredentialEvent::Revoked {
        credential_id: w.cid,
    });

    eventually("revoke hook delivered", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;
}

/// A `LeaseEvent::LeaseRevoked` carrying an attributed `credential_id`
/// (what the lease scheduler emits via `EventBus<LeaseEvent>` after
/// `LeaseLifecycle::revoke_for_credential`) must drive the revoke
/// fan-out: the row is tainted (a subsequent acquire on that exact
/// resolved row is rejected) and `on_credential_revoke` is delivered —
/// the → path end-to-end.
#[tokio::test]
async fn lease_revoked_event_taints_row_and_delivers_revoke_hook() {
    use nebula_error::{Classify, ErrorCategory};

    let w = wire(Behaviour::Ok).await;

    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(w.cid),
        lease_id: "lease-xyz".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    eventually("revoke hook delivered via lease bus", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;

    // The decisive cross-ADR assertion: the revoke fan-out tainted the
    // resolved row, so a fresh acquire on it is now rejected.
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(w.org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let acquired = w
        .mgr
        .acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &w.slot_identity,
        )
        .await;
    let err = match acquired {
        Err(e) => e,
        Ok(_) => unreachable!(
            // guard-justified: a live guard here means the lease-revoke
            // fan-out did not taint the row — the exact wiring
            // regression this test exists to catch; fail loudly.
            "acquire after a LeaseRevoked-driven revoke must be rejected (row tainted)"
        ),
    };
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "post-revoke acquire must be the Revoked/Unavailable taint rejection, got: {err}"
    );
}

/// #681 end-to-end through the wired path: a `LeaseRevoked` whose
/// resource revoke hook **hangs** must record `timed_out` inside the
/// fan-out yet still leave the row tainted (the synchronous
/// `taint_slot_for` ran outside the per-resource timeout). Proven via
/// the wired driver, not a direct `dispatch_revoke`.
#[tokio::test]
async fn lease_revoked_with_hung_hook_still_taints_row() {
    use nebula_error::{Classify, ErrorCategory};

    let w = wire(Behaviour::Hang).await;

    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(w.cid),
        lease_id: "lease-hang".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // The hung hook is entered (phase 2 reached it) — proof the revoke
    // fan-out ran through the wired driver even though it will time out.
    eventually("hung revoke hook entered", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;

    // Even while the drain tail is still timing out, the synchronous
    // phase-1 taint already revoked the row: a fresh acquire is
    // rejected. The hook having been entered above proves phase 2
    // started, which means phase 1 (the synchronous taint) already
    // completed — so the row is tainted *now*. One acquire attempt,
    // bounded so a wiring regression (taint not applied ⇒ the resident
    // acquire would otherwise succeed) fails loudly instead of hanging.
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(w.org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let acquired = tokio::time::timeout(
        Duration::from_secs(2),
        w.mgr.acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &w.slot_identity,
        ),
    )
    .await
    .expect("acquire on a tainted row must resolve immediately (rejected), not hang");
    let err = match acquired {
        Err(e) => e,
        Ok(_) => unreachable!(
            // guard-justified: a live guard means the synchronous
            // phase-1 taint did not stick across the hung phase-2 —
            // the exact #681 wiring regression; fail loudly.
            "acquire during a hung revoke must be rejected — phase-1 taint \
             ran synchronously before the timeout (#681)"
        ),
    };
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "hung-revoke acquire must hit the Revoked/Unavailable taint, got: {err}"
    );
}

/// Orphan lease (`credential_id == None`) cannot address a reverse-index
/// row — the driver must treat it as a no-op fan-out, never an error,
/// and never touch the bound resource.
#[tokio::test]
async fn orphan_lease_revoked_is_noop() {
    let w = wire(Behaviour::Ok).await;

    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: None,
        lease_id: "orphan".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // Drive a real refresh afterwards so we can prove the driver is
    // alive and processing — and that the orphan revoke did NOT deliver
    // a revoke hook.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("post-orphan refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        0,
        "an orphan LeaseRevoked (no credential id) must not deliver any revoke hook"
    );
}

/// After the bound resource is removed from the manager, a subsequent
/// rotation for that credential fans to zero rows: no stale `Bind`, no
/// bogus `failed`. Proven through the wired driver (emit `Refreshed`
/// post-remove; the resource hook must NOT be delivered and nothing
/// errors).
#[tokio::test]
async fn rotation_after_resource_removed_fans_to_zero_rows() {
    let w = wire(Behaviour::Ok).await;

    // First refresh: delivered (sanity that wiring is live).
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("pre-remove refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;

    // Remove the resource from the manager.
    w.mgr.remove(&Recording::key()).expect("resource removed");

    // Second refresh after removal. The reverse index still holds the
    // bind (unbind on remove is the registrar/activation path's job,
    // covered separately), so the fan-out DOES dispatch — but
    // `refresh_slot_for` now resolves no live row and records `failed`,
    // NOT a delivered hook. The decisive assertion: the resource hook
    // is not delivered a second time and the driver does not panic.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });

    // Give the driver ample time to process the second event.
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(
        w.rec.refresh.load(Ordering::SeqCst),
        1,
        "after the resource was removed, a rotation must NOT deliver its \
         hook again (fans to a now-dead row, recorded failed — not a \
         second hook call)"
    );
}

/// #690 review (P1, comment 3255603311 / 3255606374) — one logical
/// credential revoke double-emits across the two buses
/// (`LeaseEvent::LeaseRevoked` then the facade `CredentialEvent::Revoked`,
/// back-to-back inside one `CredentialService::revoke`). The driver's
/// per-credential dedupe window must collapse them so
/// `on_credential_revoke` fans out **exactly once** per logical revoke
/// (non-idempotent hooks must not double-fire).
#[tokio::test]
async fn duplicate_revoke_across_both_buses_fans_out_once() {
    let w = wire(Behaviour::Ok).await;

    // Lease bus first (the lease scheduler emits LeaseRevoked for the
    // released lease) ...
    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(w.cid),
        lease_id: "lease-dup".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });
    // ... then the facade emits CredentialEvent::Revoked for the SAME
    // credential — the second surfacing of one logical revoke.
    w.cred_bus.emit(CredentialEvent::Revoked {
        credential_id: w.cid,
    });

    // The first revoke is delivered.
    eventually("first revoke hook delivered", || {
        w.rec.revoke.load(Ordering::SeqCst) == 1
    })
    .await;

    // Drive a refresh afterwards and wait for it: the driver processes
    // events in order on its single task, so once the refresh is
    // observed the duplicate revoke (emitted before it) has definitely
    // been processed too — and must have been skipped by the dedupe.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("post-duplicate refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        1,
        "the lease-bus + credential-bus double-emission of ONE logical \
         revoke must fan out the revoke hook exactly once (deduped)"
    );
}

/// #690 review (CodeRabbit nitpick, fix J) — a `LeaseRevoked` carrying a
/// `credential_id` that was **never bound** (a real credential id, but no
/// reverse-index row for it) exercises the "lookup succeeds, zero binds"
/// branch — distinct from the "no credential id" orphan branch
/// ([`orphan_lease_revoked_is_noop`]). Processing must continue and the
/// revoke hook count must stay 0 (nothing bound that credential).
#[tokio::test]
async fn lease_revoked_for_never_bound_credential_is_zero_binds_noop() {
    let w = wire(Behaviour::Ok).await;

    // A real, attributed credential id — but one that no resource row
    // ever bound (distinct from `w.cid`, which is the bound one). The
    // reverse-index lookup succeeds yet yields zero rows.
    let never_bound = CredentialId::new();
    assert_ne!(never_bound, w.cid, "must be a different credential id");
    w.lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(never_bound),
        lease_id: "lease-unbound".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // Prove the driver is still alive and processing after the
    // zero-binds revoke: a real refresh for the BOUND credential is
    // still delivered.
    w.cred_bus.emit(CredentialEvent::Refreshed {
        credential_id: w.cid,
    });
    eventually("post-zero-binds refresh delivered", || {
        w.rec.refresh.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        w.rec.revoke.load(Ordering::SeqCst),
        0,
        "a LeaseRevoked for a never-bound credential must fan to zero rows \
         — no revoke hook delivered (lookup-succeeds-zero-binds branch)"
    );
}

// ── Fix D: idempotent engine driver spawn ───────────────────────────

/// No-op action executor — the engine under test never dispatches a
/// workflow; it only exercises `spawn_resource_rotation_fanout`.
fn noop_engine_with_manager(manager: Arc<Manager>) -> WorkflowEngine {
    let registry = Arc::new(ActionRegistry::new());
    let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
        Box::pin(async move { Ok(nebula_action::result::ActionResult::success(input)) })
    });
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("ActionRuntime::try_new"),
    );
    WorkflowEngine::new(runtime, metrics)
        .expect("WorkflowEngine::new")
        .with_resource_manager(manager)
}

/// #690 review (Major, comment 3255607651) —
/// `WorkflowEngine::spawn_resource_rotation_fanout` must be **single-shot**:
/// a second call must NOT spawn a second subscriber pair (which would
/// double-dispatch every refresh/revoke). The second call returns `None`,
/// and a single emitted refresh is delivered to the bound resource hook
/// **exactly once** (a second subscriber would deliver it twice).
#[tokio::test]
async fn engine_spawn_resource_rotation_fanout_is_idempotent() {
    let rec = Recorder::default();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let mgr = Arc::new(Manager::new());
    let cid = CredentialId::new();
    let slot_identity = SlotIdentity::from_bindings([("db", "wired-cred-2")]);

    mgr.register(RegistrationSpec {
        resource: Recording {
            behaviour: Behaviour::Ok,
            rec: rec.clone(),
        },
        config: NoCfg,
        scope: scope.clone(),
        slot_identity: slot_identity.clone(),
        topology: TopologyRuntime::Resident(ResidentRuntime::<Recording>::new(
            ResidentConfig::default(),
        )),
        acquire: Manager::erased_acquire_resident_for::<Recording>(),
        recovery_gate: None,
    })
    .expect("register resolved-credential row");

    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let _g = mgr
        .acquire_resident_for_identity::<Recording>(
            &ctx,
            &AcquireOptions::default(),
            &slot_identity,
        )
        .await
        .expect("warm resident runtime");
    drop(_g);

    let engine = noop_engine_with_manager(Arc::clone(&mgr));
    // Bind the resolved row into the engine-owned reverse index so a
    // rotation has a row to fan to.
    engine.resource_fanout_index().bind(
        cid,
        Recording::key(),
        scope.clone(),
        "db",
        slot_identity.clone(),
    );

    let cred_bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let lease_bus = Arc::new(EventBus::<LeaseEvent>::new(16));

    // First spawn: succeeds.
    let _driver = engine
        .spawn_resource_rotation_fanout(Arc::clone(&cred_bus), Some(Arc::clone(&lease_bus)))
        .expect("first spawn must return a driver");

    // Second spawn: idempotent — must NOT spawn a second subscriber.
    let second =
        engine.spawn_resource_rotation_fanout(Arc::clone(&cred_bus), Some(Arc::clone(&lease_bus)));
    assert!(
        second.is_none(),
        "a second spawn_resource_rotation_fanout must return None (driver \
         already running) — no second subscriber"
    );

    // One refresh emitted. With a single subscriber the bound resource
    // hook fires exactly once; a leaked second subscriber would fire it
    // twice.
    cred_bus.emit(CredentialEvent::Refreshed { credential_id: cid });

    // Wait for the hook, then drain extra scheduler turns and assert the
    // count never exceeds 1 (the double-subscribe regression).
    for _ in 0..2000 {
        if rec.refresh.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(
        rec.refresh.load(Ordering::SeqCst),
        1,
        "exactly one refresh-hook delivery — a second spawn must not have \
         created a second subscriber that double-dispatches"
    );
}
