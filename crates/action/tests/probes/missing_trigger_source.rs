//! Compile-fail fixture: TriggerAction without Source associated type.
//!
//! Expected error: E0046 "not all trait items implemented, missing: Source".

use nebula_action::{ActionError, ActionMetadata, TriggerAction, TriggerEventOutcome, TriggerSource};
use nebula_action::context::TriggerContext;

struct BadTrigger;

impl TriggerAction for BadTrigger {
    type Error = ActionError;

    fn metadata(&self) -> &ActionMetadata {
        unimplemented!()
    }

    async fn start(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }

    async fn stop(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle(
        &self,
        _ctx: &(impl TriggerContext + ?Sized),
        _event: <Self::Source as TriggerSource>::Event,
    ) -> Result<TriggerEventOutcome, ActionError> {
        Err(ActionError::fatal("not event-driven"))
    }
}

fn main() {}
