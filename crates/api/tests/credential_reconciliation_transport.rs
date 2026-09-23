//! U3c acceptance — the reconcile route is a user's way out of a retained
//! poison, walked through the HTTP transport.
//!
//! Two sequences, over the real `axum` router, the real `CredentialController`,
//! and a real refresh coordinator over a real claim store:
//!
//! 1. refresh a credential through `/credentials/{cred}/refresh`; put its claim
//!    row into expired fail-closed poison after a provider outcome becomes
//!    unknown; read the refresh route refusing that credential with
//!    409; record the provider outcome through `/credentials/{cred}/reconcile`;
//!    then refresh again and watch it work. That last step is what gives the
//!    route its meaning — without it the suite would pass on a route that
//!    records a decision nothing reads.
//! 2. reconcile a credential that was never poisoned, and read the *other* 409
//!    (`credential-reconciliation-not-required`).
//!
//! # Why this fixture composes the service itself
//!
//! Every test-util factory in `crates/api/src/ports/` (`with_store`,
//! `with_memory_store`, `with_memory_store_parts`) funnels into the private
//! `compose_credential_service`, which hardcodes `NoNetworkRefreshTransport`
//! and a `CredentialServiceBuilder` whose `build()` constructs an
//! `InMemoryRefreshClaimRepo` internally
//! (`crates/api/src/ports/credential_builder.rs`) — reachable by
//! nobody. A service built that way can refresh, but its claim store is not the
//! one any test can observe or poison, so it cannot show a 409 clearing.
//!
//! The default registry is the second half of the problem: it registers
//! `api_key` / `basic_auth` / `signing_key`, none of them refreshable, and
//! `service/capabilities.rs:154` refuses a non-refreshable type with
//! `CapabilityUnsupported` (400) *before* any claim work. So a refreshable type
//! is composed here too.
//!
//! The composition below is the one `CredentialServiceBuilder::build` performs,
//! written out against the same public seams — `EncryptionLayer`, `AuditLayer`,
//! `CredentialResolver::with_dependencies`, `LeaseLifecycle::spawn`,
//! `CredentialService::from_secure_parts` (whose rustdoc names trusted
//! in-workspace test composition as exactly why it is `pub`) — with one
//! substitution: the coordinator is built by
//! `RefreshCoordinator::new_with(Arc<dyn RefreshClaimRepo>, …)` over a claim
//! store this suite holds, so the test can both observe the poison and hand the
//! same object to the controller as the `RefreshClaimAdjudicator` that clears
//! it.
//!
//! # How the poison is produced
//!
//! By the adapter's own lifecycle, not by backdating SQL: `try_claim` takes the
//! claim, `mark_sentinel` records that provider egress began, the fixture's
//! `ManualClock` passes the lease deadline, and `reclaim_stuck` accounts the
//! incident and retains the row. That is the state
//! `crates/storage-port/src/store/refresh_claim.rs:211-227` refuses to release
//! and `RefreshClaimAdjudicator::adjudicate` is the only clearer of. Reaching it
//! through a hand-moved clock rather than a raw `UPDATE` also means the fixture
//! asserts nothing about adapter internals it does not own.

mod common;

