//! Resource metadata is authored without a schema and admitted once by its factory.

use std::sync::Arc;

use nebula_core::{Dependencies, resource_key};
use nebula_resource::{
    Error, HasCredentialSlots, KindActivator, Provider, RecordedResourceDefinition, Resident,
    ResourceActivatorRegistry, ResourceAdmissionError, ResourceConfig, ResourceContext,
    ResourceFactory, ResourceMetadata,
};
use nebula_schema::{HasSchema, SchemaKind, ValidSchema, ValidationReport};
use semver::Version;

thread_local! {
    /// Schema derivations on this thread; admission runs on the caller's
    /// thread, so tests running in parallel do not count each other's.
    static SCHEMA_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Metadata the probes' `Provider::metadata()` returns instead of their
    /// default, installed by [`admitted`] while a factory admits its definition.
    ///
    /// A factory reads metadata only from its provider type; these tests vary
    /// the authored metadata, so they install it for the admission.
    static AUTHORED: std::cell::RefCell<Option<ResourceMetadata>> =
        const { std::cell::RefCell::new(None) };
}

fn authored_or(default: impl FnOnce() -> ResourceMetadata) -> ResourceMetadata {
    AUTHORED.with_borrow(Clone::clone).unwrap_or_else(default)
}

/// Build `factory` with `authored` installed as its provider's metadata and
/// admit (and cache) its definition before the metadata is cleared.
fn admitted<F: ResourceFactory>(authored: ResourceMetadata, factory: impl FnOnce() -> F) -> F {
    AUTHORED.set(Some(authored));
    let factory = factory();
    let _admission = factory.definition();
    AUTHORED.set(None);
    factory
}

#[derive(Clone, Debug, serde::Deserialize)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "the fixture must have an empty-record wire shape, not unit null"
)]
struct EmptyRecordConfig {}

