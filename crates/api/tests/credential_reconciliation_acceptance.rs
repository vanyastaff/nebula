//! U3b acceptance: the reconciliation command clears a poisoned refresh claim.
//!
//! Two sequences run over the real SQLite refresh-claim adapter, the real
//! `CredentialController`, and the API-owned gateway:
//!
//! 1. poison a claim, send the management command carrying the observed
//!    evidence, and the claim becomes acquirable again;
//! 2. poison a claim and send nothing, and it stays denied.
//!
//! The second sequence is the control that makes the first mean something: a
//! poisoned claim never expires on its own — `try_claim` answers
//! `OutcomeUnknown` for as long as the poison row stands — so a test that only
//! asserted the post-command success could not distinguish "the command cleared
//! it" from "it was going to clear anyway".
//!
//! # What this test cannot observe
//!
//! `apps/server/src/lib.rs` declares `mod credential_runtime;` private, so the
//! server's three mappings (the command translation, the result projection, and
//! the adjudication-error mapping) are unreachable here. This file reaches the
//! **testkit's** result wildcard, which is the same shape but not the same code
//! path; those three sites are covered by `#[cfg(test)]` tests inside
//! `credential_runtime.rs` instead. The HTTP status mapping is U3c.
//!
//! # Construction
//!
//! The refresh-claim store and the credential service must share one database,
//! because the test poisons a claim the controller later adjudicates. So the
//! store is opened on a *file* in a temporary directory
//! (`sqlite://<dir>/claims.sqlite?mode=rwc`), the same form
//! `crates/storage/tests/credential_lifecycle_sqlite.rs` uses, rather than on
//! the exact `sqlite::memory:` form — that one is capped at one physical
//! connection and would give a second pool a separate database.
//! `SqliteCredentialPersistence::connect` and `refresh_claim_repo()` are both
//! public and ungated, so this uses the supported composition seam and no
//! `test-util` route into the credential design. The file, and the
//! `<path>-setup-lock` sidecar `connect` leaves beside it, live in the
//! temporary directory and are removed with it.