use std::{assert_matches, future::Future, pin::Pin, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use common::{
    create_state_with_queue, create_test_jwt, http_helpers::auth_json, port_scope, ws_path,
};
use nebula_api::{
    ApiConfig, AppState, app,
    domain::auth::backend::{
        AuthBackend, CreatePatParams, InMemoryAuthBackend, SignupRequest,
        dto::SecretString as ApiSecretString,
    },
    domain::credential::dto::CreateCredentialRequest,
    middleware::auth::AuthenticatedPrincipal,
    ports::credential_command::{
        CredentialCommandGateway, CredentialGatewayCommand, CredentialGatewayResult,
        test_gateway_from_service_with_reconciliation,
    },
};
use nebula_core::accessor::Clock;
use nebula_core::auth::{
    AuthPattern, AuthScheme, EgressShape, RefreshStrategyKind, SchemeFamily, SensitiveScheme,
};
use nebula_core::{CredentialId, UserId};
use nebula_credential::error::CredentialError;
use nebula_credential::resolve::StaticResolveResult;
use nebula_credential::runtime::{
    AcquisitionTransport, AcquisitionTransportError, CredentialResolver, LeaseLifecycle,
    LeaseLifecycleConfig, RefreshCoordConfig, RefreshCoordinator, RefreshTransport,
    RefreshTransportError, TokenPostRequest, TokenPostResponse,
};
use nebula_credential::{
    CredentialContext, CredentialMetadataDraft, CredentialRegistry, CredentialService, DispatchOps,
    ErasedPendingStore, NoopObserver, RefreshAttempt, RefreshReport, SecretString, StateSource,
    StateWireFingerprint, identity_state, register_refreshable_ops, register_runtime_ops,
};
use nebula_schema::Schema;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditSink, EncryptionLayer, EnvKeyProvider, InMemoryPendingStore,
    InMemoryRefreshClaimRepo,
};
use nebula_storage_port::store::{
    ClaimAttempt, RefreshClaimAdjudicator, RefreshClaimStore, ReplicaId,
};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the factory's
/// dev key). Not a secret: a fixed test constant.
const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

/// The credential type this suite registers. Nothing in the default registry is
/// refreshable, so the route's refresh path is unreachable without it.
const PROBE_KEY: &str = "reconcile_probe";

/// The probe's only property, and the material `refresh` rotates. Not a secret:
/// a fixture value that never leaves the process.
const PROBE_TOKEN: &str = "reconcile-probe-v1";

/// The claim lease the suite's coordinator runs with, and the value it passes
/// to `try_claim` when the fixture poisons a row. Shared so the fixture's
/// hand-moved clock expires exactly the lease the coordinator wrote.
const CLAIM_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The operator's note. A provider support ticket is exactly the kind of
/// evidence this parameter exists for.
const EVIDENCE: &str = "provider ticket 5512: the token endpoint refused the grant";

/// The three wire problem types this suite asserts, spelled as
/// `ProblemDetails::type_uri` (RFC 9457 `type`), which is what a client reads.
///
/// They are distinct rather than one `conflict` type on purpose: the U3a/U3b
/// design gave each reconciliation refusal its own problem so a client can tell
/// "nothing to reconcile" from "two observations disagree" without parsing
/// prose. `crates/api/src/error/mod.rs:593-632` is where they are built.
const RECONCILIATION_NOT_REQUIRED_TYPE: &str =
    "https://nebula.dev/problems/credential-reconciliation-not-required";
const RECONCILIATION_CONFLICT_TYPE: &str =
    "https://nebula.dev/problems/credential-reconciliation-conflict";
const OUTCOME_UNKNOWN_TYPE: &str = "https://nebula.dev/problems/outcome-unknown";

// ── A non-interactive, Refreshable probe credential ────────────────────────

/// Active family declaring an engine-drivable `RefreshToken` class, so the
/// `Refreshable` probe passes the F3 containment law at registration.
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

/// Stored state == projected scheme (identity). Holds material, so it is
/// `Sensitive`; `generation` lets `refresh` produce visibly rotated bytes.
#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop, StateWireFingerprint)]
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

identity_state!(ProbeScheme, "reconcile_probe_state", 1);

/// Create-form properties.
#[derive(Schema, Deserialize)]
struct ProbeProps {
    /// Initial secret token.
    #[field(secret, label = "Token")]
    #[validate(required)]
    token: SecretString,
}

/// The credential type under test: refreshable, non-interactive, and no other
/// lifecycle capability — the smallest type that can reach a claim store.
struct ProbeCredential;