impl HasSchema for EmptyRecordConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        SCHEMA_CALLS.set(SCHEMA_CALLS.get() + 1);
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

    const KEY: &'static str = "test.metadata-probe";

    fn metadata() -> ResourceMetadata {
        authored_or(|| {
            ResourceMetadata::new(
                nebula_resource::metadata_name!("Metadata probe"),
                "Factory admission probe",
            )
            .mark_beta()
            .with_tags(["metadata", "resource"])
        })
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
struct ExplicitMetadataProbe;

impl HasCredentialSlots for ExplicitMetadataProbe {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}

impl nebula_core::DeclaresDependencies for ExplicitMetadataProbe {}

#[async_trait::async_trait]
impl Provider for ExplicitMetadataProbe {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    fn metadata() -> ResourceMetadata {
        authored_or(|| {
            ResourceMetadata::new(nebula_resource::metadata_name!("ExplicitMetadataProbe"), "")
        })
    }

    const KEY: &'static str = "test.explicit-metadata-key";

    async fn create(
        &self,
        _config: &Self::Config,
        _context: &ResourceContext,
    ) -> Result<Self::Instance, Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl nebula_resource::ResidentProvider for ExplicitMetadataProbe {}

fn factory() -> impl ResourceFactory {
    KindActivator::<MetadataProbe, _, _>::new(
        || MetadataProbe,
        nebula_resource::topology::fixed(|| {
            Resident::new(nebula_resource::ResidentConfig::default())
        }),
    )
}

#[test]
fn factory_binds_and_caches_the_canonical_config_schema_once() {
    SCHEMA_CALLS.set(0);
    let factory = factory();

    let first = factory.definition().expect("valid static definition");
    let second = factory.definition().expect("cached static definition");

    assert!(std::ptr::eq(first, second));
    assert_eq!(SCHEMA_CALLS.get(), 1);
    assert_eq!(
        first.base().key(),
        &nebula_resource::key_of::<MetadataProbe>()
    );
    assert_eq!(first.base().name(), "Metadata probe");
    assert_eq!(first.base().tags(), ["metadata", "resource"]);
    assert_eq!(first.base().schema().kind(), SchemaKind::Record);
    assert!(first.base().schema().properties().is_empty());
}

#[test]
fn authored_shared_authoring_surface_survives_factory_admission() {
    let version = Version::new(2, 3, 4);
    let category: nebula_metadata::CatalogCategoryKey =
        "network.http".parse().expect("valid category");
    let link = nebula_metadata::CatalogLink::new(
        nebula_metadata::CatalogLinkRelation::Setup,
        "/resources/metadata-probe/setup"
            .parse()
            .expect("valid relative link"),
    );
    let documentation_link = nebula_metadata::CatalogLink::new(
        nebula_metadata::CatalogLinkRelation::Overview,
        "https://example.test/resources/metadata-probe"
            .parse()
            .expect("valid documentation link"),
    );
    let authored = ResourceMetadata::try_new("Metadata probe", "Factory admission probe")
        .expect("dynamic name is valid")
        .with_version(version.clone())
        .with_icon(nebula_metadata::Icon::None)
        .with_inline_icon("metadata-probe")
        .with_documentation_url("https://example.test/resources/metadata-probe")
        .with_categories([category.clone()])
        .add_link(link.clone())
        .with_tags(["resource"])
        .add_tag("catalog")
        .mark_experimental()
        .mark_beta();
    let factory = admitted(authored, factory);
    let metadata = factory.definition().expect("valid static definition");
    let base = metadata.base();

    assert_eq!(base.key(), &nebula_resource::key_of::<MetadataProbe>());
    assert_eq!(base.name(), "Metadata probe");
    assert_eq!(base.description(), "Factory admission probe");
    assert_eq!(base.version(), &version);
    assert_eq!(
        base.icon(),
        &nebula_metadata::Icon::inline("metadata-probe")
    );
    assert_eq!(
        base.documentation_url(),
        Some("https://example.test/resources/metadata-probe")
    );
    assert_eq!(base.categories(), std::slice::from_ref(&category));
    assert_eq!(base.links(), [documentation_link, link]);
    assert_eq!(base.tags(), ["catalog", "resource"]);
    assert_eq!(base.maturity(), nebula_metadata::MaturityLevel::Beta);
}

#[test]
fn recorded_metadata_only_readmits_against_an_exact_fresh_definition() {
    let factory = factory();
    let fresh = factory.definition().expect("valid static definition");
    let wire = serde_json::to_string(fresh).expect("admitted metadata serializes");
    let recorded: RecordedResourceDefinition =
        serde_json::from_str(&wire).expect("recorded evidence deserializes");

    let readmitted = recorded
        .readmit_against(fresh)
        .expect("exact evidence readmits");
    assert_eq!(readmitted, fresh.clone());

    let changed = ResourceMetadata::new(
        nebula_resource::metadata_name!("Metadata probe"),
        "Factory admission probe",
    )
    .with_version(Version::new(2, 0, 0));
    let changed_factory = admitted(changed, crate::factory);
    let changed = changed_factory
        .definition()
        .expect("changed static definition is valid");

    assert!(recorded.readmit_against(changed).is_err());
}

#[test]
fn forged_recorded_metadata_cannot_become_an_admitted_definition() {
    let factory = factory();
    let fresh = factory.definition().expect("valid static definition");
    let wire = serde_json::to_value(fresh).expect("admitted metadata serializes");

    let mut forged_key = wire.clone();
    forged_key["base"]["key"] = serde_json::json!("test.forged-metadata-probe");
    let mut forged_schema = wire;
    forged_schema["base"]["schema"] = serde_json::to_value(
        nebula_schema::schema_of::<String>().expect("string has a valid schema"),
    )
    .expect("valid schema serializes");

    for forged in [forged_key, forged_schema] {
        let recorded: RecordedResourceDefinition =
            serde_json::from_value(forged).expect("recorded evidence accepts valid wire shapes");
        std::assert_matches!(
            recorded.readmit_against(fresh),
            Err(nebula_resource::ResourceReadmissionError::DefinitionMismatch)
        );
    }
}

#[test]
fn explicit_metadata_is_admitted_under_the_providers_key() {
    let factory = catalog_factory(ResourceMetadata::new(
        nebula_resource::metadata_name!("Another name"),
        "",
    ));

    let metadata = factory.definition().expect("keyless metadata is admitted");
    assert_eq!(
        metadata.base().key(),
        &nebula_resource::key_of::<ExplicitMetadataProbe>(),
        "metadata names no key, so the admitted definition carries the provider's"
    );
    assert_eq!(metadata.base().name(), "Another name");
}

fn catalog_factory(authored: ResourceMetadata) -> impl ResourceFactory {
    admitted(authored, || {
        KindActivator::<ExplicitMetadataProbe, _, _>::new(
            || ExplicitMetadataProbe,
            nebula_resource::topology::fixed(|| {
                Resident::new(nebula_resource::ResidentConfig::default())
            }),
        )
    })
}

#[test]
fn shared_catalog_failures_are_cached_before_registry_mutation() {
    use nebula_metadata::{MetadataError, MetadataField};
    for (authored, expected) in [
        (
            ExplicitMetadataProbe::metadata().with_tags([" "]),
            MetadataError::BlankTag,
        ),
        (
            ResourceMetadata::new(nebula_resource::metadata_name!("Probe"), "x".repeat(8193)),
            MetadataError::FieldTooLarge(MetadataField::Description),
        ),
        (
            ResourceMetadata::try_new("\"".repeat(20_000), "").expect("nonblank name"),
            MetadataError::SharedBudgetExceeded,
        ),
    ] {
        let factory = Arc::new(catalog_factory(authored));
        let mut registry = ResourceActivatorRegistry::new();
        for _ in 0..2 {
            std::assert_matches!(factory.definition(),
                Err(ResourceAdmissionError::Definition(nebula_metadata::MetadataBuildError::Metadata(actual))) if actual == expected);
            std::assert_matches!(registry.insert(nebula_resource::key_of::<ExplicitMetadataProbe>().as_str(), factory.clone()).map(|_| ()),
                Err(ResourceAdmissionError::Definition(nebula_metadata::MetadataBuildError::Metadata(actual))) if actual == expected);
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
    let original = catalog_factory(ExplicitMetadataProbe::metadata());
    let original = original.definition().expect("valid definition");
    let recorded = RecordedResourceDefinition::from_slice(
        &serde_json::to_vec(original).expect("serializable"),
        MetadataDecodeLimits::default(),
    )
    .expect("default decoder");
    for authored in [
        ExplicitMetadataProbe::metadata()
            .with_categories(["network.http".parse().expect("category")]),
        ExplicitMetadataProbe::metadata().add_link(CatalogLink::new(
            CatalogLinkRelation::Reference,
            "/reference".parse().expect("link"),
        )),
        ExplicitMetadataProbe::metadata().with_deprecation(
            DeprecationNotice::new(Version::new(1, 0, 0))
                .with_removal(RemovalSchedule::AtVersion(Version::new(2, 0, 0)))
                .with_replacement(CatalogReference::resource(resource_key!(
                    "test.replacement"
                )))
                .with_reason("Use the replacement provider"),
        ),
    ] {
        let changed = catalog_factory(authored);
        let changed = changed.definition().expect("valid changed definition");
        assert_eq!(original.base().version(), changed.base().version());
        std::assert_matches!(
            recorded.readmit_against(changed),
            Err(nebula_resource::ResourceReadmissionError::DefinitionMismatch)
        );
        let current = RecordedResourceDefinition::from_slice(
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
    let factory = catalog_factory(ExplicitMetadataProbe::metadata());
    let admitted = factory.definition().expect("valid definition");
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
            serde_json::from_slice::<RecordedResourceDefinition>(&bytes).expect_err("invalid wire");
        assert!(!format!("{error:?} {error}").contains(CANARY));
        assert_eq!(
            RecordedResourceDefinition::from_slice(&bytes, MetadataDecodeLimits::default()),
            Err(MetadataDecodeError::InvalidRecord)
        );
    }
    let bytes = serde_json::to_vec(&wire).expect("serializable");
    let exact = MetadataDecodeLimits::new(bytes.len()).expect("valid limit");
    let shorter = MetadataDecodeLimits::new(bytes.len() - 1).expect("valid lower limit");
    let recorded = RecordedResourceDefinition::from_reader(bytes.as_slice(), exact)
        .expect("exact limit accepts");
    assert_eq!(
        recorded.readmit_against(admitted).expect("fresh evidence"),
        *admitted
    );
    assert_eq!(
        RecordedResourceDefinition::from_slice(&bytes, shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
    assert_eq!(
        RecordedResourceDefinition::from_reader(bytes.as_slice(), shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
}
