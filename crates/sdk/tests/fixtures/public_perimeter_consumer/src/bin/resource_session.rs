//! An SDK-only session provider: a pooled ledger whose transactions borrow
//! their connection, and the action-side code that runs one through a
//! resource handle, all through `nebula_sdk::integration::resource`.

use nebula_sdk::integration::resource::{
    Cost, Effect, Error, OperationError, PoolProvider, Pooled, Provider, ResourceContext,
    ResourceHandle, ResourceKey, ResourceMetadataDraft, SessionBinding, SessionClosed, SessionEnd,
    SessionProvider, SessionSpec, no_credential_slots, resource_key,
};

/// A connection that applies statements only when a transaction commits.
#[derive(Default)]
struct Conn {
    applied: Vec<String>,
}

/// A transaction borrowing its connection for its whole lifetime.
struct Tx<'c> {
    conn: &'c mut Conn,
    pending: Vec<String>,
}

impl Tx<'_> {
    fn execute(&mut self, statement: &str) -> u64 {
        self.pending.push(statement.to_owned());
        1
    }
}

#[derive(Clone)]
struct Ledger;
no_credential_slots!(Ledger);

#[async_trait::async_trait]
impl Provider for Ledger {
    type Config = ();
    type Instance = Conn;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.ledger")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("Ledger"),
            "",
        )
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<Conn, Error> {
        Ok(Conn::default())
    }
}

impl PoolProvider for Ledger {}

impl SessionProvider for Ledger {
    type Session<'c> = Tx<'c>;

    async fn open<'c>(&'c self, conn: &'c mut Conn, _slots: &'c ()) -> Result<Tx<'c>, OperationError> {
        Ok(Tx {
            conn,
            pending: Vec::new(),
        })
    }

    async fn close<'c>(&'c self, tx: Tx<'c>, end: SessionEnd) -> SessionClosed {
        if end == SessionEnd::Commit {
            tx.conn.applied.extend(tx.pending);
            SessionClosed::Committed
        } else {
            SessionClosed::RolledBack { refused: None }
        }
    }
}

/// What action code does with a managed row: one transaction, booked once.
async fn transfer(ledger: &ResourceHandle<Ledger>) -> Result<u64, Error> {
    let rows = ledger
        .session(SessionSpec::write("ledger.transfer", &1_u64), |tx, cx| {
            let key = cx.resource_key().clone();
            Box::pin(async move {
                let debit = tx.execute("update accounts set balance = balance - 1");
                let credit = tx.execute("update accounts set balance = balance + 1");
                assert_eq!(key, Ledger::key());
                Ok(debit + credit)
            })
        })
        .await?;
    Ok(rows)
}

fn main() {
    let _action_code = transfer;
    assert_eq!(Ledger::BINDING, SessionBinding::Connection);
    let spec = SessionSpec::write("ledger.transfer", &1_u64).cost(Cost::ONE);
    assert_eq!(spec.effect(), Effect::Write);
    let idempotent = SessionSpec::idempotent("ledger.settle", &("alice", 1_u64))
        .idempotency_key("settle-alice-1");
    assert_eq!(idempotent.effect(), Effect::Idempotent);
    let read = SessionSpec::read("ledger.balance").cost(Cost::FREE);
    assert!(read.effect().is_replay_safe());
}
