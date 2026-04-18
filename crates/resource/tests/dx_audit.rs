//! DX audit: three realistic use cases written as a newcomer.
//! Each use case is self-contained. Friction points are noted inline.

// ============================================================================
// FRICTION NOTE [IMPORTS]: The lib.rs re-exports ~40 items but the crate still
// requires reaching into internal module paths for several types.
//
// What I expected after reading lib.rs:
//   use nebula_resource::{Manager, Pooled, Resident, PoolConfig, ResidentConfig,
//                         TopologyRuntime, PoolRuntime, ResidentRuntime, ...};
//
// What I actually had to do: some types (PoolRuntime, ResidentRuntime, the
// topology sub-traits) are re-exported from lib.rs, but the topology-trait
// sub-modules (e.g. topology::pooled::config::Config vs PoolConfig) have
// confusing aliased names, and I had to read the integration tests to
// understand the import pattern.
// ============================================================================

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use nebula_core::{ExecutionId, ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, AcquireResilience, Manager, PoolConfig, ResidentConfig, ResourceHandle,
    ShutdownConfig,
    ctx::{BasicCtx, Ctx, ScopeLevel},
    error::{Error, ErrorKind},
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, pool::PoolRuntime, resident::ResidentRuntime},
    topology::{
        pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision},
        resident::Resident,
    },
};

// ============================================================================
// FRICTION NOTE [RESOURCE KEY]: `resource_key!` macro is in `nebula_core`,
// not `nebula_resource`. The re-export in lib.rs is:
//   pub use nebula_core::{ExecutionId, ResourceKey, WorkflowId};
// but NOT the `resource_key!` macro. So I must `use nebula_core::resource_key!`
// separately. Newcomers who only `use nebula_resource::*` will get a confusing
// "macro not found" error with no hint to look in nebula_core.
// ============================================================================

// ============================================================================
// Shared test error type
// ============================================================================

#[derive(Debug, Clone)]
struct DxTestError(String);

impl std::fmt::Display for DxTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DxTestError {}

impl From<DxTestError> for Error {
    fn from(e: DxTestError) -> Self {
        Error::transient(e.0)
    }
}

fn test_ctx() -> BasicCtx {
    BasicCtx::new(ExecutionId::new())
}

// ============================================================================
// USE CASE 1: Simple HTTP Client Pool
//
// Goal: create a fake "HTTP client" resource, pool it, acquire a handle, use
// it, let it return to pool automatically via RAII.
// ============================================================================

// FRICTION NOTE [ASSOCIATED TYPES]: The Resource trait has 5 associated types.
// Runtime vs Lease distinction is not intuitive for beginners. The doc says
// "Runtime: the live resource handle" and "Lease: what callers hold" but for
// simple cases like an HTTP client they are the same type. Nothing in the
// docs says "just use the same type for both and implement From<Runtime> for
// Lease". I had to read the pool runtime source to discover that the pool
// calls `runtime.clone().into()` to produce a Lease — meaning I need
// `Runtime: Clone + Into<Lease>` and `Lease: Into<Runtime>`. These bounds are
// NOT visible on the Resource trait itself; they appear only on acquire_pooled.

#[derive(Clone)]
struct FakeHttpClient {
    /// Simulates a connection ID. In practice this would be reqwest::Client.
    connection_id: u64,
}

#[derive(Clone)]
#[allow(dead_code)]
struct HttpClientConfig {
    base_url: String,
    pool_size: u32,
}

nebula_schema::impl_empty_has_schema!(HttpClientConfig);

impl ResourceConfig for HttpClientConfig {}

// FRICTION NOTE [ResourceConfig::validate]: The default impl accepts
// everything, which is fine — but the doc says "returns an error if invalid"
// without a concrete example of what error to return. I had to look at the
// quick-start README (which uses a different, stale API) before finding
// Error::permanent() in the error module.

#[derive(Clone)]
struct HttpClientResource {
    next_id: Arc<AtomicU64>,
}

