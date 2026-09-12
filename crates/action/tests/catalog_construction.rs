//! Recorded action metadata is the only deserializable catalog representation.

use std::sync::OnceLock;

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, MetadataVersion, RecordedActionMetadata, StatelessAction, metadata_name,
};
use nebula_core::{Dependencies, action_key};
use nebula_metadata::{
    CatalogLink, CatalogLinkRelation, CatalogReference, DeprecationNotice, MetadataDecodeError,
    MetadataDecodeLimits, MetadataError, MetadataField, RemovalSchedule,
};
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
        wire["base"]["name"] = blank_name.into();
        assert!(
            serde_json::from_value::<RecordedActionMetadata>(wire).is_err(),
            "recorded metadata accepted blank name {blank_name:?}"
        );
    }
}

#[test]
fn invalid_shared_catalog_fields_never_produce_a_factory() {
    let invalid = [
        (
            CatalogAction::metadata().with_tags([" "]),
            MetadataError::BlankTag,
        ),
        (
            ActionMetadataDraft::new(
                action_key!("catalog.action"),
                metadata_name!("Catalog action"),
                "x".repeat(8193),
            ),
            MetadataError::FieldTooLarge(MetadataField::Description),
        ),
        (
            CatalogAction::metadata()
                .with_deprecation(DeprecationNotice::new(MetadataVersion::new(2, 0, 0))),
            MetadataError::FutureDeprecation,
        ),
        (
            ActionMetadataDraft::try_new(action_key!("catalog.action"), "\"".repeat(20_000), "")
                .expect("nonblank authored name"),
            MetadataError::SharedBudgetExceeded,
        ),
    ];
    for (draft, expected) in invalid {
        for _ in 0..2 {
            let error = InstanceFactory::new(draft.clone(), CatalogAction)
                .err()
                .expect("invalid metadata must prevent factory construction");
            std::assert_matches!(error,
                nebula_action::ActionMetadataAdmissionError::Schema(nebula_metadata::MetadataBuildError::Metadata(actual))
                if actual == expected);
        }
    }
}

#[test]
fn same_semver_catalog_changes_and_leaf_extras_require_fresh_evidence() {
    let original =
        InstanceFactory::new(CatalogAction::metadata(), CatalogAction).expect("valid draft");
    let recorded = RecordedActionMetadata::from_slice(
        &serde_json::to_vec(original.metadata()).expect("serializable"),
        MetadataDecodeLimits::default(),
    )
    .expect("default decoder accepts admitted wire");
    let notice = DeprecationNotice::new(MetadataVersion::new(1, 0, 0))
        .with_removal(RemovalSchedule::AtVersion(MetadataVersion::new(2, 0, 0)))
        .with_replacement(CatalogReference::action(action_key!("catalog.replacement")))
        .with_reason("Use the replacement action");
    for draft in [
        CatalogAction::metadata().with_categories(["automation.events".parse().expect("category")]),
        CatalogAction::metadata().add_link(CatalogLink::new(
            CatalogLinkRelation::Setup,
            "/setup".parse().expect("link"),
        )),
        CatalogAction::metadata().with_deprecation(notice),
        CatalogAction::metadata()
            .with_max_concurrent(std::num::NonZeroU32::new(3).expect("positive")),
    ] {
        let changed = InstanceFactory::new(draft, CatalogAction).expect("valid changed definition");
        assert_eq!(
            original.metadata().base().version(),
            changed.metadata().base().version()
        );
        assert!(recorded.readmit_against(changed.metadata()).is_err());
        let current = RecordedActionMetadata::from_slice(
            &serde_json::to_vec(changed.metadata()).expect("serializable"),
            MetadataDecodeLimits::default(),
        )
        .expect("default decoder accepts changed admitted wire");
        assert_eq!(
            current
                .readmit_against(changed.metadata())
                .expect("exact evidence"),
            **changed.metadata()
        );
    }
}

