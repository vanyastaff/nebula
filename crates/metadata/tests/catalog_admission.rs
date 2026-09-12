//! Shared admission budgets, canonicalization, and exact evidence matching.

use std::cell::Cell;

use nebula_metadata::{
    BaseMetadata, CatalogCategoryKey, CatalogLink, CatalogLinkRelation, CatalogReference,
    DeprecationNotice, MAX_SHARED_METADATA_BYTES, MetadataBuildError, MetadataDecodeLimits,
    MetadataDraft, MetadataError, MetadataReadmissionError, RecordedBaseMetadata, RemovalSchedule,
    decode_json_slice, validate_base_compat,
};
use nebula_schema::ValidSchema;
use proptest::prelude::*;
use semver::Version;
use serde::Serialize;
use serde_json::json;

fn draft() -> MetadataDraft<String> {
    MetadataDraft::try_new("example".to_owned(), "Example", "").unwrap()
}

fn admit(draft: MetadataDraft<String>) -> BaseMetadata<String> {
    draft.bind_schema(ValidSchema::empty()).unwrap()
}

fn category(key: &str) -> CatalogCategoryKey {
    key.parse().unwrap()
}

fn link(relation: CatalogLinkRelation, target: &str) -> CatalogLink {
    CatalogLink::new(relation, target.parse().unwrap())
}

fn rejection(draft: MetadataDraft<String>) -> MetadataError {
    match draft.bind_schema(ValidSchema::empty()).unwrap_err() {
        MetadataBuildError::Metadata(error) => error,
        error => panic!("expected intrinsic failure, got {error:?}"),
    }
}

#[test]
fn discovery_is_sorted_deduplicated_and_trimmed() {
    let overview = link(CatalogLinkRelation::Overview, "/docs");
    let setup = link(CatalogLinkRelation::Setup, "/setup");
    let metadata = admit(
        draft()
            .with_categories([
                category("network"),
                category("database"),
                category("network"),
            ])
            .with_tags([" sql ", "postgres", "sql"])
            .with_links([setup.clone(), overview.clone(), setup.clone()]),
    );
    assert_eq!(
        metadata.categories(),
        [category("database"), category("network")]
    );
    assert_eq!(metadata.tags(), ["postgres", "sql"]);
    assert_eq!(metadata.links(), [overview, setup]);
    assert_eq!(metadata.documentation_url(), Some("/docs"));
    let wire = serde_json::to_value(&metadata).unwrap();
    assert!(wire.get("documentation_url").is_none());
    assert_eq!(wire["metadata_wire_version"], 2);
}

#[test]
fn overview_replacement_repairs_only_overview_guidance() {
    let conflicting = draft().with_links([
        link(CatalogLinkRelation::Overview, "/first"),
        link(CatalogLinkRelation::Overview, "/second"),
        link(CatalogLinkRelation::Setup, "/setup"),
    ]);
    assert_eq!(
        rejection(conflicting.clone()),
        MetadataError::ConflictingOverview
    );
    let repaired = admit(conflicting.with_documentation_url("/final"));
    assert_eq!(repaired.documentation_url(), Some("/final"));
    assert_eq!(repaired.links().len(), 2);
    let repaired = admit(
        draft()
            .with_documentation_url("javascript:private")
            .with_documentation_url("/valid"),
    );
    assert_eq!(repaired.documentation_url(), Some("/valid"));
    assert_eq!(
        rejection(draft().with_tags([" "]).with_documentation_url("/valid")),
        MetadataError::BlankTag
    );
}

