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

async fn ledger() -> Option<InMemoryOperationLedger> {
    Some(InMemoryOperationLedger::new())
}

operation_ledger_conformance_suite!(ledger());
