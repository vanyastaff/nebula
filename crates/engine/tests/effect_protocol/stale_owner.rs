use super::*;

pub(super) async fn rejects_stale_outcome(ports: Ports) -> Value {
    let mut fixture = Fixture::new(ProviderBehavior::Applied, ports).await;
    let execution = fixture.start().await;
    let gate = Arc::new(faults::OutcomeGate::default());
    let mut ledger = FaultLedger::new(
        fixture.ports.ledger.clone(),
        Boundary::Outcome,
        Fault::After,
    );
    ledger.outcome_gate = Some(gate.clone());
    fixture.ports.stores.operation_ledger = Arc::new(ledger);
    let engine = fixture
        .engine()
        .with_lease_ttl(std::time::Duration::from_secs(30))
        .with_lease_heartbeat_interval(std::time::Duration::from_secs(10));
    let scope = fixture.scope.clone();
    let turn = tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    assert_eq!(fixture.provider.calls.lock().len(), 1);
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
    let phase_before_stale_response = original.protocol().unwrap().phase();
    assert_eq!(
        phase_before_stale_response,
        EffectPhase::InvocationOutstanding
    );
    let stale = gate.fencing.lock().unwrap();
    assert!(
        fixture
            .ports
            .stores
            .execution
            .release_lease(&fixture.scope, &execution.to_string(), stale)
            .await
            .unwrap()
    );
    let successor = fixture
        .ports
        .stores
        .execution
        .acquire_lease(
            &fixture.scope,
            &execution.to_string(),
            "replacement-owner",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let successor_fence_differs = successor != stale;
    assert!(successor_fence_differs);
    gate.release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), turn)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            result,
            Err(nebula_engine::EngineError::Effect(
                nebula_engine::EffectExecutionError::Ledger(
                    nebula_storage_port::dto::OperationLedgerError::ExecutionLeaseRejected
                )
            ))
        ),
        "stale provider response must not finalize outcome: {result:?}"
    );
    let after_stale_response = fixture
        .ports
        .ledger
        .read_exact(&fixture.scope, original.operation().slot_id())
        .await
        .unwrap();
    assert_eq!(after_stale_response, original);
    let stale_storage_mutations = usize::from(after_stale_response != original);
    assert_eq!(
        fixture.provider.applied.lock().len(),
        1,
        "provider effect exists while stale durable outcome remains forbidden"
    );
    assert_eq!(fixture.provider.committed.lock().len(), 1);
    assert!(
        fixture
            .ports
            .stores
            .execution
            .release_lease(&fixture.scope, &execution.to_string(), successor)
            .await
            .unwrap()
    );
    json!({
        "operation_id": original.operation().operation_id().to_string(),
        "provider_calls": fixture.provider.calls.lock().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "provider_commits": fixture.provider.committed.lock().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "unique_business_effects": fixture.provider.applied.lock().len(),
        "phase_before_stale_response": format!("{phase_before_stale_response:?}"),
        "phase_after_stale_response": format!("{:?}", after_stale_response.protocol().unwrap().phase()),
        "successor_fence_differs": successor_fence_differs,
        "stale_storage_mutations": stale_storage_mutations
    })
}
