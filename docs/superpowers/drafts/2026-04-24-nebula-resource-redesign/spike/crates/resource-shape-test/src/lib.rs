//! Phase 4 spike — Resource v2 ergonomic + dispatch validation.
//!
//! Mock `Resource` impls cover:
//!
//! | Mock | Topology | Credential |
//! |------|----------|------------|
//! | [`MockKvStore`] | (none — bare Resource) | `NoCredential` |
//! | [`MockHttpClient`] | `Resident` | `NoCredential` |
//! | [`MockPostgresPool`] | `Pooled` | `SecretToken`-backed credential |
//! | [`MockKafkaTransport`] | `Transport` | `SecretToken`-backed credential |
//!
//! Two integration tests at the bottom drive the `Manager` dispatcher:
//!
//! - `parallel_dispatch_isolates_per_resource_latency` — three resources sharing a credential, one
//!   sleeping 5s, the other two completing in <100ms. Verifies wall-clock ≈ slow-resource budget
//!   (timeout, not slow-resource latency) and that siblings finish first.
//! - `parallel_dispatch_isolates_per_resource_errors` — three resources, one returning Err, the
//!   other two Ok. Verifies sibling Ok outcomes are not poisoned by the Err.
//!
//! See `Cargo.toml`'s comment in the spike root and `NOTES.md` for
//! cascade context.
#![forbid(unsafe_code)]

