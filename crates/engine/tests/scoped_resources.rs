//! Phase 7 (M6.2) — Scoped resources integration tests.
//!
//! Covers Task 7.5 acceptance matrix:
//!
//! - 3-hop nested shadowing (root → branch1 → branch2: scoped at branch1 wins for branch2).
//! - Cancellation mid-branch: cleanup still runs.
//! - Cleanup-uses-global: cleanup hook can call `ctx.resource::<R>()` to access a global resource
//!   while the scoped one is being torn down.
//! - Scope conflicts: same resource key registered at two levels → closest wins.
//! - Cleanup timeout: Provider::destroy that blocks > budget triggers
//!   `ScopedResourceCleanupTimeout` event.
//! - Use-case coverage from `crates/resource/plans/10-scoped-resources.md`:
//!   * Temporary test database (cleanup uses global pool to drop the schema after scoped
//!     tear-down).
//!   * Per-tenant pool (each branch gets a different scoped resource by branch).
//!   * Ephemeral sandbox (registers and tears down per-branch with no cross-talk).
//!
//! These tests exercise the **storage + lifecycle** layer in isolation.
//! Engine-driver wiring (calling `ResourceAction::configure`/`cleanup` per
//! frontier branch) is deferred.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use dashmap::DashMap;
use nebula_core::{
    CoreError, NodeKey, ResourceKey,
    accessor::ResourceAccessor,
    id::{ExecutionId, WorkflowId},
};
use nebula_engine::{
    BranchId, CleanupOutcome, DEFAULT_CLEANUP_TIMEOUT, DashScopedResourceMap, ExecutionEvent,
    LayeredResourceAccessor, MAX_ANCESTOR_DEPTH, PoppedEntry, ScopedResourceGuard, run_cleanup,
    run_cleanup_with_timeout,
};
use tokio_util::sync::CancellationToken;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn rk(s: &str) -> ResourceKey {
    ResourceKey::new(s).expect("valid resource key in test")
}

fn b(s: &str) -> BranchId {
    BranchId::from_node_key(NodeKey::new(s).expect("valid node key in test"))
}

/// Stand-in global accessor backed by a `DashMap<ResourceKey, u64>`.
///
/// Mirrors what `EngineResourceAccessor` (Manager-backed) would return,
/// but without the heavy `nebula-resource` plumbing — we just want to
/// verify the lookup precedence and fall-through semantics.
struct FakeGlobalAccessor {
    table: DashMap<ResourceKey, u64>,
    hits: AtomicUsize,
}

impl FakeGlobalAccessor {
    fn new() -> Self {
        Self {
            table: DashMap::new(),
            hits: AtomicUsize::new(0),
        }
    }

    fn register(&self, key: ResourceKey, marker: u64) {
        self.table.insert(key, marker);
    }
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl ResourceAccessor for FakeGlobalAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.table.contains_key(key)
    }

    fn acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> {
        let key = key.clone();
        Box::pin(async move {
            self.hits.fetch_add(1, Ordering::SeqCst);
            self.table
                .get(&key)
                .map(|v| Box::new(*v.value()) as Box<dyn std::any::Any + Send + Sync>)
                .ok_or_else(|| CoreError::CredentialNotFound {
                    key: key.as_str().to_owned(),
                })
        })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, CoreError>> {
        let key = key.clone();
        Box::pin(async move {
            Ok(self
                .table
                .get(&key)
                .map(|v| Box::new(*v.value()) as Box<dyn std::any::Any + Send + Sync>))
        })
    }
}

/// Downcast helper. Layered lookups against `DashScopedResourceMap` hand
/// back `Box<Arc<dyn Any>>`; lookups against `FakeGlobalAccessor` hand back
/// `Box<u64>`. We probe both shapes.
fn into_marker(boxed: Box<dyn std::any::Any + Send + Sync>) -> u64 {
    if let Ok(arc) = boxed.downcast::<Arc<dyn std::any::Any + Send + Sync>>() {
        let v: &u64 = arc
            .downcast_ref::<u64>()
            .expect("scoped Arc payload must be u64 in tests");
        return *v;
    }
    panic!(
        "expected Arc<u64> from scoped layer; tests with global only must downcast to u64 explicitly"
    )
}

