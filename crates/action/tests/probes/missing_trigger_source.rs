//! Compile-fail fixture: TriggerAction without Source associated type.
//!
//! Expected error: E0046 "not all trait items implemented, missing: Source".


use nebula_action::context::TriggerContext;
use nebula_action::{
    Action, ActionError, Component, TriggerAction, TriggerEventOutcome, TriggerSource, kind,
};

struct BadTrigger;

impl Component for BadTrigger {
    type Kind = kind::Action;
    type Config = serde_json::Value;

    const KEY: &'static str = "bad.trigger";

    fn metadata() -> nebula_action::Metadata {
        nebula_action::Metadata::new(nebula_action::metadata_name!("Bad"), "x")
    }
}

impl Action for BadTrigger {
    type Output = serde_json::Value;
}

impl TriggerAction for BadTrigger {
    type Error = ActionError;

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
