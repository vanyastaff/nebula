//! Manager-level integration tests for the Transport topology (R-013).
//!
//! Phase 1 cascade audit flagged the Transport surface as having zero
//! Manager-level tests — only `TransportRuntime`-direct coverage existed
//! in `basic_integration.rs`. This file exercises the public Manager
//! dispatch path (`register_transport`, `register_transport_with`,
//! `acquire_transport`, `acquire_transport`) end-to-end, plus
//! cross-cutting concerns (graceful shutdown drain, recovery-gate
//! admission, multiplexing semantics, session-limit backpressure,
//! per-resource-key isolation).
//!
//! Mock resource: `MockTransport` simulates a long-lived connection
//! (`MockTransportInner`) that issues short-lived `MockSession` leases.
//! `open_session` / `close_session` increment atomic counters so tests
//! verify the Manager dispatched correctly.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use nebula_core::{ExecutionId, ResourceKey, WorkflowId, resource_key};
use nebula_resource::{
    AcquireOptions, BoundedConfig, BoundedRuntime, Manager, RegisterOptions, RegistrationSpec,
    ResourceContext, ScopeLevel, ShutdownConfig,
    error::{Error, ErrorKind},
    recovery::{RecoveryGate, RecoveryGateConfig},
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::TopologyRuntime,
    topology::bounded::{Bounded, BoundedRelease, Capped},
    topology_tag::TopologyTag,
};

// ---------------------------------------------------------------------------
// Mock error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MockError(String);

impl std::fmt::Display for MockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MockError {}

impl From<MockError> for Error {
    fn from(e: MockError) -> Self {
        Error::transient(e.0)
    }
}

// ---------------------------------------------------------------------------
// Mock config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct MockConfig;

nebula_schema::impl_empty_has_schema!(MockConfig);

impl ResourceConfig for MockConfig {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        0
    }
}

// ---------------------------------------------------------------------------
// Mock transport
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
struct MockTransportInner {
    name: &'static str,
}

#[derive(Debug)]
#[allow(dead_code)]
struct MockSession {
    id: u64,
}

/// Shared atomic counters reused by every mock transport type minted via
/// `mock_transport_type!`. The static `Resource::key()` lives on the
/// generated wrapper struct; this inner record only carries observability
/// state (`create_counter` / `open_counter` / `close_counter`).
#[derive(Clone)]
struct MockTransport {
    create_counter: Arc<AtomicU64>,
    open_counter: Arc<AtomicU64>,
    close_counter: Arc<AtomicU64>,
}

impl MockTransport {
    fn fresh() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            open_counter: Arc::new(AtomicU64::new(0)),
            close_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

// `Resource::key` is an associated function, so we need a parameterized
// impl per key. A simple newtype-per-key keeps tests separate.
//
// Macro to mint a fresh `MockTransport`-shaped type bound to a static key.
// (Using a single type with a runtime key would break `Resource::key()`'s
//  `() -> ResourceKey` signature.)
// Folded onto `Bounded` with `Cap = Capped<N>`: the former
// `Transport::open_session` is `Bounded::acquire_one`, `close_session` is
// `BoundedRelease::release_one`, and the former `TransportConfig::max_sessions`
// (a runtime field) is the cap **typestate** const generic `N` — it cannot
// change without changing the type, by design. Each test instantiates the
// `N` it needs (`TransportA::<4>` etc.), exactly mirroring the per-test
// `max_sessions` it set on the old `TransportConfig`.
macro_rules! mock_transport_type {
    ($name:ident, $key:literal) => {
        #[derive(Clone)]
        struct $name<const N: usize> {
            inner: MockTransport,
        }

        impl<const N: usize> $name<N> {
            fn new() -> Self {
                Self {
                    inner: MockTransport::fresh(),
                }
            }

            fn open_counter(&self)   -> Arc<AtomicU64> { self.inner.open_counter.clone() }
            fn close_counter(&self)  -> Arc<AtomicU64> { self.inner.close_counter.clone() }
            fn create_counter(&self) -> Arc<AtomicU64> { self.inner.create_counter.clone() }
        }

        impl<const N: usize> Resource for $name<N> {
            type Config     = MockConfig;
            type Runtime    = Arc<MockTransportInner>;
            type Lease      = MockSession;
            type Error      = MockError;

            fn key() -> ResourceKey {
                resource_key!($key)
            }

            fn create(
                &self,
                _config: &MockConfig,
                _ctx: &ResourceContext,
            ) -> impl std::future::Future<Output = Result<Arc<MockTransportInner>, MockError>> + Send {
                let counter = self.create_counter();
                async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Ok(Arc::new(MockTransportInner { name: $key }))
                }
            }

            async fn destroy(&self, _runtime: Arc<MockTransportInner>) -> Result<(), MockError> {
                Ok(())
            }

            fn metadata() -> ResourceMetadata {
                ResourceMetadata::from_key(&Self::key())
            }
        }

        impl<const N: usize> Bounded for $name<N> {
            type Cap = Capped<N>;

            // Folds `Transport::open_session`.
            fn acquire_one(
                &self,
                _transport: &Arc<MockTransportInner>,
                _ctx: &ResourceContext,
            ) -> impl std::future::Future<Output = Result<MockSession, MockError>> + Send {
                let counter = self.open_counter();
                async move {
                    let id = counter.fetch_add(1, Ordering::Relaxed);
                    Ok(MockSession { id })
                }
            }
        }

        impl<const N: usize> BoundedRelease for $name<N> {
            // Folds `Transport::close_session` (healthy flag preserved).
            fn release_one(
                &self,
                _transport: &Arc<MockTransportInner>,
                _session: MockSession,
                _healthy: bool,
            ) -> impl std::future::Future<Output = Result<(), MockError>> + Send {
                let counter = self.close_counter();
                async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }
            }
        }
    };
}

