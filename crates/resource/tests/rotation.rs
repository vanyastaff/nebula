//! Integration tests for rotation dispatch (П2 §3.2-§3.5 coverage).
//!
//! These tests are the first end-to-end exercises of the rotation dispatch
//! path that went live in П2 Tasks 4-5 (`Manager::on_credential_refreshed` /
//! `_revoked`). All in-tree resources are `NoCredential`-bound, so the
//! production dispatcher is unreachable from `basic_integration.rs`. This
//! file ships a credential-bearing `TestResource` so the dispatch path
//! actually runs.
//!
//! Per Tech Spec §3.2-§3.5, the dispatch loop must:
//!
//! - Fan out to **every** resource bound to a `CredentialId` (reverse-index lookup), not just the
//!   first match.
//! - Run per-resource futures **concurrently** via `join_all`, so wall-clock stays close to the
//!   slowest resource rather than summing.
//! - Apply **per-resource timeouts** (security amendment B-1) so one slow resource never poisons
//!   siblings.
//! - **Isolate failures** so one resource's `Failed` outcome leaves siblings reporting their own
//!   outcomes intact.
//! - On revocation, emit a per-resource `HealthChanged{healthy:false}` inline for non-`Ok` outcomes
//!   (security amendment B-2) so subscribers that miss the aggregate still see per-resource failure
//!   signals.
//! - **Skip the reverse-index** for `NoCredential`-bound resources — the register path logs a
//!   warning if a `credential_id` is supplied alongside `Credential = NoCredential`, but never
//!   wires a dispatcher.

use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use nebula_core::{CredentialId, ResourceKey, ScopeLevel, resource_key};
use nebula_credential::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialError, CredentialMetadata,
    CredentialState, NoCredential, PublicScheme, ResolveResult, SchemeFactory, SchemeGuard,
};
use nebula_resource::{
    Manager, ManagerConfig, ResourceContext, ResourceEvent, TopologyRuntime,
    error::RefreshOutcome,
    resource::{Resource, ResourceConfig},
    runtime::pool::PoolRuntime,
    topology::pooled::{Pooled, config::Config as PoolConfig},
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

// ============================================================================
// TestScheme — a `PublicScheme` mock; `Clone` so `SchemeFactory::for_test_static`
// can hand it out N times across the dispatch fan-out.
// ============================================================================

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TestScheme {
    token: String,
}

impl AuthScheme for TestScheme {
    fn pattern() -> AuthPattern {
        AuthPattern::SecretToken
    }
}

// `PublicScheme` instead of `SensitiveScheme` keeps the fixture free of
// `ZeroizeOnDrop` requirements on the wrapped scheme — production credentials
// must use `SensitiveScheme` per §15.5, but the mock has no real secret.
impl PublicScheme for TestScheme {}

// ============================================================================
// TestState — the credential's stored form. Carries the same token field
// so `project()` is a trivial copy.
// ============================================================================

#[derive(Clone, Debug, Default, Serialize, Deserialize, ZeroizeOnDrop)]
struct TestState {
    token: String,
}

impl Zeroize for TestState {
    fn zeroize(&mut self) {
        self.token.zeroize();
    }
}

impl CredentialState for TestState {
    const KIND: &'static str = "test_credential";
    const VERSION: u32 = 1;
}

// ============================================================================
// TestCredential — minimal `Credential` impl driving the dispatch fan-out.
// ============================================================================

#[derive(Clone, Copy, Debug, Default)]
struct TestCredential;

impl Credential for TestCredential {
    type Input = ();
    type Scheme = TestScheme;
    type State = TestState;
    const KEY: &'static str = "test_credential";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("test_credential"))
            .name("Test credential")
            .description("Mock credential for rotation dispatch tests")
            .schema(<Self as Credential>::schema())
            .pattern(AuthPattern::SecretToken)
            .build()
            .expect("test credential metadata is statically valid")
    }

    fn project(state: &Self::State) -> Self::Scheme {
        TestScheme {
            token: state.token.clone(),
        }
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, ()>, CredentialError> {
        Ok(ResolveResult::Complete(TestState {
            token: "initial-token".into(),
        }))
    }
}

// ============================================================================
// TestError + TestConfig — minimal shapes the `Resource` trait needs.
// ============================================================================

#[derive(Debug)]
struct TestError(String);

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TestError: {}", self.0)
    }
}

