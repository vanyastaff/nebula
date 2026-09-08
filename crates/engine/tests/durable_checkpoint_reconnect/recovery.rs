//! Recovery from durable acceptance after all prior SQL adapters are destroyed.

use super::*;
use nebula_engine::{RecoveryTurnOutcome, RecoveryTurnRequest};
use nebula_execution::ExecutionStatus;
use nebula_storage_port::store::{ControlStartAcceptance, ControlStartHandoff};

fn engine(
    ports: Ports,
    count: &Arc<AtomicU32>,
    barrier: Option<Arc<ActionBarrier>>,
) -> Arc<WorkflowEngine> {
    let (registry, frozen) = frozen_registry_with_barrier(count, barrier);
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_, _, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new(executor)),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_lease_ttl(Duration::from_secs(1))
            .with_execution_stores(ports.stores)
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(ports.catalog)),
                frozen,
                ports.bundles,
            ),
    )
}

async fn accept_then_crash(ports: Ports, admitted: &Admitted, count: &Arc<AtomicU32>) {
    let (_, registry) = frozen_registry(count);
    let claims = ports
        .queue
        .claim_pending_for_flavor(&[0x78; 16], 1, registry.revision().id())
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    let record = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .unwrap()
        .unwrap();
    let admitted_execution_id = admitted.id.to_string();
    let control_start_handoff = ControlStartHandoff::for_claim(
        &admitted.scope,
        &admitted_execution_id,
        claims[0].token,
        registry.revision().id(),
    )
    .at_version(record.version)
    .lease_to("crashed-before-first-checkpoint", Duration::from_secs(1));
    assert!(matches!(
        ports
            .handoff
            .accept_control_start(&control_start_handoff)
            .await
            .unwrap(),
        ControlStartAcceptance::Accepted { .. }
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    // No engine is constructed: this is the exact post-acceptance crash window.
}

async fn recover(ports: Ports, admitted: &Admitted, count: &Arc<AtomicU32>, warm: bool) {
    let (_, registry) = frozen_registry(count);
    let recovery = ports.recovery.clone();
    let execution = ports.stores.execution.clone();
    let page = recovery
        .list_recoverable_turns(registry.revision().id(), None, 16)
        .await
        .unwrap();
    let candidate = page
        .turns()
        .iter()
        .find(|candidate| candidate.execution_id() == admitted.id)
        .expect("durable acceptance survives adapter destruction");
    assert_eq!(candidate.scope(), &admitted.scope);
    let engine = engine(ports, count, None);
    let request = RecoveryTurnRequest {
        handoff: recovery.as_ref(),
        holder: "reconnected-worker",
        lease_ttl: Duration::from_secs(1),
        accepted_fencing_generation: candidate.accepted_fencing_generation(),
    };
    let RecoveryTurnOutcome::Accepted(Ok(result)) = engine
        .resume_recoverable_turn(candidate.scope(), admitted.id, request)
        .await
    else {
        panic!("owner must grant recovery of the expired accepted turn");
    };
    assert_eq!(
        result.status,
        if warm {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Paused
        }
    );
    assert_eq!(
        result.node_outputs[&node_key!("predecessor")],
        admitted.input
    );
    assert_eq!(count.load(Ordering::SeqCst), if warm { 3 } else { 1 });
    if warm {
        assert_eq!(result.node_outputs[&node_key!("successor")], admitted.input);
    } else {
        let before = execution
            .get(&admitted.scope, &admitted.id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            engine
                .resume_recoverable_turn(candidate.scope(), admitted.id, request)
                .await,
            RecoveryTurnOutcome::NotReady
        ));
        let after = execution
            .get(&admitted.scope, &admitted.id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (after.version, after.state),
            (before.version, before.state),
            "unarmed Paused waits grant no recovery and make no writes"
        );
    }
}

async fn crash_after_committed_predecessor(
    ports: Ports,
    admitted: &Admitted,
    count: &Arc<AtomicU32>,
) {
    let barrier = Arc::new(ActionBarrier {
        block_at_call: 2,
        ..ActionBarrier::default()
    });
    let execution = ports.stores.execution.clone();
    let handoff = ports.handoff.clone();
    let engine = engine(ports, count, Some(barrier.clone()));
    let dispatch = EngineControlDispatch::new(
        engine,
        execution,
        handoff,
        "checkpoint-recovery-control".to_owned(),
        Duration::from_secs(1),
    );
    {
        let resume = dispatch.dispatch_resume(&admitted.scope, admitted.id, None);
        tokio::pin!(resume);
        tokio::select! {
            result = &mut resume => panic!("successor must still be blocked: {result:?}"),
            () = barrier.entered.notified() => {},
            () = tokio::time::sleep(Duration::from_secs(10)) => panic!("successor did not start"),
        }
        // Drop the driving future after real Resume durably completed the wait,
        // while the successor has no committed output. No snapshot is fabricated.
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn sqlite_accepted_turn_reconnects_cold_and_warm() {
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("accepted.db"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(sqlite(pool.clone()), &count).await;
    accept_then_crash(sqlite(pool.clone()), &admitted, &count).await;
    pool.close().await;
    for warm in [false, true] {
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        recover(sqlite(pool.clone()), &admitted, &count, warm).await;
        if !warm {
            crash_after_committed_predecessor(sqlite(pool.clone()), &admitted, &count).await;
        }
        pool.close().await;
    }
}

#[tokio::test]
async fn postgres_accepted_turn_reconnects_cold_and_warm() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "accepted_turn_runtime")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema)]);
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(postgres(pool.clone()), &count).await;
    accept_then_crash(postgres(pool.clone()), &admitted, &count).await;
    pool.close().await;
    for warm in [false, true] {
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        recover(postgres(pool.clone()), &admitted, &count, warm).await;
        if !warm {
            crash_after_committed_predecessor(postgres(pool.clone()), &admitted, &count).await;
        }
        pool.close().await;
    }
}