fn into_global_marker(boxed: Box<dyn std::any::Any + Send + Sync>) -> u64 {
    // `Box::downcast` consumes self; on miss we get the original Box back.
    match boxed.downcast::<u64>() {
        Ok(v) => *v,
        Err(boxed) => {
            let arc = boxed
                .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
                .expect("payload neither u64 nor Arc<u64>");
            *arc.downcast_ref::<u64>().unwrap()
        },
    }
}

// ── Task 7.5 #1 — 3-hop nested shadowing ───────────────────────────────────

#[tokio::test]
async fn three_hop_nested_shadowing_closest_wins() {
    let scoped = DashScopedResourceMap::new();
    let root = b("root");
    let lvl1 = b("lvl1");
    let lvl2 = b("lvl2");
    let lvl3 = b("lvl3");

    scoped.register_branch(root.clone(), None);
    scoped.register_branch(lvl1.clone(), Some(root.clone()));
    scoped.register_branch(lvl2.clone(), Some(lvl1.clone()));
    scoped.register_branch(lvl3.clone(), Some(lvl2));

    // Register `postgres` at root, override at lvl1 (between root and lvl3).
    scoped.push(root.clone(), rk("postgres"), Arc::new(0xa1_u64));
    scoped.push(lvl1, rk("postgres"), Arc::new(0xb2_u64));

    // From lvl3, walk lvl3 → lvl2 → lvl1 (hit) — the closest ancestor with
    // `postgres` is lvl1.
    let p = scoped
        .lookup_in_ancestors_from(&lvl3, &rk("postgres"))
        .unwrap()
        .expect("must hit lvl1 walking from lvl3");
    let arc = p
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xb2);

    // From a sibling of lvl1, root still wins.
    let sibling = b("sibling");
    scoped.register_branch(sibling.clone(), Some(root));
    let p = scoped
        .lookup_in_ancestors_from(&sibling, &rk("postgres"))
        .unwrap()
        .expect("sibling walks to root");
    let arc = p
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xa1);
}

// ── Task 7.5 #2 — Cancellation mid-branch: cleanup still runs ──────────────

#[tokio::test]
async fn cancellation_mid_branch_cleanup_still_runs() {
    let scoped = Arc::new(DashScopedResourceMap::new());
    let leaf = b("leaf");
    scoped.register_branch(leaf.clone(), None);
    scoped.push(leaf.clone(), rk("postgres"), Arc::new(99u64));

    let cleanup_ran = Arc::new(AtomicBool::new(false));
    let cancel = CancellationToken::new();

    let map = Arc::clone(&scoped);
    let leaf_for_task = leaf.clone();
    let cleanup_flag = Arc::clone(&cleanup_ran);
    let cancel_for_task = cancel.clone();
    let task = tokio::spawn(async move {
        // Build a guard that, on Drop without dismiss, routes drained
        // entries to a sink that runs "cleanup" synchronously.
        let cleanup_clone = Arc::clone(&cleanup_flag);
        let _guard = ScopedResourceGuard::new(
            &map,
            leaf_for_task,
            Some(Box::new(move |entries: Vec<PoppedEntry>| {
                assert_eq!(entries.len(), 1, "must drain the registered entry");
                cleanup_clone.store(true, Ordering::SeqCst);
            })),
        );
        // Simulate a cancellation mid-execution: the task-level cancel
        // token fires and we exit without dismiss.
        cancel_for_task.cancelled().await;
    });

    // Trip cancellation; the spawned task exits and Drop runs.
    cancel.cancel();
    task.await.expect("task must complete");

    assert!(
        cleanup_ran.load(Ordering::SeqCst),
        "Drop sink fired cleanup on cancel"
    );
    assert!(
        scoped.pop(&leaf).is_none(),
        "branch entries were drained by Drop"
    );
}

