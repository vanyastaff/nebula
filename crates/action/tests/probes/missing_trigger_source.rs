//! Compile-fail fixture: TriggerAction without Source associated type.
//!
//! Expected error: E0046 "not all trait items implemented, missing: Source".

use nebula_action::{ActionError, ActionMetadata, IdempotencyKey, TriggerAction};
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
}

fn main() {}
