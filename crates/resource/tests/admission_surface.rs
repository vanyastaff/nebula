//! Admission-surface tests: `AdmissionPhase`, `Load`, `try_reserve_gate`,
//! and the `acquire_any` backpressure path.
//!
//! Verifies that:
//! - A pool at max capacity reports `AdmissionPhase::Saturated` and
//!   `Load { saturation: 1.0 }`.
//! - `try_reserve_gate` returns `Err(Unavailable::Saturated)` when saturated.
//! - `acquire_any` maps the saturated gate to `ErrorKind::Backpressure`.
//! - A resident resource always reports `AdmissionPhase::Ready` and `None` load.
//! - `Unavailable::into_error` maps each variant to the expected `ErrorKind`.

use std::sync::Arc;

use nebula_core::{ExecutionId, ResourceKey, resource_key, scope::Scope};
use nebula_resource::topology::{pooled::PoolProvider, resident::ResidentProvider};
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResourceContext, ScopeLevel, SlotIdentity,
    error::{Error, ErrorKind},
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata},
    topology::{AdmissionPhase, Load, Pooled, Resident, Unavailable},
};
use tokio_util::sync::CancellationToken;

// ─── Shared helpers ──────────────────────────────────────────────────────────

fn ctx() -> ResourceContext {
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

// ─── Minimal pool resource ───────────────────────────────────────────────────

#[derive(Clone, Default)]
struct MinCfg;

nebula_schema::impl_empty_has_schema!(MinCfg);

impl ResourceConfig for MinCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct TinyPool;

impl HasCredentialSlots for TinyPool {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Provider for TinyPool {
    type Config = MinCfg;
    type Instance = ();
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.admission.tiny_pool")
    }

    async fn create(&self, _config: &MinCfg, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl PoolProvider for TinyPool {}

// ─── Minimal resident resource ───────────────────────────────────────────────

#[derive(Clone)]
struct SimpleResident;

impl HasCredentialSlots for SimpleResident {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Provider for SimpleResident {
    type Config = MinCfg;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.admission.simple_resident")
    }

    async fn create(&self, _config: &MinCfg, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl ResidentProvider for SimpleResident {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// A pool of size 1, fully occupied: `phase` == Saturated, `load` == 1.0,
/// `try_reserve_gate` returns `Err(Saturated)`.
#[tokio::test]
async fn pool_saturated_phase_and_load() {
    use nebula_resource::topology::pooled::config::Config as PoolConfig;

    let manager = Manager::new();
    let pool_rt = Pooled::<TinyPool>::new(
        PoolConfig {
            max_size: 1,
            ..Default::default()
        },
        0,
    );
    manager
        .register(RegistrationSpec {
            resource: TinyPool,
            config: MinCfg,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register succeeds");

    // Acquire the single permit — pool is now saturated.
    let _guard = manager
        .acquire_pooled::<TinyPool>(&ctx(), &AcquireOptions::default())
        .await
        .expect("first acquire succeeds");

    // Probe admission surface through the type-erased handle.
    let handle = manager
        .get_any(&TinyPool::key(), &ScopeLevel::Global)
        .expect("registered row exists");

    assert_eq!(
        handle.admission_phase(),
        AdmissionPhase::Saturated,
        "pool of 1 fully occupied must report Saturated"
    );

    let load = handle.admission_load().expect("pool must report load");
    assert!(
        (load.saturation - 1.0_f32).abs() < f32::EPSILON,
        "saturation must be 1.0 when fully occupied, got {}",
        load.saturation
    );

    let gate_err = handle
        .try_reserve_gate()
        .expect_err("saturated pool must deny try_reserve_gate");
    assert!(
        matches!(gate_err, Unavailable::Saturated { .. }),
        "gate must return Saturated, got {gate_err:?}"
    );
}

/// `Manager::admission_status` reports a `Saturated` snapshot (`load == 1.0`)
/// for a fully occupied pool and `None` for an unregistered key.
#[tokio::test]
async fn admission_status_reports_saturated_snapshot_and_none_for_unknown() {
    use nebula_resource::topology::pooled::config::Config as PoolConfig;

    let manager = Manager::new();
    let pool_rt = Pooled::<TinyPool>::new(
        PoolConfig {
            max_size: 1,
            ..Default::default()
        },
        0,
    );
    manager
        .register(RegistrationSpec {
            resource: TinyPool,
            config: MinCfg,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register succeeds");

    // Acquire the single permit — pool is now saturated.
    let _guard = manager
        .acquire_pooled::<TinyPool>(&ctx(), &AcquireOptions::default())
        .await
        .expect("first acquire succeeds");

    let status = manager
        .admission_status(&TinyPool::key(), &ScopeLevel::Global)
        .expect("registered row yields an admission snapshot");
    assert_eq!(
        status.phase,
        AdmissionPhase::Saturated,
        "saturated pool must report Saturated phase, got {:?}",
        status.phase
    );
    let load = status.load.expect("pool must report load");
    assert!(
        (load.saturation - 1.0_f32).abs() < f32::EPSILON,
        "saturation must be 1.0 when fully occupied, got {}",
        load.saturation
    );

    let unknown = resource_key!("test.admission.unregistered");
    assert!(
        manager
            .admission_status(&unknown, &ScopeLevel::Global)
            .is_none(),
        "unregistered key must yield None"
    );
}

/// `acquire_any` on a saturated pool maps the gate denial to
/// `ErrorKind::Backpressure`.
#[tokio::test]
async fn acquire_any_saturated_returns_backpressure() {
    use nebula_resource::topology::pooled::config::Config as PoolConfig;

    let manager = Arc::new(Manager::new());
    let pool_rt = Pooled::<TinyPool>::new(
        PoolConfig {
            max_size: 1,
            ..Default::default()
        },
        0,
    );
    manager
        .register(RegistrationSpec {
            resource: TinyPool,
            config: MinCfg,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register succeeds");

    // Hold the single permit.
    let _guard = manager
        .acquire_pooled::<TinyPool>(&ctx(), &AcquireOptions::default())
        .await
        .expect("first acquire succeeds");

    // A second acquire through `acquire_any` must hit the gate.
    let err = Manager::acquire_any(
        Arc::clone(&manager),
        &TinyPool::key(),
        &ctx(),
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect_err("saturated pool must reject via acquire_any");

    assert_eq!(
        *err.kind(),
        ErrorKind::Backpressure,
        "saturated admission gate must produce Backpressure, got {:?}",
        err.kind()
    );
    assert!(err.is_retryable(), "Backpressure must be retryable");
}

/// A resident resource always reports Ready phase and None load.
#[tokio::test]
async fn resident_always_ready_no_load() {
    let manager = Manager::new();
    let rt = Resident::<SimpleResident>::new(Default::default());
    manager
        .register(RegistrationSpec {
            resource: SimpleResident,
            config: MinCfg,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: rt,
            recovery_gate: None,
        })
        .expect("register succeeds");

    let handle = manager
        .get_any(&SimpleResident::key(), &ScopeLevel::Global)
        .expect("registered row exists");

    assert_eq!(
        handle.admission_phase(),
        AdmissionPhase::Ready,
        "resident always reports Ready"
    );
    assert!(
        handle.admission_load().is_none(),
        "resident always reports None load"
    );
    assert!(
        handle.try_reserve_gate().is_ok(),
        "resident try_reserve_gate always succeeds"
    );
}

/// `Unavailable::into_error` maps each variant to the expected `ErrorKind`.
#[test]
fn unavailable_into_error_kind_mapping() {
    let saturated = Unavailable::Saturated { retry_after: None }.into_error("res");
    assert_eq!(*saturated.kind(), ErrorKind::Backpressure);
    assert!(saturated.is_retryable());

    let warming = Unavailable::Warming.into_error("res");
    assert_eq!(*warming.kind(), ErrorKind::Transient);
    assert!(warming.is_retryable());

    let recovering = Unavailable::Recovering.into_error("res");
    assert_eq!(*recovering.kind(), ErrorKind::Transient);
    assert!(recovering.is_retryable());

    let tainted = Unavailable::Tainted.into_error("res");
    assert_eq!(*tainted.kind(), ErrorKind::Revoked);
    assert!(tainted.is_retryable());
}

/// `Unavailable::Saturated` with an explicit `retry_after` maps to
/// `ErrorKind::Exhausted` so the hint is preserved.
#[test]
fn unavailable_saturated_with_retry_after_maps_to_exhausted() {
    use std::time::Duration;

    let hint = Duration::from_millis(200);
    let err = Unavailable::Saturated {
        retry_after: Some(hint),
    }
    .into_error("res");
    assert_eq!(
        *err.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(hint)
        },
        "explicit retry_after must produce Exhausted, not Backpressure"
    );
    assert_eq!(err.retry_after(), Some(hint));
}

/// `Load::permits` saturation arithmetic boundary: 0/N == 0.0, N/N == 1.0.
#[test]
fn load_permits_saturation_boundaries() {
    let empty = Load::permits(0, 4);
    assert!(
        empty.saturation.abs() < f32::EPSILON,
        "0 used of 4 must be 0.0"
    );

    let full = Load::permits(4, 4);
    assert!(
        (full.saturation - 1.0_f32).abs() < f32::EPSILON,
        "4 used of 4 must be 1.0"
    );

    let zero_cap = Load::permits(0, 0);
    assert!(
        zero_cap.saturation.abs() < f32::EPSILON,
        "zero capacity must produce 0.0 (no division by zero)"
    );
}

/// `AdmissionPhase` variants are `Copy + PartialEq` and cover all expected
/// arms.
#[test]
fn admission_phase_variants_equality() {
    assert_eq!(AdmissionPhase::Ready, AdmissionPhase::Ready);
    assert_eq!(AdmissionPhase::Saturated, AdmissionPhase::Saturated);
    assert_eq!(AdmissionPhase::Warming, AdmissionPhase::Warming);
    assert_eq!(AdmissionPhase::Recovering, AdmissionPhase::Recovering);
    assert_eq!(AdmissionPhase::Tainted, AdmissionPhase::Tainted);
    assert_ne!(AdmissionPhase::Ready, AdmissionPhase::Saturated);
    assert_ne!(AdmissionPhase::Warming, AdmissionPhase::Recovering);
}
