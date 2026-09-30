//! An SDK-only action whose resource slots are resource handles: a required
//! `#[resource] ResourceHandle<Directory>` field, an optional pooled
//! `Option<ResourceHandle<Ledger>>`, a custom read `Operation`, and `?` from a
//! unit's `OperationError` into `ActionError` — all through `nebula_sdk`.

use nebula_sdk::integration::resource::{
    Cost, Effect, Error, Operation, OperationCx, OperationError, PoolProvider, Pooled, Provider,
    Resident, ResidentProvider, ResourceContext, ResourceHandle, ResourceKey,
    ResourceMetadataDraft, SentState, no_credential_slots, resource_key,
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

    async fn run(self, cx: &mut OperationCx<'_, Directory>) -> Result<u64, OperationError> {
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

struct Conn;

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
        Ok(Conn)
    }
}

impl PoolProvider for Ledger {}

/// Looks a user up through an action-scoped, read-only resource handle.
#[derive(Action)]
#[action(
    key = "example.audit_lookup",
    name = "Audit lookup",
    input = u64,
    output = u64,
    read_only
)]
struct AuditLookup {
    #[resource]
    directory: ResourceHandle<Directory>,
    #[resource]
    ledger: Option<ResourceHandle<Ledger>>,
}

impl StatelessAction for AuditLookup {
    async fn execute(
        &self,
        _input: u64,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<u64>, ActionError> {
        let found = self.directory.submit(Lookup).await?;
        let _optional_row = self.ledger.as_ref().map(ResourceHandle::resource_key);
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
