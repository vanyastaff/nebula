use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use chrono::Utc;
use nebula_storage_port::{
    CredentialAdmissionEpoch, CredentialCommit, CredentialCreate, CredentialMaterialEpoch,
    CredentialOwner, CredentialReplacement, CredentialSelector, CredentialTombstone,
    CredentialVersion, RefreshRetrySnapshot, SecretBytes, StoredCredentialHead,
    StoredCredentialOperationalHead, StoredLiveCredential,
    store::{CredentialIncidentRef, CredentialOperationStatus},
};

use super::*;
use crate::{
    BearerTokenCredential, Credential, CredentialPersistence, CredentialSlotResolver,
    StoredCredential,
};

const SECRET_CANARY: &str = "availability-secret-NEVER-READ-771";

/// What the head read answers.
#[derive(Debug, Clone, Copy)]
enum HeadAnswer {
    Status(CredentialOperationStatus),
    /// `Open`, with the head's reauth flag set.
    ReauthFlag,
    Missing,
    Unavailable,
}

/// A store that counts every read and answers the head read from a script.
#[derive(Debug)]
struct CountingStore {
    owner: CredentialOwner,
    row: StoredLiveCredential,
    answer: Mutex<HeadAnswer>,
    head_reads: AtomicUsize,
    material_reads: AtomicUsize,
    other_reads: AtomicUsize,
}

impl CountingStore {
    fn answer(&self, answer: HeadAnswer) {
        *self.answer.lock().expect("script lock") = answer;
    }

    fn assert_one_head_read_only(&self) {
        assert_eq!(self.head_reads.load(Ordering::SeqCst), 1, "one head read");
        assert_eq!(
            self.material_reads.load(Ordering::SeqCst),
            0,
            "an observation never loads material"
        );
        assert_eq!(self.other_reads.load(Ordering::SeqCst), 0);
    }
}