#[test]
fn raw_iterators_stop_at_the_overflow_sentinel_even_for_duplicates() {
    let visits = Cell::new(0);
    let endless = std::iter::repeat_with(|| {
        visits.set(visits.get() + 1);
        assert!(
            visits.get() <= 65,
            "authoring must stop at its documented sentinel"
        );
        "duplicate"
    });
    let excessive = draft().with_tags(endless);
    assert_eq!(visits.get(), 65);
    assert_eq!(
        rejection(excessive.clone()),
        MetadataError::TooManyRawEntries(nebula_metadata::MetadataField::Tags)
    );
    assert_eq!(
        admit(excessive.with_tags(["replacement"])).tags(),
        ["replacement"]
    );
    assert_eq!(
        admit(draft().with_tags(std::iter::repeat_n("same", 64))).tags(),
        ["same"]
    );
    let categories = draft().with_categories(std::iter::repeat(category("same")));
    assert_eq!(
        rejection(categories.clone()),
        MetadataError::TooManyRawEntries(nebula_metadata::MetadataField::Categories)
    );
    assert!(
        admit(categories.with_categories([]))
            .categories()
            .is_empty()
    );
    let links = draft().with_links(std::iter::repeat(link(CatalogLinkRelation::Setup, "/same")));
    assert_eq!(
        rejection(links.clone()),
        MetadataError::TooManyRawEntries(nebula_metadata::MetadataField::Links)
    );
    assert!(admit(links.with_links([])).links().is_empty());
}

#[test]
fn canonical_collection_limits_apply_after_bounded_deduplication() {
    let tags: Vec<_> = (0..32).map(|i| format!("tag{i}")).collect();
    assert_eq!(admit(draft().with_tags(tags.clone())).tags().len(), 32);
    assert_eq!(
        rejection(draft().with_tags(tags).add_tag("extra")),
        MetadataError::TooManyEntries(nebula_metadata::MetadataField::Tags)
    );
    let categories: Vec<_> = (0..16).map(|i| category(&format!("category{i}"))).collect();
    assert_eq!(
        admit(draft().with_categories(categories.clone()))
            .categories()
            .len(),
        16
    );
    assert_eq!(
        rejection(draft().with_categories(categories.into_iter().chain([category("extra")]))),
        MetadataError::TooManyEntries(nebula_metadata::MetadataField::Categories)
    );
    let links: Vec<_> = (0..16)
        .map(|i| link(CatalogLinkRelation::Reference, &format!("/doc{i}")))
        .collect();
    assert_eq!(admit(draft().with_links(links.clone())).links().len(), 16);
    assert_eq!(
        rejection(
            draft()
                .with_links(links)
                .add_link(link(CatalogLinkRelation::Setup, "/extra"))
        ),
        MetadataError::TooManyEntries(nebula_metadata::MetadataField::Links)
    );
}

#[test]
fn utf8_field_limits_and_replacement_setters_are_checked() {
    assert_eq!(
        rejection(draft().with_inline_icon("x".repeat(MAX_SHARED_METADATA_BYTES + 1))),
        MetadataError::FieldTooLarge(nebula_metadata::MetadataField::Icon)
    );
    assert_eq!(
        rejection(
            draft().with_version(
                format!("1.0.0+{}", "x".repeat(MAX_SHARED_METADATA_BYTES + 1))
                    .parse()
                    .unwrap()
            )
        ),
        MetadataError::FieldTooLarge(nebula_metadata::MetadataField::Version)
    );
    assert_eq!(
        admit(draft().with_tags(["a".repeat(64)])).tags()[0].len(),
        64
    );
    assert_eq!(
        rejection(draft().with_tags(["a".repeat(65)])),
        MetadataError::FieldTooLarge(nebula_metadata::MetadataField::Tags)
    );
    assert_eq!(
        admit(draft().with_tags(["\u{00e9}".repeat(32)])).tags()[0].len(),
        64
    );
    assert_eq!(
        rejection(draft().with_tags(["\u{00e9}".repeat(33)])),
        MetadataError::FieldTooLarge(nebula_metadata::MetadataField::Tags)
    );
    for blank in ["", " \t", "\u{2003}"] {
        assert_eq!(
            rejection(draft().with_tags([blank])),
            MetadataError::BlankTag
        );
    }
    assert_eq!(
        admit(draft().with_description("d".repeat(8192)))
            .description()
            .len(),
        8192
    );
    assert_eq!(
        rejection(draft().with_description("d".repeat(8193))),
        MetadataError::FieldTooLarge(nebula_metadata::MetadataField::Description)
    );
    assert_eq!(
        admit(
            draft()
                .with_description("d".repeat(8193))
                .with_description("new")
        )
        .description(),
        "new"
    );
    assert_eq!(
        admit(
            draft()
                .with_inline_icon("x".repeat(40000))
                .with_inline_icon("new")
        )
        .icon()
        .as_inline(),
        Some("new")
    );
}