use std::{
    assert_matches,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{Duration as ChronoDuration, Utc};
use nebula_api::{
    domain::credential::dto::CreateCredentialRequest,
    middleware::auth::AuthenticatedPrincipal,
    ports::{
        credential_command::{
            CredentialCommandGateway, CredentialGatewayCommand, CredentialGatewayError,
            CredentialGatewayResult, test_gateway_from_service_with_reconciliation,
        },
        credential_service_factory::with_store,
    },
};
use nebula_core::{CredentialId, UserId};
use nebula_credential::{AuditEvent, AuditOperation, AuditSink, CredentialService};
use nebula_storage::credential::{EnvKeyProvider, SqliteCredentialPersistence};
use nebula_storage_port::{
    CredentialPersistenceError, Scope,
    store::{
        ClaimAttempt, RefreshClaimAdjudicator, RefreshClaimStore, RefreshOutcomeDecision, ReplicaId,
    },
};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// 32 `0x42` bytes, base64 — the factory's fixed AES-256 test key. Not a
/// secret: a published test constant.
const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

/// The operator's note. A provider support ticket is exactly the kind of
/// evidence this parameter exists for.
const EVIDENCE: &str = "provider support ticket 4417: the call never reached the API";

/// Claim lease. Long enough that only the backdated row is expired.
const CLAIM_TTL: Duration = Duration::from_mins(10);

/// The first-party credential type the fixture creates. Registered by the
/// factory's default registry, and non-interactive: creating one reaches no
/// provider.
const API_KEY_TYPE: &str = "api_key";

/// The `api_key` credential's only property. Not a secret: a fixture value
/// that never leaves the temporary database.
const TEST_API_KEY: &str = "acceptance-api-key-value";

/// A composed gateway whose claim storage the test can also poison directly.
struct ReconciliationFixture {
    gateway: Arc<dyn CredentialCommandGateway>,
    claim_store: Arc<dyn RefreshClaimStore>,
    audit: Arc<RecordingAuditSink>,
    pool: SqlitePool,
    /// How many credentials this fixture has created.
    ///
    /// A live credential owns its display name exclusively within its
    /// workspace, so the fixture derives each name from this counter rather
    /// than repeating one — the collision would otherwise surface as
    /// `NameAlreadyExists` in whichever case creates two credentials, which is
    /// a fact about naming and not about the command under test.
    created_count: AtomicUsize,
    /// Owns the directory holding the database file and its sidecar.
    ///
    /// `SqliteCredentialPersistence::connect` serializes file-backed schema
    /// setup on a `<path>-setup-lock` sidecar beside the database; keeping the
    /// database in a temporary directory keeps that by-product out of the
    /// working tree and removes it with the file. Last field, so it outlives
    /// every pool that carries the path.
    _database_directory: tempfile::TempDir,
}

impl ReconciliationFixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory for the database");
        let path = directory.path().join("claims.sqlite");
        // One file, two pools: the fixture's observation pool reads and
        // backdates the very rows the service's own pool writes.
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let store = SqliteCredentialPersistence::connect(&url)
            .await
            .expect("the file-backed credential store opens and migrates");
        // One adapter, two trait objects. `refresh_claim_repo()` clones the
        // store's admitted pool, so the adjudicator reads the very rows the
        // credential path writes.
        let repo = Arc::new(store.refresh_claim_repo());
        let claim_store: Arc<dyn RefreshClaimStore> = repo.clone();
        let adjudicator: Arc<dyn RefreshClaimAdjudicator> = repo;

        let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte key"));
        let service: Arc<CredentialService> =
            with_store(store, key).expect("the credential service composes");

        let audit = Arc::new(RecordingAuditSink::default());
        let gateway = test_gateway_from_service_with_reconciliation(
            service,
            adjudicator,
            Some(Arc::clone(&audit) as Arc<dyn AuditSink>),
        );

        let options = SqliteConnectOptions::from_str(&url)
            .expect("the credential database URL parses")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("the fixture pool joins the credential database");

        Self {
            gateway,
            claim_store,
            audit,
            pool,
            created_count: AtomicUsize::new(0),
            _database_directory: directory,
        }
    }

    fn holder(&self) -> ReplicaId {
        ReplicaId::new("acceptance-holder")
    }

    fn principal(&self) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal::for_test_user(UserId::new().to_string())
    }

    fn scope(&self) -> Scope {
        Scope::new("acceptance-workspace", "acceptance-org")
    }

    /// A second workspace in the *same* org.
    ///
    /// The reconcile command's ownership read is workspace-scoped, so a
    /// sibling workspace is what a refusal has to be measured against: an org
    /// boundary would refuse for a reason this test cannot distinguish.
    fn other_scope(&self) -> Scope {
        Scope::new("other-workspace", "acceptance-org")
    }

    /// Create one credential owned by `scope` and return its id.
    ///
    /// Every case needs a credential to reconcile: the claim row carries no
    /// owner of its own, so the command's ownership read is the only thing that
    /// can attribute a claim to a tenant. Creation is also how the id is
    /// minted — the service generates it — so a case cannot name one in
    /// advance.
    async fn create(&self, scope: &Scope) -> CredentialId {
        let created = self
            .gateway
            .execute(
                &self.principal(),
                scope,
                CredentialGatewayCommand::Create(CreateCredentialRequest {
                    credential_key: API_KEY_TYPE.to_owned(),
                    name: format!(
                        "reconciliation acceptance credential {}",
                        self.created_count.fetch_add(1, Ordering::Relaxed)
                    ),
                    description: None,
                    data: serde_json::json!({ "api_key": TEST_API_KEY }),
                    tags: None,
                }),
            )
            .await
            .expect("the first-party api_key type must be creatable");
        let CredentialGatewayResult::Record(record) = created else {
            panic!("create must answer with the created record, got {created:?}");
        };
        CredentialId::parse(&record.id).expect("the gateway returns a parseable credential id")
    }

    /// Take `credential` across the provider boundary and past its lease.
    ///
    /// `credential` must already exist under its owning scope ([`Self::create`]):
    /// a claim row carries no owner of its own, so the command's ownership read
    /// is the only thing that attributes this poison to a tenant.
    ///
    /// This is the poisoned state `try_claim` answers `OutcomeUnknown` with:
    /// provider egress began, the outcome is unknown, and the lease deadline
    /// has passed. The backdate is the fixture's only raw SQL — the expiry
    /// predicate is SQLite's own clock, so there is no port-level way to reach
    /// this state and the adapter's seam deliberately exposes none.
    async fn poison(&self, credential: &CredentialId) {
        let acquired = self
            .claim_store
            .try_claim(credential, &self.holder(), CLAIM_TTL)
            .await
            .expect("a free claim is acquirable");
        let ClaimAttempt::Acquired(claim) = acquired else {
            panic!("a free claim must be acquirable, got {acquired:?}");
        };
        self.claim_store
            .mark_sentinel(&claim.token)
            .await
            .expect("the holder may mark provider egress");
        let expired_at = (Utc::now() - ChronoDuration::seconds(1)).timestamp_millis();
        sqlx::query(
            "UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2",
        )
        .bind(expired_at)
        .bind(credential.to_string())
        .execute(&self.pool)
        .await
        .expect("backdating the claim row must not fail");
    }

    /// Ask the port for `credential`'s claim.
    ///
    /// `try_claim` is the observation rather than a proxy for it: `OutcomeUnknown`
    /// is the poisoned answer, and `Acquired` means the claim was free. So a
    /// probe reporting "no longer poisoned" has also *taken* the claim, and no
    /// test may probe and then acquire the same credential again.
    async fn attempt(&self, credential: &CredentialId) -> ClaimAttempt {
        self.claim_store
            .try_claim(credential, &self.holder(), CLAIM_TTL)
            .await
            .expect("acquisition must not fail")
    }

    /// How many of `credential`'s incidents carry a recorded resolution.
    ///
    /// The adjudication port has no read of what it recorded — it answers with a
    /// verdict and nothing else — so this queries `credential_sentinel_events`
    /// directly. `adjudicated_at` is the column the resolution write stamps, and
    /// reading it is what tells a *recorded resolution* from an incident that
    /// merely exists: `count_sentinel_events_in_window` counts incidents over a
    /// window and is resolution-blind by design, so it cannot stand in for this.
    async fn recorded_resolution_count(&self, credential: &CredentialId) -> i64 {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE credential_id = ?1 AND adjudicated_at IS NOT NULL",
        )
        .bind(credential.to_string())
        .fetch_one(&self.pool)
        .await
        .expect("counting recorded resolutions must not fail");
        count
    }

    /// Send the management command for `credential` as its owning scope.
    async fn reconcile(
        &self,
        credential: &CredentialId,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<CredentialGatewayResult, CredentialGatewayError> {
        self.reconcile_in_scope(&self.scope(), credential, decision, evidence)
            .await
    }

    /// Send the management command for `credential` as `scope`.
    ///
    /// The scope is a parameter because the command's tenant binding is exactly
    /// what a case has to vary: a caller whose scope does not own `credential`
    /// must be refused, and one fixed scope cannot express that.
    async fn reconcile_in_scope(
        &self,
        scope: &Scope,
        credential: &CredentialId,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<CredentialGatewayResult, CredentialGatewayError> {
        self.gateway
            .execute(
                &self.principal(),
                scope,
                CredentialGatewayCommand::Reconcile {
                    credential_id: credential.to_string(),
                    decision,
                    evidence: evidence.to_owned(),
                },
            )
            .await
    }
}

/// Records every audit event so the emitter can be asserted.
#[derive(Debug, Default)]
struct RecordingAuditSink {
    events: std::sync::Mutex<Vec<AuditOperation>>,
}

impl AuditSink for RecordingAuditSink {
    fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
        self.events
            .lock()
            .expect("acceptance audit lock")
            .push(event.operation.clone());
        Ok(())
    }
}

