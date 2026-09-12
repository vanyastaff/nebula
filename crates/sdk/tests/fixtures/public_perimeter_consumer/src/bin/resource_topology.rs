use nebula_sdk::integration::resource::{
    AdmissionPhase, CreatedEntry, Error, HookFault, InstanceStore, Load, MaintenanceSchedule,
    Provider, ResourceContext, ResourceKey, RetainedId, RetainedLease, RetainedStore, TeardownCx,
    Ticket, Topology, Unavailable, no_credential_slots, resource_key,
};

struct OwnedConnection(String);
struct CustomProvider;
struct CustomTopology;
no_credential_slots!(CustomProvider);

#[async_trait::async_trait]
impl Provider for CustomProvider {
    type Config = ();
    type Instance = OwnedConnection;
    type Topology = CustomTopology;

    fn metadata() -> nebula_sdk::integration::resource::ResourceMetadataDraft {
        nebula_sdk::integration::resource::ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("CustomProvider"),
            "",
        )
    }

    fn key() -> ResourceKey {
        resource_key!("example.custom-topology")
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<OwnedConnection, Error> {
        Ok(OwnedConnection(String::from("exclusive connection")))
    }

    async fn destroy(&self, connection: OwnedConnection, _: TeardownCx) -> Result<(), Error> {
        drop(connection.0);
        Ok(())
    }
}

impl Topology<CustomProvider> for CustomTopology {
    type Entry = OwnedConnection;

    fn try_reserve(&self, _: &InstanceStore<Self::Entry>) -> Result<Ticket, Unavailable> {
        Ok(Ticket::infallible())
    }

    async fn create_entry(
        &self,
        provider: &CustomProvider,
        config: &(),
        ctx: &ResourceContext,
        _: &RetainedStore<Self::Entry>,
    ) -> Result<CreatedEntry<Self::Entry>, Error> {
        Ok(CreatedEntry::new(provider.create(config, ctx).await?))
    }

    fn entry_instance<'s>(&self, entry: &'s Self::Entry) -> &'s OwnedConnection {
        entry
    }
    fn into_owned_instance(&self, entry: Self::Entry) -> Option<OwnedConnection> {
        Some(entry)
    }

    async fn dispatch_credential_hook(
        &self,
        _: &CustomProvider,
        _: &InstanceStore<Self::Entry>,
        _: &RetainedStore<Self::Entry>,
        _: &str,
        _: bool,
    ) -> Result<(), HookFault> {
        Ok(())
    }

    fn phase(&self, _: &InstanceStore<Self::Entry>) -> AdmissionPhase {
        AdmissionPhase::Ready
    }
    fn load(&self, _: &InstanceStore<Self::Entry>) -> Option<Load> {
        None
    }
    fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> {
        None
    }
}

// Only a framework-supplied borrow grants access; no store is constructed here.
fn retained_lease<E: Clone + Send + Sync + 'static>(
    store: &RetainedStore<E>,
    id: RetainedId,
) -> Option<RetainedLease<'_, E>> {
    store.lease(id)
}

fn main() {
    let _ = CustomProvider::key();
    let _ = CustomTopology;
    let _ = retained_lease::<std::sync::Arc<OwnedConnection>>;
}