fn authored_size(metadata: &BaseMetadata<String>) -> usize {
    let mut wire = serde_json::to_value(metadata).unwrap();
    wire.as_object_mut().unwrap().remove("schema");
    serde_json::to_vec(&wire).unwrap().len()
}

#[test]
fn aggregate_budget_matches_exact_json_including_escaping_at_the_boundary() {
    let overhead = authored_size(&admit(draft().with_inline_icon("")));
    let content_bytes = MAX_SHARED_METADATA_BYTES - overhead;
    let exact = admit(draft().with_inline_icon("x".repeat(content_bytes)));
    assert_eq!(authored_size(&exact), MAX_SHARED_METADATA_BYTES);
    assert_eq!(
        rejection(draft().with_inline_icon("x".repeat(content_bytes + 1))),
        MetadataError::SharedBudgetExceeded
    );
    // A control character costs six bytes in canonical JSON, not one.
    let escaped = format!(
        "{}{}",
        "\u{0001}".repeat(content_bytes / 6),
        "x".repeat(content_bytes % 6)
    );
    let escaped = admit(draft().with_inline_icon(escaped));
    assert_eq!(authored_size(&escaped), MAX_SHARED_METADATA_BYTES);
    assert_eq!(
        rejection(draft().with_inline_icon("\u{0001}".repeat(content_bytes / 6 + 1))),
        MetadataError::SharedBudgetExceeded
    );
    for metadata in [exact, escaped] {
        let wire = serde_json::to_vec(&metadata).unwrap();
        let record: RecordedBaseMetadata<String> =
            decode_json_slice(&wire, MetadataDecodeLimits::default()).unwrap();
        assert_eq!(record.readmit_against(&metadata).unwrap(), metadata);
    }
}

#[test]
fn aggregate_budget_includes_generic_keys_names_versions_and_notices() {
    let oversized = "x".repeat(MAX_SHARED_METADATA_BYTES);
    assert_eq!(
        rejection(MetadataDraft::try_new(oversized.clone(), "Name", "").unwrap()),
        MetadataError::SharedBudgetExceeded
    );
    assert_eq!(
        rejection(MetadataDraft::try_new("key".to_owned(), oversized.clone(), "").unwrap()),
        MetadataError::SharedBudgetExceeded
    );
    assert_eq!(
        rejection(draft().with_version(format!("1.0.0+{oversized}").parse().unwrap())),
        MetadataError::SharedBudgetExceeded
    );
    assert_eq!(
        rejection(draft().with_deprecation(
            DeprecationNotice::new(Version::new(1, 0, 0)).with_reason(oversized)
        )),
        MetadataError::SharedBudgetExceeded
    );
}

#[test]
fn serializer_failures_are_payload_free_and_have_no_raw_source() {
    #[derive(Debug)]
    struct BrokenKey;
    impl Serialize for BrokenKey {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("private_serializer_payload"))
        }
    }
    let error = MetadataDraft::try_new(BrokenKey, "Name", "")
        .unwrap()
        .bind_schema(ValidSchema::empty())
        .unwrap_err();
    assert!(!format!("{error}: {error:?}").contains("private_serializer_payload"));
    match error {
        MetadataBuildError::Metadata(error) => {
            assert_eq!(error, MetadataError::SerializationFailed);
            assert!(std::error::Error::source(&error).is_none());
        },
        error => panic!("unexpected failure: {error:?}"),
    }
}