impl std::error::Error for TestError {}

impl From<TestError> for nebula_resource::Error {
    fn from(e: TestError) -> Self {
        nebula_resource::Error::permanent(e.0)
    }
}

#[derive(Clone, Debug, Default)]
struct TestConfig;

nebula_schema::impl_empty_has_schema!(TestConfig);

impl ResourceConfig for TestConfig {}

// ============================================================================
// TestResource — records refresh/revoke invocations via `Arc<Mutex<...>>`
// so clones share counters across the dispatch fan-out. Optional knobs
// (delay / failure flags) drive the timeout + isolation tests.
// ============================================================================

#[derive(Clone)]
struct TestResource {
    /// Counts how many times `on_credential_refresh` was called.
    refresh_count: Arc<Mutex<usize>>,
    /// Last token observed in `on_credential_refresh`.
    last_token: Arc<Mutex<Option<String>>>,
    /// Counts `on_credential_revoke` invocations.
    revoke_count: Arc<Mutex<usize>>,
    /// Optional refresh delay — drives the per-resource timeout test.
    refresh_delay: Duration,
    /// If true, `on_credential_refresh` returns `Err` deliberately.
    refresh_should_fail: bool,
    /// If true, `on_credential_revoke` returns `Err` deliberately.
    revoke_should_fail: bool,
    /// Per-resource key suffix so multiple instances share a credential
    /// without colliding on `Resource::key()` (which is type-keyed). The
    /// reverse-index keys on `ResourceKey`, so distinct resource keys
    /// must be supplied via the wrapper newtypes below.
    _phantom: std::marker::PhantomData<()>,
}

impl TestResource {
    fn new() -> Self {
        Self {
            refresh_count: Arc::new(Mutex::new(0)),
            last_token: Arc::new(Mutex::new(None)),
            revoke_count: Arc::new(Mutex::new(0)),
            refresh_delay: Duration::ZERO,
            refresh_should_fail: false,
            revoke_should_fail: false,
            _phantom: std::marker::PhantomData,
        }
    }

    fn with_refresh_delay(mut self, delay: Duration) -> Self {
        self.refresh_delay = delay;
        self
    }

    fn with_refresh_failure(mut self) -> Self {
        self.refresh_should_fail = true;
        self
    }

    fn with_revoke_failure(mut self) -> Self {
        self.revoke_should_fail = true;
        self
    }
}