#[tokio::test]
async fn reconciliation_clears_the_poison_and_the_claim_becomes_acquirable() {
    let fixture = ReconciliationFixture::new().await;
    let credential = fixture.create(&fixture.scope()).await;
    fixture.poison(&credential).await;
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "the fixture must establish the poison before the command is sent"
    );
    assert_eq!(
        fixture.recorded_resolution_count(&credential).await,
        0,
        "a poison is not a resolution: nothing may be on record before the command"
    );

    let result = fixture
        .reconcile(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            EVIDENCE,
        )
        .await
        .expect("a poisoned claim is adjudicable");

    assert_matches!(
        result,
        CredentialGatewayResult::Reconciled {
            decision: RefreshOutcomeDecision::ProviderApplied,
            changed: true,
        }
    );

    // The resolution itself, as durable state rather than as the command's own
    // return value: the claim row being clear says the poison is gone, and this
    // says a decision was recorded for it.
    assert_eq!(
        fixture.recorded_resolution_count(&credential).await,
        1,
        "the reconcile must leave exactly one recorded resolution on the incident"
    );

    // The point of the command: the claim is acquirable again.
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::Acquired(_),
        "a reconciled claim must stop answering OutcomeUnknown and become acquirable"
    );

    // The non-authoritative observation reached the injected sink.
    assert_eq!(
        fixture
            .audit
            .events
            .lock()
            .expect("acceptance audit lock")
            .as_slice(),
        &[AuditOperation::Reconcile]
    );
}

