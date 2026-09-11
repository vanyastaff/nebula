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
        ResourceMetadataDraft::from_key(resource_key!("test.wrong-default-metadata-key"))
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
    forged_key["key"] = serde_json::json!("test.forged-metadata-probe");
    let mut forged_schema = wire;
    forged_schema["schema"] = serde_json::to_value(
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
        ResourceMetadataDraft::from_key(resource_key!("test.wrong-explicit-metadata-key")),
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
