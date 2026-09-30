//! An attempt's sent state derives from its classified result
//! (`Attempt::finish`, `OperationCx::call`): an author cannot settle one by
//! hand.

use nebula_sdk::integration::resource::{
    Cost, Operation, OperationCx, OperationError, PinSlots, Provider, SentState,
};
use nebula_sdk::prelude::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct HandSettled;

impl<R: Provider + PinSlots> Operation<R> for HandSettled {
    type Output = ();
    const KEY: &'static str = "probe.hand_settled";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        attempt.settle(SentState::Sent);
        Ok(())
    }
}

fn main() {}