#[tokio::test]
async fn repeating_the_same_reconciliation_is_a_no_op_success() {
    // Superseded-replay semantics: the identical `(evidence digest, decision)`
    // pair is already on record, so the honest answer is "already recorded",
    // not a conflict. Pinning it here keeps a later reader from "fixing" the
    // no-poison branch into a refusal and calling it stricter.
    let fixture = ReconciliationFixture::new().await;
    let credential = fixture.create(&fixture.scope()).await;
    fixture.poison(&credential).await;

    let first = fixture
        .reconcile(
            &credential,
            RefreshOutcomeDecision::ProviderNotApplied,
            EVIDENCE,
        )
        .await
        .expect("the first adjudication is recorded");
    assert_matches!(
        first,
        CredentialGatewayResult::Reconciled { changed: true, .. }
    );

    let replay = fixture
        .reconcile(
            &credential,
            RefreshOutcomeDecision::ProviderNotApplied,
            EVIDENCE,
        )
        .await
        .expect("an identical replay is a success");
    assert_matches!(
        replay,
        CredentialGatewayResult::Reconciled {
            decision: RefreshOutcomeDecision::ProviderNotApplied,
            changed: false,
        }
    );

    // Two commands, two observations, one state.
    assert_eq!(
        fixture
            .audit
            .events
            .lock()
            .expect("acceptance audit lock")
            .len(),
        2
    );
}

#[tokio::test]
async fn a_poisoned_claim_stays_denied_without_reconciliation() {
    let fixture = ReconciliationFixture::new().await;
    let credential = fixture.create(&fixture.scope()).await;
    fixture.poison(&credential).await;
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "the fixture must establish the poison"
    );

    // No command. The control: the identical setup, minus the reconciliation,
    // leaves the claim denied. Nothing about the fixture is time-dependent, so
    // this cannot pass merely because the poison lapsed.
    assert!(
        fixture
            .audit
            .events
            .lock()
            .expect("acceptance audit lock")
            .is_empty(),
        "no command was sent, so no audit observation may exist"
    );
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "a poisoned claim with no reconciliation stays denied"
    );

    // A second credential in the same store, reconciled, proves the outcome
    // tracks the command and not the store: one claim cleared, one still held.
    let reconciled = fixture.create(&fixture.scope()).await;
    fixture.poison(&reconciled).await;
    fixture
        .reconcile(
            &reconciled,
            RefreshOutcomeDecision::ProviderApplied,
            EVIDENCE,
        )
        .await
        .expect("the sibling claim is adjudicable");

    assert_matches!(
        fixture.attempt(&reconciled).await,
        ClaimAttempt::Acquired(_),
        "the reconciled sibling is cleared"
    );
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "the unreconciled claim is still denied after a sibling was cleared"
    );
}

