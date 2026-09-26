#![cfg(feature = "sqlite")]

//! Cross-crate acceptance for a provider revoke whose local tombstone fails.

use std::{
    future::Future,
    pin::Pin,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::auth::{
    AuthPattern, AuthScheme, EgressShape, RefreshStrategyKind, SchemeFamily, SensitiveScheme,
};
use nebula_core::{BaseContext, Context, CredentialId, Principal, UserId};
use nebula_credential::error::CredentialError;
use nebula_credential::resolve::{StaticResolveResult, TestResult};
use nebula_credential::runtime::{
    AcquisitionTransport, AcquisitionTransportError, CredentialResolver, LeaseLifecycle,
    LeaseLifecycleConfig, RefreshCoordConfig, RefreshCoordinator, RefreshTransport,
    RefreshTransportError, TokenPostRequest, TokenPostResponse,
};
use nebula_credential::{
    AuthorizationDecision, CredentialActor, CredentialAuthorizationError, CredentialAvailability,
    CredentialBlock, CredentialCommand, CredentialCommandResult, CredentialContext,
    CredentialController, CredentialControllerError, CredentialDisplay, CredentialDisplayPatch,
    CredentialMetadataDraft, CredentialOperation, CredentialProjectionRuntime, CredentialRegistry,
    CredentialService, CredentialServiceError, CredentialSlotResolveError, CredentialSlotResolver,
    CredentialTenantAuthority, DispatchOps, ErasedPendingStore, NoopObserver, RefreshAttempt,
    RefreshReport, StateSource, StateWireFingerprint, TenantScope, identity_state,
    register_refreshable_ops, register_revocable_ops, register_runtime_ops, register_testable_ops,
};
use nebula_storage::credential::{
    CacheConfig, CacheLayer, EncryptionLayer, EnvKeyProvider, InMemoryPendingStore,
    SqliteCredentialPersistence,
};
use nebula_storage_port::store::{
    ClaimAttempt, ClaimToken, CredentialIncidentRef, CredentialOperationDecision,
    CredentialOperationIntent, CredentialOperationKind, CredentialOperationStatus, HeartbeatError,
    RefreshAdjudication, RefreshClaimAdjudicationError, RefreshClaimAdjudicator, RefreshClaimError,
    RefreshClaimReclaimer, RefreshClaimStore, RefreshOutcomeDecision, ReplicaId,
    RevokeOutcomeDecision, SentinelEscalationPolicy,
};
use nebula_storage_port::{
    CredentialCommit, CredentialCreate, CredentialMaterialEpoch, CredentialOwner,
    CredentialPersistence, CredentialPersistenceError, CredentialReplacement, CredentialSelector,
    CredentialTombstone, RefreshRetrySnapshot, Scope, StoredCredential, StoredCredentialHead,
    StoredCredentialOperationalHead,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use zeroize::{Zeroize, ZeroizeOnDrop};

const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
const EVIDENCE: &str = "provider console confirms the credential was revoked";

static REVOKE_CALLS: AtomicUsize = AtomicUsize::new(0);
static REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);
static TEST_CALLS: AtomicUsize = AtomicUsize::new(0);
static TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PAUSE_REFRESH_PROVIDER: AtomicBool = AtomicBool::new(false);
static REFRESH_PROVIDER_ENTERED: tokio::sync::Notify = tokio::sync::Notify::const_new();
static REFRESH_PROVIDER_CONTINUE: tokio::sync::Notify = tokio::sync::Notify::const_new();
static OBSERVE_REVOKE_CLAIM: AtomicBool = AtomicBool::new(false);
static REVOKE_CLAIM_ATTEMPTED: tokio::sync::Notify = tokio::sync::Notify::const_new();

struct ProbeFamily;

impl SchemeFamily for ProbeFamily {
    const EGRESS: &'static [EgressShape] = &[EgressShape::InlineSecret];

    fn refresh_classes() -> &'static [RefreshStrategyKind] {
        &[RefreshStrategyKind::RefreshToken]
    }

    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

#[derive(Clone, Deserialize, Serialize, StateWireFingerprint, Zeroize, ZeroizeOnDrop)]
struct ProbeScheme {
    token: String,
    generation: u32,
}

impl AuthScheme for ProbeScheme {
    type Family = ProbeFamily;

    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

impl SensitiveScheme for ProbeScheme {}

identity_state!(ProbeScheme, "operation_incident_probe_state", 1);

struct ProbeCredential;

#[nebula_credential::credential(key = "operation_incident_probe")]
impl ProbeCredential {
    type Properties = serde_json::Value;
    type Scheme = ProbeScheme;
    type State = ProbeScheme;

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("operation_incident_probe"),
            nebula_credential::metadata_name!("Operation Incident Probe"),
            "credential used to prove typed revoke-incident runtime behavior",
        )
    }

    fn project(state: &ProbeScheme) -> ProbeScheme {
        state.clone()
    }

    async fn resolve(
        properties: &serde_json::Value,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<ProbeScheme>, CredentialError> {
        Ok(StaticResolveResult::Complete(ProbeScheme {
            token: properties
                .get("token")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            generation: 1,
        }))
    }

    async fn refresh(state: &mut ProbeScheme, attempt: RefreshAttempt<'_>) -> RefreshReport {
        let completed = match attempt
            .dispatch(|| async {
                REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
                if PAUSE_REFRESH_PROVIDER.swap(false, Ordering::SeqCst) {
                    REFRESH_PROVIDER_ENTERED.notify_one();
                    REFRESH_PROVIDER_CONTINUE.notified().await;
                }
                Ok::<(), std::convert::Infallible>(())
            })
            .await
        {
            Ok(completed) => completed,
            Err(unknown) => return unknown.into_report(),
        };
        let ((), proof) = completed.into_parts();
        state.generation += 1;
        proof.refreshed()
    }

    async fn revoke(
        state: &mut ProbeScheme,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        REVOKE_CALLS.fetch_add(1, Ordering::SeqCst);
        state.token.clear();
        Ok(())
    }

    async fn test(
        _scheme: &ProbeScheme,
        _ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        TEST_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(TestResult::Success)
    }
}

#[derive(Debug)]
struct AllowAuthority;

#[async_trait]
impl CredentialTenantAuthority for AllowAuthority {
    async fn decide(
        &self,
        _actor: &CredentialActor,
        _scope: &Scope,
        _operation: CredentialOperation,
    ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
        Ok(AuthorizationDecision::Allow)
    }
}

#[derive(Debug)]
struct NoNetworkTransport;

impl RefreshTransport for NoNetworkTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>>
    {
        Box::pin(async { Err(RefreshTransportError::Send) })
    }
}

