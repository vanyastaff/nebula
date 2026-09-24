//! Refresh poison recovery across two first-party runtimes sharing file SQLite.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    num::NonZeroU16,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nebula_core::UserId;
use nebula_credential::{
    Acquisition, AuthorizationDecision, CredentialActor, CredentialAuthenticationBinding,
    CredentialAuthorizationError, CredentialCommand, CredentialCommandResult, CredentialController,
    CredentialLifecycleOperation, CredentialLifecycleState, CredentialOperation,
    CredentialTenantAuthority, InteractionRequest, UserInput,
    runtime::{CredentialRefreshSchedulerConfig, RefreshCoordConfig},
};
use nebula_metrics::{
    MetricsRegistry, NEBULA_CREDENTIAL_REFRESH_SCHEDULER_CANDIDATES_TOTAL,
    refresh_scheduler_candidate_outcome,
};
use nebula_storage::credential::{EnvKeyProvider, KeyProvider, SqliteCredentialPersistence};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialRefreshHorizon, CredentialRefreshPageSize,
    CredentialSelector, Scope,
    store::{
        ClaimAttempt, CredentialOperationDecision, CredentialOperationIntent, ExpiredClaim,
        RefreshClaimError, RefreshClaimReclaimer, RefreshClaimStore, RefreshOutcomeDecision,
        ReplicaId, SentinelEscalationPolicy,
    },
};
use serde_json::json;
use tokio::sync::Notify;

use super::{
    CredentialLifecyclePolicy, CredentialRuntime, compose_runtime_with_test_policy,
    refresh_runtime_ports, sqlite_pending_store,
};
use crate::credential_adapters::transport_security_tests::TlsFixture;

const CALLBACK_CODE: &str = "restart-recovery-code";
const REDIRECT_URI: &str = "https://app.example.test/oauth/callback";

#[derive(Debug)]
struct ReclaimGate {
    inner: nebula_storage::credential::SqliteRefreshClaimRepo,
    released: AtomicBool,
    released_notify: Notify,
}

