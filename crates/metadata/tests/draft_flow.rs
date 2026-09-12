//! Public draft-to-bound metadata construction and lifecycle laws.

use nebula_metadata::{
    DeprecationNotice, Icon, MaturityLevel, MetadataDraft, MetadataError, MetadataName,
    MetadataReadmissionError, MetadataVersion, RecordedBaseMetadata,
};
use nebula_schema::ValidSchema;
use pretty_assertions::assert_eq;
use proptest::prelude::*;
use semver::Version;
use serde_json::json;

fn checked_name() -> MetadataName {
    MetadataName::try_from("Example").expect("fixture name is nonblank")
}

#[test]
fn draft_binds_schema_after_all_shared_fields_are_authored() {
    let version: MetadataVersion = Version::new(2, 1, 0);
    let notice = DeprecationNotice::new(Version::new(2, 0, 0)).with_reason("superseded");
    let metadata = MetadataDraft::new("example", checked_name(), "description")
        .with_version(version.clone())
        .with_icon(Icon::inline("initial"))
        .with_url_icon("https://example.test/icon.svg")
        .with_inline_icon("final")
        .with_documentation_url("https://example.test/docs")
        .with_tags(["catalog", "network"])
        .add_tag("stable-id")
        .mark_experimental()
        .mark_beta()
        .mark_stable()
        .with_deprecation(notice.clone())
        .bind_schema(ValidSchema::empty())
        .expect("valid bounded metadata");

    assert_eq!(metadata.key(), &"example");
    assert_eq!(metadata.name(), "Example");
    assert_eq!(metadata.description(), "description");
    assert_eq!(metadata.version(), &version);
    assert_eq!(metadata.icon().as_inline(), Some("final"));
    assert_eq!(
        metadata.documentation_url(),
        Some("https://example.test/docs")
    );
    assert_eq!(metadata.tags(), ["catalog", "network", "stable-id"]);
    assert_eq!(metadata.maturity(), MaturityLevel::Deprecated);
    assert_eq!(metadata.deprecation(), Some(&notice));
    assert_eq!(metadata.schema(), &ValidSchema::empty());
}

#[test]
fn draft_try_new_validates_dynamic_names_without_exposing_them() {
    const SUBMITTED: &str = " \t\u{2003}";
    let error = MetadataDraft::try_new("example", SUBMITTED, "private-description")
        .expect_err("whitespace-only dynamic name is rejected");

    assert_eq!(error, MetadataError::BlankName);
    let rendered = format!("{error:?}: {error}");
    assert!(!rendered.contains(SUBMITTED));
    assert!(!rendered.contains("private-description"));
}

#[test]
fn bound_metadata_emits_the_versioned_shared_wire_shape() {
    let metadata = MetadataDraft::new("example", checked_name(), "description")
        .with_version(Version::new(2, 1, 0))
        .with_inline_icon("catalog")
        .with_documentation_url("https://example.test/docs")
        .with_tags(["network"])
        .mark_beta()
        .bind_schema(ValidSchema::empty())
        .expect("valid bounded metadata");

    assert_eq!(
        serde_json::to_value(metadata).expect("bound metadata serializes"),
        json!({
            "metadata_wire_version": 2,
            "key": "example",
            "name": "Example",
            "description": "description",
            "schema": ValidSchema::empty(),
            "version": "2.1.0",
            "icon": "catalog",
            "links": [{"relation": "overview", "target": "https://example.test/docs"}],
            "tags": ["network"],
            "maturity": "beta",
        })
    );
}

#[test]
fn recorded_wire_requires_an_exact_fresh_definition_for_readmission() {
    let fresh = MetadataDraft::new("example".to_owned(), checked_name(), "description")
        .with_version(Version::new(2, 1, 0))
        .mark_beta()
        .bind_schema(ValidSchema::empty())
        .expect("valid bounded metadata");
    let wire = serde_json::to_value(&fresh).expect("fresh definition serializes");
    let recorded: RecordedBaseMetadata<String> =
        serde_json::from_value(wire).expect("recorded evidence validates");

    let admitted = recorded
        .readmit_against(&fresh)
        .expect("record exactly matches fresh definition");
    assert_eq!(admitted, fresh);
}

#[test]
fn readmission_mismatch_is_typed_and_redacted() {
    const SUBMITTED: &str = "private-recorded-description";
    let recorded_definition = MetadataDraft::new("example".to_owned(), checked_name(), SUBMITTED)
        .with_version(Version::new(1, 0, 0))
        .bind_schema(ValidSchema::empty())
        .expect("valid bounded metadata");
    let wire = serde_json::to_value(recorded_definition).expect("definition serializes");
    let recorded: RecordedBaseMetadata<String> =
        serde_json::from_value(wire).expect("recorded evidence validates");
    let fresh = MetadataDraft::new("example".to_owned(), checked_name(), "new description")
        .with_version(Version::new(2, 0, 0))
        .bind_schema(ValidSchema::empty())
        .expect("valid bounded metadata");

    let error = recorded
        .readmit_against(&fresh)
        .expect_err("changed definition requires explicit migration, not silent admission");
    assert_eq!(error, MetadataReadmissionError::DefinitionMismatch);
    assert!(!format!("{error:?}: {error}").contains(SUBMITTED));
}

proptest! {
    #[test]
    fn deprecation_notice_survives_any_active_maturity_sequence(
        choices in prop::collection::vec(0_u8..3, 0..32),
    ) {
        let notice = DeprecationNotice::new(Version::new(1, 0, 0));
        let mut draft = MetadataDraft::new("example", checked_name(), "")
            .with_deprecation(notice.clone());

        for choice in choices {
            draft = match choice {
                0 => draft.mark_experimental(),
                1 => draft.mark_beta(),
                _ => draft.mark_stable(),
            };
        }

        let metadata = draft.bind_schema(ValidSchema::empty()).expect("valid bounded metadata");
        prop_assert_eq!(metadata.maturity(), MaturityLevel::Deprecated);
        prop_assert_eq!(metadata.deprecation(), Some(&notice));
    }
}