impl AcquisitionTransport for NoNetworkTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, AcquisitionTransportError>> + Send + 'a>,
    > {
        Box::pin(async { Err(AcquisitionTransportError::Send) })
    }
}

#[derive(Debug)]
struct ObservingClaims<C> {
    inner: Arc<C>,
}

impl<C> ObservingClaims<C> {
    fn new(inner: Arc<C>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<C> RefreshClaimStore for ObservingClaims<C>
where
    C: RefreshClaimStore,
{
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        holder: &ReplicaId,
        ttl: Duration,
        intent: CredentialOperationIntent,
    ) -> Result<ClaimAttempt, RefreshClaimError> {
        if matches!(intent, CredentialOperationIntent::Revoke { .. })
            && OBSERVE_REVOKE_CLAIM.swap(false, Ordering::SeqCst)
        {
            REVOKE_CLAIM_ATTEMPTED.notify_one();
        }
        self.inner.try_claim(selector, holder, ttl, intent).await
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        self.inner.heartbeat(token, ttl).await
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RefreshClaimError> {
        self.inner.release(token).await
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RefreshClaimError> {
        self.inner.mark_sentinel(token).await
    }
}

#[async_trait]
impl<C> RefreshClaimAdjudicator for ObservingClaims<C>
where
    C: RefreshClaimAdjudicator,
{
    async fn adjudicate(
        &self,
        selector: &CredentialSelector,
        incident: CredentialIncidentRef,
        decision: CredentialOperationDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RefreshClaimAdjudicationError> {
        self.inner
            .adjudicate(selector, incident, decision, evidence)
            .await
    }
}

/// The incident `selector`'s operation status publishes for reconciliation.
///
/// Reconciliation names the incident it resolves, so every `Reconcile` command
/// here reads it from the status an operator would observe.
async fn reconciliation_incident(
    store: &(impl CredentialPersistence + ?Sized),
    selector: &CredentialSelector,
) -> CredentialIncidentRef {
    match store
        .operation_status(selector)
        .await
        .expect("operation status")
    {
        CredentialOperationStatus::ReconciliationRequired { incident, .. } => incident,
        other => panic!("a poisoned credential publishes its incident, got {other:?}"),
    }
}

/// Inject one definite local tombstone failure after provider completion.
#[derive(Debug)]
struct FailFirstTombstone {
    inner: Arc<dyn CredentialPersistence>,
    armed: AtomicBool,
    pause_revoked_finalizer: AtomicBool,
    finalizer_entered: tokio::sync::Notify,
    finalizer_continue: tokio::sync::Notify,
}

impl FailFirstTombstone {
    fn new(inner: Arc<dyn CredentialPersistence>) -> Self {
        Self {
            inner,
            armed: AtomicBool::new(true),
            pause_revoked_finalizer: AtomicBool::new(false),
            finalizer_entered: tokio::sync::Notify::new(),
            finalizer_continue: tokio::sync::Notify::new(),
        }
    }

    fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }

    fn pause_revoked_finalizer(&self) {
        self.pause_revoked_finalizer.store(true, Ordering::SeqCst);
    }

    async fn wait_for_revoked_finalizer(&self) {
        self.finalizer_entered.notified().await;
    }

    fn continue_revoked_finalizer(&self) {
        self.finalizer_continue.notify_one();
    }

    #[cfg(feature = "postgres")]
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl CredentialPersistence for FailFirstTombstone {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        self.inner.get(selector).await
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        self.inner.get_head(selector).await
    }

    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        self.inner.operation_status(selector).await
    }

    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        self.inner.get_operational_head(selector).await
    }

    async fn list_operational_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialOperationalHead>, CredentialPersistenceError> {
        self.inner.list_operational_heads(owner, state_kind).await
    }

    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        self.inner.refresh_retry_snapshot(selector).await
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner.create(selector, create).await
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner.replace(selector, replacement).await
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner.tombstone(selector, tombstone).await
    }

    async fn tombstone_revoked_material(
        &self,
        selector: &CredentialSelector,
        expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        if self.pause_revoked_finalizer.swap(false, Ordering::SeqCst) {
            self.finalizer_entered.notify_one();
            self.finalizer_continue.notified().await;
        }
        if self.armed.swap(false, Ordering::SeqCst) {
            return Err(CredentialPersistenceError::Unavailable);
        }
        self.inner
            .tombstone_revoked_material(selector, expected_material_epoch)
            .await
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        self.inner.list(owner, state_kind).await
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        self.inner.list_heads(owner, state_kind).await
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        self.inner.exists(selector).await
    }
}

/// Both read-only consumers of the same secure store: the worker projection
/// runtime and the management service behind the controller.
struct Projections {
    runtime: CredentialProjectionRuntime,
    service: Arc<CredentialService>,
}

