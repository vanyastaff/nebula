use nebula_resource::{
    PinSlots, Provider,
    call::{Cost, Operation, OperationCx, OperationError, SentState},
};

// An operation is its own journaled request: without serde it is not one.
struct Unrecordable;

impl<R: Provider + PinSlots> Operation<R> for Unrecordable {
    type Output = ();
    const KEY: &'static str = "probe.unrecordable";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        attempt.settle(SentState::Sent);
        Ok(())
    }
}

fn main() {}
