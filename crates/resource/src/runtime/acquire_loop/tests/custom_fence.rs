//! A custom topology has no fence logic; the framework rejects stale checkout.

use super::*;

#[derive(Clone, Default)]
struct CustomResource {
    created: Arc<AtomicU64>,
    destroyed: Arc<AtomicU64>,
    destroy_finished: Arc<Notify>,
}

#[async_trait::async_trait]
impl Provider for CustomResource {
    type Config = PoolCfg;
    type Instance = u64;
    type Topology = CustomTopology;

    fn key() -> ResourceKey {
        resource_key!("custom-checkout-fence")
    }

    async fn create(&self, _: &PoolCfg, _: &ResourceContext) -> Result<u64, Error> {
        Ok(self.created.fetch_add(1, Ordering::SeqCst))
    }

    async fn destroy(&self, _: u64, _: TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        self.destroy_finished.notify_one();
        Ok(())
    }
}

crate::no_credential_slots!(CustomResource);

#[derive(Default)]
struct CustomTopology {
    panic_on_entry: Option<u64>,
    panic_on_guard_tag: Arc<AtomicBool>,
}

impl Topology<CustomResource> for CustomTopology {
    type Entry = u64;

    fn try_reserve(
        &self,
        _: &InstanceStore<u64>,
    ) -> Result<crate::topology::Ticket, crate::topology::Unavailable> {
        Ok(crate::topology::Ticket::infallible())
    }

    async fn create_entry(
        &self,
        resource: &CustomResource,
        config: &PoolCfg,
        ctx: &ResourceContext,
        _: &crate::RetainedStore<u64>,
    ) -> Result<crate::topology::CreatedEntry<u64>, Error> {
        resource
            .create(config, ctx)
            .await
            .map(crate::topology::CreatedEntry::new)
    }

    fn entry_instance<'entry>(&self, entry: &'entry u64) -> &'entry u64 {
        entry
    }

    fn into_owned_instance(&self, entry: u64) -> Option<u64> {
        Some(entry)
    }

    fn idle_evictable(&self, entry: &u64) -> bool {
        assert_ne!(
            self.panic_on_entry,
            Some(*entry),
            "intentional idle predicate panic"
        );
        self.panic_on_entry.is_some()
    }

    fn tag(&self) -> crate::TopologyTag {
        assert!(
            !self.panic_on_guard_tag.load(Ordering::SeqCst),
            "intentional guard metadata panic"
        );
        crate::TopologyTag::Custom
    }

    fn pools(&self) -> bool {
        true
    }
}

async fn acquire(manager: &Arc<crate::Manager>) -> ResourceGuard<CustomResource> {
    *crate::Manager::acquire_any(
        Arc::clone(manager),
        &CustomResource::key(),
        &test_ctx(),
        &AcquireOptions::default(),
        &crate::SlotIdentity::Unbound,
    )
    .await
    .unwrap()
    .downcast::<ResourceGuard<CustomResource>>()
    .unwrap()
}