fn compose_runtime<P, C>(
    raw: P,
    claims: Arc<C>,
) -> (
    CredentialController,
    CredentialResolver<dyn CredentialPersistence>,
    Projections,
    Arc<FailFirstTombstone>,
)
where
    P: CredentialPersistence + 'static,
    C: RefreshClaimStore + RefreshClaimAdjudicator + 'static,
{
    let claims = Arc::new(ObservingClaims::new(claims));
    let coordinator = Arc::new(
        RefreshCoordinator::new_with(
            claims.clone() as Arc<dyn RefreshClaimStore>,
            ReplicaId::new("operation-incident-runtime"),
            RefreshCoordConfig::default(),
        )
        .expect("valid coordinator config"),
    );
    let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("test key"));
    let encrypted: Arc<dyn CredentialPersistence> = Arc::new(EncryptionLayer::new(raw, key));
    let tombstone_fault = Arc::new(FailFirstTombstone::new(encrypted));
    let store: Arc<dyn CredentialPersistence> = tombstone_fault.clone();
    let transport = Arc::new(NoNetworkTransport);
    let resolver =
        CredentialResolver::with_dependencies(Arc::clone(&store), coordinator, transport.clone());
    let mut registry = CredentialRegistry::new();
    registry
        .register(ProbeCredential, "nebula-storage-test")
        .expect("probe registration");
    let mut ops = DispatchOps::<ErasedPendingStore>::new();
    register_runtime_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).expect("runtime ops");
    register_refreshable_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).expect("refresh ops");
    register_revocable_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).expect("revoke ops");
    register_testable_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).expect("test ops");
    let registry = Arc::new(registry);
    let ops = Arc::new(ops);
    let projection = CredentialProjectionRuntime::from_secure_parts(
        Arc::clone(&store),
        Arc::clone(&registry),
        Arc::clone(&ops),
        StateSource::LocalEncrypted,
    )
    .expect("projection runtime");
    let base = BaseContext::builder(nebula_core::Scope::default()).build_with(Principal::System);
    let service = Arc::new(CredentialService::from_secure_parts(
        Arc::clone(&store),
        resolver.clone(),
        LeaseLifecycle::spawn(
            LeaseLifecycleConfig::default(),
            None,
            None,
            base.cancellation().child_token(),
        ),
        ErasedPendingStore::new(Arc::new(InMemoryPendingStore::new())),
        registry,
        ops,
        Arc::new(NoopObserver::new()),
        transport,
        StateSource::LocalEncrypted,
    ));
    let controller = CredentialController::new(
        Arc::clone(&service),
        Arc::new(AllowAuthority),
        claims as Arc<dyn RefreshClaimAdjudicator>,
        None,
    );
    let projections = Projections {
        runtime: projection,
        service,
    };
    (controller, resolver, projections, tombstone_fault)
}

struct Fixture {
    controller: CredentialController,
    resolver: CredentialResolver<dyn CredentialPersistence>,
    projection: CredentialProjectionRuntime,
    service: Arc<CredentialService>,
    tombstone_fault: Arc<FailFirstTombstone>,
    raw: SqliteCredentialPersistence,
    claims: Arc<nebula_storage::credential::SqliteRefreshClaimRepo>,
    sql_pool: sqlx::SqlitePool,
    scope: Scope,
    actor: CredentialActor,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        REVOKE_CALLS.store(0, Ordering::SeqCst);
        REFRESH_CALLS.store(0, Ordering::SeqCst);
        TEST_CALLS.store(0, Ordering::SeqCst);

        let directory = tempfile::tempdir().expect("temp db directory");
        let path = directory.path().join("credential-operation-runtime.sqlite");
        let db = path.to_string_lossy().into_owned();
        let raw = SqliteCredentialPersistence::connect(&db)
            .await
            .expect("sqlite credential store");
        let options = SqliteConnectOptions::from_str(&db)
            .expect("sqlite path")
            .create_if_missing(true);
        let sql_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("inspection pool");
        let claims = Arc::new(raw.refresh_claim_repo());
        let (controller, resolver, projections, tombstone_fault) =
            compose_runtime(raw.clone(), Arc::clone(&claims));

        Self {
            controller,
            resolver,
            projection: projections.runtime,
            service: projections.service,
            tombstone_fault,
            raw,
            claims,
            sql_pool,
            scope: Scope::new("workspace", "organization"),
            actor: CredentialActor::user(UserId::new()),
            _directory: directory,
        }
    }

    async fn command(
        &self,
        command: CredentialCommand,
    ) -> Result<CredentialCommandResult, CredentialControllerError> {
        self.controller
            .execute(&self.actor, &self.scope, command)
            .await
    }

    async fn create(&self) -> CredentialId {
        let result = self
            .command(CredentialCommand::Create {
                credential_key: nebula_core::credential_key!("operation_incident_probe"),
                properties: json!({ "token": "provider-secret" }),
                display: CredentialDisplay::default(),
            })
            .await
            .expect("create succeeds");
        let CredentialCommandResult::Head(head) = result else {
            panic!("create returns head")
        };
        CredentialId::parse(&head.id).expect("credential id")
    }

    /// Observes `id` through the projection runtime and the management
    /// service, asserting both observers agree on one head read's answer.
    async fn observe_both(&self, id: CredentialId) -> CredentialAvailability {
        let scope = TenantScope::from_scope(&self.scope);
        let key = nebula_core::credential_key!("operation_incident_probe");
        let base =
            BaseContext::builder(nebula_core::Scope::default()).build_with(Principal::System);
        let mut observed = Vec::new();
        for resolver in [
            &self.projection as &dyn CredentialSlotResolver,
            self.service.as_ref() as &dyn CredentialSlotResolver,
        ] {
            let observer = resolver
                .as_availability_observer()
                .expect("both read-only consumers observe availability");
            observed.push(
                observer
                    .observe_availability(
                        &scope,
                        id,
                        key.clone(),
                        base.cancellation().child_token(),
                    )
                    .await
                    .expect("a live credential is observed"),
            );
        }
        assert_eq!(observed[0], observed[1], "service parity");
        observed[0].availability()
    }

    fn selector(&self, id: CredentialId) -> CredentialSelector {
        CredentialSelector::new(CredentialOwner::from_scope(&self.scope), id)
    }

    async fn expire_claim(&self, id: CredentialId, threshold: u32) {
        let affected = sqlx::query(
            "UPDATE credential_refresh_claims SET expires_at = 0 WHERE credential_id = ?1",
        )
        .bind(id.to_string())
        .execute(&self.sql_pool)
        .await
        .expect("backdate claim")
        .rows_affected();
        assert_eq!(
            affected, 1,
            "the failed finalization retains its exact claim"
        );
        self.claims
            .reclaim_stuck(
                SentinelEscalationPolicy::new(threshold, Duration::from_hours(1)).expect("policy"),
            )
            .await
            .expect("reclaim accounts the incident");
    }

    async fn seed_resolved_refresh_history(&self, id: CredentialId) {
        let selector = self.selector(id);
        sqlx::query(
            "INSERT INTO credential_sentinel_events \
             (owner_id, credential_id, detected_at, crashed_holder, generation, claim_id, \
              adjudicated_at, adjudication_decision, adjudication_evidence, \
              adjudication_evidence_digest, operation_kind, observed_material_epoch) \
             VALUES (?1, ?2, unixepoch('now') * 1000, 'historical-refresh-holder', 0, NULL, \
                     unixepoch('now') * 1000, \
                     'provider_not_applied', 'historical refresh resolution', zeroblob(32), \
                     'refresh', NULL)",
        )
        .bind(selector.owner().as_str())
        .bind(id.to_string())
        .execute(&self.sql_pool)
        .await
        .expect("seed resolved refresh incident history");
    }
}

