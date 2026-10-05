//! An SDK-only operation that makes its provider call through
//! `OperationCx::call` and classifies the client's answer once: the
//! runtime derives the sent state, the rate limit's verdict and any
//! re-attempt from the classification alone.

use std::{num::NonZeroU32, time::Duration};

use nebula_sdk::integration::resource::{
    Cost, Effect, Error, ErrorKind, Operation, OperationCx, OperationError, Provider, Resident,
    ResidentProvider, ResourceContext, ResourceHandle, ResourceKey, ResourceMetadataDraft,
    no_credential_slots, resource_key,
};
use nebula_sdk::prelude::{Deserialize, Serialize, metadata_name};

/// What the inventory client answers.
enum Answer {
    Reserved(u64),
    SlowDown(Option<Duration>),
    NoConnection,
    ConnectionLost,
    OutOfStock,
    UnknownItem,
}

/// The provider's client.
struct InventoryClient;

impl InventoryClient {
    async fn reserve(&self, item: &str, quantity: u32) -> Answer {
        std::future::ready(()).await;
        match (item, quantity) {
            ("", _) => Answer::UnknownItem,
            (_, 0) => Answer::OutOfStock,
            _ => Answer::Reserved(u64::from(quantity)),
        }
    }
}

/// Classifies one answer with the `OperationError` constructors.
fn classify(answer: Answer) -> Result<u64, OperationError> {
    match answer {
        Answer::Reserved(id) => Ok(id),
        Answer::SlowDown(after) => Err(OperationError::throttled(after)),
        Answer::NoConnection => Err(OperationError::unreachable("inventory unreachable")),
        Answer::ConnectionLost => Err(OperationError::interrupted("inventory connection lost")),
        Answer::OutOfStock => Err(OperationError::rejected("out of stock")),
        Answer::UnknownItem => Err(OperationError::rejected_as(
            ErrorKind::NotFound,
            "unknown item",
        )),
    }
}

struct Inventory;
no_credential_slots!(Inventory);

#[async_trait::async_trait]
impl Provider for Inventory {
    type Config = ();
    type Instance = InventoryClient;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.inventory")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), metadata_name!("Inventory"), "")
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<InventoryClient, Error> {
        Ok(InventoryClient)
    }
}

impl ResidentProvider for Inventory {}

/// Reserves stock; a repeat under the same key is absorbed, so an
/// interrupted attempt may be sent again, up to three attempts.
#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct Reserve {
    item: String,
    quantity: u32,
}

impl Operation<Inventory> for Reserve {
    type Output = u64;
    const KEY: &'static str = "inventory.reserve";
    const EFFECT: Effect = Effect::Idempotent;

    fn idempotency_key(&self) -> Option<String> {
        Some(format!("reserve-{}-{}", self.item, self.quantity))
    }

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::MIN.saturating_add(2)
    }

    async fn run(self, cx: &mut OperationCx<'_, Inventory>) -> Result<u64, OperationError> {
        let Self { item, quantity } = self;
        cx.call(Cost::ONE, async move |client, ()| {
            classify(client.reserve(&item, quantity).await)
        })
        .await
    }
}

/// What action code does with the inventory's resource handle.
async fn reserve(
    inventory: &ResourceHandle<Inventory>,
    item: &str,
    quantity: u32,
) -> Result<u64, Error> {
    Ok(inventory
        .submit(Reserve {
            item: item.to_owned(),
            quantity,
        })
        .await?)
}

fn main() {
    let _action_code = reserve;
    assert_eq!(classify(Answer::Reserved(3)).ok(), Some(3));
    let cases = [
        (
            Answer::SlowDown(Some(Duration::from_secs(2))),
            ErrorKind::Exhausted {
                retry_after: Some(Duration::from_secs(2)),
            },
        ),
        (Answer::NoConnection, ErrorKind::Transient),
        (Answer::ConnectionLost, ErrorKind::Transient),
        (Answer::OutOfStock, ErrorKind::Permanent),
        (Answer::UnknownItem, ErrorKind::NotFound),
    ];
    for (answer, kind) in cases {
        let error = classify(answer).expect_err("an error answer");
        assert_eq!(*error.kind(), kind);
    }
    assert_eq!(
        *OperationError::rejected_as(ErrorKind::Transient, "declined").kind(),
        ErrorKind::Permanent,
        "a definitive refusal is never retryable"
    );
}