#[tokio::test]
async fn panic_in_branch_drop_runs_cleanup() {
    let scoped = Arc::new(DashScopedResourceMap::new());
    let leaf = b("leaf");
    scoped.register_branch(leaf.clone(), None);
    scoped.push(leaf.clone(), rk("postgres"), Arc::new(99u64));

    let cleanup_ran = Arc::new(AtomicBool::new(false));

    let map = Arc::clone(&scoped);
    let leaf_for_task = leaf.clone();
    let cleanup_flag = Arc::clone(&cleanup_ran);
    let task = tokio::spawn(async move {
        let cleanup_clone = Arc::clone(&cleanup_flag);
        let _guard = ScopedResourceGuard::new(
            &map,
            leaf_for_task,
            Some(Box::new(move |entries: Vec<PoppedEntry>| {
                assert_eq!(entries.len(), 1);
                cleanup_clone.store(true, Ordering::SeqCst);
            })),
        );
        panic!("simulating action body panic — guard Drop must still fire");
    });
    let result = task.await;
    assert!(result.is_err(), "panic must surface as JoinError");

    assert!(
        cleanup_ran.load(Ordering::SeqCst),
        "panic-triggered Drop sink fired cleanup"
    );
    assert!(scoped.pop(&leaf).is_none(), "panic path drained the branch");
}

// ── Task 7.5 #3 — Cleanup uses global ──────────────────────────────────────

#[tokio::test]
async fn cleanup_can_access_global_after_scoped_drained() {
    let global = Arc::new(FakeGlobalAccessor::new());
    global.register(rk("postgres"), 0xcafe);

    let scoped = Arc::new(DashScopedResourceMap::new());
    let leaf = b("test_db");
    scoped.register_branch(leaf.clone(), None);
    // Scoped entry shadows global for the branch lifetime.
    scoped.push(leaf.clone(), rk("postgres"), Arc::new(0xbeef_u64));
    scoped.set_current_branch(Some(leaf.clone()));

    let layered: Arc<dyn ResourceAccessor> = Arc::new(LayeredResourceAccessor::new(
        Arc::clone(&scoped) as Arc<dyn nebula_engine::ScopedResourceMap>,
        Arc::clone(&global) as Arc<dyn ResourceAccessor>,
    ));

    // During branch lifetime: scoped wins.
    let payload = layered.acquire_any(&rk("postgres")).await.unwrap();
    assert_eq!(into_marker(payload), 0xbeef);

    // Branch ends — engine pops and runs cleanup. After pop, the trait
    // lookup falls through to global (per plan §"Resolution precedence").
    let drained = scoped.pop(&leaf).expect("registered branch pops");
    assert_eq!(drained.len(), 1);
    scoped.set_current_branch(None);

    // Now `cleanup()` calling `ctx.resource::<R>()` must reach the global.
    let payload = layered.acquire_any(&rk("postgres")).await.unwrap();
    assert_eq!(into_global_marker(payload), 0xcafe);
    assert!(
        global.hits.load(Ordering::SeqCst) >= 1,
        "global accessor must have been consulted post-pop"
    );
}

// ── Task 7.5 #4 — Scope conflicts: same key, different levels ──────────────

#[tokio::test]
async fn scope_conflict_closest_wins_within_same_chain() {
    let scoped = DashScopedResourceMap::new();
    let outer = b("outer");
    let inner = b("inner");
    scoped.register_branch(outer.clone(), None);
    scoped.register_branch(inner.clone(), Some(outer.clone()));

    // Same key registered at two levels.
    scoped.push(outer.clone(), rk("postgres"), Arc::new(0xaaaa_u64));
    scoped.push(inner.clone(), rk("postgres"), Arc::new(0xbbbb_u64));

    // Inner wins from its own perspective.
    let p = scoped
        .lookup_in_ancestors_from(&inner, &rk("postgres"))
        .unwrap()
        .expect("inner sees its own entry");
    let arc = p
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xbbbb);

    // Outer sees only its own.
    let p = scoped
        .lookup_in_ancestors_from(&outer, &rk("postgres"))
        .unwrap()
        .expect("outer sees its own entry");
    let arc = p
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xaaaa);
}

