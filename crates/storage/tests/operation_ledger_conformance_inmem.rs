//! Operation-ledger conformance for the in-memory reference model.
//!
//! The in-memory adapter is never a deployment target; running the shared
//! oracle against it keeps the reference model and the two SQL deployment
//! backends answering identically, so a divergence shows up as the same named
//! case failing on one of the three.

#[macro_use]
#[path = "support/operation_ledger_oracle.rs"]
mod oracle;

use nebula_storage::inmem::InMemoryOperationLedger;

async fn ledger() -> Option<(
    InMemoryOperationLedger,
    nebula_storage::InMemoryExecutionStore,
)> {
    let execution = nebula_storage::InMemoryExecutionStore::new();
    Some((InMemoryOperationLedger::new(&execution), execution))
}

operation_ledger_conformance_suite!(ledger());

#[tokio::test]
async fn missing_execution_cannot_authorize_prepare() {
    use nebula_storage_port::store::OperationLedger;
    use nebula_storage_port::{
        AttemptGeneration, DestinationCapability, EffectSlotBinding, RequestFingerprint, Scope,
    };
    let execution = nebula_storage::InMemoryExecutionStore::new();
    let ledger = InMemoryOperationLedger::new(&execution);
    let scope = Scope::new("ws", "org");
    let binding = EffectSlotBinding {
        scope: &scope,
        execution_id: "missing",
        node_key: "node",
        occurrence: "main",
        attempt_generation: AttemptGeneration::new(u64::MAX),
        fingerprint: RequestFingerprint::new(1, [0; 32]),
        destination: DestinationCapability::Opaque,
        contract: oracle::contract(DestinationCapability::Opaque),
    };
    assert!(
        ledger
            .prepare(
                &binding,
                nebula_storage_port::FencingToken::from_generation(u64::MAX)
            )
            .await
            .is_err()
    );
}
