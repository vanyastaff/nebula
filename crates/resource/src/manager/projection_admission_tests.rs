//! Queue rejection must not consume the installed epoch's hook admission.
use super::{EpochRefreshOutcome, Manager, ManagerConfig, RegistrationSpec};
use crate::resource::{HasCredentialSlots, ResourceMetadataDraft};
use crate::topology::resident::ResidentProvider;
use crate::{
    AcquireOptions, Error, Provider, Resident, ResidentConfig, ResourceConfig, ResourceContext,
    SlotCell, SlotIdentity, SlotInstallError, SlotUpdate,
};
use nebula_core::{CredentialId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialGuard, CredentialGuardMetadata, ErasedCredentialGuard};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, nebula_schema::Schema)]
struct Config;
impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct ProjectionResource {
    slot: Arc<SlotCell<CredentialGuard<u64>>>,
    calls: Arc<AtomicUsize>,
    fail: bool,
    snapshot_gate: Option<Arc<SnapshotGate>>,
}

struct SnapshotGate {
    entered: tokio::sync::Semaphore,
    revoke_entered: tokio::sync::Semaphore,
    released: std::sync::Mutex<bool>,
    resume: std::sync::Condvar,
}
#[async_trait::async_trait]
impl Provider for ProjectionResource {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;
    fn key() -> ResourceKey {
        resource_key!("projection-admission")
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("projection-admission"),
            "",
        )
    }
    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
    async fn on_credential_refresh(&self, _: &str, (): &()) -> Result<(), Error> {
        if self.snapshot_gate.is_none() {
            assert_eq!(**self.slot.load().expect("installed guard"), 99);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(Error::permanent("injected hook failure"))
        } else {
            Ok(())
        }
    }
}
impl HasCredentialSlots for ProjectionResource {
    fn declares_credential_slots() -> bool {
        true
    }
    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }
    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<CredentialGuardMetadata>)> {
        if slot != "db" {
            return None;
        }
        let snapshot = self.slot.projection_snapshot();
        if let Some(gate) = &self.snapshot_gate {
            gate.entered.add_permits(1);
            let released = gate.released.lock().expect("gate lock");
            let (released, _) = gate
                .resume
                .wait_timeout_while(released, std::time::Duration::from_secs(5), |released| {
                    !*released
                })
                .expect("gate wait");
            assert!(*released, "test must release the snapshot gate");
        }
        Some(snapshot)
    }

    fn install_credential_slot_at_generation(
        &self,
        slot: &str,
        guard: ErasedCredentialGuard,
        expected_generation: u64,
    ) -> Result<SlotUpdate, SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<u64>()
            .map_err(|_| SlotInstallError::CredentialTypeMismatch)?;
        self.slot
            .install_projected_at_generation(expected_generation, metadata, Arc::new(guard))
    }

    fn fence_credential_slot_at_generation(
        &self,
        slot: &str,
        expected_generation: u64,
        fence: &mut dyn FnMut(),
    ) -> Result<(), SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        self.slot
            .fence_projection_at_generation(expected_generation, fence)
    }

    fn revoke_credential_slot(&self, slot: &str) -> Result<SlotUpdate, SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        if let Some(gate) = &self.snapshot_gate {
            gate.revoke_entered.add_permits(1);
        }
        Ok(self.slot.revoke())
    }

    fn credential_slot_epoch(&self) -> u64 {
        self.slot.generation()
    }
    fn install_credential_slot(
        &self,
        slot: &str,
        guard: ErasedCredentialGuard,
    ) -> Result<SlotUpdate, SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<u64>()
            .map_err(|_| SlotInstallError::CredentialTypeMismatch)?;
        self.slot.install_projected(metadata, Arc::new(guard))
    }
}
#[async_trait::async_trait]
impl ResidentProvider for ProjectionResource {
    fn is_alive_sync(&self, (): &()) -> bool {
        true
    }
}

