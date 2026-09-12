//! Supported catalog value transport; admitted leaf records remain outside the SDK.

use nebula_sdk::{
    integration::{
        CatalogLink, CatalogLinkRelation, CatalogReference, DeprecationNotice, MetadataVersion,
        RemovalSchedule,
    },
    prelude::action_key,
    serde::de::DeserializeOwned,
    serde_json::{self, Value, json},
};

fn reject_positional<T: DeserializeOwned>(wire: Value) {
    let bytes = serde_json::to_vec(&wire).expect("test transport serializes");
    assert!(
        serde_json::from_slice::<T>(&bytes).is_err(),
        "catalog objects cannot be encoded positionally"
    );
}

#[test]
fn sdk_catalog_values_roundtrip_as_objects_and_reject_positional_records() {
    let link = CatalogLink::new(
        CatalogLinkRelation::Setup,
        "/setup".parse().expect("link target"),
    );
    let reference = CatalogReference::action(action_key!("example.next"));
    let notice = DeprecationNotice::new(MetadataVersion::new(1, 0, 0))
        .with_removal(RemovalSchedule::AtVersion(MetadataVersion::new(2, 0, 0)))
        .with_replacement(reference.clone())
        .with_reason("Use the next action");
    let link_wire = serde_json::to_value(&link).expect("link serializes");
    let reference_wire = serde_json::to_value(&reference).expect("reference serializes");
    let notice_wire = serde_json::to_value(&notice).expect("notice serializes");
    assert!(link_wire.is_object());
    assert!(reference_wire.is_object());
    assert!(notice_wire.is_object());
    assert_eq!(
        serde_json::from_value::<CatalogLink>(link_wire.clone()).expect("link object"),
        link
    );
    assert_eq!(
        serde_json::from_value::<CatalogReference>(reference_wire.clone())
            .expect("reference object"),
        reference
    );
    assert_eq!(
        serde_json::from_value::<DeprecationNotice>(notice_wire.clone()).expect("notice object"),
        notice
    );
    reject_positional::<CatalogLink>(json!([link_wire["relation"], link_wire["target"]]));
    reject_positional::<CatalogReference>(json!([
        reference_wire["kind"],
        reference_wire["key"],
        reference_wire["version_requirement"]
    ]));
    reject_positional::<DeprecationNotice>(json!([
        notice_wire["since"],
        notice_wire["removal"],
        notice_wire["replacement"],
        notice_wire["reason"]
    ]));
}