#[test]
fn chronology_uses_semver_precedence_and_allows_elapsed_dates() {
    let since: Version = "1.0.0+z".parse().unwrap();
    let notice = DeprecationNotice::new(since.clone());
    let metadata = admit(
        draft()
            .with_version("1.0.0+a".parse().unwrap())
            .with_deprecation(notice.clone()),
    );
    assert_eq!(metadata.deprecation().unwrap().since(), &since);
    assert_eq!(
        rejection(
            draft()
                .with_version("1.0.0-beta".parse().unwrap())
                .with_deprecation(notice.clone())
        ),
        MetadataError::FutureDeprecation
    );
    assert_eq!(
        rejection(
            draft().with_deprecation(
                notice
                    .clone()
                    .with_removal(RemovalSchedule::AtVersion("1.0.0+a".parse().unwrap()))
            )
        ),
        MetadataError::InvalidRemovalOrder
    );
    let past = notice
        .clone()
        .with_removal(RemovalSchedule::OnDate("2000-01-01".parse().unwrap()));
    assert_eq!(
        admit(draft().with_deprecation(past.clone())).deprecation(),
        Some(&past)
    );
    let repaired = DeprecationNotice::new(Version::new(2, 0, 0));
    assert_eq!(
        rejection(draft().with_deprecation(repaired.clone())),
        MetadataError::FutureDeprecation
    );
    assert_eq!(
        admit(
            draft()
                .with_deprecation(repaired)
                .with_deprecation(notice.clone())
        )
        .deprecation(),
        Some(&notice)
    );
}

#[test]
fn literal_versions_preserve_full_semver_and_defer_invalid_intent_to_binding() {
    let exact = admit(draft().with_version_literal("2.1.0-beta.2+build.7"));
    assert_eq!(exact.version().to_string(), "2.1.0-beta.2+build.7");
    assert_eq!(
        rejection(draft().with_version_literal("private-invalid-version")),
        MetadataError::InvalidVersion(nebula_metadata::MetadataField::Version)
    );
    let repaired = admit(
        draft()
            .with_version_literal("invalid")
            .with_version(Version::new(3, 0, 0)),
    );
    assert_eq!(repaired.version(), &Version::new(3, 0, 0));
    let repaired = admit(
        draft()
            .with_version_literal("invalid")
            .with_version_literal("4.0.0"),
    );
    assert_eq!(repaired.version(), &Version::new(4, 0, 0));
}

#[test]
fn guidance_can_change_without_a_major_but_exact_readmission_rejects_it() {
    let fresh = admit(draft());
    let mutations = [
        draft().with_description("changed"),
        draft().with_categories([category("network")]),
        draft().with_tags(["network"]),
        draft().with_documentation_url("/new"),
        draft().with_deprecation(
            DeprecationNotice::new(Version::new(1, 0, 0))
                .with_replacement(CatalogReference::resource("new.resource".parse().unwrap())),
        ),
    ];
    for changed in mutations {
        let changed = admit(changed);
        validate_base_compat(&fresh, &changed).unwrap();
        let recorded: RecordedBaseMetadata<String> =
            serde_json::from_value(json!(changed)).unwrap();
        assert_eq!(
            recorded.readmit_against(&fresh).unwrap_err(),
            MetadataReadmissionError::DefinitionMismatch
        );
    }
}

#[test]
fn manifest_admission_uses_the_same_discovery_chronology_and_field_budgets() {
    use nebula_metadata::{ManifestError, MetadataField, PluginManifest};
    let notice = DeprecationNotice::new(Version::new(2, 0, 0))
        .with_removal(RemovalSchedule::AtVersion(Version::new(3, 0, 0)));
    assert_eq!(
        PluginManifest::builder("example", "Example")
            .deprecation(notice.clone())
            .build()
            .unwrap_err(),
        ManifestError::Metadata(MetadataError::FutureDeprecation)
    );
    let manifest = PluginManifest::builder("example", "Example")
        .version(Version::new(2, 0, 0))
        .deprecation(notice.clone())
        .with_categories([
            category("network"),
            category("database"),
            category("network"),
        ])
        .tags(vec!["sql".into(), " postgres ".into(), "sql".into()])
        .with_documentation_url("/docs")
        .build()
        .unwrap();
    assert_eq!(
        manifest.categories(),
        [category("database"), category("network")]
    );
    assert_eq!(manifest.tags(), ["postgres", "sql"]);
    assert_eq!(manifest.deprecation(), Some(&notice));
    assert_eq!(manifest.documentation_url(), Some("/docs"));
    let wire = json!(manifest);
    assert_eq!(wire["metadata_wire_version"], 2);
    assert!(wire.get("schema").is_none());
    assert!(wire.get("base").is_none());
    assert_eq!(
        serde_json::from_value::<PluginManifest>(wire).unwrap(),
        manifest
    );
    assert_eq!(
        PluginManifest::builder("example", "Example")
            .description("d".repeat(8193))
            .build()
            .unwrap_err(),
        ManifestError::Metadata(MetadataError::FieldTooLarge(MetadataField::Description))
    );
    assert_eq!(
        PluginManifest::builder("example", "Example")
            .tags(vec![" ".into()])
            .build()
            .unwrap_err(),
        ManifestError::Metadata(MetadataError::BlankTag)
    );
    assert_eq!(
        PluginManifest::builder("example", "Example")
            .inline_icon("x".repeat(MAX_SHARED_METADATA_BYTES))
            .build()
            .unwrap_err(),
        ManifestError::Metadata(MetadataError::SharedBudgetExceeded)
    );
}