#[nebula_credential::credential(key = "reconcile_probe")]
impl ProbeCredential {
    type Properties = ProbeProps;
    type Scheme = ProbeScheme;
    type State = ProbeScheme;

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("reconcile_probe"),
            nebula_credential::metadata_name!("Reconcile Transport Probe"),
            "non-interactive refreshable credential for the U3c transport acceptance test",
        )
    }

    fn project(state: &ProbeScheme) -> ProbeScheme {
        state.clone()
    }

    async fn resolve(
        properties: &ProbeProps,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<ProbeScheme>, CredentialError> {
        Ok(StaticResolveResult::Complete(ProbeScheme {
            token: properties.token.expose_secret().to_owned(),
            generation: 1,
        }))
    }

    /// Rotate in place. No provider egress: the claim lifecycle, not an HTTP
    /// call, is what this suite exercises, and the composed transport refuses
    /// every request by construction.
    async fn refresh(state: &mut ProbeScheme, attempt: RefreshAttempt<'_>) -> RefreshReport {
        let completed = match attempt
            .dispatch(|| async { Ok::<(), std::convert::Infallible>(()) })
            .await
        {
            Ok(completed) => completed,
            Err(unknown) => return unknown.into_report(),
        };
        let ((), proof) = completed.into_parts();
        state.generation += 1;
        proof.refreshed()
    }
}

/// The registry + dispatch ops for the probe alone.
///
/// The registry's advertised capabilities must match the ops table or the
/// service build refuses with `CapabilityWithoutOps`, which is why the two are
/// built in one function.
fn probe_registry_and_ops() -> (CredentialRegistry, DispatchOps<ErasedPendingStore>) {
    let mut registry = CredentialRegistry::new();
    registry
        .register(ProbeCredential, "nebula-api-test")
        .expect("reconcile_probe registers (unique key)");

    let mut ops = DispatchOps::<ErasedPendingStore>::new();
    register_runtime_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).expect("runtime ops");
    register_refreshable_ops::<ProbeCredential, ErasedPendingStore>(&mut ops)
        .expect("refreshable ops");
    (registry, ops)
}

