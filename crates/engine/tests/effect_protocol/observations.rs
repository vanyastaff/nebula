use std::{collections::BTreeSet, fs::OpenOptions, io::BufWriter, path::Path};

use super::*;

#[derive(Debug, Clone, Copy)]
enum Backend {
    Memory,
    Sqlite,
    Postgres,
}

impl Backend {
    const fn name(self) -> &'static str {
        match self {
            Self::Memory => "in-memory",
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgresql",
        }
    }

    const fn output_env(self) -> &'static str {
        match self {
            Self::Memory => "NEBULA_REMOTE_EFFECT_IN_MEMORY_OBSERVATIONS_PATH",
            Self::Sqlite => "NEBULA_REMOTE_EFFECT_SQLITE_OBSERVATIONS_PATH",
            Self::Postgres => "NEBULA_REMOTE_EFFECT_POSTGRES_OBSERVATIONS_PATH",
        }
    }
}

async fn record_behavior(ports: Ports, behavior: ProviderBehavior, scenario: &str) -> Value {
    let fixture = Fixture::new(behavior, ports).await;
    let execution = fixture.start().await;
    let first = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await;
    let calls_after_first = fixture.provider.calls.lock().len();
    let recovery_attempted = matches!(behavior, ProviderBehavior::Ambiguous);
    if recovery_attempted {
        let _second = fixture
            .engine()
            .resume_execution(&fixture.scope, execution)
            .await;
    }
    let record = fixture
        .ports
        .ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &fixture.scope,
            &execution.to_string(),
            "send",
            "node-effect/v1",
        ))
        .await
        .unwrap()
        .unwrap();
    let protocol = record.protocol().unwrap();
    let operation_id = record.operation().operation_id();
    let provider_calls = fixture.provider.calls.lock().clone();
    let provider_commits = fixture.provider.committed.lock().clone();
    let unique_business_effects = fixture.provider.applied.lock().len();
    assert!(provider_calls.iter().all(|call| *call == operation_id));
    assert!(
        provider_commits
            .iter()
            .all(|commit| *commit == operation_id)
    );
    assert_eq!(provider_commits.len(), 1);
    assert_eq!(unique_business_effects, 1);
    let execution_completed = first.is_ok_and(|result| result.status == ExecutionStatus::Completed);
    let (expected_calls, expected_phase, expected_evidence, expected_completion) = match behavior {
        ProviderBehavior::Applied => (1, EffectPhase::Resolved, true, true),
        ProviderBehavior::Ambiguous => (1, EffectPhase::OutcomeUnknown, false, false),
        ProviderBehavior::StableRetry => (2, EffectPhase::Resolved, true, true),
        _ => panic!("ordinary observation received unsupported provider behavior"),
    };
    assert_eq!(provider_calls.len(), expected_calls);
    assert_eq!(calls_after_first, expected_calls);
    assert_eq!(
        protocol.invocations(),
        u32::try_from(expected_calls).unwrap()
    );
    assert_eq!(protocol.queries(), 0);
    assert_eq!(protocol.phase(), expected_phase);
    assert_eq!(protocol.evidence().is_some(), expected_evidence);
    assert_eq!(execution_completed, expected_completion);
    json!({
        "scenario": scenario,
        "operation_id": operation_id.to_string(),
        "provider_calls": provider_calls.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "provider_commits": provider_commits.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "unique_business_effects": unique_business_effects,
        "calls_after_first_turn": calls_after_first,
        "recovery_attempted": recovery_attempted,
        "phase": format!("{:?}", protocol.phase()),
        "invocations": protocol.invocations(),
        "queries": protocol.queries(),
        "known_evidence": protocol.evidence().is_some(),
        "execution_completed": execution_completed
    })
}

