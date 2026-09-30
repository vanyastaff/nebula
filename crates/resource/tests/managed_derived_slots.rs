//! `#[derive(Resource)]` emits `PinSlots` for the managed call facade: a
//! generated `<Name>PinnedSlots` for a credentialed struct (read through
//! `<field>()` accessors, redacted `Debug`), `()` for a slot-less one.
//!
//! This lives outside the crate because the derive's generated paths name
//! `nebula_resource` from a consumer's point of view.

use std::sync::Arc;

use nebula_core::{ResourceKey, resource_key, scope::Scope};
use nebula_credential::{
    Credential, CredentialContext, CredentialError, CredentialGuard, CredentialMetadataDraft,
    SecretString, SecretToken, StaticResolveResult,
};
use nebula_resource::{
    AcquireOptions, CredentialSlot, Error, Manager, PinSlots, RegistrationSpec, Resident,
    ResidentConfig, Resource, ResourceConfig, ResourceContext, ScopeLevel, SlotCell, SlotIdentity,
    call::{Cost, Effect, Operation, OperationCx, OperationError},
    resource::{Provider, ResourceMetadataDraft},
    topology::ResidentProvider,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

#[derive(Clone, nebula_schema::Schema)]
struct MailerConfig {
    version: u64,
}

impl ResourceConfig for MailerConfig {
    fn fingerprint(&self) -> u64 {
        self.version
    }
}

/// A credentialed resource with a raw guard slot and a credential alias slot.
#[derive(Resource)]
struct Mailer {
    #[credential(key = "smtp")]
    smtp: SlotCell<CredentialGuard<ApiToken>>,
    #[credential(key = "api")]
    api: CredentialSlot<ApiToken>,
}

#[async_trait::async_trait]
impl Provider for Mailer {
    type Config = MailerConfig;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("derived.mailer")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), nebula_resource::metadata_name!("Mailer"), "")
    }

    async fn create(&self, _: &MailerConfig, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for Mailer {}

/// A slot-less derived resource pins nothing.
#[derive(Resource)]
struct Plain;

struct ApiToken;

impl Zeroize for ApiToken {
    fn zeroize(&mut self) {}
}

impl Credential for ApiToken {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "derived.api_token";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("derived.api_token"),
            nebula_credential::metadata_name!("ApiToken"),
            "managed facade derive fixture",
        )
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _properties: &(),
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            SecretString::new("token"),
        )))
    }
}

/// Reports which slots the unit pinned.
#[derive(serde::Serialize, serde::Deserialize)]
struct ReadSlots;

impl Operation<Mailer> for ReadSlots {
    type Output = (bool, bool);
    const KEY: &'static str = "mailer.read_slots";
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OperationCx<'_, Mailer>) -> Result<Self::Output, OperationError> {
        cx.call(Cost::FREE, async |(), credentials| {
            let smtp: Option<&CredentialGuard<ApiToken>> = credentials.smtp();
            // The alias slot pins the credential's projected scheme.
            let api: Option<&CredentialGuard<SecretToken>> = credentials.api();
            Ok((smtp.is_some(), api.is_some()))
        })
        .await
    }
}

fn mailer(smtp: Option<Arc<CredentialGuard<ApiToken>>>) -> Mailer {
    let mailer = Mailer {
        smtp: SlotCell::empty(),
        api: CredentialSlot::<ApiToken>::empty(),
    };
    if let Some(guard) = smtp {
        mailer.smtp.store(guard);
    }
    mailer
}

#[test]
fn the_derive_pins_each_slot_once_and_a_rotation_reaches_the_next_pin() {
    let first = Arc::new(CredentialGuard::new(ApiToken));
    let second = Arc::new(CredentialGuard::new(ApiToken));
    let resource = mailer(Some(Arc::clone(&first)));
    let pinned = resource.pin_slots();
    resource.smtp.store(Arc::clone(&second));

    assert!(
        pinned
            .smtp()
            .is_some_and(|guard| std::ptr::eq(guard, Arc::as_ptr(&first))),
        "the pin keeps the guard it loaded"
    );
    assert!(pinned.api().is_none(), "an unbound slot pins None");
    assert_eq!(
        format!("{pinned:?}"),
        "MailerPinnedSlots { smtp: true, api: false }",
        "Debug names the slots, never the material"
    );
    assert!(
        resource
            .pin_slots()
            .smtp()
            .is_some_and(|guard| std::ptr::eq(guard, Arc::as_ptr(&second))),
        "the next pin sees the rotation"
    );

    let (): <Plain as PinSlots>::Pinned = Plain.pin_slots();
}

#[tokio::test]
async fn a_unit_reads_the_derived_pin_through_its_attempt() {
    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource: mailer(Some(Arc::new(CredentialGuard::new(ApiToken)))),
            config: MailerConfig { version: 1 },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let managed = manager
        .acquire::<Mailer>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire")
        .into_lease();
    assert_eq!(
        managed.submit(ReadSlots).await.expect("read"),
        (true, false)
    );
}
