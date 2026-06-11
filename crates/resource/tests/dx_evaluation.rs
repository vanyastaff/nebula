/// DX evaluation test — newcomer experience with nebula-resource
///
/// Rules: Only used public docs (README.md + lib.rs + doc comments).
/// Did NOT read internal modules or .claude/ context files.
///
/// All friction points are documented inline with [FRICTION] tags.
// ---------------------------------------------------------------------------
// Common error / config plumbing (unavoidable boilerplate)
// ---------------------------------------------------------------------------
use std::sync::Arc;

use nebula_core::{ExecutionId, ResourceKey};
use nebula_resource::{
    AcquireOptions, Manager, PoolConfig, Provider, RegistrationSpec, ResidentConfig,
    ResourceConfig, ResourceContext, ScopeLevel, ShutdownConfig, SlotIdentity,
    error::{Error, ErrorKind},
    resource::HasCredentialSlots,
    resource_key,
    runtime::{TopologyRuntime, pool::PoolRuntime, resident::ResidentRuntime},
    topology::{
        pooled::{BrokenCheck, Pooled},
        resident::Resident,
    },
};
use tokio_util::sync::CancellationToken;

// [FRICTION #1] ResourceConfig has no blanket impl for "accept everything".
// Every type that implements Resource needs a Config, and Config must impl
// ResourceConfig. Even if my config is just a URL, I can't use `String`
// directly — I must wrap it.
//
// I expected: something like `impl ResourceConfig for String {}` or at least
// a `#[derive(ResourceConfig)]` to generate a zero-overhead impl.

// [FRICTION #2] There is no blanket `impl From<anyhow::Error> for Error`.
// For a quick prototype a custom error type AND a From impl were previously
// required — 8+ lines before the resource itself. With the redesigned
// `Resource` trait (no `type Error` associated type) the lifecycle methods
// return `crate::Error` directly, so this boilerplate is gone.

// ---------------------------------------------------------------------------
// Use Case 1: Pooled HTTP client
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct HttpConfig {
    base_url: String,
}

nebula_schema::impl_empty_has_schema!(HttpConfig);

impl ResourceConfig for HttpConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.base_url.is_empty() {
            return Err(Error::permanent("base_url must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.base_url.hash(&mut h);
        h.finish()
    }
}

/// The "live" handle — a thin wrapper around a URL.
/// In a real app this would be reqwest::Client.
#[derive(Clone, Debug)]
struct HttpClient {
    base_url: String,
}

/// The resource descriptor — zero-size marker.
#[derive(Clone)]
struct HttpResource;

// [FRICTION #3] Previously 5 associated types. With `type Lease` and
// `type Error` removed from the trait, the impl is now just `Config` and
// `Runtime` — the two that actually matter for a simple pooled resource.

impl Provider for HttpResource {
    type Config = HttpConfig;
    type Instance = HttpClient;

    fn key() -> ResourceKey {
        resource_key!("http.client")
    }

