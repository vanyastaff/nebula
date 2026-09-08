use super::*;

#[derive(Debug, Clone, Copy)]
enum Backend {
    Memory,
    Sqlite,
    Postgres,
}

enum Database {
    Memory(Arc<nebula_storage::InMemoryExecutionStore>),
    Sqlite {
        pool: sqlx::SqlitePool,
        options: sqlx::sqlite::SqliteConnectOptions,
        _directory: tempfile::TempDir,
    },
    Postgres {
        pool: sqlx::PgPool,
        options: sqlx::postgres::PgConnectOptions,
    },
}

impl Database {
    async fn open(backend: Backend) -> Option<Self> {
        Some(match backend {
            Backend::Memory => {
                Self::Memory(Arc::new(nebula_storage::InMemoryExecutionStore::new()))
            },
            Backend::Sqlite => {
                let directory = tempfile::tempdir().unwrap();
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("effect-restart.db"))
                    .create_if_missing(true)
                    .busy_timeout(std::time::Duration::from_secs(10));
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
                nebula_storage::sqlite::init_schema(&pool).await.unwrap();
                Self::Sqlite {
                    pool,
                    options,
                    _directory: directory,
                }
            },
            Backend::Postgres => {
                let Ok(url) = std::env::var("DATABASE_URL") else {
                    assert!(
                        std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                        "required PostgreSQL restart evidence needs DATABASE_URL"
                    );
                    return None;
                };
                let pool = postgres_schema::connect_with_private_schema(&url, "effect_restart")
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
                Self::Postgres { pool, options }
            },
        })
    }
    fn ports(&self) -> Ports {
        match self {
            Self::Memory(core) => Ports::memory_core(core.clone()),
            Self::Sqlite { pool, .. } => Ports::sqlite(pool.clone()),
            Self::Postgres { pool, .. } => Ports::postgres(pool.clone()),
        }
    }
    async fn reconnect(&mut self) -> Ports {
        match self {
            Self::Memory(_) => {},
            Self::Sqlite { pool, options, .. } => {
                pool.close().await;
                *pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
            },
            Self::Postgres { pool, options } => {
                pool.close().await;
                *pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(4)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
            },
        }
        self.ports()
    }
}

#[derive(Debug, Clone, Copy)]
enum Recovery {
    Prepared,
    Outstanding,
    KnownOutput,
    UnavailableOutput,
    ChangedRequest,
    ChangedDestination,
    CrashedInvocation,
}

