//! Catalog admission and recorded evidence stay outside credential runtime authority.

use std::{
    cell::RefCell,
    error::Error as _,
    io::{self, Write},
    sync::{Arc, Mutex},
};

use nebula_core::credential_key;
use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, CredentialMetadataAdmissionError,
    CredentialMetadataDraft, CredentialRegistry, RecordedCredentialMetadata, RegisterError,
    error::CredentialError, resolve::StaticResolveResult, scheme::SecretToken,
};
use nebula_metadata::{
    CatalogLink, CatalogLinkRelation, CatalogReference, DeprecationNotice, MetadataDecodeError,
    MetadataDecodeLimits, RemovalSchedule,
};
use semver::Version;

thread_local! {
    static AUTHORED: RefCell<Option<CredentialMetadataDraft>> = const { RefCell::new(None) };
}

struct CatalogCredential;

const SCHEMA_CANARY: &str = "PRIVATE_CREDENTIAL_SCHEMA_REPORT_CANARY";

#[derive(serde::Deserialize)]
struct RejectedProperties;

impl nebula_schema::HasSchema for RejectedProperties {
    fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
        let mut report = nebula_schema::ValidationReport::new();
        report.push(
            nebula_schema::ValidationError::builder("rejected_schema")
                .message(SCHEMA_CANARY)
                .param("private", SCHEMA_CANARY)
                .build(),
        );
        Err(report)
    }
}

struct RejectedSchemaCredential;

#[nebula_credential::credential(key = "test.rejected-schema", name = "Rejected schema")]
impl RejectedSchemaCredential {
    type Properties = RejectedProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _: &RejectedProperties,
        _: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        unreachable!("invalid properties schema cannot reach credential resolution")
    }
}

#[nebula_credential::credential(key = "test.catalog-foundation", name = "Catalog credential")]
impl CatalogCredential {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    fn metadata() -> CredentialMetadataDraft {
        AUTHORED.with_borrow_mut(|draft| {
            draft
                .take()
                .expect("test supplied a draft before registration")
        })
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        (): &(),
        _: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        unreachable!("catalog admission must not execute credential resolution")
    }
}

fn draft() -> CredentialMetadataDraft {
    CredentialMetadataDraft::try_new(
        credential_key!("test.catalog-foundation"),
        "Catalog credential",
        "",
    )
    .expect("valid dynamic display name")
}

fn register(
    registry: &mut CredentialRegistry,
    draft: CredentialMetadataDraft,
) -> Result<(), RegisterError> {
    AUTHORED.set(Some(draft));
    registry.register(CatalogCredential, "catalog-foundation")
}

fn admit(draft: CredentialMetadataDraft) -> CredentialMetadata {
    let mut registry = CredentialRegistry::new();
    register(&mut registry, draft).expect("valid catalog definition");
    registry
        .metadata(CatalogCredential::KEY)
        .expect("registered metadata")
        .clone()
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture mutex")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn shared_admission_rejects_before_insertion_without_error_or_trace_payloads() {
    const CANARY: &str = "PRIVATE_CREDENTIAL_CATALOG_CANARY";
    let capture = Capture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let mut registry = CredentialRegistry::new();
        for invalid in [
            draft().with_tags([" "]),
            CredentialMetadataDraft::new(
                credential_key!("test.catalog-foundation"),
                nebula_credential::metadata_name!("Catalog credential"),
                CANARY.repeat(300),
            ),
            draft().with_documentation_url(format!("javascript:{CANARY}")),
            draft().with_deprecation(
                DeprecationNotice::new(Version::new(2, 0, 0)).with_reason(CANARY),
            ),
            CredentialMetadataDraft::try_new(
                credential_key!("test.catalog-foundation"),
                "\"".repeat(20_000),
                "",
            )
            .expect("nonblank name"),
        ] {
            for _ in 0..2 {
                let error = register(&mut registry, invalid.clone())
                    .expect_err("invalid catalog definition");
                std::assert_matches!(
                    error,
                    RegisterError::Metadata(CredentialMetadataAdmissionError::CatalogMetadata)
                );
                assert!(!format!("{error:?} {error}").contains(CANARY));
                let cause = error.source().expect("typed admission cause");
                assert!(
                    cause.source().is_none(),
                    "credential failures must not retain a schema report"
                );
                assert!(!format!("{cause:?} {cause}").contains(CANARY));
                assert!(registry.is_empty());
                assert!(registry.metadata(CatalogCredential::KEY).is_none());
            }
        }
        register(&mut registry, draft()).expect("failed admission leaves key available");
        let error = registry
            .register(RejectedSchemaCredential, "catalog-foundation")
            .expect_err("invalid properties schema must not install dispatch");
        std::assert_matches!(
            error,
            RegisterError::Metadata(CredentialMetadataAdmissionError::PropertiesSchema)
        );
        let cause = error.source().expect("typed schema admission cause");
        assert!(
            cause.source().is_none(),
            "schema reports must be discarded at the credential boundary"
        );
        assert!(!format!("{error:?} {error} {cause:?} {cause}").contains(SCHEMA_CANARY));
        assert!(registry.metadata(RejectedSchemaCredential::KEY).is_none());
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry
                .metadata(CatalogCredential::KEY)
                .expect("admitted")
                .pattern(),
            <<CatalogCredential as Credential>::Scheme as nebula_core::AuthScheme>::pattern()
        );
    });
    let trace =
        String::from_utf8(capture.0.lock().expect("capture mutex").clone()).expect("UTF-8 trace");
    assert!(
        trace.contains("CREDENTIAL:CATALOG_METADATA"),
        "the rejection trace must actually be captured: {trace}"
    );
    assert!(!trace.contains(CANARY));
    assert!(trace.contains("CREDENTIAL:PROPERTIES_SCHEMA_INVALID"));
    assert!(!trace.contains(SCHEMA_CANARY));
}