    async fn create(
        &self,
        config: &HttpConfig,
        _ctx: &ResourceContext,
    ) -> Result<HttpClient, Error> {
        Ok(HttpClient {
            base_url: config.base_url.clone(),
        })
    }
}

impl HasCredentialSlots for HttpResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl Pooled for HttpResource {
    // [FRICTION #4] Old API was `recycle_decision` + `broken_check` (methods
    // returning the enum value). New API is `is_broken` (returning BrokenCheck)
    // and `recycle` (async fn returning Result<RecycleDecision>).
    //
    // The README Quick-Start still shows the OLD names:
    //   fn recycle_decision(&self, ...) -> RecycleDecision
    //   fn broken_check(&self) -> BrokenCheck
    //
    // Actual trait has:
    //   fn is_broken(&self, runtime) -> BrokenCheck
    //   fn recycle(&self, runtime, metrics) -> impl Future<...>
    //
    // This is a BLOCKER — the README example doesn't compile.
    fn is_broken(&self, _runtime: &HttpClient) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

#[tokio::test]
async fn use_case_1_pooled_http_client() {
    let manager = Manager::new();

    let config = HttpConfig {
        base_url: "https://api.example.com".into(),
    };
    let fingerprint = config.fingerprint();
    manager
        .register(RegistrationSpec {
            resource: HttpResource,
            config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(PoolRuntime::<HttpResource>::new(
                PoolConfig::default(),
                fingerprint,
            )),
            acquire: Manager::erased_acquire_pooled_for::<HttpResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // [FRICTION #5] README shows: BasicCtx::new(ScopeLevel::Global)
    // Actual API:  BasicCtx::new(ExecutionId)  (scope defaults to Global)
    //
    // The example in README is just wrong — this is a BLOCKER for newcomers.
    // ScopeLevel isn't even passed to new(), it's set via with_scope().
    let ctx = ResourceContext::minimal(
        nebula_core::scope::Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    );

    let opts = AcquireOptions::default();

    let handle = manager
        .acquire_pooled::<HttpResource>(&ctx, &opts)
        .await
        .expect("acquire should succeed");
    // but doesn't say what Lease looks like in the pooled case.
    // I guessed HttpClient — worked because Lease = Runtime = HttpClient.
    // [FRICTION #6] Deref to what? The doc says "handle is Deref to the Lease"

    assert_eq!(handle.base_url, "https://api.example.com");
    drop(handle);
}

#[tokio::test]
async fn use_case_1_invalid_config_is_rejected() {
    // The single `register` funnel calls `config.validate()` before the row
    // is installed, so an invalid config is rejected at registration time
    // exactly as the docs promise — there is no longer a per-topology
    // shorthand that skipped validation and let a bad config slip through
    // to `create()`. Empty `base_url` fails `HttpConfig::validate`.
    let manager = Manager::new();
    let config = HttpConfig {
        base_url: String::new(),
    };
    let fingerprint = config.fingerprint();
    let result = manager.register(RegistrationSpec {
        resource: HttpResource,
        config,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(PoolRuntime::<HttpResource>::new(
            PoolConfig::default(),
            fingerprint,
        )),
        acquire: Manager::erased_acquire_pooled_for::<HttpResource>(),
        recovery_gate: None,
    });
    let err = result.expect_err("empty base_url must fail validation at register time");
    assert!(
        err.to_string().contains("base_url must not be empty"),
        "expected the ResourceConfig::validate message, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Use Case 2: Resident config store
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ConfigStoreConfig {
    env: String,
}

nebula_schema::impl_empty_has_schema!(ConfigStoreConfig);

impl ResourceConfig for ConfigStoreConfig {
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.env.hash(&mut h);
        h.finish()
    }
}

/// Our config store runtime — an in-memory map.
#[derive(Clone, Debug)]
struct ConfigStore {
    env: String,
    values: Arc<std::collections::HashMap<String, String>>,
}

#[derive(Clone)]
struct ConfigStoreResource;

impl Provider for ConfigStoreResource {
    type Config = ConfigStoreConfig;
    type Instance = ConfigStore;

    fn key() -> ResourceKey {
        resource_key!("config.store")
    }

    async fn create(
        &self,
        config: &ConfigStoreConfig,
        _ctx: &ResourceContext,
    ) -> Result<ConfigStore, Error> {
        let mut map = std::collections::HashMap::new();
        map.insert("environment".to_string(), config.env.clone());
        map.insert("log_level".to_string(), "info".to_string());
        Ok(ConfigStore {
            env: config.env.clone(),
            values: Arc::new(map),
        })
    }
}

impl HasCredentialSlots for ConfigStoreResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

// [FRICTION #8] Resident trait has a `where Self::Lease: Clone` bound on the
// trait itself. This is fine, my Lease is Clone. But it's an unusual pattern —
// most Rust traits don't have a bound on an associated type of Self in the
// trait definition. The compiler error if you forget Clone on Lease is cryptic:
//   "the trait bound `ConfigStore: Clone` is not satisfied"
// pointing to the trait impl, not the where clause source.
impl Resident for ConfigStoreResource {}

#[tokio::test]
async fn use_case_2_resident_config_store() {
    let manager = Manager::new();

    // [FRICTION #9] ResidentConfig exists but has no convenience constructor.
    // ResidentConfig::default() works but I had to check the source to verify
    // the defaults are sensible (recreate_on_failure: false, create_timeout: 30s).
    // At least it has Default, which is good.
    manager
        .register(RegistrationSpec {
            resource: ConfigStoreResource,
            config: ConfigStoreConfig {
                env: "production".into(),
            },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(ResidentRuntime::<ConfigStoreResource>::new(
                ResidentConfig::default(),
            )),
            acquire: Manager::erased_acquire_resident_for::<ConfigStoreResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = ResourceContext::minimal(
        nebula_core::scope::Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let opts = AcquireOptions::default();

    let handle = manager
        .acquire_resident::<ConfigStoreResource>(&ctx, &opts)
        .await
        .expect("acquire should succeed");

    // The first acquire triggers lazy creation — handle contains ConfigStore
    assert_eq!(handle.env, "production");
    assert_eq!(
        handle.values.get("log_level").map(String::as_str),
        Some("info")
    );

    // Second acquire returns a clone of the same instance
    let handle2 = manager
        .acquire_resident::<ConfigStoreResource>(&ctx, &opts)
        .await
        .expect("second acquire should succeed");

    assert_eq!(handle2.env, "production");
    drop(handle);
    drop(handle2);
}

// ---------------------------------------------------------------------------
// Use Case 3: DB connection with resilience + graceful_shutdown
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct DbConfig {
    dsn: String,
    pool_size: u32,
}

nebula_schema::impl_empty_has_schema!(DbConfig);

impl ResourceConfig for DbConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.dsn.is_empty() {
            return Err(Error::permanent("dsn must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.dsn.hash(&mut h);
        self.pool_size.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone, Debug)]
struct DbConnection {
    id: u64,
}

#[derive(Clone)]
struct DbResource {
    counter: Arc<AtomicU64>,
}

impl DbResource {
    fn new() -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Provider for DbResource {
    type Config = DbConfig;
    type Instance = DbConnection;

    fn key() -> ResourceKey {
        resource_key!("db.connection")
    }

    async fn create(
        &self,
        _config: &DbConfig,
        _ctx: &ResourceContext,
    ) -> Result<DbConnection, Error> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        Ok(DbConnection { id })
    }
}

impl HasCredentialSlots for DbResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl Pooled for DbResource {}

#[tokio::test]
async fn use_case_3_db_with_recovery_and_shutdown() {
    let manager = Arc::new(Manager::new());

    // Recovery gate is one `Option` field on the single `RegistrationSpec`
    // — the same one struct every registration uses. Mythos v2: manager
    // owns no acquire-resilience knob; retry composes one layer up.
    use nebula_resource::ResourceConfig as _;

    let config = DbConfig {
        dsn: "postgres://localhost/test".into(),
        pool_size: 5,
    };
    let fingerprint = config.fingerprint();
    let pool_config = PoolConfig::default();

    manager
        .register(RegistrationSpec {
            resource: DbResource::new(),
            config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(PoolRuntime::<DbResource>::new(
                pool_config,
                fingerprint,
            )),
            acquire: Manager::erased_acquire_pooled_for::<DbResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // Acquire in multiple tasks
    let m1 = Arc::clone(&manager);
    let m2 = Arc::clone(&manager);

    let t1 = tokio::spawn(async move {
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        );
        let opts = AcquireOptions::default();
        let h = m1.acquire_pooled::<DbResource>(&ctx, &opts).await.unwrap();
        assert!(h.id < 100); // just a sanity check
    });

    let t2 = tokio::spawn(async move {
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        );
        let opts = AcquireOptions::default();
        let h = m2.acquire_pooled::<DbResource>(&ctx, &opts).await.unwrap();
        assert!(h.id < 100);
    });

    t1.await.unwrap();
    t2.await.unwrap();

    // Graceful shutdown
    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_secs(5)),
        )
        .await
        .expect("graceful_shutdown must succeed with no outstanding handles");

    assert!(manager.is_shutdown());
}

fn make_test_ctx() -> ResourceContext {
    ResourceContext::minimal(
        nebula_core::scope::Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    )
}

// ---------------------------------------------------------------------------
// Error handling exploration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_not_found_on_missing_resource() {
    let manager = Manager::new();
    let ctx = make_test_ctx();
    let opts = AcquireOptions::default();

    let err = manager
        .acquire_pooled::<HttpResource>(&ctx, &opts)
        .await
        .unwrap_err();

    // [FRICTION #11] ErrorKind is re-exported from the crate root but the match
    // requires importing error::ErrorKind or using the re-export. The re-export
    // IS present in lib.rs, but the README Error Handling section only shows
    // err.is_retryable() — it doesn't show how to pattern-match on ErrorKind.
    // I had to guess the import path.
    assert!(matches!(err.kind(), ErrorKind::NotFound));
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn error_cancelled_after_shutdown() {
    let manager = Manager::new();
    let config = HttpConfig {
        base_url: "https://example.com".into(),
    };
    let fingerprint = config.fingerprint();
    manager
        .register(RegistrationSpec {
            resource: HttpResource,
            config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(PoolRuntime::<HttpResource>::new(
                PoolConfig::default(),
                fingerprint,
            )),
            acquire: Manager::erased_acquire_pooled_for::<HttpResource>(),
            recovery_gate: None,
        })
        .unwrap();

    manager.shutdown();

    let ctx = make_test_ctx();

    let opts = AcquireOptions::default();

    let err = manager
        .acquire_pooled::<HttpResource>(&ctx, &opts)
        .await
        .unwrap_err();
    // returns false for Cancelled, which is correct. The API is clean here.
    assert!(matches!(err.kind(), ErrorKind::Cancelled));
    assert!(!err.is_retryable());
}
