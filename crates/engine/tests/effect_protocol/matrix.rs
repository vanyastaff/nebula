use super::*;

#[derive(Debug, Clone, Copy)]
enum Backend {
    Memory,
    Sqlite,
    Postgres,
}

async fn run_cases(make_ports: impl Fn() -> Ports) {
    let _stale_observation = stale_owner::rejects_stale_outcome(make_ports()).await;
    {
        let mut fixture = Fixture::new(ProviderBehavior::Applied, make_ports()).await;
        fixture.definition.nodes.push(
            NodeDefinition::new(node_key!("send_again"), "Send again", "provider", "send").unwrap(),
        );
        WorkflowActivationService::new(
            fixture.ports.workflows.workflow.clone(),
            fixture.ports.workflows.versions.clone(),
            fixture.frozen.clone(),
            PlanFlavorRevisionInstaller::new(fixture.ports.writer.clone()),
            Arc::new(SystemClock),
        )
        .activate(
            &fixture.scope,
            fixture.definition.id,
            2,
            fixture.definition.clone(),
        )
        .await
        .unwrap();
        let execution = fixture.start().await;
        let result = fixture
            .engine()
            .resume_execution(&fixture.scope, execution)
            .await
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::Completed);
        let calls = fixture.provider.calls.lock();
        assert_eq!(calls.len(), 2);
        assert_ne!(
            calls[0], calls[1],
            "distinct nodes must never share an operation identity"
        );
        assert_eq!(fixture.provider.applied.lock().len(), 1);
        assert_eq!(fixture.provider.committed.lock().len(), 1);
    }
    for behavior in [
        ProviderBehavior::PreparationTimeout,
        ProviderBehavior::InvocationTimeout,
    ] {
        let fixture = Fixture::new(behavior, make_ports()).await;
        let execution = fixture.start().await;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.engine().resume_execution(&fixture.scope, execution),
        )
        .await
        .unwrap();
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
            .unwrap();
        if matches!(behavior, ProviderBehavior::PreparationTimeout) {
            assert!(record.is_none());
            assert!(fixture.provider.calls.lock().is_empty());
            assert!(matches!(
                result,
                Err(nebula_engine::EngineError::Effect(
                    nebula_engine::EffectExecutionError::Preparation(
                        EffectPreparationError::Unavailable
                    )
                ))
            ));
        } else {
            assert_eq!(fixture.provider.calls.lock().len(), 1);
            assert_eq!(
                record.unwrap().protocol().unwrap().phase(),
                EffectPhase::OutcomeUnknown
            );
        }
    }
    {
        let fixture = Fixture::new(ProviderBehavior::PendingInvocation, make_ports()).await;
        let execution = fixture.start().await;
        let engine = Arc::new(fixture.engine());
        let active = engine.clone();
        let scope = fixture.scope.clone();
        let turn = tokio::spawn(async move { active.resume_execution(&scope, execution).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.provider.entered.notified(),
        )
        .await
        .unwrap();
        assert!(engine.cancel_execution(execution));
        let _result = tokio::time::timeout(std::time::Duration::from_secs(5), turn)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.provider.dropped.notified(),
        )
        .await
        .unwrap();
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
        assert_eq!(fixture.provider.calls.lock().len(), 1);
        assert!(
            record.protocol().unwrap().evidence().is_none(),
            "cancellation cannot invent a known provider outcome"
        );
        assert_eq!(record.protocol().unwrap().invocations(), 1);
    }
    for behavior in [
        ProviderBehavior::Applied,
        ProviderBehavior::Ambiguous,
        ProviderBehavior::StableRetry,
        ProviderBehavior::StableExhausted,
        ProviderBehavior::QueryApplied,
        ProviderBehavior::QueryInconclusive,
        ProviderBehavior::Oversized,
        ProviderBehavior::BeforeBoundary,
        ProviderBehavior::Rejected,
    ] {
        let fixture = Fixture::new(behavior, make_ports()).await;
        let execution = fixture.start().await;
        let result = fixture
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
        let calls = fixture.provider.calls.lock().clone();
        assert_eq!(
            calls.len(),
            if matches!(
                behavior,
                ProviderBehavior::StableRetry
                    | ProviderBehavior::StableExhausted
                    | ProviderBehavior::BeforeBoundary
            ) {
                2
            } else {
                1
            },
            "provider call budget for {behavior:?}: {result:?}"
        );
        assert!(
            calls
                .iter()
                .all(|id| *id == record.operation().operation_id())
        );
        assert_eq!(
            fixture.provider.applied.lock().len(),
            usize::from(!matches!(
                behavior,
                ProviderBehavior::Rejected | ProviderBehavior::QueryRejected
            )),
            "stable retries must deduplicate the committed provider business effect"
        );
        assert_eq!(
            fixture.provider.committed.lock().len(),
            usize::from(!matches!(
                behavior,
                ProviderBehavior::Rejected | ProviderBehavior::QueryRejected
            )),
            "provider commit log must preserve every actual business effect for {behavior:?}"
        );
        let protocol = record.protocol().unwrap();
        if matches!(
            behavior,
            ProviderBehavior::Ambiguous
                | ProviderBehavior::StableExhausted
                | ProviderBehavior::QueryInconclusive
        ) {
            assert_eq!(protocol.phase(), EffectPhase::OutcomeUnknown);
        } else {
            assert_eq!(
                protocol.phase(),
                EffectPhase::Resolved,
                "{behavior:?}: {result:?}"
            );
            let evidence = protocol.evidence().unwrap();
            assert!(evidence.payload().len() <= 1_048_576);
            if matches!(behavior, ProviderBehavior::Oversized) {
                assert_eq!(
                    evidence.outcome(),
                    nebula_storage_port::dto::KnownOutcome::Succeeded
                );
                assert!(String::from_utf8_lossy(evidence.payload()).contains("OutputUnavailable"));
            } else if matches!(
                behavior,
                ProviderBehavior::Rejected | ProviderBehavior::QueryRejected
            ) {
                assert_eq!(
                    evidence.outcome(),
                    nebula_storage_port::dto::KnownOutcome::Failed
                );
                assert_eq!(result.unwrap().status, ExecutionStatus::Failed);
            } else {
                assert_eq!(result.unwrap().status, ExecutionStatus::Completed);
            }
        }
        let queries = fixture.provider.queries.lock();
        if matches!(
            behavior,
            ProviderBehavior::QueryApplied
                | ProviderBehavior::QueryInconclusive
                | ProviderBehavior::QueryRejected
        ) {
            assert_eq!(&*queries, &[record.operation().operation_id()]);
        } else {
            assert!(queries.is_empty());
        }
    }
    {
        let mut fixture = Fixture::new(ProviderBehavior::Applied, make_ports()).await;
        let execution = fixture.start().await;
        fixture.ports.stores.operation_ledger = Arc::new(FaultLedger::new(
            fixture.ports.ledger.clone(),
            Boundary::Prepare,
            Fault::PreparedWithoutPersistence,
        ));
        let result = fixture
            .engine()
            .resume_execution(&fixture.scope, execution)
            .await;
        assert_eq!(
            result.unwrap().status,
            ExecutionStatus::Failed,
            "a fabricated prepare outcome must fail the execution"
        );
        assert!(
            fixture.provider.calls.lock().is_empty(),
            "the provider must not receive an operation identity absent from durable storage"
        );
        assert!(
            fixture
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
                .is_none()
        );
    }
    for boundary in [Boundary::Prepare, Boundary::Grant, Boundary::Outcome] {
        for fault in [Fault::Before, Fault::After, Fault::AfterReadUnavailable] {
            let mut fixture = Fixture::new(ProviderBehavior::Applied, make_ports()).await;
            let execution = fixture.start().await;
            let ledger = Arc::new(FaultLedger::new(
                fixture.ports.ledger.clone(),
                boundary,
                fault,
            ));
            fixture.ports.stores.operation_ledger = ledger.clone();
            let result = fixture
                .engine()
                .resume_execution(&fixture.scope, execution)
                .await;
            let calls = fixture.provider.calls.lock().len();
            let expected_calls = match (boundary, fault) {
                (Boundary::Prepare, Fault::Before | Fault::AfterReadUnavailable)
                | (Boundary::Grant, _) => 0,
                _ => 1,
            };
            assert_eq!(calls, expected_calls, "{boundary:?}/{fault:?}: {result:?}");
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
                .unwrap();
            if matches!(fault, Fault::AfterReadUnavailable) {
                assert!(
                    matches!(result, Err(nebula_engine::EngineError::Effect(
                    nebula_engine::EffectExecutionError::Ledger(
                        nebula_storage_port::dto::OperationLedgerError::AcknowledgementUnknown)))),
                    "uncertain commit must remain uncertain after failed read: {result:?}"
                );
                assert!(record.is_some(), "lost ACK must not lose the original slot");
                continue;
            }
            if boundary == Boundary::Prepare {
                assert_eq!(
                    ledger
                        .natural_reads
                        .load(std::sync::atomic::Ordering::SeqCst),
                    1
                );
            }
            if boundary == Boundary::Outcome {
                let record = record.unwrap();
                let evidence = record.protocol().unwrap().evidence().unwrap();
                let attempts = ledger.outcome_attempts.lock();
                assert_eq!(
                    attempts.len(),
                    if matches!(fault, Fault::Before) { 2 } else { 1 }
                );
                assert!(attempts.iter().all(|attempt| attempt == evidence));
                assert_eq!(result.unwrap().status, ExecutionStatus::Completed);
            } else if boundary == Boundary::Grant {
                let record = record.unwrap();
                assert_eq!(
                    record.protocol().unwrap().invocations(),
                    u32::from(matches!(fault, Fault::After))
                );
                assert!(record.protocol().unwrap().evidence().is_none());
            } else if matches!(fault, Fault::Before) {
                assert!(record.is_none());
            } else {
                let record = record.unwrap();
                let calls = fixture.provider.calls.lock();
                assert_eq!(record.protocol().unwrap().phase(), EffectPhase::Resolved);
                assert_eq!(calls.as_slice(), &[record.operation().operation_id()]);
                assert_eq!(result.unwrap().status, ExecutionStatus::Completed);
            }
        }
    }
}