#[test]
fn recorded_leaf_rejects_legacy_and_redacts_unknown_keys_and_wrong_types() {
    const CANARY: &str = "PRIVATE_ACTION_WIRE_CANARY";
    let factory =
        InstanceFactory::new(CatalogAction::metadata(), CatalogAction).expect("valid draft");
    let wire = serde_json::to_value(factory.metadata()).expect("serializable");
    assert_eq!(wire["base"]["metadata_wire_version"], 2);
    let mut unknown = wire.clone();
    unknown[CANARY] = Value::String(CANARY.into());
    let mut wrong_type = wire.clone();
    wrong_type["max_concurrent"] = Value::String(CANARY.into());
    let mut unknown_variant = wire.clone();
    unknown_variant["kind"] = Value::String(CANARY.into());
    let mut no_version = wire.clone();
    no_version["base"]
        .as_object_mut()
        .expect("base object")
        .remove("metadata_wire_version");
    let mut legacy = wire.clone();
    let mut base = legacy
        .as_object_mut()
        .expect("leaf object")
        .remove("base")
        .expect("base")
        .as_object()
        .expect("base object")
        .clone();
    base.remove("metadata_wire_version");
    legacy.as_object_mut().expect("leaf object").extend(base);
    let positional = serde_json::json!([
        wire["base"],
        wire["inputs"],
        wire["outputs"],
        wire["isolation_level"],
        wire["kind"],
        wire["checkpoint_policy"],
        wire["effect_contract"],
        wire["max_concurrent"],
        wire["output_schema"],
    ]);
    for rejected in [
        unknown,
        wrong_type,
        unknown_variant,
        no_version,
        legacy,
        positional,
    ] {
        let bytes = serde_json::to_vec(&rejected).expect("serializable");
        let error =
            serde_json::from_slice::<RecordedActionMetadata>(&bytes).expect_err("invalid leaf");
        assert!(!format!("{error:?} {error}").contains(CANARY));
        assert_eq!(
            RecordedActionMetadata::from_slice(&bytes, MetadataDecodeLimits::default()),
            Err(MetadataDecodeError::InvalidRecord)
        );
    }
    let bytes = serde_json::to_vec(&wire).expect("serializable");
    let exact = MetadataDecodeLimits::new(bytes.len()).expect("within ceiling");
    let shorter = MetadataDecodeLimits::new(bytes.len() - 1).expect("positive lower limit");
    let recorded =
        RecordedActionMetadata::from_reader(bytes.as_slice(), exact).expect("exact limit accepts");
    assert_eq!(
        recorded
            .readmit_against(factory.metadata())
            .expect("fresh evidence"),
        **factory.metadata()
    );
    assert_eq!(
        RecordedActionMetadata::from_slice(&bytes, shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
    assert_eq!(
        RecordedActionMetadata::from_reader(bytes.as_slice(), shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
}

#[test]
fn hidden_version_literal_is_checked_and_the_last_setter_wins() {
    const CANARY: &str = "PRIVATE_INVALID_VERSION";
    for invalid in [
        CatalogAction::metadata().with_version_literal(CANARY),
        CatalogAction::metadata()
            .with_version(MetadataVersion::new(2, 0, 0))
            .with_version_literal(CANARY),
    ] {
        let error = InstanceFactory::new(invalid, CatalogAction)
            .err()
            .expect("invalid literal cannot admit");
        std::assert_matches!(
            error,
            nebula_action::ActionMetadataAdmissionError::Schema(
                nebula_metadata::MetadataBuildError::Metadata(MetadataError::InvalidVersion(
                    MetadataField::Version
                ))
            )
        );
        assert!(!format!("{error:?} {error}").contains(CANARY));
    }
    let version: MetadataVersion = "2.5.0-rc.7+build.009".parse().expect("full version");
    for draft in [
        CatalogAction::metadata()
            .with_version_literal(CANARY)
            .with_version(version.clone()),
        CatalogAction::metadata()
            .with_version(MetadataVersion::new(3, 0, 0))
            .with_version_literal("2.5.0-rc.7+build.009"),
    ] {
        let factory = InstanceFactory::new(draft, CatalogAction)
            .expect("last valid setter replaces old intent");
        assert_eq!(factory.metadata().base().version(), &version);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LargeRecord {
    value: String,
}

impl nebula_schema::HasSchema for LargeRecord {
    fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
        use nebula_schema::FieldCollector;
        nebula_schema::Schema::builder()
            .string(nebula_schema::field_key!("value"), |field| {
                field
                    .required()
                    .description("x".repeat(nebula_metadata::MAX_METADATA_SCHEMA_BYTES - 512))
            })
            .build()
    }
}

struct LargeCatalogAction;

impl Action for LargeCatalogAction {
    type Input = LargeRecord;
    type Output = LargeRecord;
    fn metadata() -> ActionMetadataDraft {
        CatalogAction::metadata().with_inline_icon("")
    }
    fn dependencies() -> &'static Dependencies {
        CatalogAction::dependencies()
    }
}

impl StatelessAction for LargeCatalogAction {
    async fn execute(
        &self,
        input: LargeRecord,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<LargeRecord>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn whole_canonical_leaf_budget_includes_both_schemas_and_leaf_overhead() {
    let baseline = InstanceFactory::new(LargeCatalogAction::metadata(), LargeCatalogAction)
        .expect("two near-limit schemas fit together");
    let baseline_size = serde_json::to_vec(baseline.metadata())
        .expect("serializable")
        .len();
    let available = nebula_metadata::MAX_METADATA_JSON_BYTES
        .checked_sub(baseline_size)
        .expect("baseline fits default decoder");
    assert!(
        available < nebula_metadata::MAX_SHARED_METADATA_BYTES / 2,
        "schemas dominate the whole-record budget"
    );
    let exact = InstanceFactory::new(
        LargeCatalogAction::metadata().with_inline_icon("x".repeat(available)),
        LargeCatalogAction,
    )
    .expect("exact ceiling admits");
    let bytes = serde_json::to_vec(exact.metadata()).expect("serializable");
    assert_eq!(bytes.len(), nebula_metadata::MAX_METADATA_JSON_BYTES);
    let recorded = RecordedActionMetadata::from_slice(&bytes, MetadataDecodeLimits::default())
        .expect("every admitted canonical record fits default ingress");
    assert_eq!(
        recorded
            .readmit_against(exact.metadata())
            .expect("fresh evidence"),
        **exact.metadata()
    );
    let mut oversized: Value = serde_json::from_slice(&bytes).expect("exact record is JSON");
    oversized["max_concurrent"] = Value::from(1);
    let oversized_bytes = serde_json::to_vec(&oversized).expect("oversized record serializes");
    assert!(oversized_bytes.len() > nebula_metadata::MAX_METADATA_JSON_BYTES);
    assert_eq!(
        serde_json::from_value::<RecordedActionMetadata>(oversized)
            .expect_err("structural serde also checks the canonical whole record")
            .to_string(),
        MetadataError::RecordTooLarge.to_string()
    );
    assert_eq!(
        RecordedActionMetadata::from_slice(&oversized_bytes, MetadataDecodeLimits::default()),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
    let error = InstanceFactory::new(
        LargeCatalogAction::metadata().with_inline_icon("x".repeat(available + 1)),
        LargeCatalogAction,
    )
    .err()
    .expect("one byte beyond the canonical record ceiling cannot publish a factory");
    std::assert_matches!(
        error,
        nebula_action::ActionMetadataAdmissionError::Schema(
            nebula_metadata::MetadataBuildError::Metadata(MetadataError::RecordTooLarge)
        )
    );
}
