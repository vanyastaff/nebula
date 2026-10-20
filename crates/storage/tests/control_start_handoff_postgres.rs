//! Storage-boundary oracle for atomic Control Start ownership transfer on the
//! PostgreSQL adapter.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI `postgres-conformance` job.
//! The in-memory and SQLite arms live in `control_start_handoff`.

#![cfg(feature = "postgres")]

#[path = "support/turn_recovery_oracle.rs"]
mod recovery_oracle;

#[path = "support/control_turn_oracle.rs"]
mod control_turn_oracle;

include!("support/control_start_handoff_oracle.rs");

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[tokio::test]
async fn postgres_control_start_handoff() {
    use nebula_storage::postgres::*;
    // An evidence binary never passes without a database: absence is a loud
    // failure, not a silent skip.
    let url = std::env::var("DATABASE_URL").expect(
        "control_start_handoff_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "control_start_handoff")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    oracle(Ports {
        journal: Arc::new(PgJournalReader::new(pool.clone())),
        execution: Arc::new(PgExecutionStore::new(pool.clone())),
        queue: Arc::new(PgControlQueue::new(pool.clone())),
        handoff: Arc::new(PgTurnHandoff::new(pool.clone())),
        recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
        starts: Arc::new(PgStartAcceptanceStore::new(pool.clone())),
        catalog: catalog.clone(),
        admin: catalog,
        parents: Some((
            Arc::new(PgTenantProvisioningStore::new(pool.clone())),
            Arc::new(PgWorkflowStore::new(pool)),
        )),
    })
    .await;
}

/// Ports over one private-schema pool, as each evidence case wires them.
fn postgres_ports(pool: &sqlx::PgPool) -> Ports {
    use nebula_storage::postgres::*;
    let catalog = Arc::new(PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    Ports {
        journal: Arc::new(PgJournalReader::new(pool.clone())),
        execution: Arc::new(PgExecutionStore::new(pool.clone())),
        queue: Arc::new(PgControlQueue::new(pool.clone())),
        handoff: Arc::new(PgTurnHandoff::new(pool.clone())),
        recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
        starts: Arc::new(PgStartAcceptanceStore::new(pool.clone())),
        catalog: catalog.clone(),
        admin: catalog,
        parents: Some((
            Arc::new(PgTenantProvisioningStore::new(pool.clone())),
            Arc::new(PgWorkflowStore::new(pool.clone())),
        )),
    }
}

/// Each private schema is its own recovery universe: an accepted turn in one
/// fixture must never surface in another fixture's recovery scan.
#[tokio::test]
async fn postgres_recovery_scans_are_isolated_between_fixtures() {
    use nebula_storage::postgres::init_schema;
    let url = std::env::var("DATABASE_URL").expect(
        "control_start_handoff_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let first_pool = postgres_schema::connect_with_private_schema(&url, "recovery_isolation_a")
        .await
        .unwrap();
    init_schema(&first_pool).await.unwrap();
    let second_pool = postgres_schema::connect_with_private_schema(&url, "recovery_isolation_b")
        .await
        .unwrap();
    init_schema(&second_pool).await.unwrap();
    let first = postgres_ports(&first_pool);
    let second = postgres_ports(&second_pool);

    let accepted = seed(&first).await;
    let ControlStartAcceptance::Accepted { fence } = first
        .handoff
        .accept_control_start(&accepted.request())
        .await
        .unwrap()
    else {
        panic!("the first fixture accepts its Start")
    };
    assert!(
        first
            .execution
            .release_lease(&accepted.scope, &accepted.execution, fence)
            .await
            .unwrap(),
        "the first fixture must own the lease it releases"
    );

    let first_candidates = first
        .recovery
        .list_recoverable_turns(accepted.flavor, None, 16)
        .await
        .unwrap();
    assert_eq!(first_candidates.turns().len(), 1);
    assert_eq!(
        first_candidates.turns()[0].execution_id(),
        accepted.execution
    );
    assert!(
        second
            .recovery
            .list_recoverable_turns(accepted.flavor, None, 16)
            .await
            .unwrap()
            .turns()
            .is_empty(),
        "a fixture must not observe another fixture's accepted turn"
    );
}

#[tokio::test]
async fn postgres_refusals_survive_observation_faults() {
    use nebula_storage::postgres::*;
    let url = std::env::var("DATABASE_URL").expect(
        "control_start_handoff_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "control_refusal_faults")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    let ports = Ports {
        journal: Arc::new(PgJournalReader::new(pool.clone())),
        execution: Arc::new(PgExecutionStore::new(pool.clone())),
        queue: Arc::new(PgControlQueue::new(pool.clone())),
        handoff: Arc::new(PgTurnHandoff::new(pool.clone())),
        recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
        starts: Arc::new(PgStartAcceptanceStore::new(pool.clone())),
        catalog: catalog.clone(),
        admin: catalog,
        parents: Some((
            Arc::new(PgTenantProvisioningStore::new(pool.clone())),
            Arc::new(PgWorkflowStore::new(pool.clone())),
        )),
    };
    sqlx::query(
        "CREATE OR REPLACE FUNCTION fault_injected() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'injected observation fault'; END $$ LANGUAGE plpgsql",
    )
    .execute(&pool)
    .await
    .unwrap();
    control_turn_oracle::refusals_survive_observation_faults(&ports, async |fault| {
        use control_turn_oracle::ObservationFault;
        let statements: &[&'static str] = match fault {
            ObservationFault::FailReceiptWrite => &[
                "CREATE TRIGGER fault_fail_receipt BEFORE INSERT ON execution_control_observation_receipts FOR EACH ROW EXECUTE FUNCTION fault_injected()",
            ],
            ObservationFault::FailJournalWrite => &[
                "CREATE TRIGGER fault_fail_journal BEFORE INSERT ON execution_journal FOR EACH ROW EXECUTE FUNCTION fault_injected()",
            ],
            // A deferred constraint trigger fires only at COMMIT, after every
            // statement of the refusal transaction succeeded.
            ObservationFault::LoseCommitAcknowledgement => &[
                "CREATE CONSTRAINT TRIGGER fault_lose_commit AFTER INSERT ON execution_journal DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION fault_injected()",
            ],
            // The snapshot stays present but undecodable once its check is gone.
            ObservationFault::CorruptReceiptSnapshot => &[
                "ALTER TABLE execution_control_observation_receipts DROP CONSTRAINT IF EXISTS ck_execution_control_observation_receipts__flavor_shape",
                "UPDATE execution_control_observation_receipts SET expected_flavor_id = decode('01', 'hex') WHERE outcome = 'flavor-mismatch'",
            ],
            ObservationFault::Clear => &[
                "DROP TRIGGER IF EXISTS fault_fail_receipt ON execution_control_observation_receipts",
                "DROP TRIGGER IF EXISTS fault_lose_commit ON execution_journal",
                "DROP TRIGGER IF EXISTS fault_fail_journal ON execution_journal",
            ],
        };
        for statement in statements {
            sqlx::query(*statement).execute(&pool).await.unwrap();
        }
    })
    .await;
}