/// Macro that mints a distinct `Resource` impl per resource-key slug. The
/// reverse-index keys on `ResourceKey`, so each fan-out branch needs a
/// distinct key — and `Resource::key()` is type-associated, so we need a
/// distinct type per slug. The macro emits a wrapper newtype that
/// `Deref`-forwards behaviour to a shared `TestResource` instance.
macro_rules! test_resource {
    ($name:ident, $key:literal) => {
        #[derive(Clone)]
        struct $name(TestResource);

        // Macro-generated wrapper exposes the full builder surface; not all
        // tests exercise every knob, hence per-type `dead_code` allowance.
        #[allow(dead_code)]
        impl $name {
            fn new() -> Self {
                Self(TestResource::new())
            }

            fn inner(&self) -> &TestResource {
                &self.0
            }

            fn with_refresh_delay(self, delay: Duration) -> Self {
                Self(self.0.with_refresh_delay(delay))
            }

            fn with_refresh_failure(self) -> Self {
                Self(self.0.with_refresh_failure())
            }

            fn with_revoke_failure(self) -> Self {
                Self(self.0.with_revoke_failure())
            }
        }

        impl Resource for $name {
            type Config = TestConfig;
            type Runtime = ();
            type Lease = ();
            type Error = TestError;
            type Credential = TestCredential;

            fn key() -> ResourceKey {
                resource_key!($key)
            }

            fn create(
                &self,
                _config: &Self::Config,
                _scheme: &<Self::Credential as Credential>::Scheme,
                _ctx: &ResourceContext,
            ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send {
                async { Ok(()) }
            }

            fn on_credential_refresh<'a>(
                &self,
                new_scheme: SchemeGuard<'a, Self::Credential>,
                _ctx: &'a CredentialContext,
            ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
                let count = Arc::clone(&self.0.refresh_count);
                let token_slot = Arc::clone(&self.0.last_token);
                let delay = self.0.refresh_delay;
                let should_fail = self.0.refresh_should_fail;
                async move {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    if should_fail {
                        return Err(TestError(format!(
                            "{} refresh deliberately failed",
                            stringify!($name)
                        )));
                    }
                    *count.lock().unwrap() += 1;
                    *token_slot.lock().unwrap() = Some(new_scheme.token.clone());
                    Ok(())
                }
            }

            fn on_credential_revoke(
                &self,
                _credential_id: &CredentialId,
            ) -> impl Future<Output = Result<(), Self::Error>> + Send {
                let count = Arc::clone(&self.0.revoke_count);
                let should_fail = self.0.revoke_should_fail;
                async move {
                    if should_fail {
                        return Err(TestError(format!(
                            "{} revoke deliberately failed",
                            stringify!($name)
                        )));
                    }
                    *count.lock().unwrap() += 1;
                    Ok(())
                }
            }
        }

        impl Pooled for $name {}
    };
}

// Five distinct resource types — used across the test cases. The Pool
// topology default-impls cover everything needed for registration; we
// never actually `acquire` them, so the topology choice is only a
// `register()` formality.
test_resource!(TestResourceA, "test.rotation.a");
test_resource!(TestResourceB, "test.rotation.b");
test_resource!(TestResourceC, "test.rotation.c");
test_resource!(TestResourceD, "test.rotation.d");
test_resource!(TestResourceE, "test.rotation.e");

// ============================================================================
// NoCredentialResource — used by the opt-out test to confirm the dispatch
// loop returns an empty result vec when no resources are bound to the id.
// ============================================================================

#[derive(Clone, Default)]
struct NoCredentialResource;

impl Resource for NoCredentialResource {
    type Config = TestConfig;
    type Runtime = ();
    type Lease = ();
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test.rotation.no_cred")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(())
    }
}

impl Pooled for NoCredentialResource {}

// ============================================================================
// Helpers
// ============================================================================

fn pool_topology<R: Resource + Clone + Send + Sync + 'static>() -> TopologyRuntime<R> {
    TopologyRuntime::Pool(PoolRuntime::<R>::new(PoolConfig::default(), 1))
}

fn cred_ctx() -> CredentialContext {
    CredentialContext::for_test("rotation-test-owner")
}

/// Build a `SchemeFactory<TestCredential>` that mints fresh guards
/// projecting the supplied token. Factory closures clone the token per
/// `acquire`, mirroring the engine's per-call projection.
fn token_factory(token: &str) -> SchemeFactory<TestCredential> {
    SchemeFactory::for_test_static(TestScheme {
        token: token.to_owned(),
    })
}

/// Drain the broadcast receiver synchronously after a dispatch call.
///
/// `try_recv` is enough because `Manager::on_credential_*` awaits the
/// `join_all` to completion before sending the aggregate event, so by the
/// time we drain, every event the dispatch produced is already buffered.
fn drain_events(rx: &mut tokio::sync::broadcast::Receiver<ResourceEvent>) -> Vec<ResourceEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