#[tokio::test]
async fn refusing_evidence_leaves_the_poison_standing() {
    // `InvalidEvidence` — empty here, over the byte bound in the port's own
    // suite. A refused adjudication must not clear the claim, and it must reach
    // the caller as its own error rather than as a generic failure.
    let fixture = ReconciliationFixture::new().await;
    let credential = fixture.create(&fixture.scope()).await;
    fixture.poison(&credential).await;

    let error = fixture
        .reconcile(&credential, RefreshOutcomeDecision::ProviderApplied, "")
        .await
        .expect_err("empty evidence is refused");

    assert_eq!(
        error,
        CredentialGatewayError::ReconciliationEvidenceInvalid,
        "empty evidence must keep its own gateway error"
    );
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "a refused adjudication must leave the poison standing"
    );
}

#[tokio::test]
async fn reconciling_a_healthy_credential_reports_nothing_to_reconcile() {
    let fixture = ReconciliationFixture::new().await;
    // A live credential that holds no claim at all: creation succeeds, so the
    // command's ownership read succeeds and what follows is the adjudicator's
    // refusal rather than a missing credential.
    let credential = fixture.create(&fixture.scope()).await;

    let error = fixture
        .reconcile(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            EVIDENCE,
        )
        .await
        .expect_err("an unpoisoned credential has nothing to reconcile");

    assert_eq!(
        error,
        CredentialGatewayError::ReconciliationNotRequired,
        "an unpoisoned credential must not read as an internal failure"
    );
}

#[tokio::test]
async fn another_scopes_credential_cannot_be_reconciled_and_keeps_its_poison() {
    // The command's ownership read is the whole gate: the adjudication port
    // takes no scope operand, so this arm is the one place the caller's tenant
    // meets the credential. Both scopes share one org, so what a refusal proves
    // here is workspace ownership rather than an org boundary.
    let fixture = ReconciliationFixture::new().await;
    let credential = fixture.create(&fixture.scope()).await;
    fixture.poison(&credential).await;
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "the fixture must establish the poison before the command is sent"
    );

    let refusal = fixture
        .reconcile_in_scope(
            &fixture.other_scope(),
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            EVIDENCE,
        )
        .await
        .expect_err("another workspace must not adjudicate this credential");

    assert_eq!(
        refusal,
        CredentialGatewayError::NotFound,
        "the credential is absent from the other workspace's partition, so the \
         refusal leaks no cross-tenant existence"
    );
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::OutcomeUnknown { .. },
        "a refused cross-workspace reconcile must leave the poison standing"
    );
    assert!(
        fixture
            .audit
            .events
            .lock()
            .expect("acceptance audit lock")
            .is_empty(),
        "a refused reconcile must not emit the success observation"
    );

    // The control: the owning workspace still adjudicates the same credential,
    // so the refusal above is about the caller's scope and not about the
    // credential being unadjudicable.
    let recorded = fixture
        .reconcile(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            EVIDENCE,
        )
        .await
        .expect("the owning workspace may adjudicate its own credential");
    assert_matches!(
        recorded,
        CredentialGatewayResult::Reconciled {
            decision: RefreshOutcomeDecision::ProviderApplied,
            changed: true,
        }
    );
    assert_matches!(
        fixture.attempt(&credential).await,
        ClaimAttempt::Acquired(_),
        "the owning workspace's reconcile clears the poison"
    );
}