mock_transport_type!(TransportA, "test.transport.a");
mock_transport_type!(TransportB, "test.transport.b");
mock_transport_type!(GatedTransport, "test.transport.gated");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

/// Register a bounded-topology row (the Transport fold), deriving scope +
/// resolved-credential identity / resilience / recovery from a
/// [`RegisterOptions`]. The former `register_transport[_with]` shorthands'
/// behavior is reproduced through the unified `RegistrationSpec` funnel;
/// `RegisterOptions::default()` is `Global` scope, `Unbound` identity, no
/// resilience / gate. The concurrency bound is `R::Cap` (the const-generic
/// `Capped<N>`), not a config field — the caller picks the fixture's `N`.
fn register_transport_spec<R>(
    manager: &Manager,
    resource: R,
    inner: R::Runtime,
    bounded_config: BoundedConfig,
    opts: RegisterOptions,
) -> Result<(), Error>
where
    R: BoundedRelease + Clone + Resource<Config = MockConfig> + Send + Sync + 'static,
    R::Runtime: Clone + Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    let slot_identity = opts.effective_slot_identity();
    manager.register(RegistrationSpec {
        resource: resource.clone(),
        config: MockConfig,
        scope: opts.scope,
        slot_identity,
        topology: TopologyRuntime::Bounded(BoundedRuntime::<R>::new(
            &resource,
            inner,
            bounded_config,
        )),
        acquire: Manager::erased_acquire_bounded_for::<R>(),
        recovery_gate: opts.recovery_gate,
    })
}

/// Wait briefly for `ReleaseQueue`-driven `close_session` calls to land.
///
/// `close_session` runs on a background worker — the test must yield to
/// observe the counter increment after `drop(handle)`.
async fn wait_for_releases(close: &Arc<AtomicU64>, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if close.load(Ordering::Relaxed) >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "expected {expected} close_session calls, observed {}",
        close.load(Ordering::Relaxed)
    );
}

// ---------------------------------------------------------------------------
// register_transport / acquire_transport — happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_transport_then_acquire_via_manager() {
    let manager = Manager::new();
    let resource = TransportA::<4>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "a" });

    register_transport_spec(
        &manager,
        resource.clone(),
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("transport registration should succeed");

    assert!(manager.contains(&TransportA::<4>::key()));
    assert!(manager.keys().contains(&TransportA::<4>::key()));

    let handle = manager
        .acquire_bounded::<TransportA<4>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("acquire_transport should succeed");

    assert_eq!(handle.topology_tag(), TopologyTag::Bounded);
    // Single open_session call so far.
    assert_eq!(resource.open_counter().load(Ordering::Relaxed), 1);
    // `register_transport` wraps the user-supplied runtime directly, so
    // `Resource::create` is NOT invoked at register or acquire time on this
    // path. (`create` is only called by topologies that build their own
    // runtime — Pooled/Resident — not Transport/Service/Exclusive.)
    assert_eq!(resource.create_counter().load(Ordering::Relaxed), 0);

    drop(handle);
    wait_for_releases(&resource.close_counter(), 1).await;
}