impl ReclaimGate {
    fn new(
        inner: nebula_storage::credential::SqliteRefreshClaimRepo,
        initially_released: bool,
    ) -> Self {
        Self {
            inner,
            released: AtomicBool::new(initially_released),
            released_notify: Notify::new(),
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.released_notify.notify_waiters();
    }

    async fn wait_until_released(&self) {
        while !self.released.load(Ordering::Acquire) {
            let notified = self.released_notify.notified();
            if self.released.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
    }
}

#[async_trait::async_trait]
impl RefreshClaimReclaimer for ReclaimGate {
    async fn reclaim_stuck(
        &self,
        policy: SentinelEscalationPolicy,
    ) -> Result<Vec<ExpiredClaim>, RefreshClaimError> {
        self.wait_until_released().await;
        self.inner.reclaim_stuck(policy).await
    }
}

#[derive(Debug)]
struct FixtureAuthority {
    actor: CredentialActor,
    scope: Scope,
}

#[async_trait::async_trait]
impl CredentialTenantAuthority for FixtureAuthority {
    async fn decide(
        &self,
        actor: &CredentialActor,
        scope: &Scope,
        _operation: CredentialOperation,
    ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
        Ok(if actor == &self.actor && scope == &self.scope {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        })
    }
}

fn controller(
    runtime: &CredentialRuntime,
    actor: &CredentialActor,
    scope: &Scope,
) -> CredentialController {
    CredentialController::new(
        runtime.service(),
        Arc::new(FixtureAuthority {
            actor: actor.clone(),
            scope: scope.clone(),
        }),
        Arc::clone(&runtime.adjudicator),
        Some(Arc::clone(&runtime.audit_sink)),
    )
}

fn binding() -> CredentialAuthenticationBinding {
    CredentialAuthenticationBinding::parse("R".repeat(43)).expect("valid fixture binding")
}

fn oauth_properties(provider: &TlsFixture) -> serde_json::Value {
    json!({"authorization_code": {
        "client": {"client_id": "restart-recovery-client", "client_secret": "fixture-secret"},
        "auth_url": "https://provider.example.test/authorize",
        "token_url": provider.endpoint(),
        "scopes": ["read"],
        "redirect_uri": REDIRECT_URI,
        "auth_style": "post_body"
    }})
}

async fn complete_interaction(
    controller: &CredentialController,
    actor: &CredentialActor,
    scope: &Scope,
    command: CredentialCommand,
) -> nebula_credential::CredentialHead {
    let pending = controller
        .execute(actor, scope, command)
        .await
        .expect("OAuth interaction starts");
    let CredentialCommandResult::Acquisition(Acquisition::Pending {
        token,
        interaction: InteractionRequest::Redirect { url },
    }) = pending
    else {
        panic!("OAuth command must request a redirect");
    };
    let url = url::Url::parse(&url).expect("valid authorization URL");
    let state = url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("OAuth state query parameter");
    let completed = controller
        .execute(
            actor,
            scope,
            CredentialCommand::ContinueResolve {
                credential_key: nebula_core::credential_key!("oauth2"),
                pending_token: token,
                user_input: UserInput::Callback {
                    params: HashMap::from([
                        ("code".to_owned(), CALLBACK_CODE.to_owned()),
                        ("state".to_owned(), state),
                    ]),
                },
                authentication_binding: binding(),
            },
        )
        .await
        .expect("OAuth interaction completes");
    let CredentialCommandResult::Acquisition(Acquisition::Complete { head }) = completed else {
        panic!("OAuth callback must persist a credential");
    };
    head
}

async fn open_runtime(
    database: &str,
    provider: &TlsFixture,
    refresh_config: RefreshCoordConfig,
    hold_reclaim: bool,
) -> (
    CredentialRuntime,
    SqliteCredentialPersistence,
    nebula_storage::credential::SqliteRefreshClaimRepo,
    Arc<MetricsRegistry>,
    Arc<ReclaimGate>,
) {
    let store = SqliteCredentialPersistence::connect(database)
        .await
        .expect("admitted file SQLite credential store");
    let claims = store.refresh_claim_repo();
    let mut ports = refresh_runtime_ports(store.refresh_schedule(), claims.clone());
    let reclaim_gate = Arc::new(ReclaimGate::new(claims.clone(), !hold_reclaim));
    ports.reclaimer = Arc::clone(&reclaim_gate) as Arc<dyn RefreshClaimReclaimer>;
    let key: Arc<dyn KeyProvider> = Arc::new(
        EnvKeyProvider::from_base64(super::DEVELOPMENT_KEY_BASE64).expect("fixed fixture key"),
    );
    let pending = sqlite_pending_store(&store, Arc::clone(&key), Vec::new());
    let transport = Arc::new(provider.transport(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]));
    let scheduler = CredentialRefreshSchedulerConfig {
        cadence: Duration::from_hours(1),
        horizon: CredentialRefreshHorizon::default(),
        page_size: CredentialRefreshPageSize::default(),
        max_pages_per_tick: NonZeroU16::MIN,
        concurrency: NonZeroU16::MIN,
    };
    let metrics = Arc::new(MetricsRegistry::new());
    let runtime = compose_runtime_with_test_policy(
        store.clone(),
        ports,
        pending,
        key,
        Arc::clone(&metrics),
        transport,
        CredentialLifecyclePolicy {
            refresh: refresh_config,
            scheduler,
        },
    )
    .expect("first-party runtime composes");
    (runtime, store, claims, metrics, reclaim_gate)
}

fn scheduler_outcome_count(registry: &MetricsRegistry, outcome: &str) -> u64 {
    registry
        .counter_labeled(
            NEBULA_CREDENTIAL_REFRESH_SCHEDULER_CANDIDATES_TOTAL,
            &registry.interner().single("outcome", outcome),
        )
        .expect("scheduler candidate metric is registered")
        .get()
}

fn restart_policy() -> RefreshCoordConfig {
    RefreshCoordConfig {
        // Provider work gets an integration-scale deadline. The synthetic
        // crashed claim below carries its own short TTL, so restart recovery
        // remains fast without making the real TLS refresh timing-sensitive.
        claim_ttl: Duration::from_secs(3),
        heartbeat_interval: Duration::from_millis(250),
        refresh_timeout: Duration::from_secs(2),
        reclaim_sweep_interval: Duration::from_millis(40),
        sentinel_threshold: 1,
        sentinel_window: Duration::from_mins(1),
    }
}

#[tokio::test]
async fn crashed_refresh_is_reclaimed_without_replay_then_reauthorized_once() {
    let provider = TlsFixture::refreshable_success().await;
    let directory = tempfile::tempdir().expect("temporary database directory");
    let database = directory.path().join("credentials.db");
    let database = database.to_str().expect("UTF-8 fixture path");
    let actor = CredentialActor::user(UserId::new());
    let scope = Scope::new("refresh-restart-workspace", "refresh-restart-organization");
    let owner = CredentialOwner::from_scope(&scope);

    let (mut first, first_store, first_claims, _first_metrics, _first_reclaim_gate) =
        open_runtime(database, &provider, restart_policy(), false).await;
    let first_controller = controller(&first, &actor, &scope);
    let created = complete_interaction(
        &first_controller,
        &actor,
        &scope,
        CredentialCommand::Resolve {
            credential_key: nebula_core::credential_key!("oauth2"),
            properties: oauth_properties(&provider),
            authentication_binding: binding(),
        },
    )
    .await;
    let id = nebula_core::CredentialId::parse(&created.id).expect("persisted credential id");
    let selector = CredentialSelector::new(owner, id);
    let initial_epoch = first_store
        .get_head(&selector)
        .await
        .expect("credential head read")
        .material_epoch();
    let expires_at = created
        .expires_at
        .expect("refreshable OAuth fixture carries an expiry");
    assert!(
        expires_at
            .signed_duration_since(created.updated_at)
            .num_seconds()
            <= 300,
        "the second runtime's immediate scheduler scan must encounter this credential"
    );
    assert_eq!(provider.request_count(), 1);

    // Stop every task owned by replica A before constructing its crash residue.
    // Otherwise A's 40 ms sweeper could account the short fixture claim during
    // a delayed shutdown and make replica B's startup recovery vacuous.
    drop(first_controller);
    first.shutdown().await;
    assert!(first.reclaim_sweep_is_finished());

    let claim = first_claims
        .try_claim(
            &selector,
            &ReplicaId::new("crashed-refresh-owner"),
            Duration::from_millis(60),
            CredentialOperationIntent::Refresh,
        )
        .await
        .expect("refresh claim acquired");
    let ClaimAttempt::Acquired(claim) = claim else {
        panic!("fresh credential must admit the simulated owner");
    };
    first_claims
        .mark_sentinel(&claim.token)
        .await
        .expect("provider boundary is durably marked");

    tokio::time::sleep(Duration::from_millis(90)).await;
    assert!(matches!(
        first_claims
            .try_claim(
                &selector,
                &ReplicaId::new("pre-restart-poison-probe"),
                Duration::from_millis(60),
                CredentialOperationIntent::Refresh,
            )
            .await
            .expect("expired claim remains readable"),
        ClaimAttempt::OutcomeUnknown { .. }
    ));
    let before_restart = first_store
        .get_head(&selector)
        .await
        .expect("credential head remains readable before restart");
    assert!(!before_restart.reauth_required());
    assert_eq!(before_restart.material_epoch(), initial_epoch);
    drop(first);
    drop(first_store);
    drop(first_claims);

    let (mut second, second_store, second_claims, second_metrics, second_reclaim_gate) =
        open_runtime(database, &provider, restart_policy(), true).await;
    let second_controller = controller(&second, &actor, &scope);

    tokio::time::timeout(Duration::from_secs(2), async {
        while scheduler_outcome_count(
            &second_metrics,
            refresh_scheduler_candidate_outcome::OUTCOME_UNKNOWN,
        ) == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("startup scheduler observes the durable outcome-unknown claim");
    assert_eq!(
        provider.request_count(),
        1,
        "scheduler must refuse replay when it encounters durable poison"
    );
    second_reclaim_gate.release();

    let reauth_head = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let result = second_controller
                .execute(&actor, &scope, CredentialCommand::Get { credential_id: id })
                .await
                .expect("credential head remains readable");
            let CredentialCommandResult::Head(head) = result else {
                panic!("get must return a head");
            };
            if head.reauth_required {
                break head;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("startup reclaim reaches the durable reauth transition");
    assert!(reauth_head.reauth_required);
    assert_eq!(
        reauth_head.lifecycle,
        CredentialLifecycleState::ReconciliationRequired {
            operation: Some(CredentialLifecycleOperation::Refresh),
        },
        "durable poison remains the public availability gate until adjudication"
    );
    let escalated = second_store
        .get_head(&selector)
        .await
        .expect("credential head read");
    assert!(escalated.material_epoch() > initial_epoch);
    assert_eq!(
        provider.request_count(),
        1,
        "restart must not replay provider egress"
    );

    let reconciled = second_controller
        .execute(
            &actor,
            &scope,
            CredentialCommand::Reconcile {
                credential_id: id,
                decision: CredentialOperationDecision::Refresh(
                    RefreshOutcomeDecision::ProviderNotApplied,
                ),
                evidence: "provider audit confirms no refresh was applied".to_owned(),
            },
        )
        .await
        .expect("authorized adjudication clears the poisoned claim");
    let CredentialCommandResult::Reconciled(adjudication) = reconciled else {
        panic!("reconcile must return its durable adjudication");
    };
    assert!(adjudication.changed);
    let after_adjudication = second_controller
        .execute(&actor, &scope, CredentialCommand::Get { credential_id: id })
        .await
        .expect("credential head remains readable after adjudication");
    let CredentialCommandResult::Head(after_adjudication) = after_adjudication else {
        panic!("get must return a head");
    };
    assert!(after_adjudication.reauth_required);
    assert_eq!(
        after_adjudication.lifecycle,
        CredentialLifecycleState::ReauthRequired,
        "clearing the incident must expose the still-durable reauthorization requirement"
    );
    let probe = second_claims
        .try_claim(
            &selector,
            &ReplicaId::new("post-adjudication-probe"),
            Duration::from_millis(60),
            CredentialOperationIntent::Refresh,
        )
        .await
        .expect("claim store remains available");
    let ClaimAttempt::Acquired(probe) = probe else {
        panic!("adjudication must clear durable poison");
    };
    second_claims
        .release(probe.token)
        .await
        .expect("probe claim releases before recovery");

    let restored = complete_interaction(
        &second_controller,
        &actor,
        &scope,
        CredentialCommand::Reauthorize {
            credential_id: id,
            properties: oauth_properties(&provider),
            authentication_binding: binding(),
        },
    )
    .await;
    assert_eq!(restored.id, id.to_string());
    assert_eq!(restored.lifecycle, CredentialLifecycleState::Ready);
    assert_eq!(provider.request_count(), 2);

    let refreshed = second_controller
        .execute(
            &actor,
            &scope,
            CredentialCommand::Refresh { credential_id: id },
        )
        .await
        .expect("restored credential refreshes");
    assert!(matches!(refreshed, CredentialCommandResult::Refreshed(_)));
    assert_eq!(
        provider.request_count(),
        3,
        "one refresh issues one provider request"
    );

    drop(second_controller);
    second.shutdown().await;
    assert!(second.reclaim_sweep_is_finished());
}
