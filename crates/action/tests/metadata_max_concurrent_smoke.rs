//! Admitted action metadata exposes immutable concurrency intent.

use std::{num::NonZeroU32, sync::OnceLock};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadata, ActionResult,
    InstanceFactory, RecordedActionDefinition, StatelessAction,
};
use nebula_core::Dependencies;

struct Probe;

impl Action for Probe {
    const KEY: &'static str = "test.maxc";
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        draft()
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

const LIMIT: NonZeroU32 = NonZeroU32::new(4).expect("four is non-zero");

nebula_action::stateless_entry!(
    LimitedProbe(Probe) = "test.maxc",
    draft().with_max_concurrent(LIMIT),
);

impl StatelessAction for Probe {
    async fn execute(
        &self,
        input: Self::Input,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

fn draft() -> ActionMetadata {
    ActionMetadata::new(
        nebula_action::metadata_name!("test"),
        "max_concurrent smoke",
    )
}

#[test]
fn default_is_none() {
    let factory = InstanceFactory::new(Probe).expect("metadata admits");
    assert_eq!(factory.definition().max_concurrent(), None);
}

#[test]
fn recorded_evidence_readmits_against_fresh_definition() {
    let factory = InstanceFactory::new(LimitedProbe(Probe)).expect("metadata admits");
    let wire = serde_json::to_string(factory.definition()).expect("metadata serializes");
    let recorded: RecordedActionDefinition =
        serde_json::from_str(&wire).expect("recorded evidence decodes");
    let readmitted = recorded
        .readmit_against(factory.definition())
        .expect("exact evidence readmits");
    assert_eq!(readmitted.max_concurrent(), Some(LIMIT));
}

#[test]
fn serialization_omits_absent_limit() {
    let factory = InstanceFactory::new(Probe).expect("metadata admits");
    let wire = serde_json::to_value(factory.definition()).expect("metadata serializes");
    assert!(wire.get("max_concurrent").is_none());
}

#[test]
fn recorded_evidence_rejects_zero_limit() {
    let factory = InstanceFactory::new(Probe).expect("metadata admits");
    let mut wire = serde_json::to_value(factory.definition()).expect("metadata serializes");
    wire.as_object_mut()
        .expect("metadata wire is an object")
        .insert("max_concurrent".into(), serde_json::json!(0));
    assert!(serde_json::from_value::<RecordedActionDefinition>(wire).is_err());
}