// ---------------------------------------------------------------------------
// Multiplexing — multiple sessions on a single transport
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multiple_sessions_share_one_transport() {
    let manager = Manager::new();
    let resource = TransportA::<8>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "multiplex" });

    register_transport_spec(
        &manager,
        resource.clone(),
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("register");

    // Acquire 5 sessions in parallel — `join_all` issues all five
    // `acquire_transport` futures concurrently, exercising the
    // multiplexing path under real concurrency rather than serial awaits.
    let manager_ref = &manager;
    let acquires = (0..5).map(|_| async move {
        manager_ref
            .acquire_bounded::<TransportA<8>>(&ctx(), &AcquireOptions::default())
            .await
            .expect("acquire")
    });
    let handles: Vec<_> = futures::future::join_all(acquires).await;

    assert_eq!(handles.len(), 5);
    for h in &handles {
        assert_eq!(h.topology_tag(), TopologyTag::Bounded);
    }

    assert_eq!(
        resource.open_counter().load(Ordering::Relaxed),
        5,
        "five concurrent open_session calls expected"
    );

    // All 5 dropped — close_session called 5 times (background, async).
    drop(handles);
    wait_for_releases(&resource.close_counter(), 5).await;
}

// ---------------------------------------------------------------------------
// Session limit — semaphore backpressure at the Manager level
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_limit_returns_backpressure_when_exhausted() {
    let manager = Manager::new();
    let resource = TransportA::<2>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "limit" });

    register_transport_spec(
        &manager,
        resource.clone(),
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_millis(50),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("register");

    let h1 = manager
        .acquire_bounded::<TransportA<2>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("first acquire");
    let h2 = manager
        .acquire_bounded::<TransportA<2>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("second acquire");

    // Third must time out as Backpressure (semaphore exhausted).
    let result = manager
        .acquire_bounded::<TransportA<2>>(&ctx(), &AcquireOptions::default())
        .await;
    let err = result.expect_err("third acquire must fail");
    assert!(
        matches!(err.kind(), ErrorKind::Backpressure),
        "expected Backpressure, got {err:?}"
    );

    // Releasing a session unblocks subsequent acquires.
    drop(h1);
    wait_for_releases(&resource.close_counter(), 1).await;

    let h3 = manager
        .acquire_bounded::<TransportA<2>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("third acquire after release");
    assert_eq!(h3.topology_tag(), TopologyTag::Bounded);

    drop(h2);
    drop(h3);
    wait_for_releases(&resource.close_counter(), 3).await;
}

// ---------------------------------------------------------------------------
// register_transport_with — recovery gate path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_transport_with_recovery_gate_admits_when_idle() {
    let manager = Manager::new();
    let resource = GatedTransport::<4>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "gated" });
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

    register_transport_spec(
        &manager,
        resource.clone(),
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default().with_recovery_gate(gate.clone()),
    )
    .expect("transport registration with recovery gate");

    // Gate is `Idle` by default — acquires pass through unimpeded.
    let handle = manager
        .acquire_bounded::<GatedTransport<4>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("acquire under healthy gate");

    assert_eq!(handle.topology_tag(), TopologyTag::Bounded);
    assert_eq!(resource.open_counter().load(Ordering::Relaxed), 1);

    drop(handle);
    wait_for_releases(&resource.close_counter(), 1).await;
}

// ---------------------------------------------------------------------------
// register_transport_with — default options registration on happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_transport_with_default_options_succeeds_on_happy_path() {
    // Mythos v2: `with_resilience` deleted. Registration uses
    // `RegisterOptions::default()`; topology-layer `acquire_timeout` is
    // the only acquire-bounded knob.
    let manager = Manager::new();
    let resource = TransportA::<4>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "resilient" });

    register_transport_spec(
        &manager,
        resource.clone(),
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("transport registration succeeds with default options");

    let handle = manager
        .acquire_bounded::<TransportA<4>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("acquire under default options");

    assert_eq!(handle.topology_tag(), TopologyTag::Bounded);
    assert_eq!(resource.open_counter().load(Ordering::Relaxed), 1);

    drop(handle);
    wait_for_releases(&resource.close_counter(), 1).await;
}

