#![cfg(feature = "rotation")]

//! Integration: the engine credential-rotation fan-out drives the typed
//! `nebula_resource::Manager` slot ports per resolved registry row, with
//! per-resource timeout isolation (ADR-0036).
//!
//! Exercises the public engine surface
//! (`nebula_engine::credential::rotation::{ResourceFanoutIndex,
//! RotationOutcome}`) against a real `nebula_resource::Manager` holding
//! multiple resolved-credential registrations under one `(key, scope)`. The
//! invariant under test: one slow/wedged resource MUST NOT abort or fail its
//! siblings — every row's outcome is independent.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::CredentialId;
use nebula_engine::credential::rotation::{ResourceFanoutIndex, RotationOutcome};
use nebula_resource::{
    AcquireOptions, Manager, RegisterOptions, ResidentConfig, Resource, ResourceConfig,
    ResourceContext, error::Error as ResourceError, resource::ResourceMetadata,
    topology::resident::Resident,
};
use tokio_util::sync::CancellationToken;

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

#[derive(Clone)]
struct Cfg;
nebula_schema::impl_empty_has_schema!(Cfg);
impl ResourceConfig for Cfg {
    fn validate(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// `Err` is intentionally absent here — the mixed ok/err/timeout case is
/// covered by the `resource_fanout` lib unit tests. This integration test
/// proves only the cross-`Manager` timeout-isolation path end-to-end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Ok,
    Hang,
}

/// Resident resource whose refresh hook behaviour is selected by the
/// resolved slot identity it was registered under (shared map). Resident so
/// its runtime persists in `rt.current()` after one acquire (no pool
/// idle-queue release race).
#[derive(Clone)]
struct Ctl {
    identity: u64,
    behaviour: Arc<std::sync::Mutex<std::collections::HashMap<u64, Behaviour>>>,
    refresh_entered: Arc<AtomicUsize>,
}

impl Resource for Ctl {
    type Config = Cfg;
    type Runtime = ();
    type Lease = ();
    type Error = HookError;

    fn key() -> ResourceKey {
        resource_key!("it-fanout-ctl")
    }

    async fn create(&self, _c: &Cfg, _x: &ResourceContext) -> Result<(), HookError> {
        Ok(())
    }

    async fn on_credential_refresh(&self, _s: &str, _r: &()) -> Result<(), HookError> {
        self.refresh_entered.fetch_add(1, Ordering::SeqCst);
        let b = *self
            .behaviour
            .lock()
            .expect("behaviour map")
            .get(&self.identity)
            .unwrap_or(&Behaviour::Ok);
        match b {
            Behaviour::Ok => Ok(()),
            Behaviour::Hang => {
                std::future::pending::<()>().await;
                unreachable!()
            },
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for Ctl {
    fn is_alive_sync(&self, _r: &()) -> bool {
        true
    }
}

/// The headline ADR-0036 invariant proven through the public engine API:
/// a wedged resource times out in isolation; both healthy siblings still
/// refresh, and the aggregate accounts for every bound row.
#[tokio::test]
async fn engine_fanout_isolates_a_wedged_resource_from_siblings() {
    let behaviour = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let refresh_entered = Arc::new(AtomicUsize::new(0));
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let mgr = Manager::new();
    let idx = ResourceFanoutIndex::new();
    let cid = CredentialId::new();

    let (a, b, c) = (0xA_u64, 0xB_u64, 0xC_u64);
    behaviour.lock().unwrap().insert(a, Behaviour::Ok);
    behaviour.lock().unwrap().insert(b, Behaviour::Hang);
    behaviour.lock().unwrap().insert(c, Behaviour::Ok);

    for &id in &[a, b, c] {
        mgr.register_resident_with(
            Ctl {
                identity: id,
                behaviour: Arc::clone(&behaviour),
                refresh_entered: Arc::clone(&refresh_entered),
            },
            Cfg,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_identity(id),
        )
        .expect("register resolved-credential row");

        // Warm each tenant's resident runtime so the rotation hook has a
        // live `&Runtime` (resident keeps it in `rt.current()`).
        let ctx = ResourceContext::minimal(
            Scope {
                org_id: Some(org),
                ..Default::default()
            },
            CancellationToken::new(),
        );
        let _g = mgr
            .acquire_resident_for::<Ctl>(&ctx, &AcquireOptions::default(), id)
            .await
            .expect("warm tenant runtime");

        idx.bind(cid, Ctl::key(), scope.clone(), "db", id);
    }

    // Per-resource budget: the wedged row times out fast; the two OK rows
    // complete well within it.
    let out: RotationOutcome = idx
        .dispatch_refresh(cid, &mgr, Duration::from_millis(150))
        .await;

    assert_eq!(
        out,
        RotationOutcome {
            success: 2,
            failed: 0,
            timed_out: 1,
        },
        "the wedged resource must time out in isolation; both siblings still refresh"
    );
    assert_eq!(
        out.dispatched(),
        3,
        "every bound resolved row is accounted for"
    );
    assert_eq!(
        refresh_entered.load(Ordering::SeqCst),
        3,
        "every resource's hook ran — the per-resource timeout did not cancel siblings"
    );
}

/// No bound row → a no-op fan-out (not an error), through the public API.
#[tokio::test]
async fn engine_fanout_unbound_credential_is_noop() {
    let idx = ResourceFanoutIndex::new();
    let mgr = Manager::new();
    let out = idx
        .dispatch_revoke(CredentialId::new(), &mgr, Duration::from_secs(1))
        .await;
    assert_eq!(out, RotationOutcome::default());
}