fn assert_failed_effect(
    result: &nebula_engine::ExecutionResult,
    expected: nebula_engine::EffectExecutionError,
) {
    assert_eq!(result.status, ExecutionStatus::Failed);
    let expected = nebula_engine::EngineError::Effect(expected).to_string();
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
async fn restart_recovers_only_persisted_authority(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    for recovery in [
        Recovery::Prepared,
        Recovery::Outstanding,
        Recovery::KnownOutput,
        Recovery::UnavailableOutput,
        Recovery::ChangedRequest,
        Recovery::ChangedDestination,
        Recovery::CrashedInvocation,
    ] {
        let behavior = match recovery {
            Recovery::CrashedInvocation => ProviderBehavior::PendingInvocation,
            Recovery::UnavailableOutput => ProviderBehavior::Oversized,
            _ => ProviderBehavior::Applied,
        };
        let mut fixture = Fixture::new(behavior, database.ports()).await;
        let execution = fixture.start().await;
        let admitted = fixture
            .ports
            .stores
            .execution
            .get(&fixture.scope, &execution.to_string())
            .await
            .unwrap()
            .unwrap();
        let (boundary, fault) = match recovery {
            Recovery::Outstanding => (Boundary::Grant, Fault::After),
            Recovery::KnownOutput | Recovery::UnavailableOutput => {
                (Boundary::Outcome, Fault::AfterReadUnavailable)
            },
            _ => (Boundary::Prepare, Fault::AfterReadUnavailable),
        };
        if matches!(recovery, Recovery::CrashedInvocation) {
            let engine = fixture.engine();
            let scope = fixture.scope.clone();
            let turn =
                tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                fixture.provider.entered.notified(),
            )
            .await
            .unwrap();
            turn.abort();
            assert!(turn.await.unwrap_err().is_cancelled());
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                fixture.provider.dropped.notified(),
            )
            .await
            .unwrap();
        } else {
            fixture.ports.stores.operation_ledger = Arc::new(FaultLedger::new(
                fixture.ports.ledger.clone(),
                boundary,
                fault,
            ));
            let first = fixture
                .engine()
                .resume_execution(&fixture.scope, execution)
                .await;
            assert!(
                matches!(
                    first,
                    Err(nebula_engine::EngineError::Effect(
                        nebula_engine::EffectExecutionError::Ledger(
                            nebula_storage_port::dto::OperationLedgerError::AcknowledgementUnknown
                        )
                    ))
                ),
                "{recovery:?}: {first:?}"
            );
        }
        let original = fixture
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
        assert_eq!(
            fixture.provider.calls.lock().len(),
            usize::from(matches!(
                recovery,
                Recovery::KnownOutput | Recovery::UnavailableOutput | Recovery::CrashedInvocation
            ))
        );
        let persisted = fixture
            .ports
            .stores
            .execution
            .get(&fixture.scope, &execution.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.state, admitted.state,
            "deferred recovery must retain the exact original checkpoint"
        );
        assert_eq!(persisted.version, admitted.version);
        fixture.ports = database.reconnect().await;
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        match recovery {
            Recovery::ChangedRequest => {
                *fixture.provider.request_override.lock() = Some(json!({"amount": 8}));
            },
            Recovery::ChangedDestination => {
                *fixture.provider.destination.lock() =
                    b"provider/account-b/auth-binding-a".to_vec();
            },
            _ => {},
        }
        let second = fixture
            .engine()
            .resume_execution(&fixture.scope, execution)
            .await;
        let record = fixture
            .ports
            .ledger
            .read_exact(&fixture.scope, original.operation().slot_id())
            .await
            .unwrap();
        assert_eq!(
            fixture.provider.committed.lock().len(),
            fixture.provider.calls.lock().len(),
            "opaque provider commit log must not hide duplicate calls across recovery"
        );
        assert_eq!(
            record.operation(),
            original.operation(),
            "recovery must keep the minted identity"
        );
        match recovery {
            Recovery::Prepared | Recovery::KnownOutput => {
                let result = second.unwrap();
                assert_eq!(result.status, ExecutionStatus::Completed);
                assert_eq!(
                    result.node_outputs[&node_key!("send")],
                    json!({"receipt": "provider-receipt-a"})
                );
                assert_eq!(fixture.provider.calls.lock().len(), 1);
                if matches!(recovery, Recovery::KnownOutput) {
                    assert_eq!(
                        record, original,
                        "output recovery must replay exact frozen evidence"
                    );
                }
            },
            Recovery::UnavailableOutput => {
                assert_eq!(fixture.provider.calls.lock().len(), 1);
                assert_eq!(record, original);
                assert_eq!(
                    record.protocol().unwrap().evidence().unwrap().outcome(),
                    nebula_storage_port::dto::KnownOutcome::Succeeded
                );
                assert_failed_effect(
                    &second.unwrap(),
                    nebula_engine::EffectExecutionError::OutputUnavailable {
                        operation_id: original.operation().operation_id(),
                    },
                );
            },
            Recovery::Outstanding | Recovery::CrashedInvocation => {
                assert_eq!(
                    fixture.provider.calls.lock().len(),
                    usize::from(matches!(recovery, Recovery::CrashedInvocation)),
                    "a persisted permit with lost ACK is not invocation authority"
                );
                assert_eq!(
                    record.protocol().unwrap().phase(),
                    EffectPhase::OutcomeUnknown
                );
                assert_failed_effect(
                    &second.unwrap(),
                    nebula_engine::EffectExecutionError::OutcomeUnknown {
                        operation_id: original.operation().operation_id(),
                    },
                );
            },
            Recovery::ChangedRequest | Recovery::ChangedDestination => {
                assert_eq!(fixture.provider.calls.lock().len(), 0);
                assert_eq!(
                    record, original,
                    "binding mismatch must leave the ledger unchanged"
                );
                assert_failed_effect(
                    &second.unwrap(),
                    nebula_engine::EffectExecutionError::Ledger(
                        nebula_storage_port::dto::OperationLedgerError::OperationMismatch {
                            slot_id: original.operation().slot_id(),
                        },
                    ),
                );
            },
        }
    }
}