// FRICTION NOTE [Resource::key() is STATIC]: The trait method `fn key() ->
// ResourceKey` has no `&self`. This means you can't use instance data to
// derive the key, even though the struct has data. This is intentional
// (type-level identity) but the docs don't explain this constraint. I initially
// wrote `fn key(&self)` and got a confusing "method not in trait" error.
impl Resource for HttpClientResource {
    type Config = HttpClientConfig;
    type Runtime = FakeHttpClient;
    type Lease = FakeHttpClient;
    type Error = DxTestError;
    type Auth = ();

    fn key() -> ResourceKey {
        resource_key!("http.client")
    }

    fn create(
        &self,
        _config: &HttpClientConfig,
        _auth: &(),
        _ctx: &dyn Ctx,
    ) -> impl std::future::Future<Output = Result<FakeHttpClient, DxTestError>> + Send {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        async move { Ok(FakeHttpClient { connection_id: id }) }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

// FRICTION NOTE [Pooled requires Runtime: Clone + Into<Lease>]: The Pooled
// trait itself has no bounds on Clone/Into, but acquire_pooled on Manager
// requires `R::Runtime: Clone + Into<R::Lease>` and `R::Lease: Into<R::Runtime>`.
// These bounds are not visible at trait definition time. When you write the impl
// and try to compile, you get a constraint error only at the call site, not at
// the impl site. The error message says nothing like "your Runtime must be
// Clone+Into<Lease>". I had to read acquire_pooled's where clause explicitly.
//
// ADDITIONAL FRICTION NOTE [From<T> for T conflict]: When Runtime == Lease
// (same type), the natural instinct is to write `impl From<T> for T`. This
// conflicts with the blanket `impl<T> From<T> for T` in core. The correct
// approach (which core already handles) is to just NOT write that impl. But
// nothing in the docs says "if Runtime == Lease you don't need From impls".
// SEVERITY: Minor — confusing compile error for newcomers.

impl Pooled for HttpClientResource {
    fn is_broken(&self, _runtime: &FakeHttpClient) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    async fn recycle(
        &self,
        _runtime: &FakeHttpClient,
        _metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, DxTestError> {
        Ok(RecycleDecision::Keep)
    }
}

#[tokio::test]
async fn use_case_1_http_client_pool() {
    let resource = HttpClientResource {
        next_id: Arc::new(AtomicU64::new(0)),
    };

    // FRICTION NOTE [TOPOLOGYRUNTIME CONSTRUCTION]: To register a resource,
    // I must construct a TopologyRuntime<R> manually. There is no
    // `Manager::register_pooled(resource, config, PoolConfig::default())`.
    // Instead, the pattern is:
    //   let pool_rt = PoolRuntime::<MyResource>::new(pool_config, fingerprint);
    //   manager.register(..., TopologyRuntime::Pool(pool_rt), ...);
    //
    // The `fingerprint: u64` parameter to PoolRuntime::new has no
    // documentation. I had to read the source to understand it's a config
    // change-detection token (zero is fine for initial registration).
    // SEVERITY: Major — forces every user to understand internal pool plumbing.

    let pool_rt = PoolRuntime::<HttpClientResource>::new(
        PoolConfig {
            max_size: 4,
            ..PoolConfig::default()
        },
        0, // initial fingerprint — undocumented parameter
    );

    let manager = Manager::new();

    // FRICTION NOTE [register() PARAMETER COUNT]: register() takes 7 parameters.
    // The typical call for a basic pool is:
    //   register(resource, config, auth, scope, topology, resilience, recovery_gate)
    // The last two are almost always None for basic usage.
    // This is called out in a comment (`// Reason: register is a constructor ...`)
    // but it's still painful. A builder or at least a `register_simple()` helper
    // would eliminate 2 None arguments for the 90% case.
    // SEVERITY: Major

    manager
        .register(
            resource.clone(),
            HttpClientConfig {
                base_url: "https://api.example.com".into(),
                pool_size: 4,
            },
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None, // resilience
            None, // recovery_gate
        )
        .expect("registration should succeed");

    let ctx = test_ctx();

    // FRICTION NOTE [acquire_pooled AUTH PARAMETER]: The acquire_pooled
    // signature is:
    //   async fn acquire_pooled<R>(&self, auth: &R::Auth, ctx: &dyn Ctx, options: &AcquireOptions)
    //
    // When Auth = (), you must pass `&()`. This is not obvious. The
    // README example uses `manager.acquire(&key, &ctx)` which doesn't exist.
    // I expected: `manager.acquire::<HttpClientResource>(&ctx, &opts)`.
    // Having to pass auth at acquire-time when most resources don't use
    // it adds noise to every call site.
    // SEVERITY: Minor

    let handle: ResourceHandle<HttpClientResource> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    // Use the handle — dereferences to R::Lease (FakeHttpClient)
    let _client_id = handle.connection_id;

    // Handle drops here -> RAII returns to pool
    drop(handle);

    // FRICTION NOTE [SHUTDOWN]: graceful_shutdown takes a ShutdownConfig struct.
    // The default (30s) is not directly accessible as a const; you must either
    // construct ShutdownConfig::default() or inline it.
    // Minor: `manager.graceful_shutdown(ShutdownConfig::default()).await` works
    // but there's no `manager.shutdown_with_defaults().await` shortcut.
    manager
        .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_millis(100)))
        .await
        .expect("graceful_shutdown must succeed");
}

// ============================================================================
// USE CASE 2: Resident Config Store (singleton-like)
//
// Goal: create a single shared "config store" instance, register as Resident,
// acquire it, read a value.
// ============================================================================

/// Fake in-memory config store — simulates something loaded from a file.
#[derive(Clone)]
struct ConfigStore {
    values: Arc<std::collections::HashMap<String, String>>,
}

#[derive(Clone)]
struct ConfigStoreConfig {
    path: String,
}

nebula_schema::impl_empty_has_schema!(ConfigStoreConfig);

impl ResourceConfig for ConfigStoreConfig {}

#[derive(Clone)]
struct ConfigStoreResource;

impl Resource for ConfigStoreResource {
    type Config = ConfigStoreConfig;
    type Runtime = ConfigStore;
    type Lease = ConfigStore;
    type Error = DxTestError;
    type Auth = ();