#[tokio::test]
async fn rejected_projection_hook_retries_once_but_accepted_failure_does_not() {
    for (fail, unqualified) in [
        (false, None),
        (true, None),
        (false, Some(true)),
        (false, Some(false)),
    ] {
        let manager = Manager::with_config(ManagerConfig::default().with_release_queue_workers(1));
        let resource = ProjectionResource {
            slot: Arc::new(SlotCell::empty()),
            calls: Arc::new(AtomicUsize::new(0)),
            fail,
            snapshot_gate: None,
        };
        let identity = SlotIdentity::from_bindings([("db", "oauth")]);
        manager
            .register(RegistrationSpec {
                resource: resource.clone(),
                config: Config,
                scope: ScopeLevel::Global,
                slot_identity: identity.clone(),
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
            })
            .expect("register");
        let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let lease = manager
            .acquire_resident_for_identity::<ProjectionResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("warm resident");
        drop(lease);
        let blocker = CancellationToken::new();
        let mut saturated = false;
        for _ in 0..20_000 {
            let blocker = blocker.clone();
            if let Ok(receipt) = manager.release_queue.submit_coordinator(move || {
                Box::pin(async move {
                    blocker.cancelled().await;
                    Ok(())
                })
            }) {
                receipt.detach();
            } else {
                saturated = true;
                break;
            }
        }
        assert!(saturated, "exercise actual bounded queue rejection");
        let cid = CredentialId::new();
        let guard = || {
            ErasedCredentialGuard::from_typed(
                CredentialGuard::new(99_u64),
                CredentialGuardMetadata::new(cid, "oauth".parse().expect("key"), 2, 2),
            )
        };
        let key = ProjectionResource::key();
        assert!(
            manager
                .install_and_refresh_slot_for_identity(
                    &key,
                    ScopeLevel::Global,
                    "db",
                    &identity,
                    guard()
                )
                .await
                .is_err()
        );
        assert_eq!(
            resource
                .slot
                .projection_metadata()
                .expect("published despite rejection")
                .material_epoch(),
            2
        );
        assert_eq!(resource.calls.load(Ordering::SeqCst), 0);
        if let Some(use_store) = unqualified {
            if use_store {
                resource.slot.store(Arc::new(CredentialGuard::new(123_u64)));
            } else {
                assert!(resource.slot.take().is_some());
            }
        }
        blocker.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(receipt) = manager
                    .release_queue
                    .submit_coordinator(|| Box::pin(async { Ok(()) }))
                {
                    let _ = receipt.wait().await;
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queue recovers");
        let (first, duplicate) = tokio::join!(
            manager.install_and_refresh_slot_for_identity(
                &key,
                ScopeLevel::Global,
                "db",
                &identity,
                guard()
            ),
            manager.install_and_refresh_slot_for_identity(
                &key,
                ScopeLevel::Global,
                "db",
                &identity,
                guard()
            )
        );
        if let Some(use_store) = unqualified {
            assert!(matches!(first, Ok(EpochRefreshOutcome::Stale { .. })));
            assert!(matches!(duplicate, Ok(EpochRefreshOutcome::Stale { .. })));
            assert_eq!(resource.calls.load(Ordering::SeqCst), 0);
            assert_eq!(
                resource.slot.load().map(|guard| **guard),
                use_store.then_some(123)
            );
            assert!(resource.slot.projection_metadata().is_none());
            continue;
        }
        assert_eq!(first.is_err(), fail);
        if !fail {
            assert!(matches!(first, Ok(EpochRefreshOutcome::Applied(_))));
        }
        assert!(matches!(duplicate, Ok(EpochRefreshOutcome::Stale { .. })));
        assert_eq!(resource.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            manager
                .install_and_refresh_slot_for_identity(
                    &key,
                    ScopeLevel::Global,
                    "db",
                    &identity,
                    guard()
                )
                .await,
            Ok(EpochRefreshOutcome::Stale { .. })
        ));
        assert_eq!(resource.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unqualified_slot_write_fences_refresh_hook_admission() {
    let manager = Arc::new(Manager::new());
    let gate = Arc::new(SnapshotGate {
        entered: tokio::sync::Semaphore::new(0),
        revoke_entered: tokio::sync::Semaphore::new(0),
        released: std::sync::Mutex::new(false),
        resume: std::sync::Condvar::new(),
    });
    let resource = ProjectionResource {
        slot: Arc::new(SlotCell::empty()),
        calls: Arc::new(AtomicUsize::new(0)),
        fail: false,
        snapshot_gate: Some(Arc::clone(&gate)),
    };
    let identity = SlotIdentity::from_bindings([("db", "oauth")]);
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<ProjectionResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("warm"),
    );
    let projection = tokio::spawn({
        let manager = Arc::clone(&manager);
        async move {
            manager
                .install_and_refresh_slot_for_identity(
                    &ProjectionResource::key(),
                    ScopeLevel::Global,
                    "db",
                    &identity,
                    ErasedCredentialGuard::from_typed(
                        CredentialGuard::new(99_u64),
                        CredentialGuardMetadata::new(
                            CredentialId::new(),
                            "oauth".parse().expect("key"),
                            1,
                            1,
                        ),
                    ),
                )
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.entered.acquire())
        .await
        .expect("snapshot reached")
        .expect("permit")
        .forget();

    resource.slot.store(Arc::new(CredentialGuard::new(123_u64)));
    *gate.released.lock().expect("gate lock") = true;
    gate.resume.notify_all();

    let error = projection
        .await
        .expect("projection task")
        .expect_err("superseded projection must not submit its hook");
    assert_eq!(error.kind(), &crate::ErrorKind::Permanent);
    let managed = manager
        .lookup_any_for_slot_identity_structural(
            &ProjectionResource::key(),
            &ScopeLevel::Global,
            &SlotIdentity::from_bindings([("db", "oauth")]),
        )
        .expect("managed row");
    assert!(
        managed
            .pending_projection_hooks()
            .lock()
            .expect("pending hooks")
            .is_empty(),
        "a superseding raw write must discard the obsolete pending hook"
    );
    assert_eq!(resource.calls.load(Ordering::SeqCst), 0);
    assert_eq!(resource.slot.load().map(|guard| **guard), Some(123));
    assert!(resource.slot.projection_metadata().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_revoke_cannot_split_projection_install_and_hook_admission() {
    let manager = Arc::new(Manager::new());
    let gate = Arc::new(SnapshotGate {
        entered: tokio::sync::Semaphore::new(0),
        revoke_entered: tokio::sync::Semaphore::new(0),
        released: std::sync::Mutex::new(false),
        resume: std::sync::Condvar::new(),
    });
    let resource = ProjectionResource {
        slot: Arc::new(SlotCell::empty()),
        calls: Arc::new(AtomicUsize::new(0)),
        fail: false,
        snapshot_gate: Some(Arc::clone(&gate)),
    };
    let identity = SlotIdentity::from_bindings([("db", "oauth")]);
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register");
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<ProjectionResource>(
                &context,
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("warm"),
    );
    let cid = CredentialId::new();
    let projection = tokio::spawn({
        let manager = Arc::clone(&manager);
        let identity = identity.clone();
        async move {
            manager
                .install_and_refresh_slot_for_identity(
                    &ProjectionResource::key(),
                    ScopeLevel::Global,
                    "db",
                    &identity,
                    ErasedCredentialGuard::from_typed(
                        CredentialGuard::new(99_u64),
                        CredentialGuardMetadata::new(cid, "oauth".parse().expect("key"), 1, 1),
                    ),
                )
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.entered.acquire())
        .await
        .expect("snapshot reached")
        .expect("permit")
        .forget();
    let (started, ready) = tokio::sync::oneshot::channel();
    let revoke = tokio::spawn({
        let manager = Arc::clone(&manager);
        let identity = identity.clone();
        async move {
            started.send(()).expect("revoke observer");
            manager
                .revoke_credential_slot_for_identity(
                    &ProjectionResource::key(),
                    ScopeLevel::Global,
                    "db",
                    &identity,
                )
                .await
        }
    });
    ready.await.expect("revoke started");
    let crossed = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        gate.revoke_entered.acquire(),
    )
    .await
    .is_ok();
    *gate.released.lock().expect("gate lock") = true;
    gate.resume.notify_all();
    let installed = projection.await.expect("projection task");
    let revoked = revoke.await.expect("revoke task");
    assert!(
        !crossed,
        "terminal revoke must not enter between projection snapshot and queue admission"
    );
    assert!(matches!(installed, Ok(EpochRefreshOutcome::Applied(_))));
    assert!(matches!(revoked, Ok(super::EpochRevokeOutcome::Applied(_))));
    assert!(resource.slot.load().is_none());
    let calls = resource.calls.load(Ordering::SeqCst);
    let late = manager
        .install_and_refresh_slot_for_identity(
            &ProjectionResource::key(),
            ScopeLevel::Global,
            "db",
            &identity,
            ErasedCredentialGuard::from_typed(
                CredentialGuard::new(99_u64),
                CredentialGuardMetadata::new(cid, "oauth".parse().expect("key"), 2, 2),
            ),
        )
        .await;
    assert_eq!(
        late.expect_err("revoked admission").kind(),
        &crate::ErrorKind::Revoked
    );
    assert!(
        manager
            .refresh_slot_for_identity(
                &ProjectionResource::key(),
                ScopeLevel::Global,
                "db",
                &identity
            )
            .await
            .is_err()
    );
    assert_eq!(resource.calls.load(Ordering::SeqCst), calls);
}