// ============================================================================
// Test 1 — single-resource dispatch (basic happy path).
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_dispatches_to_single_resource() {
    let manager = Manager::new();
    let cred = cred_ctx();
    let cid = CredentialId::new();

    let resource = TestResourceA::new();
    let observer = resource.clone();

    manager
        .register::<TestResourceA>(
            resource,
            TestConfig,
            ScopeLevel::Global,
            pool_topology::<TestResourceA>(),
            None,
            None,
            Some(cid),
            None,
        )
        .expect("register succeeds for credential-bearing resource with credential_id");

    // Subscribe BEFORE dispatch so the broadcast channel has the receiver
    // ready when the aggregate event lands.
    let mut rx = manager.subscribe_events();
    // Drop the `Registered` event the registration above broadcast.
    let _ = drain_events(&mut rx);

    let factory = token_factory("post-refresh-token");
    let results = manager
        .on_credential_refreshed::<TestCredential>(&cid, factory, &cred)
        .await
        .expect("dispatch loop never returns Err for the per-resource path");

    assert_eq!(results.len(), 1, "one resource bound to credential");
    assert!(
        matches!(results[0].1, RefreshOutcome::Ok),
        "expected RefreshOutcome::Ok, got {:?}",
        results[0].1,
    );
    assert_eq!(results[0].0, TestResourceA::key());

    assert_eq!(*observer.inner().refresh_count.lock().unwrap(), 1);
    assert_eq!(
        observer.inner().last_token.lock().unwrap().as_deref(),
        Some("post-refresh-token"),
    );

    let events = drain_events(&mut rx);
    let aggregate = events
        .iter()
        .find_map(|ev| match ev {
            ResourceEvent::CredentialRefreshed {
                credential_id,
                resources_affected,
                outcome,
            } => Some((*credential_id, *resources_affected, *outcome)),
            _ => None,
        })
        .expect("CredentialRefreshed event emitted");
    assert_eq!(aggregate.0, cid);
    assert_eq!(aggregate.1, 1);
    assert_eq!(aggregate.2.ok, 1);
    assert_eq!(aggregate.2.failed, 0);
    assert_eq!(aggregate.2.timed_out, 0);
}

// ============================================================================
// Test 2 — parallel dispatch fan-out.
//
// Five resources, each with `refresh_delay = 200ms`. Sequential dispatch
// would take ~1s; parallel dispatch should land in ~200-400ms. We assert
// `< 800ms` for headroom on slow CI.
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_dispatches_parallel_to_multiple_resources() {
    let manager = Manager::new();
    let cred = cred_ctx();
    let cid = CredentialId::new();

    let delay = Duration::from_millis(200);
    let res_a = TestResourceA::new().with_refresh_delay(delay);
    let res_b = TestResourceB::new().with_refresh_delay(delay);
    let res_c = TestResourceC::new().with_refresh_delay(delay);
    let res_d = TestResourceD::new().with_refresh_delay(delay);
    let res_e = TestResourceE::new().with_refresh_delay(delay);

    // Resource newtypes are distinct types, so a homogeneous array won't
    // work. Hold the inner counters directly — they share the
    // `Arc<Mutex<...>>` shape regardless of wrapper type.
    let observers: [TestResource; 5] = [
        res_a.inner().clone(),
        res_b.inner().clone(),
        res_c.inner().clone(),
        res_d.inner().clone(),
        res_e.inner().clone(),
    ];

    macro_rules! register {
        ($r:ident, $ty:ident) => {
            manager
                .register::<$ty>(
                    $r,
                    TestConfig,
                    ScopeLevel::Global,
                    pool_topology::<$ty>(),
                    None,
                    None,
                    Some(cid),
                    None,
                )
                .expect("register succeeds")
        };
    }
    register!(res_a, TestResourceA);
    register!(res_b, TestResourceB);
    register!(res_c, TestResourceC);
    register!(res_d, TestResourceD);
    register!(res_e, TestResourceE);

    let factory = token_factory("parallel-token");
    let started = Instant::now();
    let results = manager
        .on_credential_refreshed::<TestCredential>(&cid, factory, &cred)
        .await
        .expect("dispatch loop succeeds");
    let elapsed = started.elapsed();

    assert_eq!(results.len(), 5, "all 5 resources reached");
    assert!(
        results.iter().all(|(_, o)| matches!(o, RefreshOutcome::Ok)),
        "all outcomes should be Ok",
    );
    assert!(
        elapsed < Duration::from_millis(800),
        "dispatch should be parallel, not sequential — wall-clock was {elapsed:?} (expected < 800ms; sequential would be ~1s)",
    );

    for obs in &observers {
        assert_eq!(*obs.refresh_count.lock().unwrap(), 1);
        assert_eq!(
            obs.last_token.lock().unwrap().as_deref(),
            Some("parallel-token"),
        );
    }
}