#[tokio::test]
async fn revoke_incident_blocks_use_and_only_matching_adjudication_tombstones() {
    let _serial = TEST_SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let id = fixture.create().await;
    let selector = fixture.selector(id);
    assert_eq!(
        fixture.observe_both(id).await,
        CredentialAvailability::Available,
        "a fresh credential is usable"
    );

    let revoke = fixture
        .command(CredentialCommand::Revoke { credential_id: id })
        .await;
    let revoke_error = revoke.as_ref().expect_err("local tombstone fails");
    for rendered in [format!("{revoke_error}"), format!("{revoke_error:?}")] {
        assert!(!rendered.contains("provider-secret"));
        assert!(!rendered.contains(&id.to_string()));
    }
    assert!(matches!(
        revoke,
        Err(CredentialControllerError::Service(
            CredentialServiceError::RevokePostProviderPersistence
        ))
    ));
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 1);
    assert!(matches!(
        fixture
            .raw
            .operation_status(&selector)
            .await
            .expect("operation status"),
        CredentialOperationStatus::InFlight {
            operation: CredentialOperationKind::Revoke
        }
    ));
    assert_eq!(
        fixture.observe_both(id).await,
        CredentialAvailability::Blocked(CredentialBlock::OperationInFlight {
            operation: CredentialOperationKind::Revoke
        })
    );

    fixture.expire_claim(id, 99).await;
    assert_eq!(
        fixture.observe_both(id).await,
        CredentialAvailability::Blocked(CredentialBlock::ReconciliationRequired {
            operation: CredentialOperationKind::Revoke
        })
    );
    let status = fixture
        .raw
        .operation_status(&selector)
        .await
        .expect("operation status");
    let CredentialOperationStatus::ReconciliationRequired {
        operation: CredentialOperationKind::Revoke,
        incident,
    } = status
    else {
        panic!("an expired revoke claim requires reconciliation, got {status:?}")
    };
    let listed = fixture
        .command(CredentialCommand::List)
        .await
        .expect("list remains available while revoke needs reconciliation");
    let CredentialCommandResult::Heads(heads) = listed else {
        panic!("list returns heads")
    };
    let listed = heads
        .iter()
        .find(|head| head.id == id)
        .expect("poisoned credential remains in management list");
    assert!(matches!(
        listed.lifecycle,
        nebula_credential::CredentialLifecycleState::ReconciliationRequired {
            operation: Some(nebula_credential::CredentialLifecycleOperation::Revoke),
            incident: Some(listed_incident),
        } if listed_incident == incident
    ));
    let base = BaseContext::builder(nebula_core::Scope::default()).build_with(Principal::System);
    assert!(matches!(
        fixture
            .projection
            .resolve_slot(
                &TenantScope::from_scope(&fixture.scope),
                id,
                nebula_core::credential_key!("operation_incident_probe"),
                nebula_credential::Capabilities::empty(),
                base.cancellation().child_token(),
            )
            .await,
        Err(CredentialSlotResolveError::OperationBlocked {
            operation: CredentialOperationKind::Revoke
        })
    ));

    assert!(matches!(
        fixture
            .resolver
            .resolve_scoped::<ProbeCredential>(&selector)
            .await,
        Err(nebula_credential::runtime::ResolveError::OperationBlocked {
            operation: CredentialOperationKind::Revoke
        })
    ));
    assert!(matches!(
        fixture
            .resolver
            .resolve_with_refresh::<ProbeCredential>(
                &selector,
                &CredentialContext::for_owner(fixture.scope.credential_owner_id())
            )
            .await,
        Err(nebula_credential::runtime::ResolveError::OperationBlocked {
            operation: CredentialOperationKind::Revoke
        })
    ));
    assert!(matches!(
        fixture
            .command(CredentialCommand::Test { credential_id: id })
            .await,
        Err(CredentialControllerError::Service(
            CredentialServiceError::OperationBlocked {
                operation: CredentialOperationKind::Revoke
            }
        ))
    ));
    assert_eq!(REFRESH_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(TEST_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 1);

    let wrong = fixture
        .command(CredentialCommand::Reconcile {
            credential_id: id,
            incident,
            decision: CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderApplied),
            evidence: EVIDENCE.to_owned(),
        })
        .await;
    let wrong_error = wrong.as_ref().expect_err("refresh cannot resolve revoke");
    for rendered in [format!("{wrong_error}"), format!("{wrong_error:?}")] {
        assert!(!rendered.contains("provider-secret"));
        assert!(!rendered.contains(EVIDENCE));
        assert!(!rendered.contains(&id.to_string()));
    }
    assert!(matches!(
        wrong,
        Err(CredentialControllerError::Adjudication(
            RefreshClaimAdjudicationError::OperationMismatch {
                recorded_operation: CredentialOperationKind::Revoke,
            }
        ))
    ));
    assert!(matches!(
        fixture
            .raw
            .operation_status(&selector)
            .await
            .expect("claim retained"),
        CredentialOperationStatus::ReconciliationRequired {
            operation: CredentialOperationKind::Revoke,
            incident: retained,
        } if retained == incident
    ));

    let decision = CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderRevoked);
    let first = fixture
        .command(CredentialCommand::Reconcile {
            credential_id: id,
            incident,
            decision,
            evidence: EVIDENCE.to_owned(),
        })
        .await
        .expect("matching revoke adjudication");
    let CredentialCommandResult::Reconciled(first) = first else {
        panic!("reconciled result")
    };
    assert!(first.changed);
    assert!(matches!(
        fixture
            .raw
            .get(&selector)
            .await
            .expect("physical tombstone"),
        StoredCredential::Tombstoned(_)
    ));

    let replay = fixture
        .command(CredentialCommand::Reconcile {
            credential_id: id,
            incident,
            decision,
            evidence: EVIDENCE.to_owned(),
        })
        .await
        .expect("identical adjudication is idempotent");
    let CredentialCommandResult::Reconciled(replay) = replay else {
        panic!("reconciled replay")
    };
    assert!(!replay.changed);
    assert_eq!(replay.evidence_digest, first.evidence_digest);
    assert_eq!(
        REVOKE_CALLS.load(Ordering::SeqCst),
        1,
        "adjudication never replays provider revoke"
    );

    // A display-only write preserves material authority but advances the row
    // version. Pause after provider revoke and before local finalization to
    // prove the epoch-based finalizer tolerates that harmless concurrent CAS.
    let raced_id = fixture.create().await;
    let raced_selector = fixture.selector(raced_id);
    fixture.tombstone_fault.disarm();
    fixture.tombstone_fault.pause_revoked_finalizer();
    let revoke = fixture.command(CredentialCommand::Revoke {
        credential_id: raced_id,
    });
    let update = async {
        fixture.tombstone_fault.wait_for_revoked_finalizer().await;
        let update = fixture
            .command(CredentialCommand::Update {
                credential_id: raced_id,
                properties: None,
                expected_version: None,
                display: CredentialDisplayPatch {
                    display_name: Some("renamed during revoke finalization".to_owned()),
                    ..CredentialDisplayPatch::default()
                },
            })
            .await;
        fixture.tombstone_fault.continue_revoked_finalizer();
        update
    };
    let (revoke, update) = tokio::join!(revoke, update);
    assert!(matches!(
        update.expect("display-only update preserves material epoch"),
        CredentialCommandResult::Head(_)
    ));
    assert!(matches!(
        revoke.expect("epoch-based revoke finalizer ignores display version bump"),
        CredentialCommandResult::Revoked
    ));
    assert!(matches!(
        fixture
            .raw
            .get(&raced_selector)
            .await
            .expect("raced credential tombstone"),
        StoredCredential::Tombstoned(_)
    ));
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn refresh_winner_makes_waiting_revoke_report_version_conflict_without_provider_replay() {
    let _serial = TEST_SERIAL.lock().await;
    REVOKE_CALLS.store(0, Ordering::SeqCst);
    REFRESH_CALLS.store(0, Ordering::SeqCst);
    TEST_CALLS.store(0, Ordering::SeqCst);

    let directory = tempfile::tempdir().expect("temp db directory");
    let path = directory
        .path()
        .join("credential-cached-revoke-race.sqlite");
    let db = path.to_string_lossy().into_owned();
    let raw = SqliteCredentialPersistence::connect(&db)
        .await
        .expect("sqlite credential store");
    let claims = Arc::new(raw.refresh_claim_repo());
    let cached = Arc::new(CacheLayer::new(raw.clone(), CacheConfig::default()));
    let (revoke_controller, _, _, revoke_fault) =
        compose_runtime(Arc::clone(&cached), Arc::clone(&claims));
    let (refresh_controller, _, _, _) = compose_runtime(raw.clone(), claims);
    revoke_fault.disarm();
    let scope = Scope::new("cached-revoke-race", "nebula-storage-test");
    let actor = CredentialActor::user(UserId::new());
    let created = revoke_controller
        .execute(
            &actor,
            &scope,
            CredentialCommand::Create {
                credential_key: nebula_core::credential_key!("operation_incident_probe"),
                properties: json!({ "token": "provider-secret" }),
                display: CredentialDisplay::default(),
            },
        )
        .await
        .expect("create succeeds");
    let CredentialCommandResult::Head(head) = created else {
        panic!("create returns head")
    };
    let id = CredentialId::parse(&head.id).expect("credential id");
    let selector = CredentialSelector::new(CredentialOwner::from_scope(&scope), id);
    let cached_before = cached.get(&selector).await.expect("warm material cache");
    let StoredCredential::Live(cached_before) = cached_before else {
        panic!("created credential is live")
    };
    let old_epoch = cached_before.material_epoch();

    PAUSE_REFRESH_PROVIDER.store(true, Ordering::SeqCst);
    OBSERVE_REVOKE_CLAIM.store(true, Ordering::SeqCst);

    let refresh = refresh_controller.execute(
        &actor,
        &scope,
        CredentialCommand::Refresh { credential_id: id },
    );
    let revoke = async {
        REFRESH_PROVIDER_ENTERED.notified().await;
        revoke_controller
            .execute(
                &actor,
                &scope,
                CredentialCommand::Revoke { credential_id: id },
            )
            .await
    };
    let release_refresh = async {
        REVOKE_CLAIM_ATTEMPTED.notified().await;
        REFRESH_PROVIDER_CONTINUE.notify_one();
    };
    let (refresh, revoke, ()) = tokio::join!(refresh, revoke, release_refresh);

    assert!(matches!(
        refresh.expect("refresh winner persists its advanced material"),
        CredentialCommandResult::Refreshed(_)
    ));
    assert!(matches!(
        revoke,
        Err(CredentialControllerError::Service(
            CredentialServiceError::VersionConflict { .. }
        ))
    ));
    assert_eq!(REFRESH_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(
        REVOKE_CALLS.load(Ordering::SeqCst),
        0,
        "revoke must reject the stale material epoch before provider dispatch"
    );

    let cached_after = cached
        .get(&selector)
        .await
        .expect("physical material cache remains readable");
    let StoredCredential::Live(cached_after) = cached_after else {
        panic!("refresh keeps credential live")
    };
    assert_eq!(
        cached_after.material_epoch(),
        old_epoch,
        "the material read used to classify coalescing is demonstrably stale"
    );
    let authoritative = raw
        .get_operational_head(&selector)
        .await
        .expect("authoritative operational head");
    assert_ne!(
        authoritative.head().material_epoch(),
        old_epoch,
        "authoritative classification observes the refresh winner's epoch"
    );
}

#[tokio::test]
async fn revoke_sweep_never_applies_refresh_reauthentication_escalation() {
    let _serial = TEST_SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let id = fixture.create().await;
    let selector = fixture.selector(id);
    let before = fixture
        .raw
        .operation_status(&selector)
        .await
        .expect("initial status");
    let CredentialOperationStatus::Open {
        material_epoch: before_epoch,
        reauth_required: false,
        ..
    } = before
    else {
        panic!("a new credential starts open")
    };
    fixture.seed_resolved_refresh_history(id).await;

    let revoke = fixture
        .command(CredentialCommand::Revoke { credential_id: id })
        .await;
    assert!(matches!(
        revoke,
        Err(CredentialControllerError::Service(
            CredentialServiceError::RevokePostProviderPersistence
        ))
    ));
    fixture.expire_claim(id, 1).await;
    let incident = reconciliation_incident(&fixture.raw, &selector).await;

    let reconciled = fixture
        .command(CredentialCommand::Reconcile {
            credential_id: id,
            incident,
            decision: CredentialOperationDecision::Revoke(
                RevokeOutcomeDecision::ProviderNotRevoked,
            ),
            evidence: "provider audit proves the revoke request was rejected".to_owned(),
        })
        .await
        .expect("provider-not-revoked clears only the revoke incident");
    assert!(matches!(
        reconciled,
        CredentialCommandResult::Reconciled(adjudication) if adjudication.changed
    ));
    assert!(matches!(
        fixture
            .raw
            .operation_status(&selector)
            .await
            .expect("open after reconciliation"),
        CredentialOperationStatus::Open {
            material_epoch,
            reauth_required: false,
            ..
        } if material_epoch == before_epoch
    ));
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(REFRESH_CALLS.load(Ordering::SeqCst), 0);
}

/// The contract's "old use revision does not admit": a revoke claim that was
/// acquired and then abandoned before the provider boundary closes use by
/// itself and reopens by expiry alone. A consumer that bound the first
/// projection must see a different use revision on the next one, although
/// the material authority is unchanged.
#[tokio::test]
async fn abandoned_revoke_claim_reprojects_at_the_next_admission_epoch() {
    let _serial = TEST_SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let id = fixture.create().await;
    let selector = fixture.selector(id);
    let project = || async {
        let base =
            BaseContext::builder(nebula_core::Scope::default()).build_with(Principal::System);
        fixture
            .projection
            .resolve_slot(
                &TenantScope::from_scope(&fixture.scope),
                id,
                nebula_core::credential_key!("operation_incident_probe"),
                nebula_credential::Capabilities::empty(),
                base.cancellation().child_token(),
            )
            .await
            .expect("an open credential projects")
    };

    let first = project().await;
    let first_material = first.metadata().material_epoch();
    let first_admission = first.metadata().admission_epoch();
    drop(first);

    let material_epoch = CredentialMaterialEpoch::try_from(first_material)
        .expect("the projected material epoch is valid");
    let claimed = fixture
        .claims
        .try_claim(
            &selector,
            &ReplicaId::new("abandoning-revoker"),
            Duration::from_secs(30),
            CredentialOperationIntent::Revoke { material_epoch },
        )
        .await
        .expect("the revoke claim reaches the backend");
    assert!(matches!(claimed, ClaimAttempt::Acquired(_)));
    assert!(matches!(
        fixture
            .projection
            .resolve_slot(
                &TenantScope::from_scope(&fixture.scope),
                id,
                nebula_core::credential_key!("operation_incident_probe"),
                nebula_credential::Capabilities::empty(),
                BaseContext::builder(nebula_core::Scope::default())
                    .build_with(Principal::System)
                    .cancellation()
                    .child_token(),
            )
            .await,
        Err(CredentialSlotResolveError::OperationBlocked {
            operation: CredentialOperationKind::Revoke
        })
    ));
    // Abandon it: the holder never marks the sentinel, and the claim lapses.
    let affected =
        sqlx::query("UPDATE credential_refresh_claims SET expires_at = 0 WHERE credential_id = ?1")
            .bind(id.to_string())
            .execute(&fixture.sql_pool)
            .await
            .expect("backdate the abandoned claim")
            .rows_affected();
    assert_eq!(affected, 1);

    let second = project().await;
    assert_eq!(
        second.metadata().material_epoch(),
        first_material,
        "an abandoned revoke changes no material"
    );
    assert_eq!(
        second.metadata().admission_epoch(),
        first_admission + 1,
        "the first projection's use revision no longer admits"
    );
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 0);
}

