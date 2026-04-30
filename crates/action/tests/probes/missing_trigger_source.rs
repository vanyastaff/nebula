//! Compile-fail fixture: TriggerAction without Source associated type.
//!
//! Expected error: E0046 "not all trait items implemented, missing: Source".

use std::sync::OnceLock;

use nebula_action::context::TriggerContext;
use nebula_action::{
    Action, ActionError, ActionMetadata, TriggerAction, TriggerEventOutcome, TriggerSource,
};
use nebula_core::{Dependencies, action_key};
use nebula_schema::{HasSchema, ValidSchema};

struct BadTrigger;

impl Action for BadTrigger {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| ActionMetadata::new(action_key!("bad.trigger"), "Bad", "x"))
    }
    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
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