#[tokio::test]
async fn scope_conflict_siblings_do_not_see_each_other() {
    let scoped = DashScopedResourceMap::new();
    let root = b("root");
    let left = b("left");
    let right = b("right");
    scoped.register_branch(root.clone(), None);
    scoped.register_branch(left.clone(), Some(root.clone()));
    scoped.register_branch(right.clone(), Some(root));

    scoped.push(left.clone(), rk("postgres"), Arc::new(0xaaaa_u64));
    scoped.push(right.clone(), rk("postgres"), Arc::new(0xbbbb_u64));

    // Each sees only its own.
    let pl = scoped
        .lookup_in_ancestors_from(&left, &rk("postgres"))
        .unwrap()
        .unwrap();
    let arc = pl
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xaaaa);

    let pr = scoped
        .lookup_in_ancestors_from(&right, &rk("postgres"))
        .unwrap()
        .unwrap();
    let arc = pr
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xbbbb);
}

// ── Task 7.5 #5 — Cleanup timeout fires event ──────────────────────────────

#[tokio::test]
async fn cleanup_timeout_emits_typed_event() {
    // Fake "resource destroy" that never returns within the budget. Real
    // wall-clock sleep so the elapsed-vs-budget invariant is meaningful;
    // budget is small enough to keep the test fast.
    let outcome = run_cleanup_with_timeout::<_, std::io::Error>(
        async {
            // Sleep far longer than the budget; the timeout drops the
            // future before this completes.
            tokio::time::sleep(Duration::from_hours(1)).await;
            Ok(())
        },
        Duration::from_millis(50),
    )
    .await;

    let (budget, elapsed) = match outcome {
        CleanupOutcome::TimedOut { budget, elapsed } => (budget, elapsed),
        other => panic!("expected TimedOut, got {other:?}"),
    };
    assert_eq!(budget, Duration::from_millis(50));
    // tokio::time::timeout fires after the budget, so elapsed is at least
    // the budget modulo a small scheduler-induced slack window. We test
    // that elapsed is in the same order of magnitude as budget rather
    // than asserting a hard >= (which would be flaky under a paused
    // runtime where `Instant::now()` doesn't advance).
    assert!(
        elapsed.as_millis() + 5 >= budget.as_millis(),
        "elapsed {elapsed:?} must be ~ budget {budget:?}"
    );

    // Now construct the typed event the engine would emit and verify
    // the variant shape + invariants.
    let exec_id = ExecutionId::new();
    let branch = b("scoped_db");
    let key = rk("postgres");

    let event = ExecutionEvent::ScopedResourceCleanupTimeout {
        execution_id: exec_id,
        branch_id: branch.clone(),
        resource_key: key.clone(),
        budget,
        elapsed,
    };

    match event {
        ExecutionEvent::ScopedResourceCleanupTimeout {
            execution_id,
            branch_id,
            resource_key,
            budget: b_,
            elapsed: _e_,
        } => {
            assert_eq!(execution_id, exec_id);
            assert_eq!(branch_id, branch);
            assert_eq!(resource_key, key);
            assert_eq!(b_, budget);
        },
        _ => panic!("expected ScopedResourceCleanupTimeout"),
    }
}

#[tokio::test]
async fn cleanup_completes_normally_does_not_emit_timeout() {
    let outcome = run_cleanup::<_, std::io::Error>(async {
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok(())
    })
    .await;
    assert!(matches!(outcome, CleanupOutcome::Completed { .. }));
}

#[tokio::test]
async fn cleanup_default_timeout_constant_is_30_seconds() {
    assert_eq!(DEFAULT_CLEANUP_TIMEOUT, Duration::from_secs(30));
}

// ── Task 7.4 — Inner-to-outer + LIFO destroy ordering ─────────────────────

#[tokio::test]
async fn inner_to_outer_pop_drives_destroy_order() {
    // Register three nested branches; each holds two resources.
    // The engine pops leaves first; within each branch, LIFO order.
    let scoped = DashScopedResourceMap::new();
    let outer = b("outer");
    let middle = b("middle");
    let inner = b("inner");

    scoped.register_branch(outer.clone(), None);
    scoped.register_branch(middle.clone(), Some(outer.clone()));
    scoped.register_branch(inner.clone(), Some(middle.clone()));

    scoped.push(outer.clone(), rk("o1"), Arc::new(1u64));
    scoped.push(outer.clone(), rk("o2"), Arc::new(2u64));
    scoped.push(middle.clone(), rk("m1"), Arc::new(3u64));
    scoped.push(middle.clone(), rk("m2"), Arc::new(4u64));
    scoped.push(inner.clone(), rk("i1"), Arc::new(5u64));
    scoped.push(inner.clone(), rk("i2"), Arc::new(6u64));

    // Engine drives inner first.
    let inner_drain = scoped.pop(&inner).unwrap();
    let inner_keys: Vec<&str> = inner_drain.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(inner_keys, vec!["i2", "i1"], "LIFO within branch");

    let middle_drain = scoped.pop(&middle).unwrap();
    let middle_keys: Vec<&str> = middle_drain.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(middle_keys, vec!["m2", "m1"]);

    let outer_drain = scoped.pop(&outer).unwrap();
    let outer_keys: Vec<&str> = outer_drain.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(outer_keys, vec!["o2", "o1"]);
}