/// A clock the fixture moves by hand, so "expired" costs no sleeping.
///
/// The in-memory claim adapter is the only backend where a test can reach
/// expiry (`crates/storage/src/credential/refresh_claim/in_memory.rs`: the SQL
/// backends decide it in the database), and it takes its clock as a constructor
/// argument for exactly this reason.
struct ManualClock {
    now: std::sync::Mutex<DateTime<Utc>>,
}

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: std::sync::Mutex::new(Utc::now()),
        })
    }

    fn advance(&self, by: ChronoDuration) {
        *self.now.lock().expect("the manual clock is never poisoned") += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("the manual clock is never poisoned")
    }

    fn monotonic(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

/// Audit sink that keeps nothing.
///
/// The suite asserts the claim lifecycle; the credential audit observation is
/// not what routes the request, and a second recorder would only add a fixture
/// to keep in step with `AuditEvent`.
#[derive(Debug)]
struct SilentAuditSink;

impl AuditSink for SilentAuditSink {
    fn record(&self, _event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
        Ok(())
    }
}

/// Refuses every request. Both transport traits it must satisfy are only ever
/// consulted by an OAuth-shaped credential reaching for a token endpoint.
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

/// The composed service plus the claim store it reads, so the suite can poison
/// exactly the rows the route consults.
struct ProbeFixture {
    state: AppState,
    gateway: Arc<dyn CredentialCommandGateway>,
    claims: Arc<InMemoryRefreshClaimRepo>,
    clock: Arc<ManualClock>,
    token: String,
    credential: CredentialId,
}

impl ProbeFixture {
    async fn new() -> Self {
        let clock = ManualClock::new();
        let claims = Arc::new(InMemoryRefreshClaimRepo::with_clock(
            Arc::clone(&clock) as Arc<dyn Clock>
        ));

        let config = RefreshCoordConfig {
            claim_ttl: CLAIM_TTL,
            ..RefreshCoordConfig::default()
        };
        let coordinator = Arc::new(
            RefreshCoordinator::new_with(
                Arc::clone(&claims) as Arc<dyn RefreshClaimStore>,
                ReplicaId::new("u3c-transport-fixture"),
                config,
            )
            .expect("the default coordinator config satisfies its own invariants"),
        );

        let (registry, ops) = probe_registry_and_ops();
        let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte key"));
        let store = nebula_storage::credential::SqliteCredentialPersistence::connect_memory()
            .await
            .expect("the in-memory credential store opens and migrates");

        // The `Audit(Encryption(raw))` stack, in the order
        // `CredentialServiceBuilder::build` composes it.
        let encrypted = EncryptionLayer::new(store, key);
        let persistence: Arc<dyn CredentialPersistence> = Arc::new(encrypted);
        let layered = AuditLayer::new(Arc::clone(&persistence), Arc::new(SilentAuditSink));
        let store: Arc<dyn CredentialPersistence> = Arc::new(layered);

        let transport = Arc::new(NoNetworkTransport);
        let resolver = CredentialResolver::with_dependencies(
            Arc::clone(&store),
            Arc::clone(&coordinator),
            transport.clone(),
        );
        let service = Arc::new(CredentialService::from_secure_parts(
            store,
            resolver,
            LeaseLifecycle::spawn(
                LeaseLifecycleConfig::default(),
                None,
                None,
                CancellationToken::new(),
            ),
            ErasedPendingStore::new(Arc::new(InMemoryPendingStore::new())),
            Arc::new(registry),
            Arc::new(ops),
            Arc::new(NoopObserver::new()),
            transport,
            StateSource::LocalEncrypted,
        ));

        // The controller takes the adjudicator as its own trait object; the same
        // adapter answers `try_claim` and `adjudicate`, so the fixture can watch
        // the state the route refuses on and clear it in the same object.
        let adjudicator: Arc<dyn RefreshClaimAdjudicator> =
            Arc::clone(&claims) as Arc<dyn RefreshClaimAdjudicator>;
        let gateway =
            test_gateway_from_service_with_reconciliation(Arc::clone(&service), adjudicator, None);

        // The shared harness wires the first-party gateway over the default
        // registry; this line replaces it with the probe one. Everything else
        // (resolvers, the CSRF/auth stack, the JWT secret) is the Phase-1/2
        // harness every other HTTP test uses.
        let (state, _queue) = create_state_with_queue().await;
        let state = state.with_credential_gateway(Arc::clone(&gateway));

        let principal = AuthenticatedPrincipal::for_test_user(UserId::new().to_string());
        let created = gateway
            .execute(
                &principal,
                &port_scope(),
                CredentialGatewayCommand::Create(CreateCredentialRequest {
                    credential_key: PROBE_KEY.to_owned(),
                    name: "U3c reconcile probe".to_owned(),
                    description: None,
                    data: json!({ "token": PROBE_TOKEN }),
                    tags: None,
                }),
            )
            .await
            .expect("the probe type must be creatable through the gateway");
        let CredentialGatewayResult::Record(record) = created else {
            panic!("create must answer with the created record, got {created:?}");
        };

        Self {
            state,
            gateway,
            claims,
            clock,
            token: create_test_jwt(),
            credential: CredentialId::parse(&record.id)
                .expect("the gateway returns a parseable credential id"),
        }
    }

    fn principal(&self) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal::for_test_user(UserId::new().to_string())
    }

    fn holder(&self) -> ReplicaId {
        ReplicaId::new("u3c-transport-poisoner")
    }

    fn refresh_uri(&self) -> String {
        ws_path(&format!("/credentials/{}/refresh", self.credential))
    }

    fn reconcile_uri(&self) -> String {
        ws_path(&format!("/credentials/{}/reconcile", self.credential))
    }

    /// Send one authenticated, CSRF-carrying request through the real router.
    async fn send(&self, uri: &str, body: &serde_json::Value) -> (StatusCode, serde_json::Value) {
        let config = ApiConfig::for_test();
        let app = app::build_app(self.state.clone(), &config);
        let response = app
            .oneshot(auth_json("POST", uri, &self.token, body))
            .await
            .expect("the router must answer");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the response body must be readable");
        let parsed = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
        (status, parsed)
    }

    /// Take the credential's claim across the provider boundary and past its
    /// lease, producing retained poison that reconciliation can adjudicate.
    ///
    /// Every step is the port's own lifecycle: the adapter's expiry predicate
    /// is the clock, and no state is reached by writing behind the service's
    /// back.
    async fn poison(&self) {
        let acquired = self
            .claims
            .try_claim(
                &CredentialSelector::new(
                    CredentialOwner::from_scope(&port_scope()),
                    self.credential,
                ),
                &self.holder(),
                CLAIM_TTL,
            )
            .await
            .expect("a free claim is acquirable");
        let ClaimAttempt::Acquired(claim) = acquired else {
            panic!("a free claim must be acquirable, got {acquired:?}");
        };
        self.claims
            .mark_sentinel(&claim.token)
            .await
            .expect("the holder may mark provider egress");
        // One second past the lease, not exactly onto it: `try_claim` and
        // `reclaim_stuck` both treat `expires_at == now` as still valid
        // (`in_memory.rs:129` and `:251`), so advancing by the TTL alone leaves
        // a row the sweep will not touch.
        self.clock
            .advance(ChronoDuration::from_std(CLAIM_TTL).expect("the ttl fits a chrono duration"));
        self.clock.advance(ChronoDuration::seconds(1));

        // Reconciliation can adjudicate the expired poison directly. Threshold
        // accounting belongs to a backend that also owns the credential
        // aggregate; this reference claim adapter deliberately does not.
    }

    /// Ask the port for the credential's claim, on the same trait object the
    /// coordinator uses.
    ///
    /// `try_claim` is the observation rather than a proxy for it: `OutcomeUnknown`
    /// *is* the poisoned answer. A probe reporting `Acquired` has also taken the
    /// claim, so no case may probe and then expect the coordinator to acquire.
    async fn attempt(&self) -> ClaimAttempt {
        self.claims
            .try_claim(
                &CredentialSelector::new(
                    CredentialOwner::from_scope(&port_scope()),
                    self.credential,
                ),
                &self.holder(),
                CLAIM_TTL,
            )
            .await
            .expect("acquisition must not fail")
    }
}