#[tokio::test]
async fn custom_topology_checkout_destroys_recycled_entry_after_epoch_only_revoke() {
    let manager = Arc::new(crate::Manager::new());
    let resource = CustomResource::default();
    manager
        .register(crate::RegistrationSpec {
            resource: resource.clone(),
            config: PoolCfg,
            scope: crate::ScopeLevel::Global,
            slot_identity: crate::SlotIdentity::Unbound,
            topology: CustomTopology::default(),
            recovery_gate: None,
        })
        .unwrap();
    let first = acquire(&manager).await;
    assert_eq!(*first, 0);
    let _release_outcome = first.release().await.unwrap();
    let row = manager
        .lookup::<CustomResource>(&crate::ScopeLevel::Global)
        .unwrap();
    assert_eq!(
        row.store.len().await,
        1,
        "the actual framework idle store must contain the released entry"
    );
    row.bump_revoke_epoch();
    let replacement = acquire(&manager).await;
    assert_eq!(
        *replacement, 1,
        "checkout must never lease the stale entry again"
    );
    assert_eq!(resource.created.load(Ordering::SeqCst), 2);
    resource.destroy_finished.notified().await;
    assert_eq!(
        resource.destroyed.load(Ordering::SeqCst),
        1,
        "the checkout fence, not terminal shutdown, must destroy the stale entry"
    );
    let _release_outcome = replacement.release().await.unwrap();
    manager
        .graceful_shutdown(crate::ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(resource.destroyed.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn panicking_idle_predicate_preserves_all_owned_entries_for_teardown() {
    let manager = Arc::new(crate::Manager::with_config(
        crate::ManagerConfig::default().with_release_queue_workers(1),
    ));
    let resource = CustomResource::default();
    manager
        .register(crate::RegistrationSpec {
            resource: resource.clone(),
            config: PoolCfg,
            scope: crate::ScopeLevel::Global,
            slot_identity: crate::SlotIdentity::Unbound,
            topology: CustomTopology {
                panic_on_entry: Some(1),
                ..Default::default()
            },
            recovery_gate: None,
        })
        .unwrap();
    let first = acquire(&manager).await;
    let second = acquire(&manager).await;
    let third = acquire(&manager).await;
    let _first_outcome = first.release().await.unwrap();
    let _second_outcome = second.release().await.unwrap();
    let _third_outcome = third.release().await.unwrap();
    let row = manager
        .lookup::<CustomResource>(&crate::ScopeLevel::Global)
        .unwrap();
    let sweeping = Arc::clone(&row);
    assert_eq!(
        tokio::spawn(async move { sweeping.run_maintenance().await })
            .await
            .expect("author predicate panic must be isolated without unwinding the store drain"),
        3
    );
    assert_eq!(
        row.release_queue
            .submit_release(|| Box::pin(async { Ok(()) }))
            .expect("open queue must accept cleanup checkpoint")
            .wait()
            .await
            .unwrap(),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        resource.destroyed.load(Ordering::SeqCst),
        3,
        "earlier, panicking, and later entries all reach teardown"
    );
    assert_eq!(row.release_queue.dropped_count(), 0);
    manager
        .graceful_shutdown(crate::ShutdownConfig::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn panicking_guard_metadata_keeps_created_entry_armed() {
    let manager = Arc::new(crate::Manager::with_config(
        crate::ManagerConfig::default().with_release_queue_workers(1),
    ));
    let resource = CustomResource::default();
    let panic_tag = Arc::new(AtomicBool::new(false));
    manager
        .register(crate::RegistrationSpec {
            resource: resource.clone(),
            config: PoolCfg,
            scope: crate::ScopeLevel::Global,
            slot_identity: crate::SlotIdentity::Unbound,
            topology: CustomTopology {
                panic_on_guard_tag: Arc::clone(&panic_tag),
                ..Default::default()
            },
            recovery_gate: None,
        })
        .unwrap();
    let row = manager
        .lookup::<CustomResource>(&crate::ScopeLevel::Global)
        .unwrap();
    panic_tag.store(true, Ordering::SeqCst);
    let acquiring = Arc::clone(&row);
    assert!(
        tokio::spawn(async move {
            acquiring
                .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
                .await
        })
        .await
        .is_err()
    );
    assert_eq!(
        row.release_queue
            .submit_release(|| Box::pin(async { Ok(()) }))
            .expect("open queue must accept cleanup checkpoint")
            .wait()
            .await
            .unwrap(),
        SubmissionOutcome::Completed,
    );
    assert_eq!(resource.created.load(Ordering::SeqCst), 1);
    assert_eq!(
        resource.destroyed.load(Ordering::SeqCst),
        1,
        "metadata panic must not drop the raw created entry"
    );
    assert_eq!(row.release_queue.dropped_count(), 0);
    panic_tag.store(false, Ordering::SeqCst);
    manager
        .graceful_shutdown(crate::ShutdownConfig::default())
        .await
        .unwrap();
}