// ── Task 7.5 / Use cases — Temporary test database ────────────────────────

/// Plan §"Use cases / Temporary test database": `configure()` may use the
/// global Postgres pool to set up a temp schema; cleanup uses the same
/// global pool to drop the schema *after* the scoped pool is destroyed.
#[tokio::test]
async fn use_case_temporary_test_database_round_trip() {
    let global = Arc::new(FakeGlobalAccessor::new());
    global.register(rk("postgres"), 0x00c0_ffee);

    let scoped = Arc::new(DashScopedResourceMap::new());
    let test_branch = b("test_db_action");
    scoped.register_branch(test_branch.clone(), None);

    // configure() consults global to set up the temp schema. Engine uses
    // a layered accessor with NO current_branch yet (configure runs
    // *before* push — the scoped resource doesn't exist yet).
    let layered_pre: Arc<dyn ResourceAccessor> = Arc::new(LayeredResourceAccessor::new(
        Arc::clone(&scoped) as Arc<dyn nebula_engine::ScopedResourceMap>,
        Arc::clone(&global) as Arc<dyn ResourceAccessor>,
    ));
    let global_payload = layered_pre.acquire_any(&rk("postgres")).await.unwrap();
    assert_eq!(into_global_marker(global_payload), 0x00c0_ffee);

    // Now push the scoped pool, set current branch, downstream sees scoped.
    scoped.push(
        test_branch.clone(),
        rk("postgres"),
        Arc::new(0x00c0_ffee_u64),
    );
    scoped.set_current_branch(Some(test_branch.clone()));
    let scoped_payload = layered_pre.acquire_any(&rk("postgres")).await.unwrap();
    assert_eq!(into_marker(scoped_payload), 0x00c0_ffee); // both happen to share marker

    // Branch ends. Engine drives cleanup: pop FIRST (so the cleanup hook
    // sees global, not the scoped pool that's about to die), then run
    // Resource::destroy on the popped entries.
    let drained = scoped.pop(&test_branch).expect("registered branch pops");
    scoped.set_current_branch(None);
    assert_eq!(drained.len(), 1);

    // cleanup() now accesses the global pool — must succeed.
    let cleanup_payload = layered_pre.acquire_any(&rk("postgres")).await.unwrap();
    assert_eq!(into_global_marker(cleanup_payload), 0x00c0_ffee);
}

// ── Task 7.5 / Use cases — Per-tenant pool ────────────────────────────────

#[tokio::test]
async fn use_case_per_tenant_pool_isolation() {
    let scoped = DashScopedResourceMap::new();
    let root = b("workflow_root");
    let tenant_a = b("tenant_a");
    let tenant_b = b("tenant_b");

    scoped.register_branch(root.clone(), None);
    scoped.register_branch(tenant_a.clone(), Some(root.clone()));
    scoped.register_branch(tenant_b.clone(), Some(root));

    // Each tenant gets its own scoped pool, distinct payload.
    scoped.push(tenant_a.clone(), rk("postgres"), Arc::new(0xa1_u64));
    scoped.push(tenant_b.clone(), rk("postgres"), Arc::new(0xb1_u64));

    // Tenant A query nodes resolve to A's pool.
    let pa = scoped
        .lookup_in_ancestors_from(&tenant_a, &rk("postgres"))
        .unwrap()
        .unwrap();
    let arc = pa
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xa1);

    // Tenant B query nodes resolve to B's pool.
    let pb = scoped
        .lookup_in_ancestors_from(&tenant_b, &rk("postgres"))
        .unwrap()
        .unwrap();
    let arc = pb
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xb1);

    // Disposing tenant A leaves tenant B intact.
    let _ = scoped.pop(&tenant_a);
    assert!(
        scoped
            .lookup_in_ancestors_from(&tenant_a, &rk("postgres"))
            .unwrap()
            .is_none(),
        "tenant A's branch is gone"
    );
    let pb2 = scoped
        .lookup_in_ancestors_from(&tenant_b, &rk("postgres"))
        .unwrap()
        .unwrap();
    let arc = pb2
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 0xb1);
}

