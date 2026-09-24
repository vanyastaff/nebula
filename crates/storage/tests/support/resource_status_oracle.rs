//! Shared behavioral oracle for cross-process resource status stores.

use std::time::Duration;

use nebula_storage_port::Scope;
use nebula_storage_port::dto::{
    LiveResourceStatus, ResourceStatusPhase, ResourceStatusSnapshot, StatusWorkerId,
};
use nebula_storage_port::store::ResourceStatusStore;

/// Backend-owned time control used by the conformance target.
#[async_trait::async_trait]
pub(crate) trait ResourceStatusTimeControl {
    /// Lets `duration` of store time pass (sleep or advance a manual clock).
    async fn pass(&self, duration: Duration);
    /// Moves `worker`'s heartbeat expiry to two hours before store now,
    /// past the prune horizon.
    async fn expire_long_ago(&self, worker: &StatusWorkerId);
}

const LIVE_TTL: Duration = Duration::from_mins(1);
const RESOURCE: &str = "res_status_primary";

fn scope() -> Scope {
    Scope::new("status-ws", "status-org")
}

fn worker(name: &str) -> StatusWorkerId {
    StatusWorkerId::new(name).expect("valid worker id")
}

fn snapshot(
    resource_id: &str,
    phase: ResourceStatusPhase,
    row_version: u64,
) -> ResourceStatusSnapshot {
    ResourceStatusSnapshot {
        resource_id: resource_id.to_owned(),
        phase,
        healthy: phase == ResourceStatusPhase::Ready,
        accepting: true,
        row_version,
    }
}

async fn live(store: &impl ResourceStatusStore, target: &Scope) -> Vec<LiveResourceStatus> {
    store
        .live_for(target, RESOURCE)
        .await
        .expect("live status reads")
}

pub(crate) async fn publish_then_read_back(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    let published = snapshot(RESOURCE, ResourceStatusPhase::Ready, 3);
    store.heartbeat(&alpha, LIVE_TTL).await.expect("heartbeat");
    store
        .publish(&scope(), &alpha, &published)
        .await
        .expect("publish");

    assert_eq!(
        live(&store, &scope()).await,
        vec![LiveResourceStatus {
            worker_id: alpha,
            snapshot: published,
        }]
    );
    assert!(
        store
            .live_for(&scope(), "res_status_other")
            .await
            .expect("other row reads")
            .is_empty(),
        "a snapshot is visible only under its own row id"
    );
}

pub(crate) async fn tenant_isolation(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    store.heartbeat(&alpha, LIVE_TTL).await.expect("heartbeat");
    store
        .publish(
            &scope(),
            &alpha,
            &snapshot(RESOURCE, ResourceStatusPhase::Ready, 1),
        )
        .await
        .expect("publish");

    for foreign in [
        Scope::new("status-ws-other", "status-org"),
        Scope::new("status-ws", "status-org-other"),
    ] {
        assert!(
            live(&store, &foreign).await.is_empty(),
            "{foreign:?} must not see another tenant's status"
        );
        store
            .withdraw(&foreign, &alpha, RESOURCE)
            .await
            .expect("foreign withdraw is a no-op");
    }
    assert_eq!(live(&store, &scope()).await.len(), 1);
}

pub(crate) async fn workers_are_ordered_by_id(store: impl ResourceStatusStore) {
    let names = ["worker-b", "worker-a", "worker-c"];
    for name in names {
        let owner = worker(name);
        store.heartbeat(&owner, LIVE_TTL).await.expect("heartbeat");
        store
            .publish(
                &scope(),
                &owner,
                &snapshot(RESOURCE, ResourceStatusPhase::Ready, 1),
            )
            .await
            .expect("publish");
    }
    let order: Vec<String> = live(&store, &scope())
        .await
        .into_iter()
        .map(|status| status.worker_id.as_str().to_owned())
        .collect();
    assert_eq!(order, ["worker-a", "worker-b", "worker-c"]);
}