/// RFC 9457 `type` of a problem response, which is the machine-readable half of
/// the refusal a client acts on.
fn problem_type(problem: &serde_json::Value) -> &str {
    problem["type"].as_str().unwrap_or_default()
}

/// A request carrying a **PAT** rather than the fixture's session JWT.
///
/// No CSRF header or cookie: a PAT is a bearer grant with no ambient browser
/// authority to protect, so the CSRF middleware does not apply to it. Sending
/// the session's CSRF pair here would prove nothing about the token under test.
fn pat_json(uri: &str, pat: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {pat}"))
        .body(Body::from(
            serde_json::to_vec(body).expect("the request body must serialize"),
        ))
        .expect("the request must build")
}

// ── R2: refresh → poison → 409 → reconcile → refresh ─────────────────────────

#[tokio::test]
async fn reconcile_through_the_route_clears_a_retained_poison_and_refresh_works_again() {
    let fixture = ProbeFixture::new().await;

    // 1. The credential refreshes through the route, on the probe type. This is
    //    the reachable path CARRY-FORWARD.md §9 describes, minus the ambiguous
    //    provider outcome the fixture fabricates below.
    let (status, refreshed) = fixture.send(&fixture.refresh_uri(), &json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the probe credential must refresh through the route: {refreshed}"
    );
    assert_eq!(refreshed["refreshed"], true, "{refreshed}");

    // 2. Retain the poison: provider egress began and the lease ran out.
    fixture.poison().await;
    assert!(
        matches!(fixture.attempt().await, ClaimAttempt::OutcomeUnknown { .. }),
        "the retained claim must read as outcome-unknown; a poison the port does \
         not answer that way would make step 3 pass for the wrong reason"
    );

    // 3. The refresh route now refuses the credential, and keeps refusing it:
    //    the claim row is durable and outlives its own expiry.
    let (status, problem) = fixture.send(&fixture.refresh_uri(), &json!({})).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a poisoned claim must refuse the refresh, got {problem}"
    );
    assert_eq!(
        problem_type(&problem),
        OUTCOME_UNKNOWN_TYPE,
        "the refusal a retained poison produces is the outcome-unknown problem. \
         It is not the reconciliation-required problem: that one is the L1-waiter \
         path (`runtime/refresh/coordinator.rs:967`), which a first, uncoalesced \
         refresh never reaches — the claim answer here is `OutcomeUnknown`, and \
         `capabilities.rs:495` maps it to `CredentialServiceError::OutcomeUnknown`. \
         Both problem types name reconciliation as the remedy; {problem}"
    );
    assert_eq!(
        problem["status"], 409,
        "the problem document's own status must agree: {problem}"
    );
    // The remedy the brief's step 3 is about: the refusal is only useful if it
    // tells the operator what to do. It does — the detail names reconciliation
    // as the required step before a retry — which is what makes the route in
    // step 4 the answer the client is being pointed at.
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("reconcile credential state")),
        "a retained poison must refuse with a remedy, not a bare conflict: {problem}"
    );

    // 4. The remedy, through the route that did not exist before this unit.
    let (status, reconciled) = fixture
        .send(
            &fixture.reconcile_uri(),
            &json!({ "decision": "provider_not_applied", "evidence": EVIDENCE }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an operator decision must be recordable: {reconciled}"
    );
    assert_eq!(
        reconciled["decision"], "provider_not_applied",
        "{reconciled}"
    );
    assert_eq!(
        reconciled["changed"], true,
        "the first adjudication of this incident reports that it wrote: {reconciled}"
    );

    // 4b. Repeating the identical request is the idempotent recommit, and it is
    //     a success with `changed: false`, not a conflict — the distinction the
    //     response shape exists to carry.
    let (status, repeated) = fixture
        .send(
            &fixture.reconcile_uri(),
            &json!({ "decision": "provider_not_applied", "evidence": EVIDENCE }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an identical recommit is a no-op success, not a conflict: {repeated}"
    );
    assert_eq!(repeated["changed"], false, "{repeated}");

    // 5. The assertion that gives the route its meaning: the same refresh call
    //    that answered 409 in step 3 now runs a refresh. A 200 carrying
    //    `refreshed: true` is only reachable after `try_claim` acquired a claim,
    //    and the coordinator acquires none while the poison stands.
    let (status, refreshed) = fixture.send(&fixture.refresh_uri(), &json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reconciling must make the credential refreshable again: {refreshed}"
    );
    assert_eq!(
        refreshed["refreshed"], true,
        "the refresh must actually run, not fall back to stored material: {refreshed}"
    );

    // Control: the credential the suite refreshed through the gateway is the one
    // the route reconciled — nothing here is scoped to a phantom row.
    let record = fixture
        .gateway
        .execute(
            &fixture.principal(),
            &port_scope(),
            CredentialGatewayCommand::Get {
                credential_id: fixture.credential.to_string(),
            },
        )
        .await
        .expect("the reconciled credential must still be readable");
    assert!(
        matches!(record, CredentialGatewayResult::Record(_)),
        "the credential survives the poison and the reconciliation: {record:?}"
    );
}

// ── The other 409 ────────────────────────────────────────────────────────────

#[tokio::test]
async fn reconcile_refuses_a_credential_that_was_never_poisoned() {
    let fixture = ProbeFixture::new().await;

    let (status, problem) = fixture
        .send(
            &fixture.reconcile_uri(),
            &json!({ "decision": "provider_applied", "evidence": EVIDENCE }),
        )
        .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a credential with no retained claim has nothing to reconcile: {problem}"
    );
    assert_eq!(
        problem_type(&problem),
        RECONCILIATION_NOT_REQUIRED_TYPE,
        "the refusal must be the not-required problem, not the conflict one: a \
         client that could not tell them apart would retry an operator decision \
         that will never apply. {problem}"
    );
}