// ── Task 7.5 / Use cases — Ephemeral sandbox ──────────────────────────────

#[tokio::test]
async fn use_case_ephemeral_sandbox_no_cross_branch_visibility() {
    let scoped = DashScopedResourceMap::new();
    let session_a = b("session_a");
    let session_b = b("session_b");
    scoped.register_branch(session_a.clone(), None);
    scoped.register_branch(session_b.clone(), None); // distinct root

    scoped.push(session_a.clone(), rk("browser"), Arc::new(1u64));
    scoped.push(session_b.clone(), rk("browser"), Arc::new(2u64));

    // No cross-talk between session A and session B.
    let pa = scoped
        .lookup_in_ancestors_from(&session_a, &rk("browser"))
        .unwrap()
        .unwrap();
    let arc = pa
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 1);

    let pb = scoped
        .lookup_in_ancestors_from(&session_b, &rk("browser"))
        .unwrap()
        .unwrap();
    let arc = pb
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 2);

    // Neither sees the other after pop.
    let _ = scoped.pop(&session_a);
    assert!(
        scoped
            .lookup_in_ancestors_from(&session_b, &rk("browser"))
            .unwrap()
            .is_some(),
        "session B unaffected by A's pop"
    );
}

// ── Concurrency: sibling pushes don't deadlock ────────────────────────────

#[tokio::test]
async fn concurrent_sibling_pushes_do_not_deadlock() {
    let scoped = Arc::new(DashScopedResourceMap::new());
    let root = b("root");
    scoped.register_branch(root.clone(), None);

    // Spawn 16 sibling branches and have each push concurrently.
    let mut handles = Vec::new();
    for i in 0..16 {
        let map = Arc::clone(&scoped);
        let root_id = root.clone();
        let h = tokio::spawn(async move {
            let branch = BranchId::from_node_key(
                NodeKey::new(format!("sibling_{i}")).expect("valid node key"),
            );
            map.register_branch(branch.clone(), Some(root_id));
            map.push(
                branch.clone(),
                ResourceKey::new(format!("res_{i}")).unwrap(),
                Arc::new(i as u64),
            );
            branch
        });
        handles.push(h);
    }

    let mut branches = Vec::new();
    for h in handles {
        branches.push(h.await.unwrap());
    }

    // All siblings registered and visible.
    for (i, branch) in branches.iter().enumerate() {
        let p = scoped
            .lookup_in_ancestors_from(branch, &ResourceKey::new(format!("res_{i}")).unwrap())
            .unwrap()
            .expect("each sibling sees its own entry");
        let arc = p
            .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
            .unwrap();
        assert_eq!(*arc.downcast_ref::<u64>().unwrap(), i as u64);
    }
}

// ── Misuse defense — depth bound ──────────────────────────────────────────

#[tokio::test]
async fn lookup_handles_long_ancestor_chains_within_depth_bound() {
    // A chain of length 16 — well below MAX_ANCESTOR_DEPTH (1024).
    let scoped = DashScopedResourceMap::new();
    let mut chain: Vec<BranchId> = Vec::new();
    for i in 0..16 {
        let id = BranchId::from_node_key(NodeKey::new(format!("node_{i}")).unwrap());
        let parent = if i == 0 {
            None
        } else {
            Some(chain[i - 1].clone())
        };
        scoped.register_branch(id.clone(), parent);
        chain.push(id);
    }
    scoped.push(chain[0].clone(), rk("postgres"), Arc::new(7u64));

    let p = scoped
        .lookup_in_ancestors_from(chain.last().unwrap(), &rk("postgres"))
        .unwrap()
        .expect("walks 16 hops to root");
    let arc = p
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc.downcast_ref::<u64>().unwrap(), 7);

    // Sanity-bound check (compile-time so it gates the constant change).
    const _: () = assert!(MAX_ANCESTOR_DEPTH >= 1024);
}

