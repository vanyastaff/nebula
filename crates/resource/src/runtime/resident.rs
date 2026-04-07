//! Resident runtime — one shared instance, clone on acquire.
//!
//! The resident runtime holds a single [`Cell`] containing the shared
//! runtime. On acquire, the runtime is cloned into an owned handle.
//! If the runtime is missing or stale, it is (re)created.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::warn;

use crate::cell::Cell;
use crate::ctx::Ctx;
use crate::error::Error;
use crate::handle::ResourceHandle;
use crate::options::AcquireOptions;
use crate::resource::Resource;
use crate::topology::resident::Resident;
use crate::topology::resident::config::Config;
use crate::topology_tag::TopologyTag;

/// Runtime state for a resident topology.
///
/// Holds a single shared runtime instance in a lock-free [`Cell`].
/// On acquire, the runtime is cloned into an owned [`ResourceHandle`].
///
/// A `create_lock` mutex serialises the slow path (create / recreate) while
/// keeping the fast path (load + liveness check) entirely lock-free.
pub struct ResidentRuntime<R: Resource> {
    cell: Cell<R::Runtime>,
    config: Config,
    /// Serialises the create / recreate slow path.
    create_lock: Mutex<()>,
}

impl<R: Resource> ResidentRuntime<R> {
    /// Creates a new resident runtime with the given configuration.
    pub fn new(config: Config) -> Self {
        Self {
            cell: Cell::new(),
            config,
            create_lock: Mutex::new(()),
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns `true` if the cell currently holds a runtime instance.
    pub fn is_initialized(&self) -> bool {
        self.cell.is_some()
    }
}

impl<R> ResidentRuntime<R>
where
    R: Resident + Send + Sync + 'static,
    R::Lease: Clone,
    R::Runtime: Clone + Send + 'static,
{
    /// Acquires a clone of the shared runtime instance.
    ///
    /// **Fast path** (lock-free): load from cell, check liveness, clone.
    ///
    /// **Slow path** (mutex-serialised): create or recreate the runtime.
    /// A double-check after lock acquisition prevents duplicate creates
    /// when multiple callers race on an empty or stale cell.
    ///
    /// # Errors
    ///
    /// Returns an error if creation fails or if the runtime is not alive
    /// and `recreate_on_failure` is disabled.
    pub async fn acquire(
        &self,
        resource: &R,
        resource_config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
        _options: &AcquireOptions,
    ) -> Result<ResourceHandle<R>, Error>
    where
        R::Runtime: Into<R::Lease>,
    {
        // Fast path — lock-free load + liveness check.
        if let Some(existing) = self.cell.load()
            && resource.is_alive_sync(&existing)
        {
            let lease: R::Lease = (*existing).clone().into();
            return Ok(ResourceHandle::owned(
                lease,
                R::key(),
                TopologyTag::Resident,
            ));
        }

        // Slow path — serialise create / recreate.
        let _guard = self.create_lock.lock().await;

        // Double-check: another task may have created while we waited.
        if let Some(existing) = self.cell.load() {
            if resource.is_alive_sync(&existing) {
                let lease: R::Lease = (*existing).clone().into();
                return Ok(ResourceHandle::owned(
                    lease,
                    R::key(),
                    TopologyTag::Resident,
                ));
            }

            // Still not alive — destroy and recreate if configured.
            if !self.config.recreate_on_failure {
                return Err(Error::transient("resident runtime is not alive"));
            }

            // Take the old runtime out and best-effort destroy.
            if let Some(old) = self.cell.take() {
                match Arc::try_unwrap(old) {
                    Ok(owned) => {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(10), resource.destroy(owned))
                                .await;
                    }
                    Err(arc) => {
                        warn!(
                            resource = %R::key(),
                            refs = Arc::strong_count(&arc),
                            "cannot exclusively destroy resident runtime; \
                             another handle still held — dropping Arc"
                        );
                    }
                }
            }
        }

        // Create a new runtime.
        let runtime = match tokio::time::timeout(
            self.config.create_timeout,
            resource.create(resource_config, auth, ctx),
        )
        .await
        {
            Ok(Ok(rt)) => rt,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(Error::transient("resident: create timed out")),
        };

        let lease: R::Lease = runtime.clone().into();
        self.cell.store(Arc::new(runtime));

        Ok(ResourceHandle::owned(
            lease,
            R::key(),
            TopologyTag::Resident,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::BasicCtx;
    use crate::options::AcquireOptions;
    use crate::resource::{ResourceConfig, ResourceMetadata};
    use nebula_core::{ExecutionId, ResourceKey, resource_key};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    #[derive(Clone)]
    struct MockResident {
        alive: Arc<AtomicBool>,
        create_count: Arc<AtomicU32>,
    }

    impl MockResident {
        fn new() -> Self {
            Self {
                alive: Arc::new(AtomicBool::new(true)),
                create_count: Arc::new(AtomicU32::new(0)),
            }
        }
    }

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

    impl ResourceConfig for bool {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Resource for MockResident {
        type Config = bool;
        type Runtime = u32;
        type Lease = u32;
        type Error = MockError;
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("mock-resident")
        }

        fn create(
            &self,
            _config: &bool,
            _auth: &(),
            _ctx: &dyn Ctx,
        ) -> impl std::future::Future<Output = Result<u32, MockError>> + Send {
            let count = self.create_count.fetch_add(1, Ordering::Relaxed);
            async move {
                // Yield to increase the chance of concurrent interleaving.
                tokio::task::yield_now().await;
                Ok(count + 100)
            }
        }

        fn destroy(
            &self,
            _runtime: u32,
        ) -> impl std::future::Future<Output = Result<(), MockError>> + Send {
            async { Ok(()) }
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for MockResident {
        fn is_alive_sync(&self, _runtime: &u32) -> bool {
            self.alive.load(Ordering::Relaxed)
        }
    }

    fn test_ctx() -> BasicCtx {
        BasicCtx::new(ExecutionId::new())
    }

    #[tokio::test]
    async fn concurrent_acquire_creates_only_once() {
        let resource = MockResident::new();
        let rt = Arc::new(ResidentRuntime::<MockResident>::new(Config::default()));
        let ctx = Arc::new(test_ctx());

        // Spawn 10 concurrent acquires on an empty cell.
        let mut handles = Vec::new();
        for _ in 0..10 {
            let r = resource.clone();
            let runtime = Arc::clone(&rt);
            let c = Arc::clone(&ctx);
            handles.push(tokio::spawn(async move {
                runtime
                    .acquire(&r, &true, &(), c.as_ref(), &AcquireOptions::default())
                    .await
                    .unwrap()
            }));
        }

        for h in handles {
            let _ = h.await.unwrap();
        }

        // Only one create should have happened.
        assert_eq!(
            resource.create_count.load(Ordering::Relaxed),
            1,
            "concurrent acquires on empty cell should create exactly once"
        );
    }

    #[tokio::test]
    async fn acquire_creates_on_first_call() {
        let resource = MockResident::new();
        let rt = ResidentRuntime::<MockResident>::new(Config::default());
        let ctx = test_ctx();

        let handle = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        assert_eq!(*handle, 100);
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn acquire_reuses_existing_instance() {
        let resource = MockResident::new();
        let rt = ResidentRuntime::<MockResident>::new(Config::default());
        let ctx = test_ctx();

        let h1 = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        let h2 = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await
            .unwrap();

        // Both should have the same value — only one create.
        assert_eq!(*h1, *h2);
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn acquire_recreates_when_not_alive_and_configured() {
        let resource = MockResident::new();
        let config = Config {
            recreate_on_failure: true,
            ..Default::default()
        };
        let rt = ResidentRuntime::<MockResident>::new(config);
        let ctx = test_ctx();

        // First acquire — creates.
        let h1 = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        assert_eq!(*h1, 100);

        // Mark as not alive.
        resource.alive.store(false, Ordering::Relaxed);

        // Second acquire — should recreate.
        resource.alive.store(true, Ordering::Relaxed);
        // Need to mark not alive for the check, then alive for the new instance.
        resource.alive.store(false, Ordering::Relaxed);
        // Actually, after recreate the new instance will be checked on next acquire.
        // Let's just test that recreate happens.
        resource.alive.store(true, Ordering::Relaxed);

        // Mark not alive so existing is rejected.
        resource.alive.store(false, Ordering::Relaxed);
        // The acquire will destroy old, create new. The new one won't be checked
        // via is_alive_sync on the same acquire call — it's stored and returned.
        let h2 = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        assert_eq!(*h2, 101); // Second creation.
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 2);
    }

    // A resource whose `create()` never returns — for timeout tests.
    #[derive(Clone)]
    struct HangingResident;

    impl Resource for HangingResident {
        type Config = bool;
        type Runtime = u32;
        type Lease = u32;
        type Error = MockError;
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("hanging-resident")
        }

        fn create(
            &self,
            _config: &bool,
            _auth: &(),
            _ctx: &dyn Ctx,
        ) -> impl std::future::Future<Output = Result<u32, MockError>> + Send {
            async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(0)
            }
        }

        fn destroy(
            &self,
            _runtime: u32,
        ) -> impl std::future::Future<Output = Result<(), MockError>> + Send {
            async { Ok(()) }
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for HangingResident {}

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resident_create_timeout_does_not_deadlock() {
        let resource = HangingResident;
        let config = Config {
            create_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let rt = Arc::new(ResidentRuntime::<HangingResident>::new(config));
        let ctx = Arc::new(test_ctx());

        // First acquire should fail quickly with a timeout, not hang.
        let result = rt
            .acquire(
                &resource,
                &true,
                &(),
                ctx.as_ref(),
                &AcquireOptions::default(),
            )
            .await;
        assert!(result.is_err(), "first acquire should time out");

        // Second acquire must also fail quickly — the create_lock must have
        // been released after the first timeout.
        let result = rt
            .acquire(
                &resource,
                &true,
                &(),
                ctx.as_ref(),
                &AcquireOptions::default(),
            )
            .await;
        assert!(
            result.is_err(),
            "second acquire should time out (lock released)"
        );
    }

    #[tokio::test]
    async fn acquire_fails_when_not_alive_and_no_recreate() {
        let resource = MockResident::new();
        let config = Config {
            recreate_on_failure: false,
            ..Default::default()
        };
        let rt = ResidentRuntime::<MockResident>::new(config);
        let ctx = test_ctx();

        // First acquire — creates.
        let _h1 = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await
            .unwrap();

        // Mark as not alive.
        resource.alive.store(false, Ordering::Relaxed);

        // Second acquire — should fail.
        let result = rt
            .acquire(&resource, &true, &(), &ctx, &AcquireOptions::default())
            .await;
        assert!(result.is_err());
    }
}
