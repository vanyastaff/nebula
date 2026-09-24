use nebula_resource::state::ResourcePhase;

use super::*;

fn live(
    worker: &str,
    phase: ResourceStatusPhase,
    healthy: bool,
    accepting: bool,
) -> LiveResourceStatus {
    LiveResourceStatus {
        worker_id: StatusWorkerId::new(worker).expect("valid worker id"),
        snapshot: ResourceStatusSnapshot {
            resource_id: "res_test".to_owned(),
            phase,
            healthy,
            accepting,
            row_version: 1,
        },
    }
}

#[test]
fn no_serving_worker_means_no_status() {
    assert_eq!(aggregate(&[]), None);
}

#[test]
fn aggregate_reports_the_least_healthy_phase_across_workers() {
    let status = aggregate(&[
        live("a", ResourceStatusPhase::Ready, true, true),
        live("b", ResourceStatusPhase::Failed, false, false),
        live("c", ResourceStatusPhase::Reloading, false, true),
    ])
    .expect("serving workers produce a status");
    assert_eq!(status.phase, "failed");
    assert!(
        !status.healthy,
        "one unhealthy worker makes the row unhealthy"
    );
    assert!(
        status.accepting,
        "one accepting worker keeps the row accepting"
    );
    assert_eq!(status.instances, 3);

    let all_ready = aggregate(&[
        live("a", ResourceStatusPhase::Ready, true, true),
        live("b", ResourceStatusPhase::Ready, true, true),
    ])
    .expect("serving workers produce a status");
    assert_eq!(all_ready.phase, "ready");
    assert!(all_ready.healthy);
}

#[test]
fn every_persisted_phase_has_a_severity() {
    for phase in ResourceStatusPhase::ALL {
        assert!(
            SEVERITY.contains(&phase),
            "{phase:?} must be ranked so aggregation never falls back"
        );
    }
}

#[test]
fn projection_matches_canonical_display_and_phase_predicates() {
    for phase in [
        ResourcePhase::Initializing,
        ResourcePhase::Ready,
        ResourcePhase::Reloading,
        ResourcePhase::Draining,
        ResourcePhase::ShuttingDown,
        ResourcePhase::Failed,
    ] {
        let (persisted, healthy, accepting) = project(phase);
        assert_eq!(
            persisted.as_str(),
            phase.to_string(),
            "persisted token must equal nebula-resource's Display for {phase:?}"
        );
        assert_eq!(healthy, matches!(phase, ResourcePhase::Ready));
        assert_eq!(accepting, phase.is_accepting());
    }
}

#[tokio::test]
async fn stored_status_reads_what_live_workers_published_and_forgets_withdrawn_ones() {
    let store: Arc<dyn ResourceStatusStore> =
        Arc::new(nebula_storage::inmem::InMemoryResourceStatusStore::new());
    let scope = Scope::new("ws_status", "org_status");
    let reader = StoredResourceStatus::new(Arc::clone(&store));
    assert_eq!(
        reader.runtime_status(&scope, "res_a").await.unwrap(),
        None,
        "nothing published: inactive"
    );

    for (worker, phase, healthy) in [
        ("worker:a", ResourceStatusPhase::Ready, true),
        ("worker:b", ResourceStatusPhase::Draining, false),
    ] {
        let worker = StatusWorkerId::new(worker).unwrap();
        store
            .heartbeat(&worker, Duration::from_secs(30))
            .await
            .unwrap();
        store
            .publish(
                &scope,
                &worker,
                &ResourceStatusSnapshot {
                    resource_id: "res_a".to_owned(),
                    phase,
                    healthy,
                    accepting: healthy,
                    row_version: 3,
                },
            )
            .await
            .unwrap();
    }

    let status = reader
        .runtime_status(&scope, "res_a")
        .await
        .unwrap()
        .expect("two live workers serve the row");
    assert_eq!(status.phase, "draining");
    assert_eq!(status.instances, 2);
    assert!(!status.healthy);
    assert!(status.accepting);

    let other_scope = Scope::new("ws_other", "org_status");
    assert_eq!(
        reader.runtime_status(&other_scope, "res_a").await.unwrap(),
        None,
        "another workspace never sees this row's status"
    );

    store
        .withdraw_worker(&StatusWorkerId::new("worker:b").unwrap())
        .await
        .unwrap();
    let status = reader
        .runtime_status(&scope, "res_a")
        .await
        .unwrap()
        .expect("one worker still serves the row");
    assert_eq!((status.phase, status.instances), ("ready", 1));
}
