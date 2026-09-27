//! An SDK-only action whose resource slots are managed rows: a required
//! `#[resource] ManagedRow<Directory>` field, an optional pooled
//! `Option<ManagedRow<Ledger>>`, a custom `Operation`, `?` from a unit's
//! `OpError` into `ActionError`, and one session — all through
//! `nebula_sdk`.

use nebula_sdk::integration::resource::{
    Cost, Effect, Error, ManagedRow, OpCx, OpError, Operation, PoolProvider, Pooled, Provider,
    Resident, ResidentProvider, ResourceContext, ResourceKey, ResourceMetadataDraft, SentState,
    SessionClosed, SessionEnd, SessionProvider, SessionSpec, no_credential_slots, resource_key,
};
use nebula_sdk::prelude::{
    Action, ActionContext, ActionError, ActionResult, StatelessAction, metadata_name,
};

/// A shared directory client counting its lookups.
#[derive(Clone)]
struct Directory;
no_credential_slots!(Directory);

#[async_trait::async_trait]
impl Provider for Directory {
    type Config = ();
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.directory")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), metadata_name!("Directory"), "")
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<u64, Error> {
        Ok(7)
    }
}

impl ResidentProvider for Directory {}

/// Looks a user up: one read attempt.
struct Lookup;

impl Operation<Directory> for Lookup {
    type Output = u64;
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, Directory>) -> Result<u64, OpError> {
        let attempt = cx.attempt(Cost::ONE).await?;
        let found = *attempt.instance();
        attempt.settle(SentState::Sent);
        Ok(found)
    }
}

/// A pooled ledger whose transactions borrow their connection.
#[derive(Clone)]
struct Ledger;
no_credential_slots!(Ledger);

#[derive(Default)]
struct Conn {
    applied: Vec<u64>,
}

struct Tx<'c> {
    conn: &'c mut Conn,
    pending: Vec<u64>,
}

#[async_trait::async_trait]
impl Provider for Ledger {
    type Config = ();
    type Instance = Conn;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.ledger")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), metadata_name!("Ledger"), "")
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<Conn, Error> {
        Ok(Conn::default())
    }
}

impl PoolProvider for Ledger {}

impl SessionProvider for Ledger {
    type Session<'c> = Tx<'c>;

    async fn open<'c>(&'c self, conn: &'c mut Conn, (): &'c ()) -> Result<Tx<'c>, OpError> {
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

/// Records a user's lookup in the ledger when one is bound.
#[derive(Action)]
#[action(
    key = "example.audit_lookup",
    name = "Audit lookup",
    input = u64,
    output = u64
)]
struct AuditLookup {
    #[resource]
    directory: ManagedRow<Directory>,
    #[resource]
    ledger: Option<ManagedRow<Ledger>>,
}

impl StatelessAction for AuditLookup {
    async fn execute(
        &self,
        input: u64,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<u64>, ActionError> {
        let found = self.directory.submit(Lookup).await?;
        if let Some(ledger) = &self.ledger {
            ledger
                .session(SessionSpec::new(Cost::ONE), move |tx, _cx| {
                    Box::pin(async move {
                        tx.pending.push(input);
                        Ok(())
                    })
                })
                .await?;
        }
        Ok(ActionResult::success(found))
    }
}

fn main() {
    let slots = AuditLookup::dependencies().slot_fields();
    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|slot| !slot.lazy));
    let required: Vec<bool> = slots.iter().map(|slot| slot.required).collect();
    assert_eq!(required, [true, false], "the ledger row is optional");
    assert_eq!(Lookup::EFFECT, Effect::Read);
}
