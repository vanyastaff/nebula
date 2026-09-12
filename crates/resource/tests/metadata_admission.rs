//! Resource metadata is authored without a schema and admitted once by its factory.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use nebula_core::{Dependencies, ResourceKey, resource_key};
use nebula_error::Classify as _;
use nebula_resource::{
    Error, HasCredentialSlots, KindActivator, MetadataBuildError, Provider,
    RecordedResourceMetadata, Resident, ResourceActivatorRegistry, ResourceConfig, ResourceContext,
    ResourceFactory, ResourceMetadataDraft,
};
use nebula_schema::{HasSchema, SchemaKind, ValidSchema, ValidationReport};
use semver::Version;

static SCHEMA_CALLS: AtomicUsize = AtomicUsize::new(0);
static DEFAULT_DRAFT_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, serde::Deserialize)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "the fixture must have an empty-record wire shape, not unit null"
)]
struct EmptyRecordConfig {}

impl HasSchema for EmptyRecordConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        SCHEMA_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(ValidSchema::empty())
    }
}

impl ResourceConfig for EmptyRecordConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct MetadataProbe;

impl HasCredentialSlots for MetadataProbe {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}

impl nebula_core::DeclaresDependencies for MetadataProbe {
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

#[async_trait::async_trait]
impl Provider for MetadataProbe {
    type Config = EmptyRecordConfig;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.metadata-probe")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("Metadata probe"),
            "Factory admission probe",
        )
        .mark_beta()
        .with_tags(["metadata", "resource"])
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _context: &ResourceContext,
    ) -> Result<Self::Instance, Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl nebula_resource::ResidentProvider for MetadataProbe {}

#[derive(Clone)]
struct DefaultMismatchProbe;

impl HasCredentialSlots for DefaultMismatchProbe {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}

impl nebula_core::DeclaresDependencies for DefaultMismatchProbe {}

#[async_trait::async_trait]
impl Provider for DefaultMismatchProbe {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.default-metadata-key")
    }

    fn metadata() -> ResourceMetadataDraft {
        DEFAULT_DRAFT_CALLS.fetch_add(1, Ordering::SeqCst);
        ResourceMetadataDraft::new(
            resource_key!("test.wrong-default-metadata-key"),
            nebula_resource::metadata_name!("test.wrong-default-metadata-key"),
            "",
        )
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _context: &ResourceContext,
    ) -> Result<Self::Instance, Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl nebula_resource::ResidentProvider for DefaultMismatchProbe {}

#[derive(Clone)]
struct ExplicitMismatchProbe;

impl HasCredentialSlots for ExplicitMismatchProbe {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}

impl nebula_core::DeclaresDependencies for ExplicitMismatchProbe {}

#[async_trait::async_trait]
impl Provider for ExplicitMismatchProbe {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("ExplicitMismatchProbe"),
            "",
        )
    }

    fn key() -> ResourceKey {
        resource_key!("test.explicit-metadata-key")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _context: &ResourceContext,
    ) -> Result<Self::Instance, Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl nebula_resource::ResidentProvider for ExplicitMismatchProbe {}

fn factory() -> impl ResourceFactory {
    KindActivator::<MetadataProbe, _, _>::new(
        || MetadataProbe,
        || Resident::new(nebula_resource::ResidentConfig::default()),
    )
}