// ── The third 409: two observations disagree ─────────────────────────────────

/// The conflict refusal end to end — the arm the mapping unit tests cover and
/// the route had no runtime case for.
///
/// It is also the case that shows a changed evidence is not a way out of a
/// refusal: the recorded resolution is keyed on the `(evidence, decision)` pair,
/// so a disagreement is permanent, while repeating the *identical* request is
/// the no-op success the published contract promises.
#[tokio::test]
async fn reconcile_refuses_a_second_observation_that_disagrees() {
    let fixture = ProbeFixture::new().await;
    fixture.poison().await;

    let recorded = json!({ "decision": "provider_applied", "evidence": EVIDENCE });
    let (status, response) = fixture.send(&fixture.reconcile_uri(), &recorded).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the first observation is the operator's remedy: {response}"
    );
    assert_eq!(response["changed"], true, "{response}");

    // Same decision, different evidence: a different `(evidence, decision)`
    // pair, so the resolution already on record disagrees and no retry resolves
    // it. The claim row is gone by now, so this refusal comes from the recorded
    // resolution rather than from the retained poison.
    let disagreeing = json!({
        "decision": "provider_applied",
        "evidence": "second operator note: the support ticket was reassigned",
    });
    let (status, problem) = fixture.send(&fixture.reconcile_uri(), &disagreeing).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a disagreeing evidence must not silently overwrite the recorded \
         resolution: {problem}"
    );
    assert_eq!(
        problem_type(&problem),
        RECONCILIATION_CONFLICT_TYPE,
        "the refusal must be the conflict problem, not the not-required one: a \
         client that could not tell them apart would read a settled credential \
         as one with nothing to reconcile. {problem}"
    );

    // The other arm of the same predicate: the identical request is a success
    // with `changed = false`, so a client that lost the first acknowledgement
    // and can reproduce the exact bytes is not left holding a conflict.
    let (status, recommitted) = fixture.send(&fixture.reconcile_uri(), &recorded).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "repeating the identical request must stay a no-op success: {recommitted}"
    );
    assert_eq!(
        recommitted["changed"], false,
        "a recommit records nothing new and must say so: {recommitted}"
    );
}

