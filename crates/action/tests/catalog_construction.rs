//! Recorded action metadata is the only deserializable catalog representation.

use std::sync::OnceLock;

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, RecordedActionMetadata, StatelessAction, metadata_name,
};
use nebula_core::{Dependencies, action_key};
use serde_json::Value;

struct CatalogAction;

impl Action for CatalogAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("catalog.action"),
            metadata_name!("Catalog action"),
            "Catalog admission fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for CatalogAction {
    async fn execute(
        &self,
        input: Value,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn recorded_metadata_readmits_only_against_factory_admission() {
    let factory = InstanceFactory::new(CatalogAction::metadata(), CatalogAction)
        .expect("valid action contract");
    let encoded = serde_json::to_value(factory.metadata()).expect("admitted metadata serializes");
    let recorded: RecordedActionMetadata =
        serde_json::from_value(encoded).expect("recorded evidence deserializes");

    let readmitted = recorded
        .readmit_against(factory.metadata())
        .expect("exact factory definition readmits recorded evidence");

    assert_eq!(readmitted, **factory.metadata());
}

#[test]
fn recorded_metadata_rejects_blank_wire_names() {
    let factory = InstanceFactory::new(CatalogAction::metadata(), CatalogAction)
        .expect("valid action contract");
    let admitted = serde_json::to_value(factory.metadata()).expect("admitted metadata serializes");

    for blank_name in ["", " \t", "\u{3000}"] {
        let mut wire = admitted.clone();
        wire["name"] = blank_name.into();
        assert!(
            serde_json::from_value::<RecordedActionMetadata>(wire).is_err(),
            "recorded metadata accepted blank name {blank_name:?}"
        );
    }
}
