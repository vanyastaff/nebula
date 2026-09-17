use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use nebula_core::{CredentialKey, NodeKey};

use super::*;

/// Test fixture: scoped map that holds a single registered key with a
/// boxed marker payload.
#[derive(Debug)]
struct OneKeyScopedMap {
    registered: ResourceKey,
    payload_marker: u64,
    hits: AtomicUsize,
}

impl OneKeyScopedMap {
    fn new(registered: ResourceKey, payload_marker: u64) -> Self {
        Self {
            registered,
            payload_marker,
            hits: AtomicUsize::new(0),
        }
    }
}

impl ScopedResourceMap for OneKeyScopedMap {
    fn lookup_in_ancestors<'a>(
        &'a self,
        key: &'a ResourceKey,
    ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>> {
        Box::pin(async move {
            if key == &self.registered {
                self.hits.fetch_add(1, Ordering::SeqCst);
                Ok(Some(Box::new(self.payload_marker) as ScopedLookup))
            } else {
                Ok(None)
            }
        })
    }

    fn has_in_ancestors(&self, key: &ResourceKey) -> bool {
        key == &self.registered
    }
}

/// Test fixture: global accessor that stores keyed `u64` markers.
struct TestGlobalAccessor {
    registered: ResourceKey,
    payload_marker: u64,
    hits: AtomicUsize,
}

impl TestGlobalAccessor {
    fn new(registered: ResourceKey, payload_marker: u64) -> Self {
        Self {
            registered,
            payload_marker,
            hits: AtomicUsize::new(0),
        }
    }
}

impl fmt::Debug for TestGlobalAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestGlobalAccessor")
            .field("registered", &self.registered)
            .finish()
    }
}

impl ResourceAccessor for TestGlobalAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        key == &self.registered
    }

    fn acquire_any(&self, key: &ResourceKey) -> BoxFut<'_, Result<ScopedLookup, CoreError>> {
        let key_owned = key.clone();
        Box::pin(async move {
            if key_owned == self.registered {
                self.hits.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(self.payload_marker) as ScopedLookup)
            } else {
                Err(CoreError::credential_not_found(
                    CredentialKey::new(key_owned.as_str())
                        .expect("ResourceKey format is CredentialKey-compatible"),
                ))
            }
        })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<ScopedLookup>, CoreError>> {
        let key_owned = key.clone();
        Box::pin(async move {
            if key_owned == self.registered {
                Ok(Some(Box::new(self.payload_marker) as ScopedLookup))
            } else {
                Ok(None)
            }
        })
    }
}

fn rk(key: &str) -> ResourceKey {
    ResourceKey::new(key).expect("valid resource key in test")
}

fn marker(boxed: ScopedLookup) -> u64 {
    *boxed
        .downcast::<u64>()
        .expect("test fixture stores u64 markers")
}

fn arc_marker(boxed: ScopedLookup) -> u64 {
    let arc = boxed
        .downcast::<Arc<dyn Any + Send + Sync>>()
        .expect("Dash storage hands back Arc-payloads");
    let v: &u64 = arc
        .downcast_ref::<u64>()
        .expect("test fixture stores u64 markers");
    *v
}

fn b(name: &str) -> BranchId {
    BranchId::from_node_key(NodeKey::new(name).expect("valid node key in test"))
}

// ── EmptyScopedResourceMap (Phase 6 default) ─────────────────────────

#[tokio::test]
async fn empty_scoped_map_always_misses() {
    let map = EmptyScopedResourceMap;
    let key = rk("postgres");
    assert!(!map.has_in_ancestors(&key));
    assert!(map.lookup_in_ancestors(&key).await.unwrap().is_none());
}

// ── DashScopedResourceMap basic API ──────────────────────────────────

#[tokio::test]
async fn dash_register_branch_idempotent() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    map.register_branch(root.clone(), None);
    map.register_branch(root.clone(), None); // re-register
    assert!(
        map.pop(&root).is_some(),
        "re-registration must keep the entry alive"
    );
}