// ── The digest is on the wire: the retry identity a lost ack needs ──────────

/// The success response carries the hex digest of the evidence on record.
///
/// The digest is the durable half of the reconciliation retry identity: a
/// client that lost the first acknowledgement can confirm its original
/// evidence matches what is on record, and repeating the exact request then
/// comes back as a `changed = false` no-op rather than a 409.
#[tokio::test]
async fn reconcile_success_reports_the_digest_of_the_evidence_on_record() {
    let fixture = ProbeFixture::new().await;
    fixture.poison().await;

    let body = json!({ "decision": "provider_not_applied", "evidence": EVIDENCE });
    let (status, reconciled) = fixture.send(&fixture.reconcile_uri(), &body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an operator decision must be recordable: {reconciled}"
    );
    let recorded_digest = hex::encode(Sha256::digest(EVIDENCE.as_bytes()));
    assert_eq!(
        reconciled["evidence_digest"], recorded_digest,
        "the success response must carry the hex digest of the evidence just \
         recorded: {reconciled}"
    );

    // The no-op recommit carries the same digest: the identity on record does
    // not change when nothing new is written.
    let (status, repeated) = fixture.send(&fixture.reconcile_uri(), &body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an identical recommit is a no-op success: {repeated}"
    );
    assert_eq!(repeated["changed"], false, "{repeated}");
    assert_eq!(
        repeated["evidence_digest"], recorded_digest,
        "the recommit must report the digest on record, unchanged: {repeated}"
    );
}

/// The conflict problem document carries the recorded pair, so a client can
/// confirm what its evidence disagreed with.
///
/// The `evidence_digest` extension is the digest of the evidence on record —
/// pinning that it *differs* from the digest of the evidence the client just
/// sent is the point of this case. A disagreement is permanent by design: the
/// pair is the identity, so no later submission can overwrite what is on
/// record.
#[tokio::test]
async fn conflict_problem_reports_the_recorded_digest_and_decision() {
    let fixture = ProbeFixture::new().await;
    fixture.poison().await;

    let recorded = json!({ "decision": "provider_applied", "evidence": EVIDENCE });
    let (status, response) = fixture.send(&fixture.reconcile_uri(), &recorded).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the first observation is the operator's remedy: {response}"
    );

    let disagreeing_evidence = "second operator note: the support ticket was reassigned";
    let disagreeing = json!({
        "decision": "provider_applied",
        "evidence": disagreeing_evidence,
    });
    let (status, problem) = fixture.send(&fixture.reconcile_uri(), &disagreeing).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a disagreeing evidence must refuse: {problem}"
    );
    assert_eq!(
        problem_type(&problem),
        RECONCILIATION_CONFLICT_TYPE,
        "the refusal must be the conflict problem: {problem}"
    );

    let recorded_digest = hex::encode(Sha256::digest(EVIDENCE.as_bytes()));
    let request_digest = hex::encode(Sha256::digest(disagreeing_evidence.as_bytes()));
    assert_ne!(
        recorded_digest, request_digest,
        "the two fixture notes must digest differently, or the case would pin nothing"
    );
    assert_eq!(
        problem["evidence_digest"], recorded_digest,
        "the conflict problem must name the digest on record, which differs \
         from the request's own: {problem}"
    );
    assert_eq!(
        problem["recorded_decision"], "provider_applied",
        "the conflict problem must name the recorded decision in its wire \
         spelling: {problem}"
    );
}