#[async_trait]
impl CredentialPersistence for CountingStore {
    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        self.head_reads.fetch_add(1, Ordering::SeqCst);
        if selector.owner() != &self.owner || selector.credential_id() != self.row.credential_id() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let answer = *self.answer.lock().expect("script lock");
        let head = StoredCredentialHead::from(&self.row);
        let open = |reauth_required| CredentialOperationStatus::Open {
            version: head.version(),
            material_epoch: head.material_epoch(),
            admission_epoch: CredentialAdmissionEpoch::MIN,
            reauth_required,
        };
        match answer {
            HeadAnswer::Status(status) => Ok(StoredCredentialOperationalHead::new(head, status)),
            HeadAnswer::ReauthFlag => {
                let row = StoredLiveCredential::new(
                    self.row.credential_id(),
                    Some("default".to_owned()),
                    self.row.credential_key().to_owned(),
                    SecretBytes::new(SECRET_CANARY.as_bytes().to_vec()),
                    self.row.state_kind().to_owned(),
                    self.row.state_version(),
                    self.row.version(),
                    self.row.material_epoch(),
                    Utc::now(),
                    Utc::now(),
                    None,
                    true,
                    serde_json::Map::new(),
                    None,
                )
                .expect("flagged row is valid");
                let head = StoredCredentialHead::from(&row);
                // The status alone does not carry the flag here: the head does.
                Ok(StoredCredentialOperationalHead::new(head, open(false)))
            },
            HeadAnswer::Missing => Err(CredentialPersistenceError::NotFound),
            HeadAnswer::Unavailable => Err(CredentialPersistenceError::Unavailable),
        }
    }

    async fn get(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        self.material_reads.fetch_add(1, Ordering::SeqCst);
        Ok(StoredCredential::Live(self.row.clone()))
    }

    async fn get_head(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        self.other_reads.fetch_add(1, Ordering::SeqCst);
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn operation_status(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        self.other_reads.fetch_add(1, Ordering::SeqCst);
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list_operational_heads(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialOperationalHead>, CredentialPersistenceError> {
        self.other_reads.fetch_add(1, Ordering::SeqCst);
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn refresh_retry_snapshot(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        self.other_reads.fetch_add(1, Ordering::SeqCst);
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn create(
        &self,
        _selector: &CredentialSelector,
        _create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn replace(
        &self,
        _selector: &CredentialSelector,
        _replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn tombstone(
        &self,
        _selector: &CredentialSelector,
        _tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn tombstone_revoked_material(
        &self,
        _selector: &CredentialSelector,
        _expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list_heads(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn exists(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }
}

fn fixture() -> (CountingStore, TenantScope, CredentialId, CredentialKey) {
    let scope = TenantScope::new("org-observe", "workspace-observe");
    let id = CredentialId::new();
    let now = Utc::now();
    let row = StoredLiveCredential::new(
        id,
        Some("default".to_owned()),
        BearerTokenCredential::KEY.to_owned(),
        SecretBytes::new(SECRET_CANARY.as_bytes().to_vec()),
        "secret_token".to_owned(),
        1,
        CredentialVersion::MIN,
        CredentialMaterialEpoch::MIN,
        now,
        now,
        None,
        false,
        serde_json::Map::new(),
        None,
    )
    .expect("fixture row is valid");
    let store = CountingStore {
        owner: scope.owner().clone(),
        row,
        answer: Mutex::new(HeadAnswer::Status(CredentialOperationStatus::Open {
            version: CredentialVersion::MIN,
            material_epoch: CredentialMaterialEpoch::MIN,
            admission_epoch: CredentialAdmissionEpoch::MIN,
            reauth_required: false,
        })),
        head_reads: AtomicUsize::new(0),
        material_reads: AtomicUsize::new(0),
        other_reads: AtomicUsize::new(0),
    };
    let key = CredentialKey::new(BearerTokenCredential::KEY).expect("fixture key is valid");
    (store, scope, id, key)
}

fn incident() -> CredentialIncidentRef {
    serde_json::from_value(serde_json::json!("00000000-0000-0000-0000-000000000000"))
        .expect("an incident id deserializes from its UUID spelling")
}

async fn observe(
    store: &CountingStore,
    scope: &TenantScope,
    id: CredentialId,
    key: CredentialKey,
) -> Result<CredentialAvailabilityObservation, CredentialObserveError> {
    observe_availability_with(
        store,
        &StateSource::LocalEncrypted,
        scope,
        id,
        key,
        CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn every_operation_status_maps_to_one_availability_from_one_head_read() {
    use CredentialOperationKind::{LegacyUnclassified, Refresh, Revoke};

    let open = |reauth_required| {
        HeadAnswer::Status(CredentialOperationStatus::Open {
            version: CredentialVersion::MIN,
            material_epoch: CredentialMaterialEpoch::MIN,
            admission_epoch: CredentialAdmissionEpoch::MIN,
            reauth_required,
        })
    };
    let cases = [
        (open(false), CredentialAvailability::Available),
        (
            open(true),
            CredentialAvailability::Blocked(CredentialBlock::ReauthRequired),
        ),
        (
            HeadAnswer::ReauthFlag,
            CredentialAvailability::Blocked(CredentialBlock::ReauthRequired),
        ),
        (
            HeadAnswer::Status(CredentialOperationStatus::InFlight { operation: Refresh }),
            CredentialAvailability::RefreshInFlight,
        ),
        (
            HeadAnswer::Status(CredentialOperationStatus::InFlight { operation: Revoke }),
            CredentialAvailability::Blocked(CredentialBlock::OperationInFlight {
                operation: Revoke,
            }),
        ),
        (
            HeadAnswer::Status(CredentialOperationStatus::InFlight {
                operation: LegacyUnclassified,
            }),
            CredentialAvailability::Blocked(CredentialBlock::OperationInFlight {
                operation: LegacyUnclassified,
            }),
        ),
        (
            HeadAnswer::Status(CredentialOperationStatus::ReconciliationRequired {
                operation: Revoke,
                incident: incident(),
            }),
            CredentialAvailability::Blocked(CredentialBlock::ReconciliationRequired {
                operation: Revoke,
            }),
        ),
        (
            HeadAnswer::Status(CredentialOperationStatus::ReconciliationRequired {
                operation: Refresh,
                incident: incident(),
            }),
            CredentialAvailability::Blocked(CredentialBlock::ReconciliationRequired {
                operation: Refresh,
            }),
        ),
    ];
    for (answer, expected) in cases {
        let (store, scope, id, key) = fixture();
        store.answer(answer);
        let observation = observe(&store, &scope, id, key)
            .await
            .expect("a live head is observed");
        assert_eq!(observation.availability(), expected, "{answer:?}");
        assert_eq!(observation.material_epoch(), 1);
        assert_eq!(observation.revision(), 1);
        store.assert_one_head_read_only();
        let rendered = format!("{observation:?}");
        assert!(!rendered.contains(SECRET_CANARY));
    }
}

#[tokio::test]
async fn a_wrong_key_is_refused_after_the_head_read() {
    let (store, scope, id, _) = fixture();
    let other = CredentialKey::new("other_contract").expect("valid key");
    assert_eq!(
        observe(&store, &scope, id, other).await,
        Err(CredentialObserveError::WrongCredentialKey)
    );
    store.assert_one_head_read_only();
}

#[tokio::test]
async fn a_missing_or_foreign_credential_is_absent_without_a_tombstone_read() {
    let (store, scope, id, key) = fixture();
    store.answer(HeadAnswer::Missing);
    assert_eq!(
        observe(&store, &scope, id, key.clone()).await,
        Err(CredentialObserveError::Absent)
    );
    store.assert_one_head_read_only();

    let (store, _, id, key) = fixture();
    let foreign = TenantScope::new("org-other", "workspace-other");
    assert_eq!(
        observe(&store, &foreign, id, key).await,
        Err(CredentialObserveError::Absent)
    );
    store.assert_one_head_read_only();
}

#[tokio::test]
async fn an_unavailable_store_is_unavailable() {
    let (store, scope, id, key) = fixture();
    store.answer(HeadAnswer::Unavailable);
    assert_eq!(
        observe(&store, &scope, id, key).await,
        Err(CredentialObserveError::Unavailable)
    );
}

/// An external provider the observer must never reach.
#[derive(Debug)]
struct UnreachedProvider;

impl crate::provider::ExternalProvider for UnreachedProvider {
    fn resolve<'a>(
        &'a self,
        _reference: &'a crate::provider::ExternalReference,
    ) -> crate::provider::ProviderFuture<'a> {
        crate::provider::ProviderFuture::ready(Ok(
            crate::provider::ProviderResolution::from_secret(crate::SecretString::new(
                SECRET_CANARY,
            )),
        ))
    }

    fn provider_name(&self) -> &'static str {
        "unreached"
    }
}

#[tokio::test]
async fn an_external_source_is_refused_before_any_read() {
    let (store, scope, id, key) = fixture();
    let result = observe_availability_with(
        &store,
        &StateSource::External(std::sync::Arc::new(UnreachedProvider)),
        &scope,
        id,
        key,
        CancellationToken::new(),
    )
    .await;
    assert_eq!(result, Err(CredentialObserveError::SourceUnavailable));
    assert_eq!(store.head_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_cancelled_observation_reports_cancelled() {
    let (store, scope, id, key) = fixture();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = observe_availability_with(
        &store,
        &StateSource::LocalEncrypted,
        &scope,
        id,
        key,
        cancel,
    )
    .await;
    assert_eq!(result, Err(CredentialObserveError::Cancelled));
    assert_eq!(store.material_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_projection_runtime_observes_through_its_resolver_upcast() {
    let (store, scope, id, key) = fixture();
    let store = std::sync::Arc::new(store);
    let mut registry = crate::CredentialRegistry::new();
    registry
        .register(BearerTokenCredential, "observe-test")
        .expect("fixture registration is unique");
    let mut ops = crate::DispatchOps::new();
    crate::register_runtime_ops::<BearerTokenCredential, crate::ErasedPendingStore>(&mut ops)
        .expect("fixture ops registration is unique");
    let runtime = crate::CredentialProjectionRuntime::from_secure_parts(
        std::sync::Arc::clone(&store) as std::sync::Arc<dyn CredentialPersistence>,
        std::sync::Arc::new(registry),
        std::sync::Arc::new(ops),
        StateSource::LocalEncrypted,
    )
    .expect("projection runtime");
    let resolver: &dyn CredentialSlotResolver = &runtime;
    let observer = resolver
        .as_availability_observer()
        .expect("the projection runtime is an observer");
    let observation = observer
        .observe_availability(&scope, id, key, CancellationToken::new())
        .await
        .expect("observed");
    assert_eq!(
        observation.availability(),
        CredentialAvailability::Available
    );
    store.assert_one_head_read_only();
}