#[tokio::test]
async fn dash_push_returns_false_for_unregistered_branch() {
    let map = DashScopedResourceMap::new();
    let unknown = b("unknown");
    let pushed = map.push(unknown, rk("postgres"), Arc::new(0xaaaau64));
    assert!(
        !pushed,
        "push to unregistered branch must reject (engine invariant)"
    );
}

#[tokio::test]
async fn dash_push_then_lookup_at_same_branch() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    map.register_branch(root.clone(), None);
    map.push(root.clone(), rk("postgres"), Arc::new(0xaaaau64));

    let payload = map
        .lookup_in_ancestors_from(&root, &rk("postgres"))
        .unwrap()
        .expect("registered key must be found");
    assert_eq!(arc_marker(payload), 0xaaaa);
}

#[tokio::test]
async fn dash_pop_returns_entries_lifo() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    map.register_branch(root.clone(), None);
    map.push(root.clone(), rk("postgres"), Arc::new(1u64));
    map.push(root.clone(), rk("redis"), Arc::new(2u64));
    map.push(root.clone(), rk("kafka"), Arc::new(3u64));

    let popped = map.pop(&root).expect("registered branch yields entries");
    let order: Vec<&str> = popped.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(
        order,
        vec!["kafka", "redis", "postgres"],
        "pop must return entries in reverse registration order (LIFO)"
    );
}

#[tokio::test]
async fn dash_pop_unknown_branch_returns_none() {
    let map = DashScopedResourceMap::new();
    assert!(map.pop(&b("never-registered")).is_none());
}

#[tokio::test]
async fn dash_pop_empty_branch_returns_some_empty() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    map.register_branch(root.clone(), None);
    let popped = map.pop(&root).expect("registered branch always pops");
    assert!(popped.is_empty(), "empty branch yields empty Vec");
}

// ── Three-hop nested shadowing (Task 7.5 #1) ─────────────────────────

#[tokio::test]
async fn dash_three_hop_nested_shadowing_closest_wins() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    let lvl1 = b("lvl1");
    let lvl2 = b("lvl2");
    let lvl3 = b("lvl3");

    map.register_branch(root.clone(), None);
    map.register_branch(lvl1.clone(), Some(root.clone()));
    map.register_branch(lvl2.clone(), Some(lvl1.clone()));
    map.register_branch(lvl3.clone(), Some(lvl2.clone()));

    // Register the same key at root and at lvl2.
    map.push(root, rk("postgres"), Arc::new(0xa110_u64));
    map.push(lvl2, rk("postgres"), Arc::new(0xb220_u64));

    // From lvl3, the closest ancestor with `postgres` is lvl2.
    let p3 = map
        .lookup_in_ancestors_from(&lvl3, &rk("postgres"))
        .unwrap()
        .expect("expected hit walking lvl3 → lvl2");
    assert_eq!(arc_marker(p3), 0xb220);

    // From lvl1, only root has `postgres`.
    let p1 = map
        .lookup_in_ancestors_from(&lvl1, &rk("postgres"))
        .unwrap()
        .expect("expected hit walking lvl1 → root");
    assert_eq!(arc_marker(p1), 0xa110);
}

#[tokio::test]
async fn dash_pop_does_not_affect_parent_entries() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    let child = b("child");
    map.register_branch(root.clone(), None);
    map.register_branch(child.clone(), Some(root.clone()));
    map.push(root.clone(), rk("postgres"), Arc::new(0xa1_u64));
    map.push(child.clone(), rk("redis"), Arc::new(0xb2_u64));

    // Drop the child; root must remain visible.
    let _ = map.pop(&child);

    let p = map
        .lookup_in_ancestors_from(&root, &rk("postgres"))
        .unwrap()
        .expect("root entry must outlive child pop");
    assert_eq!(arc_marker(p), 0xa1);

    // The child's parent pointer is gone, but root still works.
    assert!(
        map.lookup_in_ancestors_from(&root, &rk("redis"))
            .unwrap()
            .is_none(),
        "child's redis must not leak up into root scope"
    );
}