pub(crate) async fn publish_replaces_previous_view(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    store.heartbeat(&alpha, LIVE_TTL).await.expect("heartbeat");
    store
        .publish(
            &scope(),
            &alpha,
            &snapshot(RESOURCE, ResourceStatusPhase::Initializing, 1),
        )
        .await
        .expect("first publish");
    let replacement = ResourceStatusSnapshot {
        resource_id: RESOURCE.to_owned(),
        phase: ResourceStatusPhase::Draining,
        healthy: false,
        accepting: false,
        row_version: 2,
    };
    store
        .publish(&scope(), &alpha, &replacement)
        .await
        .expect("second publish");

    let statuses = live(&store, &scope()).await;
    assert_eq!(statuses.len(), 1, "one row per worker and resource");
    assert_eq!(statuses[0].snapshot, replacement);
}

pub(crate) async fn withdraw_removes_one_view(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    let beta = worker("worker-beta");
    for owner in [&alpha, &beta] {
        store.heartbeat(owner, LIVE_TTL).await.expect("heartbeat");
        store
            .publish(
                &scope(),
                owner,
                &snapshot(RESOURCE, ResourceStatusPhase::Ready, 1),
            )
            .await
            .expect("publish");
    }
    store
        .withdraw(&scope(), &alpha, RESOURCE)
        .await
        .expect("withdraw");

    let statuses = live(&store, &scope()).await;
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].worker_id, beta);
    store
        .withdraw(&scope(), &alpha, RESOURCE)
        .await
        .expect("repeated withdraw is a no-op");
}

pub(crate) async fn withdraw_worker_removes_everything(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    let beta = worker("worker-beta");
    for owner in [&alpha, &beta] {
        store.heartbeat(owner, LIVE_TTL).await.expect("heartbeat");
        for resource_id in [RESOURCE, "res_status_second"] {
            store
                .publish(
                    &scope(),
                    owner,
                    &snapshot(resource_id, ResourceStatusPhase::Ready, 1),
                )
                .await
                .expect("publish");
        }
    }
    store
        .withdraw_worker(&alpha)
        .await
        .expect("withdraw worker");

    for resource_id in [RESOURCE, "res_status_second"] {
        let statuses = store
            .live_for(&scope(), resource_id)
            .await
            .expect("live status reads");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].worker_id, beta);
    }
    // The heartbeat went with the snapshots: publishing again without a new
    // heartbeat stays invisible.
    store
        .publish(
            &scope(),
            &alpha,
            &snapshot(RESOURCE, ResourceStatusPhase::Ready, 2),
        )
        .await
        .expect("republish");
    let statuses = live(&store, &scope()).await;
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].worker_id, beta);
}

pub(crate) async fn snapshot_without_heartbeat_is_not_live(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    store
        .publish(
            &scope(),
            &alpha,
            &snapshot(RESOURCE, ResourceStatusPhase::Ready, 1),
        )
        .await
        .expect("publish");
    assert!(live(&store, &scope()).await.is_empty());

    store.heartbeat(&alpha, LIVE_TTL).await.expect("heartbeat");
    assert_eq!(
        live(&store, &scope()).await.len(),
        1,
        "a snapshot published before the first heartbeat becomes live with it"
    );
}

pub(crate) async fn expired_heartbeat_hides_snapshots(
    store: impl ResourceStatusStore,
    time: &impl ResourceStatusTimeControl,
) {
    let alpha = worker("worker-alpha");
    let beta = worker("worker-beta");
    store
        .heartbeat(&alpha, Duration::from_millis(50))
        .await
        .expect("short heartbeat");
    store.heartbeat(&beta, LIVE_TTL).await.expect("heartbeat");
    for owner in [&alpha, &beta] {
        store
            .publish(
                &scope(),
                owner,
                &snapshot(RESOURCE, ResourceStatusPhase::Ready, 1),
            )
            .await
            .expect("publish");
    }
    assert_eq!(live(&store, &scope()).await.len(), 2);

    time.pass(Duration::from_millis(150)).await;
    let statuses = live(&store, &scope()).await;
    assert_eq!(statuses.len(), 1, "the expired worker's snapshot is hidden");
    assert_eq!(statuses[0].worker_id, beta);

    store.heartbeat(&alpha, LIVE_TTL).await.expect("renewal");
    assert_eq!(
        live(&store, &scope()).await.len(),
        2,
        "a renewed heartbeat revives a recently expired worker's snapshot"
    );
}