/// A refresh marks its sentinel before the provider call. That write closes use
/// by advancing the admission epoch alone — never the row version — so the
/// refresh write-back lands on the version it read, with no rebase.
#[tokio::test]
async fn refresh_write_back_needs_no_rebase_over_its_own_sentinel() {
    let _serial = TEST_SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let id = fixture.create().await;
    let selector = fixture.selector(id);
    let CredentialOperationStatus::Open {
        version: before_version,
        material_epoch: before_material,
        admission_epoch: before_admission,
        ..
    } = fixture
        .raw
        .operation_status(&selector)
        .await
        .expect("initial status")
    else {
        panic!("a new credential starts open")
    };

    let refreshed = fixture
        .command(CredentialCommand::Refresh { credential_id: id })
        .await
        .expect("refresh succeeds");
    assert!(matches!(refreshed, CredentialCommandResult::Refreshed(_)));
    assert_eq!(REFRESH_CALLS.load(Ordering::SeqCst), 1);

    // The claim is released after the command answers, so read the committed
    // aggregate columns directly rather than waiting for the status to reopen.
    let (version, material_epoch, admission_epoch, reauth_required): (i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT version, material_epoch, admission_epoch, reauth_required \
             FROM credentials WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(&fixture.sql_pool)
        .await
        .expect("the refreshed row is readable");
    assert_eq!(
        version,
        before_version.get() + 1,
        "the write-back is the only version step: the sentinel never moved it"
    );
    assert_eq!(material_epoch, before_material.get() + 1);
    assert_eq!(
        admission_epoch,
        before_admission.get() + 2,
        "the sentinel and the material advance each closed use once"
    );
    assert_eq!(reauth_required, 0);
}

#[tokio::test]
async fn legacy_unclassified_poison_refuses_inference_but_allows_terminal_recovery() {
    let _serial = TEST_SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let id = fixture.create().await;
    let selector = fixture.selector(id);
    let claim = fixture
        .claims
        .try_claim(
            &selector,
            &ReplicaId::new("pre-typed-operation-runtime"),
            Duration::from_secs(30),
            CredentialOperationIntent::Refresh,
        )
        .await
        .expect("seed claim");
    let ClaimAttempt::Acquired(claim) = claim else {
        panic!("fresh credential claim is acquired")
    };
    fixture
        .claims
        .mark_sentinel(&claim.token)
        .await
        .expect("seed provider boundary");
    sqlx::query(
        "UPDATE credential_refresh_claims \
         SET operation_kind = 'legacy_unclassified', observed_material_epoch = NULL, \
             expires_at = 0 \
         WHERE owner_id = ?1 AND credential_id = ?2",
    )
    .bind(selector.owner().as_str())
    .bind(id.to_string())
    .execute(&fixture.sql_pool)
    .await
    .expect("simulate migration of a pre-typed sentinel");
    fixture
        .claims
        .reclaim_stuck(SentinelEscalationPolicy::new(1, Duration::from_hours(1)).expect("policy"))
        .await
        .expect("account legacy poison");
    let incident = CredentialIncidentRef::from_uuid(claim.token.claim_id);
    assert!(matches!(
        fixture
            .raw
            .operation_status(&selector)
            .await
            .expect("legacy operation status"),
        CredentialOperationStatus::ReconciliationRequired {
            operation: CredentialOperationKind::LegacyUnclassified,
            incident: published,
        } if published == incident
    ));

    for decision in [
        CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderNotApplied),
        CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderNotRevoked),
    ] {
        let result = fixture
            .command(CredentialCommand::Reconcile {
                credential_id: id,
                incident,
                decision,
                evidence: "legacy operation cannot be inferred".to_owned(),
            })
            .await;
        assert!(matches!(
            result,
            Err(CredentialControllerError::Adjudication(
                RefreshClaimAdjudicationError::OperationMismatch {
                    recorded_operation: CredentialOperationKind::LegacyUnclassified
                }
            ))
        ));
    }

    // This fixture normally fails the first tombstone to exercise post-provider
    // finalization. Legacy recovery tests the real delete path, so disable that
    // unrelated fault before issuing the terminal command.
    fixture.tombstone_fault.disarm();
    assert!(matches!(
        fixture
            .command(CredentialCommand::Delete { credential_id: id })
            .await
            .expect("explicit terminal recovery remains available"),
        CredentialCommandResult::Deleted
    ));
    assert!(matches!(
        fixture
            .raw
            .get(&selector)
            .await
            .expect("physical terminal record"),
        StoredCredential::Tombstoned(_)
    ));
    let (retained_claims,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM credential_refresh_claims \
         WHERE owner_id = ?1 AND credential_id = ?2",
    )
    .bind(selector.owner().as_str())
    .bind(id.to_string())
    .fetch_one(&fixture.sql_pool)
    .await
    .expect("retained legacy claim count");
    assert_eq!(
        retained_claims, 1,
        "delete does not invent a provider outcome"
    );

    let replacement = fixture.create().await;
    assert_ne!(replacement, id);
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(REFRESH_CALLS.load(Ordering::SeqCst), 0);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_revoke_incidents_adjudicate_without_provider_replay() {
    use nebula_storage::credential::PgCredentialPersistence;
    use sqlx::{Connection as _, PgConnection, PgPool};

    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "NEBULA_REQUIRE_POSTGRES=1 but DATABASE_URL is absent"
            );
            return;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };
    let _serial = TEST_SERIAL.lock().await;
    const RECLAIM_LOCK: i64 = 0x4E42_5246_434C_414D;
    let mut advisory = PgConnection::connect(&url)
        .await
        .expect("postgres advisory session");
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RECLAIM_LOCK)
        .execute(&mut advisory)
        .await
        .expect("serialize credential reclaim tests");

    REVOKE_CALLS.store(0, Ordering::SeqCst);
    REFRESH_CALLS.store(0, Ordering::SeqCst);
    TEST_CALLS.store(0, Ordering::SeqCst);
    let raw = PgCredentialPersistence::connect(&url)
        .await
        .expect("postgres credential store");
    let claims = Arc::new(raw.refresh_claim_repo());
    let pool = PgPool::connect(&url)
        .await
        .expect("postgres inspection pool");
    let (controller, _resolver, _projection, fault) =
        compose_runtime(raw.clone(), Arc::clone(&claims));
    let scope = Scope::new(
        format!("operation-incidents-{}", uuid::Uuid::new_v4()),
        "nebula-storage-test",
    );
    let actor = CredentialActor::user(UserId::new());

    async fn execute(
        controller: &CredentialController,
        actor: &CredentialActor,
        scope: &Scope,
        command: CredentialCommand,
    ) -> Result<CredentialCommandResult, CredentialControllerError> {
        controller.execute(actor, scope, command).await
    }
    async fn create(
        controller: &CredentialController,
        actor: &CredentialActor,
        scope: &Scope,
    ) -> CredentialId {
        let result = execute(
            controller,
            actor,
            scope,
            CredentialCommand::Create {
                credential_key: nebula_core::credential_key!("operation_incident_probe"),
                properties: json!({ "token": "postgres-provider-secret" }),
                display: CredentialDisplay::default(),
            },
        )
        .await
        .expect("postgres create");
        let CredentialCommandResult::Head(head) = result else {
            panic!("create returns head")
        };
        CredentialId::parse(&head.id).expect("credential id")
    }
    async fn expire_and_reclaim(
        pool: &PgPool,
        claims: &nebula_storage::credential::PgRefreshClaimRepo,
        selector: &CredentialSelector,
    ) {
        let affected = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = clock_timestamp() - interval '1 second' \
             WHERE owner_id = $1 AND credential_id = $2",
        )
        .bind(selector.owner().as_str())
        .bind(selector.credential_id().to_string())
        .execute(pool)
        .await
        .expect("expire postgres claim")
        .rows_affected();
        assert_eq!(affected, 1);
        claims
            .reclaim_stuck(
                SentinelEscalationPolicy::new(99, Duration::from_hours(1)).expect("policy"),
            )
            .await
            .expect("account postgres incident");
    }

    let revoked_id = create(&controller, &actor, &scope).await;
    let revoked_selector = CredentialSelector::new(CredentialOwner::from_scope(&scope), revoked_id);
    assert!(matches!(
        execute(
            &controller,
            &actor,
            &scope,
            CredentialCommand::Revoke {
                credential_id: revoked_id
            }
        )
        .await,
        Err(CredentialControllerError::Service(
            CredentialServiceError::RevokePostProviderPersistence
        ))
    ));
    expire_and_reclaim(&pool, claims.as_ref(), &revoked_selector).await;
    let revoked_incident = reconciliation_incident(&raw, &revoked_selector).await;
    assert!(matches!(
        execute(
            &controller,
            &actor,
            &scope,
            CredentialCommand::Reconcile {
                credential_id: revoked_id,
                incident: revoked_incident,
                decision: CredentialOperationDecision::Refresh(
                    RefreshOutcomeDecision::ProviderApplied
                ),
                evidence: EVIDENCE.to_owned(),
            }
        )
        .await,
        Err(CredentialControllerError::Adjudication(
            RefreshClaimAdjudicationError::OperationMismatch {
                recorded_operation: CredentialOperationKind::Revoke
            }
        ))
    ));
    let revoked = CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderRevoked);
    for expected_changed in [true, false] {
        let result = execute(
            &controller,
            &actor,
            &scope,
            CredentialCommand::Reconcile {
                credential_id: revoked_id,
                incident: revoked_incident,
                decision: revoked,
                evidence: EVIDENCE.to_owned(),
            },
        )
        .await
        .expect("postgres revoke adjudication");
        assert!(matches!(
            result,
            CredentialCommandResult::Reconciled(adjudication)
                if adjudication.changed == expected_changed
        ));
    }
    assert!(matches!(
        raw.get(&revoked_selector)
            .await
            .expect("postgres tombstone"),
        StoredCredential::Tombstoned(_)
    ));

    fault.arm();
    let retained_id = create(&controller, &actor, &scope).await;
    let retained_selector =
        CredentialSelector::new(CredentialOwner::from_scope(&scope), retained_id);
    let before_epoch = match raw
        .operation_status(&retained_selector)
        .await
        .expect("initial status")
    {
        CredentialOperationStatus::Open { material_epoch, .. } => material_epoch,
        _ => panic!("new credential is open"),
    };
    assert!(matches!(
        execute(
            &controller,
            &actor,
            &scope,
            CredentialCommand::Revoke {
                credential_id: retained_id
            }
        )
        .await,
        Err(CredentialControllerError::Service(
            CredentialServiceError::RevokePostProviderPersistence
        ))
    ));
    expire_and_reclaim(&pool, claims.as_ref(), &retained_selector).await;
    let retained_incident = reconciliation_incident(&raw, &retained_selector).await;
    execute(
        &controller,
        &actor,
        &scope,
        CredentialCommand::Reconcile {
            credential_id: retained_id,
            incident: retained_incident,
            decision: CredentialOperationDecision::Revoke(
                RevokeOutcomeDecision::ProviderNotRevoked,
            ),
            evidence: "provider audit proves no postgres revoke".to_owned(),
        },
    )
    .await
    .expect("postgres provider-not-revoked adjudication");
    assert!(matches!(
        raw.operation_status(&retained_selector)
            .await
            .expect("open retained credential"),
        CredentialOperationStatus::Open {
            material_epoch,
            reauth_required: false,
            ..
        } if material_epoch == before_epoch
    ));
    assert_eq!(REVOKE_CALLS.load(Ordering::SeqCst), 2);

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RECLAIM_LOCK)
        .execute(&mut advisory)
        .await
        .expect("release reclaim test lock");
}