#[tokio::test]
async fn dash_set_current_branch_routes_trait_lookup() {
    let map = DashScopedResourceMap::new();
    let root = b("root");
    let leaf = b("leaf");
    map.register_branch(root.clone(), None);
    map.register_branch(leaf.clone(), Some(root.clone()));
    map.push(root.clone(), rk("postgres"), Arc::new(0xa1_u64));

    // Without `set_current_branch`, the trait-level lookup misses.
    assert!(
        map.lookup_in_ancestors(&rk("postgres"))
            .await
            .unwrap()
            .is_none()
    );

    // After stamping, the leaf walk finds `postgres` in root.
    map.set_current_branch(Some(leaf));
    let payload = map
        .lookup_in_ancestors(&rk("postgres"))
        .await
        .unwrap()
        .expect("current_branch walk should hit root");
    assert_eq!(arc_marker(payload), 0xa1);
}

#[tokio::test]
async fn dash_lookup_bounded_by_max_ancestor_depth() {
    // Construct a long chain that should still terminate via cycle
    // detection / depth cap if anything goes wrong.
    let map = DashScopedResourceMap::new();
    let names: Vec<BranchId> = (0..8)
        .map(|i| {
            BranchId::from_node_key(NodeKey::new(format!("n{i}")).expect("valid node key in test"))
        })
        .collect();
    for (i, n) in names.iter().enumerate() {
        let parent = if i == 0 {
            None
        } else {
            Some(names[i - 1].clone())
        };
        map.register_branch(n.clone(), parent);
    }
    map.push(names[0].clone(), rk("postgres"), Arc::new(42u64));

    // From the deepest branch, the walk should terminate at the root
    // and return the payload in finite time.
    let p = map
        .lookup_in_ancestors_from(names.last().unwrap(), &rk("postgres"))
        .unwrap()
        .expect("walk must terminate at root");
    assert_eq!(arc_marker(p), 42);
}

// ── Layered accessor (Phase 6 contract preservation) ─────────────────

