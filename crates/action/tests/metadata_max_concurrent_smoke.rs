//! Admitted action metadata exposes immutable concurrency intent.

use std::{num::NonZeroU32, sync::OnceLock};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, RecordedActionMetadata, StatelessAction,
};
use nebula_core::{ActionKey, Dependencies};

struct Probe;

impl Action for Probe {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        draft()
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for Probe {
    async fn execute(
        &self,
        input: Self::Input,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

fn draft() -> ActionMetadataDraft {
    ActionMetadataDraft::new(
        ActionKey::new("test.maxc").expect("valid key"),
        nebula_action::metadata_name!("test"),
        "max_concurrent smoke",
    )
}

#[test]
fn default_is_none() {
    let factory = InstanceFactory::new(draft(), Probe).expect("metadata admits");
    assert_eq!(factory.metadata().max_concurrent(), None);
}

#[test]
fn recorded_evidence_readmits_against_fresh_definition() {
    let limit = NonZeroU32::new(4).expect("four is non-zero");
    let factory =
        InstanceFactory::new(draft().with_max_concurrent(limit), Probe).expect("metadata admits");
    let wire = serde_json::to_string(factory.metadata()).expect("metadata serializes");
    let recorded: RecordedActionMetadata =
        serde_json::from_str(&wire).expect("recorded evidence decodes");
    let readmitted = recorded
        .readmit_against(factory.metadata())
        .expect("exact evidence readmits");
    assert_eq!(readmitted.max_concurrent(), Some(limit));
}

#[test]
fn serialization_omits_absent_limit() {
    let factory = InstanceFactory::new(draft(), Probe).expect("metadata admits");
    let wire = serde_json::to_value(factory.metadata()).expect("metadata serializes");
    assert!(wire.get("max_concurrent").is_none());
}

#[test]
fn recorded_evidence_rejects_zero_limit() {
    let factory = InstanceFactory::new(draft(), Probe).expect("metadata admits");
    let mut wire = serde_json::to_value(factory.metadata()).expect("metadata serializes");
    wire.as_object_mut()
        .expect("metadata wire is an object")
        .insert("max_concurrent".into(), serde_json::json!(0));
    assert!(serde_json::from_value::<RecordedActionMetadata>(wire).is_err());
}