// ── The permission this route exists to require ──────────────────────────────

/// One user, two PATs, one workspace role: `credentials:reconcile` is the whole
/// difference, and it is deliberately not `credentials:write`.
///
/// The route records what the provider already did to a credential whose local
/// outcome is unknown. That is a different authority from changing what the
/// credential is, which is why it does not reuse the write path — so a caller
/// holding only `credentials:write` must be refused before the handler runs.
#[tokio::test]
async fn a_pat_holding_only_credentials_write_cannot_reconcile() {
    let fixture = ProbeFixture::new().await;
    fixture.poison().await;

    // The fixture's harness wires no membership store, and the RBAC
    // middleware's no-store test bypass resolves that to `WorkspaceAdmin`
    // (`crates/api/src/middleware/rbac.rs`), so the tenant-role half of
    // `require_permission` is satisfied for both tokens below. That leaves the
    // *auth grant* as the only thing that can refuse, which is exactly the seam
    // a separate `Permission::CredentialReconcile` exists to reach: a PAT
    // carries a permission set, never a workspace role.
    let backend = Arc::new(InMemoryAuthBackend::new());
    let profile = backend
        .register_user(SignupRequest {
            email: "u3c-reconcile-pat@nebula.dev".to_owned(),
            password: ApiSecretString::new("hunter22".to_owned()),
            display_name: "U3C Reconcile PAT".to_owned(),
        })
        .await
        .expect("the PAT holder registers");
    let write_only_pat = backend
        .create_pat(
            &profile.user_id,
            pat_params("u3c-write-only", &["credentials:write"]),
        )
        .await
        .expect("the write-only PAT mints");
    let reconcile_pat = backend
        .create_pat(
            &profile.user_id,
            pat_params("u3c-reconcile", &["credentials:reconcile"]),
        )
        .await
        .expect("the reconcile-scoped PAT mints");

    let backend_dyn: Arc<dyn AuthBackend> = backend;
    let app = app::build_app(
        fixture.state.clone().with_auth_backend(backend_dyn),
        &ApiConfig::for_test(),
    );
    let body = json!({ "decision": "provider_applied", "evidence": EVIDENCE });

    let refused = app
        .clone()
        .oneshot(pat_json(
            &fixture.reconcile_uri(),
            &write_only_pat.plaintext,
            &body,
        ))
        .await
        .expect("the router must answer");
    assert_eq!(
        refused.status(),
        StatusCode::FORBIDDEN,
        "a token that may write the credential must not decide what the provider \
         did with its unknown refresh: {:?}",
        refused.status()
    );

    // A refusal has to leave the state it refused on alone, or the 403 would
    // only mean the caller read the answer after the command had already run.
    assert_matches!(
        fixture.attempt().await,
        ClaimAttempt::OutcomeUnknown { .. },
        "the refused reconcile must leave the poison standing"
    );

    // The control: same user, same workspace role, same credential, same body —
    // only the grant differs. It reaches the handler and clears the poison, so
    // the 403 above is the grant check rather than a fixture that refuses every
    // caller.
    let admitted = app
        .oneshot(pat_json(
            &fixture.reconcile_uri(),
            &reconcile_pat.plaintext,
            &body,
        ))
        .await
        .expect("the router must answer");
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "`credentials:reconcile` is the scope this route names and must be admitted"
    );
    assert_matches!(
        fixture.attempt().await,
        ClaimAttempt::Acquired(_),
        "the admitted reconcile clears the poison"
    );
}

fn pat_params(name: &str, scopes: &[&str]) -> CreatePatParams {
    CreatePatParams {
        name: name.to_owned(),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        ttl_seconds: None,
    }
}