#[test]
fn same_semver_catalog_changes_and_pattern_require_exact_fresh_evidence() {
    let original = admit(draft());
    let recorded = RecordedCredentialMetadata::from_slice(
        &serde_json::to_vec(&original).expect("serializable"),
        MetadataDecodeLimits::default(),
    )
    .expect("default decoder");
    for changed in [
        draft().with_categories(["auth.tokens".parse().expect("category")]),
        draft().add_link(CatalogLink::new(
            CatalogLinkRelation::Setup,
            "/setup".parse().expect("link"),
        )),
        draft().with_deprecation(
            DeprecationNotice::new(Version::new(1, 0, 0))
                .with_removal(RemovalSchedule::AtVersion(Version::new(2, 0, 0)))
                .with_replacement(CatalogReference::credential(credential_key!(
                    "test.replacement"
                )))
                .with_reason("Use replacement credential"),
        ),
    ] {
        let changed = admit(changed);
        assert_eq!(original.version(), changed.version());
        assert!(recorded.readmit_against(&changed).is_err());
        let current = RecordedCredentialMetadata::from_slice(
            &serde_json::to_vec(&changed).expect("serializable"),
            MetadataDecodeLimits::default(),
        )
        .expect("default decoder");
        assert_eq!(
            current.readmit_against(&changed).expect("exact evidence"),
            changed
        );
    }
    let mut forged = serde_json::to_value(&original).expect("serializable");
    forged["pattern"] =
        serde_json::to_value(nebula_credential::AuthPattern::NoAuth).expect("pattern serializes");
    let forged: RecordedCredentialMetadata =
        serde_json::from_value(forged).expect("recognized pattern records as evidence");
    assert!(forged.readmit_against(&original).is_err());
}

#[test]
fn leaf_wire_rejects_legacy_and_redacts_unknown_keys_and_wrong_types() {
    const CANARY: &str = "PRIVATE_CREDENTIAL_WIRE_CANARY";
    let admitted = admit(draft());
    let wire = serde_json::to_value(&admitted).expect("serializable");
    assert_eq!(wire["base"]["metadata_wire_version"], 2);
    let mut unknown = wire.clone();
    unknown[CANARY] = serde_json::json!(CANARY);
    let mut wrong_type = wire.clone();
    wrong_type["base"] = serde_json::json!(CANARY);
    let mut unknown_variant = wire.clone();
    unknown_variant["pattern"] = serde_json::json!(CANARY);
    let mut legacy = wire["base"].clone();
    legacy
        .as_object_mut()
        .expect("base object")
        .remove("metadata_wire_version");
    legacy["pattern"] = wire["pattern"].clone();
    let mut missing_version = wire.clone();
    missing_version["base"]
        .as_object_mut()
        .expect("base object")
        .remove("metadata_wire_version");
    let positional = serde_json::json!([wire["base"], wire["pattern"]]);
    for rejected in [
        unknown,
        wrong_type,
        unknown_variant,
        legacy,
        missing_version,
        positional,
    ] {
        let bytes = serde_json::to_vec(&rejected).expect("serializable");
        let error =
            serde_json::from_slice::<RecordedCredentialMetadata>(&bytes).expect_err("invalid wire");
        assert!(!format!("{error:?} {error}").contains(CANARY));
        assert_eq!(
            RecordedCredentialMetadata::from_slice(&bytes, MetadataDecodeLimits::default()),
            Err(MetadataDecodeError::InvalidRecord)
        );
    }
    let bytes = serde_json::to_vec(&wire).expect("serializable");
    let exact = MetadataDecodeLimits::new(bytes.len()).expect("valid limit");
    let shorter = MetadataDecodeLimits::new(bytes.len() - 1).expect("valid lower limit");
    let recorded = RecordedCredentialMetadata::from_reader(bytes.as_slice(), exact)
        .expect("exact limit accepts");
    assert_eq!(
        recorded.readmit_against(&admitted).expect("fresh evidence"),
        admitted
    );
    assert_eq!(
        RecordedCredentialMetadata::from_slice(&bytes, shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
    assert_eq!(
        RecordedCredentialMetadata::from_reader(bytes.as_slice(), shorter),
        Err(MetadataDecodeError::EnvelopeTooLarge)
    );
}