// ---------------------------------------------------------------------------
// Two transports, distinct keys — Manager dispatches correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_isolates_transports_by_key() {
    let manager = Manager::new();
    let res_a = TransportA::<2>::new();
    let res_b = TransportB::<2>::new();
    let inner_a = Arc::new(MockTransportInner { name: "a" });
    let inner_b = Arc::new(MockTransportInner { name: "b" });

    register_transport_spec(
        &manager,
        res_a.clone(),
        inner_a,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("register A");
    register_transport_spec(
        &manager,
        res_b.clone(),
        inner_b,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("register B");

    assert!(manager.contains(&TransportA::<2>::key()));
    assert!(manager.contains(&TransportB::<2>::key()));
    assert_eq!(manager.keys().len(), 2);

    let h_a = manager
        .acquire_bounded::<TransportA<2>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("acquire A");
    let h_b = manager
        .acquire_bounded::<TransportB<2>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("acquire B");

    assert_eq!(res_a.open_counter().load(Ordering::Relaxed), 1);
    assert_eq!(res_b.open_counter().load(Ordering::Relaxed), 1);
    assert_eq!(res_a.close_counter().load(Ordering::Relaxed), 0);
    assert_eq!(res_b.close_counter().load(Ordering::Relaxed), 0);

    drop(h_a);
    drop(h_b);
    wait_for_releases(&res_a.close_counter(), 1).await;
    wait_for_releases(&res_b.close_counter(), 1).await;
}

// ---------------------------------------------------------------------------
// Manager::remove drops the registration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remove_drops_transport_registration() {
    let manager = Manager::new();
    let resource = TransportA::<2>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "removable" });

    register_transport_spec(
        &manager,
        resource,
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("register");

    assert!(manager.contains(&TransportA::<2>::key()));

    manager.remove(&TransportA::<2>::key()).expect("remove");

    assert!(!manager.contains(&TransportA::<2>::key()));

    // Acquire on a removed key returns NotFound.
    let result = manager
        .acquire_bounded::<TransportA<2>>(&ctx(), &AcquireOptions::default())
        .await;
    let err = result.expect_err("acquire after remove must fail");
    assert!(
        matches!(err.kind(), ErrorKind::NotFound),
        "expected NotFound, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// graceful_shutdown drains held sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_drains_held_sessions() {
    let manager = Manager::new();
    let resource = TransportA::<4>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "shutdown" });

    register_transport_spec(
        &manager,
        resource.clone(),
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default(),
    )
    .expect("register");

    let handle = manager
        .acquire_bounded::<TransportA<4>>(&ctx(), &AcquireOptions::default())
        .await
        .expect("acquire");
    assert_eq!(resource.open_counter().load(Ordering::Relaxed), 1);

    // Drop the handle BEFORE graceful_shutdown so the drain has only the
    // queued close_session work to flush. graceful_shutdown should observe
    // close_counter == 1 after it returns.
    drop(handle);

    let _report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("graceful_shutdown should succeed");

    assert!(
        manager.is_shutdown(),
        "manager should be in shutdown state after graceful_shutdown"
    );
    assert_eq!(
        resource.close_counter().load(Ordering::Relaxed),
        1,
        "close_session must have run before graceful_shutdown returned"
    );

    // Acquire after shutdown is rejected.
    let result = manager
        .acquire_bounded::<TransportA<4>>(&ctx(), &AcquireOptions::default())
        .await;
    assert!(
        result.is_err(),
        "acquire after graceful_shutdown must fail (got Ok)"
    );
}

// ---------------------------------------------------------------------------
// ScopeLevel — register at non-Global scope and verify lookup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_transport_with_custom_scope() {
    let manager = Manager::new();
    let resource = TransportA::<4>::new();
    let transport_inner = Arc::new(MockTransportInner { name: "scoped" });
    let workflow_id = WorkflowId::new();

    register_transport_spec(
        &manager,
        resource,
        transport_inner,
        BoundedConfig {
            acquire_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            drain_timeout: None,
        },
        RegisterOptions::default().with_scope(ScopeLevel::Workflow(workflow_id)),
    )
    .expect("transport registration with custom scope");

    assert!(manager.contains(&TransportA::<4>::key()));

    // lookup<R> at the registered scope returns the ManagedResource.
    let managed = manager
        .lookup::<TransportA<4>>(&ScopeLevel::Workflow(workflow_id))
        .expect("lookup at registered Workflow scope");
    assert_eq!(managed.generation(), 0);

    // Acquire with a context whose `scope_level()` resolves to the same
    // Workflow level as the registration. `scope_level()` returns the
    // most specific level derivable from the bag (Execution > Workflow >
    // ...), so we set ONLY `workflow_id` to keep the resolved level at
    // Workflow rather than falling through to Execution.
    let scoped_ctx = {
        use nebula_core::scope::Scope;
        use tokio_util::sync::CancellationToken;
        let scope = Scope {
            workflow_id: Some(workflow_id),
            ..Default::default()
        };
        ResourceContext::minimal(scope, CancellationToken::new())
    };
    let handle = manager
        .acquire_bounded::<TransportA<4>>(&scoped_ctx, &AcquireOptions::default())
        .await
        .expect("acquire under matching workflow scope");
    assert_eq!(handle.topology_tag(), TopologyTag::Bounded);
    drop(handle);

    // Lookup at Global (without a Global registration) must NOT resolve to the
    // workflow-scoped registration — confirms scope isolation.
    let err = manager
        .lookup::<TransportA<4>>(&ScopeLevel::Global)
        .err()
        .expect("lookup at Global must fail when only Workflow scope is registered");
    assert!(
        matches!(err.kind(), ErrorKind::NotFound),
        "expected NotFound at Global scope, got {err:?}"
    );
}