#[tokio::test]
async fn scoped_only_hit_returns_scoped_payload() {
    let key = rk("postgres");
    let scoped = Arc::new(OneKeyScopedMap::new(key.clone(), 0xaaaa));
    let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
    let layered = LayeredResourceAccessor::new(scoped.clone(), global.clone());

    let payload = layered.acquire_any(&key).await.unwrap();
    assert_eq!(marker(payload), 0xaaaa);
    assert_eq!(scoped.hits.load(Ordering::SeqCst), 1);
    assert_eq!(global.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn global_only_hit_falls_through() {
    let scoped_key = rk("postgres");
    let global_key = rk("redis");
    let scoped = Arc::new(OneKeyScopedMap::new(scoped_key, 0xaaaa));
    let global = Arc::new(TestGlobalAccessor::new(global_key.clone(), 0xbbbb));
    let layered = LayeredResourceAccessor::new(scoped.clone(), global.clone());

    let payload = layered.acquire_any(&global_key).await.unwrap();
    assert_eq!(marker(payload), 0xbbbb);
    assert_eq!(global.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn scoped_wins_over_global_at_same_key() {
    let key = rk("postgres");
    let scoped = Arc::new(OneKeyScopedMap::new(key.clone(), 0xaaaa));
    let global = Arc::new(TestGlobalAccessor::new(key.clone(), 0xbbbb));
    let layered = LayeredResourceAccessor::new(scoped.clone(), global.clone());

    let payload = layered.acquire_any(&key).await.unwrap();
    assert_eq!(marker(payload), 0xaaaa, "scoped layer must win");
    assert_eq!(global.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_in_both_returns_error() {
    let scoped = Arc::new(OneKeyScopedMap::new(rk("postgres"), 0xaaaa));
    let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
    let layered = LayeredResourceAccessor::new(scoped, global);

    let result = layered.acquire_any(&rk("kafka")).await;
    assert!(
        matches!(result, Err(CoreError::CredentialNotFound { .. })),
        "expected CredentialNotFound, got {result:?}"
    );
}

#[tokio::test]
async fn try_acquire_any_returns_none_when_missing_in_both() {
    let scoped = Arc::new(OneKeyScopedMap::new(rk("postgres"), 0xaaaa));
    let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
    let layered = LayeredResourceAccessor::new(scoped, global);

    let result = layered.try_acquire_any(&rk("kafka")).await.unwrap();
    assert!(result.is_none());
}

#[test]
fn has_walks_both_layers() {
    let scoped = Arc::new(OneKeyScopedMap::new(rk("postgres"), 0xaaaa));
    let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
    let layered = LayeredResourceAccessor::new(scoped, global);

    assert!(layered.has(&rk("postgres")), "scoped layer reports key");
    assert!(layered.has(&rk("redis")), "global layer reports key");
    assert!(!layered.has(&rk("kafka")), "neither layer has key");
}

#[test]
fn global_only_constructor_uses_empty_scoped() {
    let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
    let layered = LayeredResourceAccessor::global_only(global);
    assert!(!layered.has(&rk("postgres")));
    assert!(layered.has(&rk("redis")));
}

// ── Cleanup driver (Task 7.4) ────────────────────────────────────────

#[tokio::test]
async fn cleanup_completes_within_budget() {
    let outcome = run_cleanup::<_, std::io::Error>(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok(())
    })
    .await;
    assert!(matches!(outcome, CleanupOutcome::Completed { .. }));
}

#[tokio::test(start_paused = true)]
async fn cleanup_times_out_when_overrunning_budget() {
    let outcome = run_cleanup_with_timeout::<_, std::io::Error>(
        async {
            tokio::time::sleep(Duration::from_secs(1000)).await;
            Ok(())
        },
        Duration::from_millis(50),
    )
    .await;
    assert!(
        matches!(outcome, CleanupOutcome::TimedOut { budget, .. } if budget == Duration::from_millis(50)),
        "expected TimedOut, got {outcome:?}"
    );
}

#[tokio::test]
async fn cleanup_reports_failure() {
    let outcome = run_cleanup::<_, &'static str>(async { Err("boom") }).await;
    assert!(
        matches!(&outcome, CleanupOutcome::Failed { error, .. } if error == "boom"),
        "expected Failed, got {outcome:?}"
    );
}

// ── ScopedResourceGuard cancel-safety ────────────────────────────────

#[tokio::test]
async fn guard_dismiss_keeps_entries_visible() {
    let map = DashScopedResourceMap::new();
    let leaf = b("leaf");
    map.register_branch(leaf.clone(), None);
    map.push(leaf.clone(), rk("postgres"), Arc::new(0xa1_u64));

    {
        let mut guard = ScopedResourceGuard::new(&map, leaf.clone(), None);
        guard.dismiss();
        // No drained vec returned; map state untouched.
        drop(guard);
    }
    // After dismiss, the entries are still in the map (engine drives explicit cleanup).
    let popped = map.pop(&leaf).expect("dismissed guard does not pop");
    assert_eq!(popped.len(), 1);
}

#[tokio::test]
async fn guard_into_drained_pops_entries() {
    let map = DashScopedResourceMap::new();
    let leaf = b("leaf");
    map.register_branch(leaf.clone(), None);
    map.push(leaf.clone(), rk("postgres"), Arc::new(0xa1_u64));

    let drained = ScopedResourceGuard::new(&map, leaf.clone(), None).into_drained();
    assert_eq!(drained.len(), 1);
    // After explicit drain, second pop yields None (branch removed).
    assert!(map.pop(&leaf).is_none());
}

#[tokio::test]
async fn guard_drop_routes_to_panic_sink() {
    let map = DashScopedResourceMap::new();
    let leaf = b("leaf");
    map.register_branch(leaf.clone(), None);
    map.push(leaf.clone(), rk("postgres"), Arc::new(0xa1_u64));

    let received: Arc<parking_lot::Mutex<Option<Vec<PoppedEntry>>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let received_clone = Arc::clone(&received);
    {
        let _guard = ScopedResourceGuard::new(
            &map,
            leaf.clone(),
            Some(Box::new(move |entries| {
                *received_clone.lock() = Some(entries);
            })),
        );
        // Drop without dismiss simulates panic-cancel exit.
    }

    let captured = received.lock().take().expect("panic sink fired");
    assert_eq!(captured.len(), 1);
    assert!(map.pop(&leaf).is_none(), "branch removed by Drop pop");
}