#[test]
fn schema_has_a_separate_exact_budget_and_default_transport_can_round_trip_it() {
    use nebula_schema::{FieldCollector, Schema, field_key};
    let schema = |description: String| {
        Schema::builder()
            .string(field_key!("field"), |field| field.description(description))
            .build()
            .unwrap()
    };
    let overhead = serde_json::to_vec(&schema(String::new())).unwrap().len();
    let content_bytes = nebula_metadata::MAX_METADATA_SCHEMA_BYTES - overhead;
    let exact_schema = schema("x".repeat(content_bytes));
    assert_eq!(
        serde_json::to_vec(&exact_schema).unwrap().len(),
        nebula_metadata::MAX_METADATA_SCHEMA_BYTES
    );
    let metadata = draft().bind_schema(exact_schema).unwrap();
    let bytes = serde_json::to_vec(&metadata).unwrap();
    assert!(bytes.len() <= nebula_metadata::MAX_METADATA_JSON_BYTES);
    let recorded: RecordedBaseMetadata<String> =
        decode_json_slice(&bytes, MetadataDecodeLimits::default()).unwrap();
    assert_eq!(recorded.readmit_against(&metadata).unwrap(), metadata);
    match draft()
        .bind_schema(schema("x".repeat(content_bytes + 1)))
        .unwrap_err()
    {
        MetadataBuildError::Metadata(error) => {
            assert_eq!(error, MetadataError::SchemaBudgetExceeded);
        },
        error => panic!("unexpected schema budget failure: {error:?}"),
    }
}

#[test]
fn manifest_packaging_cannot_admit_semver_fields_its_decoder_would_reject() {
    use nebula_metadata::{ManifestError, MetadataField, PluginDependency, PluginManifest};
    let suffix = "a".repeat(MAX_SHARED_METADATA_BYTES);
    let version = format!("1.0.0+{suffix}").parse().unwrap();
    let error = PluginManifest::builder("example", "Example")
        .nebula_version(version)
        .build()
        .unwrap_err();
    assert_eq!(
        error,
        ManifestError::Metadata(MetadataError::FieldTooLarge(
            MetadataField::ManifestNebulaVersion
        ))
    );
    let requirement = format!("=1.0.0-{suffix}").parse().unwrap();
    let error = PluginManifest::builder("example", "Example")
        .dependency(PluginDependency::new("other".parse().unwrap(), requirement))
        .build()
        .unwrap_err();
    assert_eq!(
        error,
        ManifestError::Metadata(MetadataError::FieldTooLarge(
            MetadataField::ManifestDependencies
        ))
    );
}

proptest! {
    #[test]
    fn canonical_sets_are_order_and_duplicate_independent(tags in prop::collection::vec("[a-z]{1,10}", 0..24)) {
        let fresh = admit(draft().with_tags(tags.clone()));
        let reordered = tags.into_iter().rev().flat_map(|tag| [format!(" {tag} "), tag]);
        let equivalent = admit(draft().with_tags(reordered));
        prop_assert_eq!(&fresh, &equivalent);
        let recorded: RecordedBaseMetadata<String> = serde_json::from_value(json!(equivalent))?;
        prop_assert_eq!(recorded.readmit_against(&fresh)?, fresh);
    }
}