pub(crate) async fn long_dead_heartbeat_is_pruned_with_its_snapshots(
    store: impl ResourceStatusStore,
    time: &impl ResourceStatusTimeControl,
) {
    let alpha = worker("worker-alpha");
    let beta = worker("worker-beta");
    store.heartbeat(&alpha, LIVE_TTL).await.expect("heartbeat");
    store
        .publish(
            &scope(),
            &alpha,
            &snapshot(RESOURCE, ResourceStatusPhase::Ready, 1),
        )
        .await
        .expect("publish");
    time.expire_long_ago(&alpha).await;
    assert!(live(&store, &scope()).await.is_empty());

    // Any worker's heartbeat prunes the long-dead one and its snapshots, so a
    // later renewal by the dead worker does not resurrect stale status.
    store
        .heartbeat(&beta, LIVE_TTL)
        .await
        .expect("pruning heartbeat");
    store
        .heartbeat(&alpha, LIVE_TTL)
        .await
        .expect("late renewal");
    assert!(live(&store, &scope()).await.is_empty());
}

pub(crate) async fn values_round_trip(store: impl ResourceStatusStore) {
    let alpha = worker("worker-alpha");
    store.heartbeat(&alpha, LIVE_TTL).await.expect("heartbeat");
    let large = u64::try_from(i64::MAX).expect("i64::MAX fits u64");
    for (index, phase) in ResourceStatusPhase::ALL.into_iter().enumerate() {
        let row_version = large - u64::try_from(index).expect("small index");
        for (healthy, accepting) in [(true, false), (false, true)] {
            let published = ResourceStatusSnapshot {
                resource_id: RESOURCE.to_owned(),
                phase,
                healthy,
                accepting,
                row_version,
            };
            store
                .publish(&scope(), &alpha, &published)
                .await
                .expect("publish");
            let statuses = live(&store, &scope()).await;
            assert_eq!(statuses.len(), 1);
            assert_eq!(statuses[0].snapshot, published, "{phase:?} round-trips");
        }
    }

    let unrepresentable = snapshot(RESOURCE, ResourceStatusPhase::Ready, u64::MAX);
    assert!(
        store
            .publish(&scope(), &alpha, &unrepresentable)
            .await
            .is_err(),
        "a row version beyond i64::MAX is rejected, not truncated"
    );
    assert_eq!(
        live(&store, &scope()).await[0].snapshot.row_version,
        large - 6
    );
}

macro_rules! resource_status_conformance_suite {
    ($factory:expr) => {
        resource_status_conformance_suite!(@cases $factory;
            publish_then_read_back,
            tenant_isolation,
            workers_are_ordered_by_id,
            publish_replaces_previous_view,
            withdraw_removes_one_view,
            withdraw_worker_removes_everything,
            snapshot_without_heartbeat_is_not_live,
            values_round_trip;
            expired_heartbeat_hides_snapshots,
            long_dead_heartbeat_is_pruned_with_its_snapshots
        );
    };
    (@cases $factory:expr; $($case:ident),+; $($timed:ident),+) => {
        $(
            #[tokio::test]
            async fn $case() {
                let Some((store, _time)) = $factory.await else {
                    panic!(
                        "{}: backend unreachable — the case cannot run and must fail \
                         rather than pass unchecked; reach the backend (set DATABASE_URL for \
                         postgres) or run without this feature",
                        stringify!($case)
                    );
                };
                oracle::$case(store).await;
            }
        )+
        $(
            #[tokio::test]
            async fn $timed() {
                let Some((store, time)) = $factory.await else {
                    panic!(
                        "{}: backend unreachable — the case cannot run and must fail \
                         rather than pass unchecked; reach the backend (set DATABASE_URL for \
                         postgres) or run without this feature",
                        stringify!($timed)
                    );
                };
                oracle::$timed(store, &time).await;
            }
        )+
    };
}