    fn key() -> ResourceKey {
        resource_key!("config.store")
    }

    fn create(
        &self,
        config: &ConfigStoreConfig,
        _auth: &(),
        _ctx: &dyn Ctx,
    ) -> impl std::future::Future<Output = Result<ConfigStore, DxTestError>> + Send {
        let path = config.path.clone();
        async move {
            // Simulate loading from file
            let mut values = std::collections::HashMap::new();
            values.insert("db.host".into(), "localhost".into());
            values.insert("db.port".into(), "5432".into());
            values.insert("loaded_from".into(), path);
            Ok(ConfigStore {
                values: Arc::new(values),
            })
        }
    }
}

// FRICTION NOTE [Resident BOUNDS ON LEASE]: The Resident trait has
//   `where Self::Lease: Clone`
// as a supertrait bound. This means when you implement Resident, you need
// your Lease type to impl Clone. The error if you forget is a compile error
// pointing at the Resident impl, not at the missing Clone impl on Lease.
// This is a mild footgun — the error message doesn't say "add Clone to Lease".
impl Resident for ConfigStoreResource {}

// FRICTION NOTE [ResidentRuntime REQUIRES Runtime: Clone + Into<Lease>]:
// Same issue as pooled — the bounds are on the impl block inside
// ResidentRuntime, not visible at the trait level. The error manifests
// at acquire_resident call site with a deeply nested trait constraint message.
#[tokio::test]
async fn use_case_2_resident_config_store() {
    let manager = Manager::new();

    let resident_rt = ResidentRuntime::<ConfigStoreResource>::new(ResidentConfig {
        recreate_on_failure: false,
        create_timeout: Duration::from_secs(5),
    });

    // FRICTION NOTE [ResidentConfig FIELD NAMES]: The config struct fields are
    // `recreate_on_failure` and `create_timeout`. For a "config store" use case
    // the name `recreate_on_failure` is slightly odd — it sounds like the
    // resident might fail on acquire, but what it really means is "recreate the
    // shared instance if it goes stale". The name is technically correct but
    // takes some thought to parse.
    // SEVERITY: Nit

    manager
        .register(
            ConfigStoreResource,
            ConfigStoreConfig {
                path: "/etc/app/config.json".into(),
            },
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("resident registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceHandle<ConfigStoreResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("resident acquire should succeed");

    // Read a value from the config store
    let db_host = handle.values.get("db.host").expect("db.host should exist");
    assert_eq!(db_host, "localhost");

    // A second acquire gets the same shared instance (clone under the hood)
    let handle2: ResourceHandle<ConfigStoreResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");

    assert_eq!(handle2.values.get("db.port").unwrap(), "5432");

    drop(handle);
    drop(handle2);

    manager
        .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_millis(100)))
        .await
        .expect("graceful_shutdown must succeed");
}

// ============================================================================
// USE CASE 3: Database Connection Pool with Resilience + Graceful Shutdown
//
// Goal: pool fake DB connections, add resilience (timeout + retry), spawn
// multiple tasks that acquire simultaneously, then graceful_shutdown.
// ============================================================================

#[derive(Clone)]
struct FakeDbConnection {
    id: u64,
}

#[derive(Clone)]
struct DbConfig {
    host: String,
}

nebula_schema::impl_empty_has_schema!(DbConfig);

impl ResourceConfig for DbConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            return Err(Error::permanent("host must not be empty"));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct DbResource {
    create_count: Arc<AtomicU64>,
    fail_create: Arc<std::sync::atomic::AtomicBool>,
}

impl DbResource {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
            fail_create: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl Resource for DbResource {
    type Config = DbConfig;
    type Runtime = FakeDbConnection;
    type Lease = FakeDbConnection;
    type Error = DxTestError;
    type Auth = ();

    fn key() -> ResourceKey {
        resource_key!("db.connection")
    }

    fn create(
        &self,
        _config: &DbConfig,
        _auth: &(),
        _ctx: &dyn Ctx,
    ) -> impl std::future::Future<Output = Result<FakeDbConnection, DxTestError>> + Send {
        let count = self.create_count.clone();
        let fail = self.fail_create.clone();
        async move {
            if fail.load(Ordering::Relaxed) {
                return Err(DxTestError("connection refused".into()));
            }
            let id = count.fetch_add(1, Ordering::Relaxed);
            Ok(FakeDbConnection { id })
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Pooled for DbResource {
    fn is_broken(&self, _runtime: &FakeDbConnection) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn use_case_3_db_pool_with_resilience_and_shutdown() {
    let db = DbResource::new();
    let pool_rt = PoolRuntime::<DbResource>::new(
        PoolConfig {
            max_size: 5,
            min_size: 1,
            create_timeout: Duration::from_secs(5),
            ..PoolConfig::default()
        },
        0,
    );

    let manager = Arc::new(Manager::new());

    // FRICTION NOTE [RESILIENCE WIRING]: AcquireResilience presets are easy
    // to find and use. `AcquireResilience::standard()` is discoverable from
    // lib.rs re-exports. This part of the API is actually good.
    let resilience = AcquireResilience::fast(); // 10s timeout, 2 retries

    manager
        .register(
            db.clone(),
            DbConfig {
                host: "localhost:5432".into(),
            },
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            Some(resilience),
            None,
        )
        .expect("db registration should succeed");

    // Acquire handles from multiple tasks concurrently
    let mut join_handles = Vec::new();
    for _ in 0..4 {
        let mgr = Arc::clone(&manager);
        join_handles.push(tokio::spawn(async move {
            let ctx = test_ctx();
            let handle: ResourceHandle<DbResource> = mgr
                .acquire_pooled(&(), &ctx, &AcquireOptions::default())
                .await
                .expect("task acquire should succeed");

            let _conn_id = handle.id;
            tokio::time::sleep(Duration::from_millis(20)).await;
            // RAII: handle released on task exit
            drop(handle);
        }));
    }

    for jh in join_handles {
        jh.await.expect("task should complete");
    }

    // FRICTION NOTE [GRACEFUL SHUTDOWN + ARC]: Manager does not implement
    // Clone or take Arc internally. If you want to share a Manager, you wrap
    // it in Arc<Manager>. Then graceful_shutdown takes &self, so Arc::clone
    // lets you hold and call it. This pattern works fine but there's no
    // mention of "wrap in Arc for multi-task sharing" in the docs or README.
    // The README says "Share via `Arc<Manager>` across tasks" only in the
    // struct doc comment — easy to miss.
    // SEVERITY: Minor

    manager
        .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_secs(1)))
        .await
        .expect("graceful_shutdown must succeed");

    // Manager is shut down — new acquires should fail
    // FRICTION NOTE [ResourceHandle NOT DEBUG]: ResourceHandle<R> does not
    // implement Debug. This means you cannot call .expect_err() or .unwrap_err()
    // on a Result<ResourceHandle<R>, E> because those methods require T: Debug.
    // You must use .is_err() + .err().unwrap() instead.
    // This is a significant ergonomics miss — Debug should be derivable or
    // manually implemented on ResourceHandle since it only needs to show the
    // key and topology_tag, not the full lease (which may not be Debug).
    // SEVERITY: Major
    let ctx = test_ctx();
    let result: Result<ResourceHandle<DbResource>, Error> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await;
    assert!(result.is_err());
    assert_eq!(*result.err().unwrap().kind(), ErrorKind::Cancelled);
}

// ============================================================================
// ERROR HANDLING TEST: trigger known error variants
// ============================================================================

#[tokio::test]
async fn error_handling_not_found_on_unregistered_resource() {
    let manager = Manager::new();
    let ctx = test_ctx();

    // FRICTION NOTE [ERROR MATCHING]: ErrorKind uses #[non_exhaustive], which
    // means a match arm requires `_ =>` for forward-compat. This is correct
    // for an evolving library, but means you can't get an exhaustive "did I
    // handle all cases?" compile check. The ErrorKind variants are specific
    // enough (Transient/Permanent/Exhausted/Backpressure/NotFound/Cancelled).
    // SEVERITY: Nit (correct design, just notable)

    let result: Result<ResourceHandle<HttpClientResource>, Error> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await;

    // same ResourceHandle<R>: !Debug issue — must use .err().unwrap()
    assert!(result.is_err());
    let err = result.err().unwrap();

    match err.kind() {
        ErrorKind::NotFound => {}, // expected
        other => panic!("expected NotFound, got {other:?}"),
    }

    // FRICTION NOTE [ERROR DISPLAY]: Display format is "[key] message".
    // But for NotFound, resource_key() is set. You can retrieve the key
    // separately. However, the Display output says
    // "[http.client] resource not found: http.client" — the key appears twice,
    // once as the prefix and once inside the message. Small cosmetic issue.
    let msg = err.to_string();
    assert!(msg.contains("http.client"), "error message: {msg}");
}

#[tokio::test]
async fn error_handling_invalid_config_validate() {
    // FRICTION NOTE [CONFIG VALIDATION]: ResourceConfig::validate() is called
    // inside register() but only if the config impl overrides the default.
    // The default is a no-op. There's no indication in Manager::register()
    // docs that it calls validate() — I had to read the source to confirm.
    // SEVERITY: Minor — should be documented in the # Errors section.

    #[derive(Clone)]
    struct AlwaysInvalidConfig;
    nebula_schema::impl_empty_has_schema!(AlwaysInvalidConfig);
    impl ResourceConfig for AlwaysInvalidConfig {
        fn validate(&self) -> Result<(), Error> {
            Err(Error::permanent("invalid config"))
        }
    }

    // We'd need a full Resource impl to test this, but the pattern is:
    // manager.register(resource, AlwaysInvalidConfig, ...) -> Err(Permanent)
    // Validation happens inside register() before storing.

    // Verify directly instead:
    let config = AlwaysInvalidConfig;
    let result = config.validate();
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::Permanent);
}