// ============================================================================
// Test 3 — per-resource timeout isolation (security amendment B-1).
//
// Three resources: A (50ms), B (5s), C (50ms). Manager timeout: 200ms.
// Outcomes: [Ok, TimedOut, Ok]. Wall-clock < 600ms (would be 5s+ if the
// slow one weren't isolated).
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_per_resource_timeout_isolates_slow_one() {
    let manager = Manager::with_config(ManagerConfig {
        credential_rotation_timeout: Duration::from_millis(200),
        ..ManagerConfig::default()
    });
    let cred = cred_ctx();
    let cid = CredentialId::new();

    let a = TestResourceA::new().with_refresh_delay(Duration::from_millis(50));
    let b = TestResourceB::new().with_refresh_delay(Duration::from_secs(5));
    let c = TestResourceC::new().with_refresh_delay(Duration::from_millis(50));

    let observer_a = a.clone();
    let observer_b = b.clone();
    let observer_c = c.clone();

    macro_rules! register {
        ($r:ident, $ty:ident) => {
            manager
                .register::<$ty>(
                    $r,
                    TestConfig,
                    ScopeLevel::Global,
                    pool_topology::<$ty>(),
                    None,
                    None,
                    Some(cid),
                    None,
                )
                .expect("register succeeds")
        };
    }
    register!(a, TestResourceA);
    register!(b, TestResourceB);
    register!(c, TestResourceC);

    let factory = token_factory("isolation-token");
    let started = Instant::now();
    let results = manager
        .on_credential_refreshed::<TestCredential>(&cid, factory, &cred)
        .await
        .expect("dispatch loop succeeds");
    let elapsed = started.elapsed();

    assert_eq!(results.len(), 3, "all 3 resources reported outcomes");
    assert!(
        elapsed < Duration::from_millis(600),
        "slow resource must not block siblings — wall-clock was {elapsed:?} (expected < 600ms)",
    );

    // Index outcomes by key — `join_all` preserves input order, but assert
    // by key for resilience to upstream ordering changes.
    let by_key: std::collections::HashMap<_, _> = results.into_iter().collect();
    assert!(
        matches!(by_key.get(&TestResourceA::key()), Some(RefreshOutcome::Ok)),
        "A should succeed",
    );
    assert!(
        matches!(
            by_key.get(&TestResourceB::key()),
            Some(RefreshOutcome::TimedOut { .. })
        ),
        "B should time out, got {:?}",
        by_key.get(&TestResourceB::key()),
    );
    assert!(
        matches!(by_key.get(&TestResourceC::key()), Some(RefreshOutcome::Ok)),
        "C should succeed",
    );

    // A and C ran the body; B was timed out before completion.
    assert_eq!(*observer_a.inner().refresh_count.lock().unwrap(), 1);
    assert_eq!(*observer_b.inner().refresh_count.lock().unwrap(), 0);
    assert_eq!(*observer_c.inner().refresh_count.lock().unwrap(), 1);
}