async fn run_cases_with_bounded_state(make_ports: impl Fn() -> Ports) {
    Box::pin(run_cases(make_ports)).await;
}

#[tokio::test]
async fn reconciliation_rejection_is_terminal_without_reinvocation() {
    let fixture = Fixture::new(ProviderBehavior::QueryRejected, Ports::memory()).await;
    let execution = fixture.start().await;
    let result = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await
        .unwrap();
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
    let evidence = protocol.evidence().unwrap();

    assert_eq!(fixture.provider.calls.lock().len(), 1);
    assert!(fixture.provider.applied.lock().is_empty());
    assert!(fixture.provider.committed.lock().is_empty());
    assert_eq!(
        fixture.provider.queries.lock().as_slice(),
        &[record.operation().operation_id()]
    );
    assert_eq!(protocol.phase(), EffectPhase::Resolved);
    assert_eq!(
        evidence.outcome(),
        nebula_storage_port::dto::KnownOutcome::Failed
    );
    std::assert_matches!(
        evidence.source(),
        nebula_storage_port::dto::OutcomeEvidenceSource::Reconciliation(_)
    );
    assert_eq!(result.status, ExecutionStatus::Failed);
    let expected =
        nebula_engine::EngineError::Effect(nebula_engine::EffectExecutionError::Rejected {
            operation_id: record.operation().operation_id(),
            code: nebula_action::effect::EffectFailureCode::Rejected,
        })
        .to_string();
    assert_eq!(
        result
            .node_errors
            .get(&node_key!("send"))
            .map(String::as_str),
        Some(expected.as_str())
    );
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn provider_fault_matrix(#[case] backend: Backend) {
    match backend {
        Backend::Memory => run_cases_with_bounded_state(Ports::memory).await,
        Backend::Sqlite => {
            let directory = tempfile::tempdir().unwrap();
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(directory.path().join("effects.db"))
                .create_if_missing(true)
                .busy_timeout(std::time::Duration::from_secs(10));
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(4)
                .connect_with(options)
                .await
                .unwrap();
            nebula_storage::sqlite::init_schema(&pool).await.unwrap();
            run_cases_with_bounded_state(|| Ports::sqlite(pool.clone())).await;
            pool.close().await;
        },
        Backend::Postgres => {
            let Ok(url) = std::env::var("DATABASE_URL") else {
                assert!(
                    std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                    "required PostgreSQL provider evidence needs DATABASE_URL"
                );
                return;
            };
            let pool = postgres_schema::connect_with_private_schema(&url, "effect_protocol")
                .await
                .unwrap();
            nebula_storage::postgres::init_schema(&pool).await.unwrap();
            run_cases_with_bounded_state(|| Ports::postgres(pool.clone())).await;
            pool.close().await;
        },
    }
}
