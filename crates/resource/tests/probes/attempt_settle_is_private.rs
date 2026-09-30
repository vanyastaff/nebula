use nebula_resource::{
    PinSlots, Provider,
    call::{Cost, Operation, OperationCx, OperationError, SentState},
};

// An attempt's sent state derives from its classified result
// (`Attempt::finish`, `OperationCx::call`); an author cannot settle one by hand.
#[derive(serde::Serialize, serde::Deserialize)]
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