// ============================================================================
// Test 4 — per-resource failure isolation.
//
// Three resources, middle one returns Err. Outer dispatch returns Ok;
// inner outcomes are [Ok, Failed, Ok].
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_failure_isolates_one_resource() {
    let manager = Manager::new();
    let cred = cred_ctx();
    let cid = CredentialId::new();

    let a = TestResourceA::new();
    let b = TestResourceB::new().with_refresh_failure();
    let c = TestResourceC::new();

    let observer_a = a.clone();
    let observer_b = b.clone();
    let observer_c = c.clone();

    macro_rules! register {
        ($r:ident, $ty:ident) => {
            manager
                .register::<$ty>(
                    $r,
                    TestConfig,
                    ScopeLevel::Global,
                    pool_topology::<$ty>(),
                    None,
                    None,
                    Some(cid),
                    None,
                )
                .expect("register succeeds")
        };
    }
    register!(a, TestResourceA);
    register!(b, TestResourceB);
    register!(c, TestResourceC);

    let factory = token_factory("isolation-failure-token");
    let results = manager
        .on_credential_refreshed::<TestCredential>(&cid, factory, &cred)
        .await
        .expect("dispatch loop succeeds even when one resource fails");

    assert_eq!(results.len(), 3);
    let by_key: std::collections::HashMap<_, _> = results.into_iter().collect();
    assert!(
        matches!(by_key.get(&TestResourceA::key()), Some(RefreshOutcome::Ok)),
        "A should succeed",
    );
    assert!(
        matches!(
            by_key.get(&TestResourceB::key()),
            Some(RefreshOutcome::Failed(_))
        ),
        "B should fail, got {:?}",
        by_key.get(&TestResourceB::key()),
    );
    assert!(
        matches!(by_key.get(&TestResourceC::key()), Some(RefreshOutcome::Ok)),
        "C should succeed",
    );

    assert_eq!(*observer_a.inner().refresh_count.lock().unwrap(), 1);
    assert_eq!(
        *observer_b.inner().refresh_count.lock().unwrap(),
        0,
        "B's failure short-circuits before count increment",
    );
    assert_eq!(*observer_c.inner().refresh_count.lock().unwrap(), 1);
}

// ============================================================================
// Test 5 — revoke emits HealthChanged for failed resources only
// (security amendment B-2).
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_emits_health_changed_for_failures() {
    let manager = Manager::new();
    let cid = CredentialId::new();

    let a = TestResourceA::new();
    let b = TestResourceB::new().with_revoke_failure();

    let observer_a = a.clone();
    let observer_b = b.clone();

    macro_rules! register {
        ($r:ident, $ty:ident) => {
            manager
                .register::<$ty>(
                    $r,
                    TestConfig,
                    ScopeLevel::Global,
                    pool_topology::<$ty>(),
                    None,
                    None,
                    Some(cid),
                    None,
                )
                .expect("register succeeds")
        };
    }
    register!(a, TestResourceA);
    register!(b, TestResourceB);

    // Subscribe BEFORE dispatch.
    let mut rx = manager.subscribe_events();
    // Drop the two `Registered` events the registrations broadcast.
    let _ = drain_events(&mut rx);

    let results = manager
        .on_credential_revoked(&cid)
        .await
        .expect("dispatch loop succeeds even when one resource fails");

    assert_eq!(results.len(), 2);

    // A succeeded → counter incremented; B's failure short-circuits.
    assert_eq!(*observer_a.inner().revoke_count.lock().unwrap(), 1);
    assert_eq!(*observer_b.inner().revoke_count.lock().unwrap(), 0);

    let events = drain_events(&mut rx);

    // HealthChanged{healthy:false} only for the failed resource (B).
    let health_events: Vec<_> = events
        .iter()
        .filter_map(|ev| match ev {
            ResourceEvent::HealthChanged { key, healthy } => Some((key.clone(), *healthy)),
            _ => None,
        })
        .collect();
    assert_eq!(
        health_events,
        vec![(TestResourceB::key(), false)],
        "HealthChanged should fire ONLY for the failed resource",
    );

    // Aggregate CredentialRevoked event has `outcome.failed = 1`.
    let aggregate = events
        .iter()
        .find_map(|ev| match ev {
            ResourceEvent::CredentialRevoked {
                credential_id,
                resources_affected,
                outcome,
            } => Some((*credential_id, *resources_affected, *outcome)),
            _ => None,
        })
        .expect("CredentialRevoked aggregate event emitted");
    assert_eq!(aggregate.0, cid);
    assert_eq!(aggregate.1, 2);
    assert_eq!(aggregate.2.ok, 1);
    assert_eq!(aggregate.2.failed, 1);
    assert_eq!(aggregate.2.timed_out, 0);
}