async fn record_crash(ports: Ports) -> Value {
    let fixture = Fixture::new(ProviderBehavior::PendingInvocation, ports).await;
    let execution = fixture.start().await;
    let engine = Arc::new(fixture.engine());
    let crashed_engine = Arc::downgrade(&engine);
    let turn_engine = Arc::clone(&engine);
    let scope = fixture.scope.clone();
    let turn = tokio::spawn(async move { turn_engine.resume_execution(&scope, execution).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.provider.entered.notified(),
    )
    .await
    .unwrap();
    drop(engine);
    turn.abort();
    assert!(turn.await.unwrap_err().is_cancelled());
    let engine_recreated = crashed_engine.upgrade().is_none();
    assert!(
        engine_recreated,
        "the interrupted engine must be fully dropped"
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.provider.dropped.notified(),
    )
    .await
    .unwrap();
    let before = fixture.provider.calls.lock().len();
    let _recovery = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await;
    let record = fixture
        .ports
        .ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &fixture.scope,
            &execution.to_string(),
            "send",
            "node-effect/v1",
        ))
        .await
        .unwrap()
        .unwrap();
    let operation_id = record.operation().operation_id();
    let provider_calls = fixture.provider.calls.lock().clone();
    let provider_commits = fixture.provider.committed.lock().clone();
    let unique_business_effects = fixture.provider.applied.lock().len();
    let calls_after_recovery = provider_calls.len();
    let phase = record.protocol().unwrap().phase();
    assert_eq!(before, 1);
    assert_eq!(calls_after_recovery, 1);
    assert_eq!(provider_calls, vec![operation_id]);
    assert_eq!(provider_commits, vec![operation_id]);
    assert_eq!(unique_business_effects, 1);
    assert_eq!(phase, EffectPhase::InvocationOutstanding);
    json!({
        "scenario": "crashed-invocation-recovery",
        "operation_id": operation_id.to_string(),
        "provider_calls": provider_calls.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "provider_commits": provider_commits.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "unique_business_effects": unique_business_effects,
        "calls_before_recovery": before,
        "calls_after_recovery": calls_after_recovery,
        "phase": format!("{phase:?}"),
        "engine_recreated": engine_recreated
    })
}

fn write_report(backend: Backend, scenarios: Vec<Value>) {
    let Ok(path) = std::env::var(backend.output_env()) else {
        return;
    };
    let path = Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let names = scenarios
        .iter()
        .map(|scenario| scenario["scenario"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), scenarios.len());
    let report = json!({
        "producer_version": 1,
        "contract": "remote-effect-protocol",
        "scenario_inventory_version": 1,
        "backend": backend.name(),
        "scenarios": scenarios
    });
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(BufWriter::new(file), &report).unwrap();
}

async fn run(backend: Backend, make_ports: impl Fn() -> Ports) {
    let scenarios = vec![
        record_behavior(make_ports(), ProviderBehavior::Applied, "applied").await,
        record_behavior(
            make_ports(),
            ProviderBehavior::Ambiguous,
            "opaque-ambiguity-recovery",
        )
        .await,
        record_behavior(
            make_ports(),
            ProviderBehavior::StableRetry,
            "stable-key-retry",
        )
        .await,
        record_crash(make_ports()).await,
        {
            let mut value = stale_owner::rejects_stale_outcome(make_ports()).await;
            value["scenario"] = json!("stale-owner-fence");
            value
        },
    ];
    write_report(backend, scenarios);
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn remote_effect_protocol_raw_observations(#[case] backend: Backend) {
    match backend {
        Backend::Memory => run(backend, Ports::memory).await,
        Backend::Sqlite => {
            let directory = tempfile::tempdir().unwrap();
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(directory.path().join("remote-effects.db"))
                .create_if_missing(true)
                .busy_timeout(std::time::Duration::from_secs(10));
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(4)
                .connect_with(options)
                .await
                .unwrap();
            nebula_storage::sqlite::init_schema(&pool).await.unwrap();
            run(backend, || Ports::sqlite(pool.clone())).await;
            pool.close().await;
        },
        Backend::Postgres => {
            let Ok(url) = std::env::var("DATABASE_URL") else {
                assert!(
                    std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                    "required PostgreSQL remote-effect evidence needs DATABASE_URL"
                );
                return;
            };
            let pool = postgres_schema::connect_with_private_schema(&url, "effect_observations")
                .await
                .unwrap();
            nebula_storage::postgres::init_schema(&pool).await.unwrap();
            run(backend, || Ports::postgres(pool.clone())).await;
            pool.close().await;
        },
    }
}
