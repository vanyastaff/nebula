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
        assert_eq!(**self.slot.load().expect("installed guard"), 99);
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
    for fail in [false, true] {
        let manager = Manager::with_config(ManagerConfig::default().with_release_queue_workers(1));
        let resource = ProjectionResource {
            slot: Arc::new(SlotCell::empty()),
            calls: Arc::new(AtomicUsize::new(0)),
            fail,
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