// ============================================================================
// Test 6 — `NoCredential` resources opt out of the reverse-index.
//
// A `NoCredential`-bound resource is registered. A dispatch call with a
// random `CredentialId` returns an empty result vec (the resource is not
// in the reverse-index).
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_credential_resource_skips_reverse_index() {
    let manager = Manager::new();
    let cred = cred_ctx();
    let unrelated_cid = CredentialId::new();

    let resource = NoCredentialResource;
    manager
        .register::<NoCredentialResource>(
            resource,
            TestConfig,
            ScopeLevel::Global,
            pool_topology::<NoCredentialResource>(),
            None,
            None,
            // No credential_id — the NoCredential opt-out path.
            None,
            None,
        )
        .expect("register succeeds for NoCredential resource without credential_id");

    // Dispatch refresh to an unrelated credential — the reverse-index has
    // no entry for it, so no resource is dispatched to.
    //
    // Use `NoCredential` (not `TestCredential`) for the dispatch type
    // parameter, because the reverse-index lookup returns zero
    // dispatchers; the type-id check never runs. Either works for the
    // empty-vec assertion, but `NoCredential` is the realistic call shape
    // for "engine asks resources bound to a no-auth credential."
    let factory: SchemeFactory<NoCredential> = SchemeFactory::for_test_static(());
    let refresh_results = manager
        .on_credential_refreshed::<NoCredential>(&unrelated_cid, factory, &cred)
        .await
        .expect("dispatch loop succeeds");
    assert!(
        refresh_results.is_empty(),
        "NoCredential resource never enters the reverse-index, so no dispatchers fire — got {refresh_results:?}",
    );

    // Same for revoke.
    let revoke_results = manager
        .on_credential_revoked(&unrelated_cid)
        .await
        .expect("dispatch loop succeeds");
    assert!(
        revoke_results.is_empty(),
        "NoCredential resource never enters the reverse-index — got {revoke_results:?}",
    );
}

// ============================================================================
// Test 7 — `NoCredential` resource registered WITH a credential_id (the
// warn-and-ignore opt-out path).
//
// Complement to Test 6: confirms that even when the caller mistakenly
// supplies a `credential_id` for a `NoCredential`-bound resource, the
// register path warns and discards the id rather than wiring a dispatcher.
// A subsequent dispatch against that id must return zero results — proving
// the validate-then-write split (CodeRabbit 🔴 #1) preserves the existing
// opt-out semantics.
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_credential_resource_with_credential_id_warns_and_ignores() {
    let manager = Manager::new();
    let cred = cred_ctx();
    let cid = CredentialId::new();

    let resource = NoCredentialResource;
    manager
        .register::<NoCredentialResource>(
            resource,
            TestConfig,
            ScopeLevel::Global,
            pool_topology::<NoCredentialResource>(),
            None,
            None,
            // Supply a credential_id — the register path should log a warn
            // and refuse to wire the dispatcher rather than fail.
            Some(cid),
            None,
        )
        .expect("register succeeds for NoCredential resource even when credential_id supplied");

    // Dispatch refresh against the id we supplied — must still find no
    // dispatchers because NoCredential opts out unconditionally.
    let factory: SchemeFactory<NoCredential> = SchemeFactory::for_test_static(());
    let refresh_results = manager
        .on_credential_refreshed::<NoCredential>(&cid, factory, &cred)
        .await
        .expect("dispatch loop succeeds");
    assert!(
        refresh_results.is_empty(),
        "NoCredential resource must not appear in reverse-index even when credential_id provided; got {refresh_results:?}",
    );

    // Same for revoke — the warn-and-ignore path leaves the index empty.
    let revoke_results = manager
        .on_credential_revoked(&cid)
        .await
        .expect("dispatch loop succeeds");
    assert!(
        revoke_results.is_empty(),
        "NoCredential resource must not appear in reverse-index for revoke either; got {revoke_results:?}",
    );
}