#[test]
fn factory_binds_and_caches_the_canonical_config_schema_once() {
    SCHEMA_CALLS.store(0, Ordering::SeqCst);
    let factory = factory();

    let first = factory.metadata().expect("valid static definition");
    let second = factory.metadata().expect("cached static definition");

    assert!(std::ptr::eq(first, second));
    assert_eq!(SCHEMA_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(first.base().key(), &MetadataProbe::key());
    assert_eq!(first.base().name(), "Metadata probe");
    assert_eq!(first.base().tags(), ["metadata", "resource"]);
    assert_eq!(first.base().schema().kind(), SchemaKind::Record);
    assert!(first.base().schema().fields().is_empty());
}

#[test]
fn recorded_metadata_only_readmits_against_an_exact_fresh_definition() {
    let factory = factory();
    let fresh = factory.metadata().expect("valid static definition");
    let wire = serde_json::to_string(fresh).expect("admitted metadata serializes");
    let recorded: RecordedResourceMetadata =
        serde_json::from_str(&wire).expect("recorded evidence deserializes");

    let readmitted = recorded
        .readmit_against(fresh)
        .expect("exact evidence readmits");
    assert_eq!(readmitted, fresh.clone());

    let changed = ResourceMetadataDraft::new(
        MetadataProbe::key(),
        nebula_resource::metadata_name!("Metadata probe"),
        "Factory admission probe",
    )
    .with_version(Version::new(2, 0, 0));
    let changed_factory = KindActivator::<MetadataProbe, _, _>::with_metadata(
        changed,
        || MetadataProbe,
        || Resident::new(nebula_resource::ResidentConfig::default()),
    );
    let changed = changed_factory
        .metadata()
        .expect("changed static definition is valid");

    assert!(recorded.readmit_against(changed).is_err());
}

#[test]
fn forged_recorded_metadata_cannot_become_an_admitted_definition() {
    let factory = factory();
    let fresh = factory.metadata().expect("valid static definition");
    let wire = serde_json::to_value(fresh).expect("admitted metadata serializes");

    let mut forged_key = wire.clone();
    forged_key["base"]["key"] = serde_json::json!("test.forged-metadata-probe");
    let mut forged_schema = wire;
    forged_schema["base"]["schema"] = serde_json::to_value(
        nebula_schema::schema_of::<String>().expect("string has a valid schema"),
    )
    .expect("valid schema serializes");

    for forged in [forged_key, forged_schema] {
        let recorded: RecordedResourceMetadata =
            serde_json::from_value(forged).expect("recorded evidence accepts valid wire shapes");
        std::assert_matches!(
            recorded.readmit_against(fresh),
            Err(nebula_resource::MetadataReadmissionError::DefinitionMismatch)
        );
    }
}

#[test]
fn provider_default_draft_key_mismatch_is_cached_and_rejected() {
    DEFAULT_DRAFT_CALLS.store(0, Ordering::SeqCst);
    let factory = KindActivator::<DefaultMismatchProbe, _, _>::new(
        || DefaultMismatchProbe,
        || Resident::new(nebula_resource::ResidentConfig::default()),
    );

    for _ in 0..2 {
        let error = factory
            .metadata()
            .expect_err("default metadata key mismatch must fail admission");
        assert_eq!(error.code().as_str(), "RESOURCE:METADATA_KEY_MISMATCH");
        assert!(
            matches!(
                error,
                MetadataBuildError::KeyMismatch { expected, actual }
                    if expected == resource_key!("test.default-metadata-key")
                        && actual == resource_key!("test.wrong-default-metadata-key")
            ),
            "default metadata mismatch must retain both typed keys"
        );
    }
    assert_eq!(
        DEFAULT_DRAFT_CALLS.load(Ordering::SeqCst),
        1,
        "failed metadata admission must be cached"
    );
}

#[test]
fn explicit_draft_key_mismatch_fails_before_registry_mutation() {
    let factory = KindActivator::<ExplicitMismatchProbe, _, _>::with_metadata(
        ResourceMetadataDraft::new(
            resource_key!("test.wrong-explicit-metadata-key"),
            nebula_resource::metadata_name!("test.wrong-explicit-metadata-key"),
            "",
        ),
        || ExplicitMismatchProbe,
        || Resident::new(nebula_resource::ResidentConfig::default()),
    );
    let mut registry = ResourceActivatorRegistry::new();

    let error = match registry.insert("test.explicit-metadata-key", Arc::new(factory)) {
        Ok(_) => panic!("mismatched explicit draft must fail admission"),
        Err(error) => error,
    };

    assert_eq!(error.code().as_str(), "RESOURCE:METADATA_KEY_MISMATCH");
    assert!(
        matches!(
            error,
            MetadataBuildError::KeyMismatch { expected, actual }
                if expected == resource_key!("test.explicit-metadata-key")
                    && actual == resource_key!("test.wrong-explicit-metadata-key")
        ),
        "explicit metadata mismatch must retain both typed keys"
    );
    assert!(
        registry.is_empty(),
        "failed admission must not mutate the registry"
    );
}

fn catalog_factory(draft: ResourceMetadataDraft) -> impl ResourceFactory {
    KindActivator::<ExplicitMismatchProbe, _, _>::with_metadata(
        draft,
        || ExplicitMismatchProbe,
        || Resident::new(nebula_resource::ResidentConfig::default()),
    )
}

#[test]
fn shared_catalog_failures_are_cached_before_registry_mutation() {
    use nebula_metadata::{MetadataError, MetadataField};
    for (draft, expected) in [
        (
            ExplicitMismatchProbe::metadata().with_tags([" "]),
            MetadataError::BlankTag,
        ),
        (
            ResourceMetadataDraft::new(
                ExplicitMismatchProbe::key(),
                nebula_resource::metadata_name!("Probe"),
                "x".repeat(8193),
            ),
            MetadataError::FieldTooLarge(MetadataField::Description),
        ),
        (
            ResourceMetadataDraft::try_new(ExplicitMismatchProbe::key(), "\"".repeat(20_000), "")
                .expect("nonblank name"),
            MetadataError::SharedBudgetExceeded,
        ),
    ] {
        let factory = Arc::new(catalog_factory(draft));
        let mut registry = ResourceActivatorRegistry::new();
        for _ in 0..2 {
            std::assert_matches!(factory.metadata(),
                Err(MetadataBuildError::Definition(nebula_metadata::MetadataBuildError::Metadata(actual))) if actual == expected);
            std::assert_matches!(registry.insert(ExplicitMismatchProbe::key().as_str(), factory.clone()).map(|_| ()),
                Err(MetadataBuildError::Definition(nebula_metadata::MetadataBuildError::Metadata(actual))) if actual == expected);
            assert!(registry.is_empty());
        }
    }
}

#[test]
fn same_semver_catalog_changes_require_exact_fresh_evidence() {
    use nebula_metadata::{
        CatalogLink, CatalogLinkRelation, CatalogReference, DeprecationNotice,
        MetadataDecodeLimits, RemovalSchedule,
    };
    let original = catalog_factory(ExplicitMismatchProbe::metadata());
    let original = original.metadata().expect("valid definition");
    let recorded = RecordedResourceMetadata::from_slice(
        &serde_json::to_vec(original).expect("serializable"),
        MetadataDecodeLimits::default(),
    )
    .expect("default decoder");
    for draft in [
        ExplicitMismatchProbe::metadata()
            .with_categories(["network.http".parse().expect("category")]),
        ExplicitMismatchProbe::metadata().add_link(CatalogLink::new(
            CatalogLinkRelation::Reference,
            "/reference".parse().expect("link"),
        )),
        ExplicitMismatchProbe::metadata().with_deprecation(
            DeprecationNotice::new(Version::new(1, 0, 0))
                .with_removal(RemovalSchedule::AtVersion(Version::new(2, 0, 0)))
                .with_replacement(CatalogReference::resource(resource_key!(
                    "test.replacement"
                )))
                .with_reason("Use the replacement provider"),
        ),
    ] {
        let changed = catalog_factory(draft);
        let changed = changed.metadata().expect("valid changed definition");
        assert_eq!(original.base().version(), changed.base().version());
        std::assert_matches!(
            recorded.readmit_against(changed),
            Err(nebula_metadata::MetadataReadmissionError::DefinitionMismatch)
        );
        let current = RecordedResourceMetadata::from_slice(
            &serde_json::to_vec(changed).expect("serializable"),
            MetadataDecodeLimits::default(),
        )
        .expect("default decoder");
        assert_eq!(
            current.readmit_against(changed).expect("fresh evidence"),
            *changed
        );
    }
}

#[test]
fn leaf_wire_is_closed_redacted_and_transport_bounded() {
    use nebula_metadata::{MetadataDecodeError, MetadataDecodeLimits};
    const CANARY: &str = "PRIVATE_RESOURCE_WIRE_CANARY";
    let factory = catalog_factory(ExplicitMismatchProbe::metadata());
    let admitted = factory.metadata().expect("valid definition");
    let wire = serde_json::to_value(admitted).expect("serializable");
    assert_eq!(wire["base"]["metadata_wire_version"], 2);
    let mut unknown = wire.clone();
    unknown[CANARY] = serde_json::json!(CANARY);
    let mut wrong_type = wire.clone();
    wrong_type["base"] = serde_json::json!(CANARY);
    let mut legacy = wire["base"].clone();
    legacy
        .as_object_mut()
        .expect("base object")
        .remove("metadata_wire_version");
    let mut missing_version = wire.clone();
    missing_version["base"]
        .as_object_mut()
        .expect("base object")
        .remove("metadata_wire_version");
    let positional = serde_json::json!([wire["base"]]);
    for rejected in [unknown, wrong_type, legacy, missing_version, positional] {
        let bytes = serde_json::to_vec(&rejected).expect("serializable");
        let error =
            serde_json::from_slice::<RecordedResourceMetadata>(&bytes).expect_err("invalid wire");
        assert!(!format!("{error:?} {error}").contains(CANARY));
        assert_eq!(
            RecordedResourceMetadata::from_slice(&bytes, MetadataDecodeLimits::default()),
            Err(MetadataDecodeError::InvalidRecord)
        );
    }
    let bytes = serde_json::to_vec(&wire).expect("serializable");
    let exact = MetadataDecodeLimits::new(bytes.len()).expect("valid limit");
    let shorter = MetadataDecodeLimits::new(bytes.len() - 1).expect("valid lower limit");
    let recorded = RecordedResourceMetadata::from_reader(bytes.as_slice(), exact)
        .expect("exact limit accepts");
    assert_eq!(
        recorded.readmit_against(admitted).expect("fresh evidence"),
        *admitted
    );
    assert_eq!(
        RecordedResourceMetadata::from_slice(&bytes, shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
    assert_eq!(
        RecordedResourceMetadata::from_reader(bytes.as_slice(), shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
}
