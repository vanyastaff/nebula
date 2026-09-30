//! An SDK-only credentialed resource: a derived bearer-token slot, read by a
//! unit only through its pinned snapshot. Every name comes from
//! `nebula_sdk`; the derive resolves its generated paths through the SDK.

use std::sync::Arc;

use nebula_sdk::integration::credential::BearerTokenCredential;
use nebula_sdk::integration::resource::{
    Cost, CredentialGuard, CredentialSlot, CredentialUnavailableReason, Effect, Error, ErrorKind,
    Lease, Operation, OperationCx, OperationError, PinSlots, Provider, Resident, ResidentProvider,
    Resource, ResourceContext, ResourceKey, ResourceMetadataDraft, SentState, resource_key,
};
use nebula_sdk::prelude::{Deserialize, SecretString, SecretToken, Serialize};

const TOKEN: &str = "ghp_fixture_secret";

#[derive(Resource)]
struct GitHub {
    #[credential(key = "token")]
    token: CredentialSlot<BearerTokenCredential>,
}

#[async_trait::async_trait]
impl Provider for GitHub {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.github")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("GitHub"),
            "",
        )
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for GitHub {}

/// Reads the pinned token's length; never the live slot.
#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct TokenLength;

impl Operation<GitHub> for TokenLength {
    type Output = usize;
    const KEY: &'static str = "github.token_length";
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OperationCx<'_, GitHub>) -> Result<usize, OperationError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        let token: Option<&CredentialGuard<SecretToken>> = attempt.credentials().token();
        let Some(token) = token else {
            attempt.settle(SentState::NotSent);
            return Err(OperationError::new(
                ErrorKind::CredentialUnavailable {
                    reason: CredentialUnavailableReason::Absent,
                },
                "no bearer token bound",
            ));
        };
        let length = token.token().expose_secret().len();
        attempt.settle(SentState::Sent);
        Ok(length)
    }
}

async fn action_code(github: &Lease<GitHub>) -> Result<usize, Error> {
    Ok(github.submit(TokenLength).await?)
}

fn main() {
    let _action_code = action_code;
    let github = GitHub {
        token: CredentialSlot::<BearerTokenCredential>::empty(),
    };
    assert!(github.pin_slots().token().is_none(), "unbound pins None");

    github
        .token
        .store(Arc::new(CredentialGuard::new(SecretToken::new(
            SecretString::new(TOKEN),
        ))));
    let pinned = github.pin_slots();
    assert!(pinned.token().is_some());
    assert!(
        !format!("{pinned:?}").contains(TOKEN),
        "pinned slots never print material"
    );
}