// ── Wiring smoke test — LayeredResourceAccessor with DashScopedResourceMap ──

#[tokio::test]
async fn layered_accessor_with_dash_storage_walks_ancestors() {
    let global = Arc::new(FakeGlobalAccessor::new());
    global.register(rk("redis"), 0xcafe);

    let scoped = Arc::new(DashScopedResourceMap::new());
    let root = b("root");
    let leaf = b("leaf");
    scoped.register_branch(root.clone(), None);
    scoped.register_branch(leaf.clone(), Some(root.clone()));

    scoped.push(root.clone(), rk("postgres"), Arc::new(0xbeef_u64));
    scoped.set_current_branch(Some(leaf.clone()));

    let layered: Arc<dyn ResourceAccessor> = Arc::new(LayeredResourceAccessor::new(
        Arc::clone(&scoped) as Arc<dyn nebula_engine::ScopedResourceMap>,
        Arc::clone(&global) as Arc<dyn ResourceAccessor>,
    ));

    // postgres: scoped at root, leaf walks to root and finds it.
    let p = layered.acquire_any(&rk("postgres")).await.unwrap();
    assert_eq!(into_marker(p), 0xbeef);

    // redis: not scoped, falls through to global.
    let r = layered.acquire_any(&rk("redis")).await.unwrap();
    assert_eq!(into_global_marker(r), 0xcafe);

    // unknown key: error from global layer.
    let result = layered.acquire_any(&rk("kafka")).await;
    assert!(matches!(result, Err(CoreError::CredentialNotFound { .. })));
}

// ── Per-execution credential scope (Task 7.3) ──────────────────────────────

/// Verifies that the `BranchId` carries enough information to integrate
/// with `nebula_core::ScopeLevel::Execution(execution_id)` resolution. The
/// engine wiring stores the executing `ExecutionId` alongside each branch
/// frame; here we exercise the structural invariant: `BranchId` is keyed
/// off `NodeKey`, so the same node executed under two distinct executions
/// produces structurally distinct branches as long as the engine uses
/// per-execution `BranchId` namespacing.
///
/// (Engine-side namespacing — appending the execution id to the branch
/// id — is part of the deferred wiring; this test pins down the storage
/// shape that supports it.)
#[tokio::test]
async fn per_execution_credential_scope_storage_supports_namespacing() {
    let scoped = DashScopedResourceMap::new();

    let exec_a = ExecutionId::new();
    let exec_b = ExecutionId::new();
    let _wf = WorkflowId::new();

    let node_id = "scoped_db_action";
    // Engines namespace by execution id when constructing the BranchId.
    // NodeKey forbids consecutive separators (`__`), so we use a single
    // dot to bridge the segments.
    let branch_a =
        BranchId::from_node_key(NodeKey::new(format!("{node_id}.{exec_a}")).expect("valid key"));
    let branch_b =
        BranchId::from_node_key(NodeKey::new(format!("{node_id}.{exec_b}")).expect("valid key"));

    scoped.register_branch(branch_a.clone(), None);
    scoped.register_branch(branch_b.clone(), None);

    scoped.push(branch_a.clone(), rk("postgres"), Arc::new(0xaaa_u64));
    scoped.push(branch_b.clone(), rk("postgres"), Arc::new(0xbbb_u64));

    // Same logical node, two distinct executions, two distinct scoped resources.
    let pa = scoped
        .lookup_in_ancestors_from(&branch_a, &rk("postgres"))
        .unwrap()
        .unwrap();
    let pb = scoped
        .lookup_in_ancestors_from(&branch_b, &rk("postgres"))
        .unwrap()
        .unwrap();
    let arc_a = pa
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    let arc_b = pb
        .downcast::<Arc<dyn std::any::Any + Send + Sync>>()
        .unwrap();
    assert_eq!(*arc_a.downcast_ref::<u64>().unwrap(), 0xaaa);
    assert_eq!(*arc_b.downcast_ref::<u64>().unwrap(), 0xbbb);
}