pub mod compile_fail;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadata,
    CredentialState, NoPendingState, ResolveResult, SecretString, credential_key,
    scheme::SecretToken,
};
use nebula_schema::HasSchema;
use resource_shape::{
    Exclusive, NoCredential, Pooled, Resident, Resource, ResourceContext, ResourceKey, Service,
    Transport,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Shared mock error type ────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MockError {
    #[error("intentional refresh failure: {0}")]
    Refresh(&'static str),
    #[error("intentional revoke failure: {0}")]
    Revoke(&'static str),
    #[error("resource creation failed: {0}")]
    Create(&'static str),
}

// ── Shared mock credential (for credential-bearing resources) ─────────

/// A real-shaped `Credential` impl that projects a `SecretToken`.
///
/// We use this instead of `nebula_credential::credentials::ApiKeyCredential`
/// because the spike does not need to depend on `nebula-schema` derives.
/// Spike only needs SOMETHING that satisfies the `Credential` bound and
/// uses a real production scheme (`SecretToken`); the test harness
/// fabricates the scheme directly when driving rotation.
pub struct StaticTokenCredential;

#[derive(Clone, Serialize, Deserialize)]
pub struct StaticTokenState {
    pub token: SecretToken,
}

impl CredentialState for StaticTokenState {
    const KIND: &'static str = "static_token_state";
    const VERSION: u32 = 1;
}

impl Credential for StaticTokenCredential {
    type Input = ();
    type Scheme = SecretToken;
    type State = StaticTokenState;
    type Pending = NoPendingState;

    const KEY: &'static str = "static_token_credential";

    fn metadata() -> CredentialMetadata
    where
        Self: Sized,
    {
        CredentialMetadata::builder()
            .key(credential_key!("static_token_credential"))
            .name("Static token (test mock)")
            .description("Spike-only credential producing a SecretToken.")
            .schema(<() as HasSchema>::schema())
            .pattern(AuthPattern::SecretToken)
            .build()
            .expect("static metadata complete")
    }

    fn project(state: &StaticTokenState) -> SecretToken {
        state.token.clone()
    }

    async fn resolve(
        _values: &nebula_schema::FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<StaticTokenState, NoPendingState>, CredentialError> {
        Ok(ResolveResult::Complete(StaticTokenState {
            token: SecretToken::new(SecretString::new("spike-default-token")),
        }))
    }
}

// ── Mock 1 — `MockKvStore`: bare Resource, NoCredential ───────────────

/// A bare in-process key/value store with no authentication.
///
/// Demonstrates `type Credential = NoCredential;` on a Resource that has
/// no topology sub-trait — i.e. it's used directly via `Resource::create`
/// without a Pool/Resident/etc. wrapper. (Real `nebula-resource` doesn't
/// allow this — every resource picks a topology — but the spike covers
/// it to confirm the bare `Resource` shape compiles standalone.)
pub struct MockKvStore;

#[derive(Clone, Default)]
pub struct MockKvConfig;

#[derive(Clone, Default)]
pub struct MockKvRuntime;

impl Resource for MockKvStore {
    type Config = MockKvConfig;
    type Runtime = MockKvRuntime;
    type Lease = MockKvRuntime;
    type Error = MockError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        ResourceKey("mock.kv_store")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<NoCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(MockKvRuntime)
    }

    // Note: no `on_credential_refresh` override. Default no-op accepted.
    // This is exit-criterion #1: a NoCredential resource with no override
    // must compile cleanly.
}

// ── Mock 2 — `MockHttpClient`: Resident, NoCredential ─────────────────

/// A reqwest-shaped mock — Resident topology, no authentication.
///
/// Demonstrates that opting out of credentials does NOT prevent a
/// resource from picking a topology sub-trait.
#[derive(Clone)]
pub struct MockHttpClient;

#[derive(Clone, Default)]
pub struct MockHttpConfig;

#[derive(Clone, Default)]
pub struct MockHttpRuntime;

impl Resource for MockHttpClient {
    type Config = MockHttpConfig;
    type Runtime = MockHttpRuntime;
    type Lease = MockHttpRuntime;
    type Error = MockError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        ResourceKey("mock.http_client")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<NoCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(MockHttpRuntime)
    }
}

impl Resident for MockHttpClient {}

// ── Mock 3 — `MockPostgresPool`: Pooled, credential-bearing ───────────

/// The headline mock — a Pooled resource whose `Credential` projects a
/// `SecretToken`. `on_credential_refresh` is the blue-green swap target
/// from Tech Spec §3.6.
///
/// Internals: we don't actually build a Postgres pool. We simulate the
/// blue-green swap with an `Arc<RwLock<RefreshCounter>>` and bump the
/// counter inside `on_credential_refresh`. Tests assert the bump, that's
/// what proves the hook runs end-to-end.
pub struct MockPostgresPool {
    pub refresh_count: Arc<AtomicUsize>,
    /// If set, `on_credential_refresh` sleeps this long. Used to drive
    /// the per-resource isolation test.
    pub refresh_sleep: Option<Duration>,
    /// If set, `on_credential_refresh` returns Err. Used to drive
    /// per-resource error isolation.
    pub fail_refresh: bool,
}

impl MockPostgresPool {
    pub fn fast() -> Self {
        Self {
            refresh_count: Arc::new(AtomicUsize::new(0)),
            refresh_sleep: None,
            fail_refresh: false,
        }
    }

    pub fn slow(duration: Duration) -> Self {
        Self {
            refresh_count: Arc::new(AtomicUsize::new(0)),
            refresh_sleep: Some(duration),
            fail_refresh: false,
        }
    }

    pub fn failing() -> Self {
        Self {
            refresh_count: Arc::new(AtomicUsize::new(0)),
            refresh_sleep: None,
            fail_refresh: true,
        }
    }

    pub fn refresh_count(&self) -> usize {
        self.refresh_count.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Default)]
pub struct MockPgConfig;

#[derive(Clone, Default)]
pub struct MockPgPool;

impl Resource for MockPostgresPool {
    type Config = MockPgConfig;
    type Runtime = MockPgPool;
    type Lease = MockPgPool;
    type Error = MockError;
    type Credential = StaticTokenCredential;

    fn key() -> ResourceKey {
        ResourceKey("mock.postgres_pool")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        scheme: &<StaticTokenCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        // Production would feed `scheme.token()` into the Postgres
        // connect string. Spike just borrows it to demonstrate the API.
        let _ = scheme.token();
        Ok(MockPgPool)
    }

    async fn on_credential_refresh(
        &self,
        _new_scheme: &<StaticTokenCredential as Credential>::Scheme,
    ) -> Result<(), Self::Error> {
        if let Some(d) = self.refresh_sleep {
            tokio::time::sleep(d).await;
        }
        if self.fail_refresh {
            return Err(MockError::Refresh("intentional"));
        }
        self.refresh_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Pooled for MockPostgresPool {}

// ── Mock 4 — `MockKafkaTransport`: Transport, credential-bearing ──────

/// Demonstrates that Transport composes with the credential reshape.
pub struct MockKafkaTransport {
    pub refresh_count: Arc<AtomicUsize>,
}

impl MockKafkaTransport {
    pub fn new() -> Self {
        Self {
            refresh_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Default for MockKafkaTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Default)]
pub struct MockKafkaConfig;

#[derive(Clone, Default)]
pub struct MockKafkaTransportRuntime;

#[derive(Clone, Default)]
pub struct MockKafkaSession;

impl Resource for MockKafkaTransport {
    type Config = MockKafkaConfig;
    type Runtime = MockKafkaTransportRuntime;
    type Lease = MockKafkaSession;
    type Error = MockError;
    type Credential = StaticTokenCredential;

    fn key() -> ResourceKey {
        ResourceKey("mock.kafka_transport")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<StaticTokenCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(MockKafkaTransportRuntime)
    }

    async fn on_credential_refresh(
        &self,
        _new_scheme: &<StaticTokenCredential as Credential>::Scheme,
    ) -> Result<(), Self::Error> {
        self.refresh_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Transport for MockKafkaTransport {
    async fn open_session(
        &self,
        _transport: &Self::Runtime,
        _ctx: &ResourceContext,
    ) -> Result<Self::Lease, Self::Error> {
        Ok(MockKafkaSession)
    }
}

// ── Compile-only proofs of additional topology shapes ─────────────────
//
// These exist so the spike covers Service + Exclusive even though no
// integration test exercises them. They prove `type Credential` /
// `on_credential_refresh` compose cleanly with each topology sub-trait.

#[allow(dead_code)]
pub struct MockServiceResource;

#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct ServiceToken;

impl Resource for MockServiceResource {
    type Config = ();
    type Runtime = ();
    type Lease = ServiceToken;
    type Error = MockError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        ResourceKey("mock.service")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<NoCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(())
    }
}

impl Service for MockServiceResource {
    async fn acquire_token(
        &self,
        _runtime: &Self::Runtime,
        _ctx: &ResourceContext,
    ) -> Result<Self::Lease, Self::Error> {
        Ok(ServiceToken)
    }
}

#[allow(dead_code)]
pub struct MockExclusiveResource;

impl Resource for MockExclusiveResource {
    type Config = ();
    type Runtime = ();
    type Lease = ();
    type Error = MockError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        ResourceKey("mock.exclusive")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<NoCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(())
    }
}

impl Exclusive for MockExclusiveResource {}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use nebula_credential::{CredentialId, SecretString, scheme::SecretToken};
    use resource_shape::{DispatchOutcome, Manager};

    use super::*;

    fn fresh_scheme() -> SecretToken {
        SecretToken::new(SecretString::new("rotated-token"))
    }

    /// Reverse-index write path is populated for credential-bearing
    /// resources and skipped for `NoCredential` resources.
    #[tokio::test]
    async fn register_populates_reverse_index_for_credential_bearing_only() {
        let manager = Manager::new();

        // KvStore is NoCredential — must not be inserted.
        manager
            .register::<MockKvStore>(Arc::new(MockKvStore), None)
            .await
            .unwrap();

        // HttpClient is NoCredential — must not be inserted.
        manager
            .register::<MockHttpClient>(Arc::new(MockHttpClient), None)
            .await
            .unwrap();

        // PostgresPool is credential-bearing — inserts.
        let cred_id = CredentialId::new();
        let pg = Arc::new(MockPostgresPool::fast());
        manager
            .register::<MockPostgresPool>(Arc::clone(&pg), Some(cred_id))
            .await
            .unwrap();

        assert_eq!(manager.dispatcher_count(&cred_id).await, 1);

        // Different credential id — different bucket.
        let other_id = CredentialId::new();
        assert_eq!(manager.dispatcher_count(&other_id).await, 0);
    }

    /// Three resources share one credential. One is deliberately slow
    /// (3s sleep, well above the 250ms per-resource budget). Whole-batch
    /// wall-clock must reflect the budget, not the slow sleep — this is
    /// the per-resource isolation invariant from Strategy §4.3.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_dispatch_isolates_per_resource_latency() {
        let manager = Manager::with_timeout(Duration::from_millis(250));

        let cred_id = CredentialId::new();

        let fast_a = Arc::new(MockPostgresPool::fast());
        let fast_b = Arc::new(MockPostgresPool::fast());
        let slow = Arc::new(MockPostgresPool::slow(Duration::from_secs(3)));

        manager
            .register::<MockPostgresPool>(Arc::clone(&fast_a), Some(cred_id))
            .await
            .unwrap();
        manager
            .register::<MockPostgresPool>(Arc::clone(&fast_b), Some(cred_id))
            .await
            .unwrap();
        manager
            .register::<MockPostgresPool>(Arc::clone(&slow), Some(cred_id))
            .await
            .unwrap();

        let scheme = fresh_scheme();
        let started = Instant::now();
        let outcome = manager.on_credential_refreshed(&cred_id, &scheme).await;
        let elapsed = started.elapsed();

        // Two fast acks + one timeout. Order matches insertion order.
        assert_eq!(outcome.per_resource.len(), 3);
        assert!(matches!(outcome.per_resource[0], DispatchOutcome::Ok));
        assert!(matches!(outcome.per_resource[1], DispatchOutcome::Ok));
        assert!(matches!(outcome.per_resource[2], DispatchOutcome::TimedOut));

        // Wall-clock should be near the per-resource budget (250ms),
        // NOT the slow resource's 3s sleep. Give generous headroom for
        // CI scheduling noise but well below the slow sleep.
        assert!(
            elapsed < Duration::from_millis(1500),
            "expected isolation; whole batch took {elapsed:?}",
        );

        // Fast resources must have actually run (counter bumped).
        assert_eq!(fast_a.refresh_count(), 1);
        assert_eq!(fast_b.refresh_count(), 1);
        // Slow resource never reached its bump (we cancelled the future
        // via timeout before sleep finished).
        assert_eq!(slow.refresh_count(), 0);
    }

    /// One resource returns Err; siblings still get Ok. This is the
    /// security amendment B-1 invariant from
    /// `phase-2-security-lead-review.md:60-66` — a single misbehaving
    /// resource cannot starve siblings of the new scheme.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_dispatch_isolates_per_resource_errors() {
        let manager = Manager::with_timeout(Duration::from_secs(1));

        let cred_id = CredentialId::new();

        let ok_one = Arc::new(MockPostgresPool::fast());
        let ok_two = Arc::new(MockPostgresPool::fast());
        let failing = Arc::new(MockPostgresPool::failing());

        manager
            .register::<MockPostgresPool>(Arc::clone(&ok_one), Some(cred_id))
            .await
            .unwrap();
        manager
            .register::<MockPostgresPool>(Arc::clone(&failing), Some(cred_id))
            .await
            .unwrap();
        manager
            .register::<MockPostgresPool>(Arc::clone(&ok_two), Some(cred_id))
            .await
            .unwrap();

        let scheme = fresh_scheme();
        let outcome = manager.on_credential_refreshed(&cred_id, &scheme).await;

        assert_eq!(outcome.per_resource.len(), 3);
        assert!(matches!(outcome.per_resource[0], DispatchOutcome::Ok));
        assert!(matches!(
            outcome.per_resource[1],
            DispatchOutcome::Failed(_)
        ));
        assert!(matches!(outcome.per_resource[2], DispatchOutcome::Ok));

        // Both Ok resources actually ran.
        assert_eq!(ok_one.refresh_count(), 1);
        assert_eq!(ok_two.refresh_count(), 1);
        // Failing one bailed before incrementing.
        assert_eq!(failing.refresh_count(), 0);
        // RotationOutcome reflects the partial failure.
        assert!(!outcome.all_ok());
    }

    /// Heterogeneous resource types (Pool + Transport) bound to the same
    /// credential id all receive the rotation hook. Demonstrates that
    /// the dispatcher's type erasure works across topology variants.
    #[tokio::test]
    async fn parallel_dispatch_crosses_topology_variants() {
        let manager = Manager::new();

        let cred_id = CredentialId::new();
        let pg = Arc::new(MockPostgresPool::fast());
        let kafka = Arc::new(MockKafkaTransport::new());

        manager
            .register::<MockPostgresPool>(Arc::clone(&pg), Some(cred_id))
            .await
            .unwrap();
        manager
            .register::<MockKafkaTransport>(Arc::clone(&kafka), Some(cred_id))
            .await
            .unwrap();

        let scheme = fresh_scheme();
        let outcome = manager.on_credential_refreshed(&cred_id, &scheme).await;
        assert_eq!(outcome.per_resource.len(), 2);
        assert!(outcome.all_ok());

        assert_eq!(pg.refresh_count(), 1);
        assert_eq!(kafka.refresh_count.load(Ordering::SeqCst), 1);
    }

    /// `register::<R>` for credential-bearing R requires a Some(id);
    /// passing None is a misuse the manager surfaces.
    #[tokio::test]
    async fn register_credential_bearing_without_id_errors() {
        let manager = Manager::new();
        let pg = Arc::new(MockPostgresPool::fast());
        let err = manager
            .register::<MockPostgresPool>(pg, None)
            .await
            .unwrap_err();
        assert!(err.contains("credential id"));
    }

    /// Revoke dispatcher symmetric to refresh: parallel + per-resource
    /// timeout isolation, but no scheme passed.
    #[tokio::test]
    async fn revocation_dispatch_calls_each_resource() {
        let manager = Manager::new();
        let cred_id = CredentialId::new();
        let pg = Arc::new(MockPostgresPool::fast());
        let kafka = Arc::new(MockKafkaTransport::new());

        manager
            .register::<MockPostgresPool>(Arc::clone(&pg), Some(cred_id))
            .await
            .unwrap();
        manager
            .register::<MockKafkaTransport>(Arc::clone(&kafka), Some(cred_id))
            .await
            .unwrap();

        // Default `on_credential_revoke` is no-op Ok — every dispatch
        // should ack `Ok`. Production would assert side effects (pool
        // teardown, mark tainted, etc.); spike confirms the hook fires.
        let outcome = manager.on_credential_revoked(&cred_id).await;
        assert_eq!(outcome.per_resource.len(), 2);
        assert!(outcome.all_ok());
    }

    /// Compile-only: confirms `<R::Credential as Credential>::Scheme`
    /// pulled out as a named type works at the call site without ugly
    /// fully-qualified syntax. This is exit-criterion #4 from the
    /// task brief.
    #[allow(dead_code)]
    fn ergonomic_callsite_proof() {
        // You can name the projected scheme in a fn signature.
        fn _takes_pg_scheme(_scheme: &<MockPostgresPool as Resource>::Credential) {}

        // You can spell the scheme via `Credential::Scheme` projection.
        fn _takes_pg_scheme_v2(
            _scheme: &<<MockPostgresPool as Resource>::Credential as Credential>::Scheme,
        ) {
        }

        // The NoCredential opt-out also reads naturally.
        fn _kv_compiles() {
            let _kv = MockKvStore;
        }
    }
}
