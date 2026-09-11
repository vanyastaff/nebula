//! Catalog validation emits diagnostic outcomes without definition payloads.

use std::error::Error as _;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use nebula_core::ActionKey;
use nebula_error::{Classify, ErrorCategory, ErrorSeverity};
use nebula_metadata::{
    DeprecationNotice, MaturityLevel, MetadataDraft, MetadataError, MetadataReadmissionError,
    PluginManifest, RecordedBaseMetadata, validate_base_compat,
};
use nebula_schema::ValidSchema;
use semver::Version;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("test capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn draft_name_rejection_does_not_expose_submitted_text() {
    const SUBMITTED: &str = " \t\u{2003}";
    let error = MetadataDraft::try_new("example", SUBMITTED, "private-description")
        .expect_err("blank draft name is rejected");

    let rendered = format!("{error:?}: {error}");
    assert!(!rendered.contains(SUBMITTED));
    assert!(!rendered.contains("private-description"));
}

#[test]
fn intrinsic_errors_have_stable_validation_classification() {
    for (error, code) in [
        (MetadataError::BlankName, "METADATA:BLANK_NAME"),
        (
            MetadataError::MissingDeprecationNotice,
            "METADATA:MISSING_DEPRECATION",
        ),
        (MetadataError::InvalidKey, "METADATA:INVALID_KEY"),
    ] {
        assert_eq!(error.code().as_str(), code);
        assert_eq!(error.category(), ErrorCategory::Validation);
        assert_eq!(error.severity(), ErrorSeverity::Error);
        assert!(!error.is_retryable());
    }

    let error = MetadataReadmissionError::DefinitionMismatch;
    assert_eq!(
        error.code().as_str(),
        "METADATA:RECORDED_DEFINITION_MISMATCH"
    );
    assert_eq!(error.category(), ErrorCategory::Validation);
    assert_eq!(error.severity(), ErrorSeverity::Error);
    assert!(!error.is_retryable());
}

#[test]
fn manifest_errors_preserve_typed_sources() {
    let name_error = PluginManifest::builder("example", " ")
        .build()
        .expect_err("blank name");
    assert_eq!(name_error.code().as_str(), "MANIFEST:INVALID_METADATA");
    assert_eq!(
        name_error
            .source()
            .expect("intrinsic cause")
            .downcast_ref::<MetadataError>(),
        Some(&MetadataError::BlankName),
    );

    let key_error = PluginManifest::builder("bad!key", "Example")
        .build()
        .expect_err("invalid key");
    assert_eq!(key_error.code().as_str(), "MANIFEST:INVALID_KEY");
    let expected = "bad!key"
        .parse::<nebula_core::PluginKey>()
        .expect_err("invalid key");
    assert_eq!(
        key_error
            .source()
            .expect("key parser cause")
            .downcast_ref::<nebula_core::PluginKeyParseError>(),
        Some(&expected),
    );
}

#[test]
fn construction_and_revision_traces_exclude_submitted_payloads() {
    const SUBMITTED: &str = "submitted_private_payload";
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_writer(capture.clone())
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let report = nebula_schema::ValidationError::builder(SUBMITTED)
            .message(SUBMITTED)
            .build();
        let schema_error = nebula_metadata::MetadataBuildError::from(
            nebula_schema::ValidationReport::from(report),
        );
        assert!(!format!("{schema_error:?}: {schema_error}").contains(SUBMITTED));
        let metadata = MetadataDraft::try_new(
            SUBMITTED.parse::<ActionKey>().expect("valid key"),
            SUBMITTED,
            SUBMITTED,
        )
        .expect("valid metadata")
        .bind_schema(ValidSchema::empty());
        let deprecated = MetadataDraft::try_new(SUBMITTED, SUBMITTED, SUBMITTED)
            .expect("valid metadata")
            .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)).reason(SUBMITTED))
            .mark_stable()
            .bind_schema(ValidSchema::empty());
        assert_eq!(deprecated.maturity(), MaturityLevel::Deprecated);

        let name_error = MetadataDraft::try_new(SUBMITTED, " ", SUBMITTED).expect_err("blank name");
        assert!(!format!("{name_error:?}: {name_error}").contains(SUBMITTED));
        let mut invalid_lifecycle = serde_json::to_value(&metadata).expect("metadata serializes");
        invalid_lifecycle["maturity"] = "deprecated".into();
        serde_json::from_value::<RecordedBaseMetadata<ActionKey>>(invalid_lifecycle)
            .expect_err("notice required");
        let previous = MetadataDraft::try_new(
            SUBMITTED.parse::<ActionKey>().expect("valid key"),
            SUBMITTED,
            SUBMITTED,
        )
        .expect("valid metadata")
        .with_version(Version::new(2, 0, 0))
        .bind_schema(ValidSchema::empty());
        let recorded: RecordedBaseMetadata<ActionKey> =
            serde_json::from_value(serde_json::to_value(&metadata).expect("metadata serializes"))
                .expect("recorded metadata validates");
        recorded
            .readmit_against(&previous)
            .expect_err("changed static definition rejects recorded evidence");
        validate_base_compat(&metadata, &previous).expect_err("version regression");
        let renamed = MetadataDraft::try_new(
            "different".parse::<ActionKey>().expect("valid key"),
            SUBMITTED,
            SUBMITTED,
        )
        .expect("valid metadata")
        .bind_schema(ValidSchema::empty());
        validate_base_compat(&renamed, &metadata).expect_err("identity change");

        PluginManifest::builder(SUBMITTED, SUBMITTED)
            .description(SUBMITTED)
            .deprecation(DeprecationNotice::new(Version::new(1, 0, 0)).reason(SUBMITTED))
            .build()
            .expect("valid manifest");
        let key_error = PluginManifest::builder(format!("{SUBMITTED}!"), SUBMITTED)
            .build()
            .expect_err("invalid key");
        assert!(!format!("{key_error:?}: {key_error}").contains(SUBMITTED));
    });

    let output = String::from_utf8(capture.0.lock().expect("test capture lock").clone())
        .expect("UTF-8 tracing output");
    assert!(output.contains("metadata.try_construct_draft"), "{output}");
    assert!(output.contains("metadata.bind_schema"), "{output}");
    assert!(output.contains("metadata.reject_schema"), "{output}");
    assert!(output.contains("METADATA:INVALID_SCHEMA"), "{output}");
    assert!(output.contains("metadata.resolve_maturity"), "{output}");
    assert!(output.contains("metadata.readmit_recorded"), "{output}");
    assert!(
        output.contains("METADATA:RECORDED_DEFINITION_MISMATCH"),
        "{output}"
    );
    assert!(output.contains("METADATA:VERSION_REGRESSED"), "{output}");
    assert!(output.contains("METADATA:KEY_CHANGED"), "{output}");
    assert!(output.contains("MANIFEST:INVALID_KEY"), "{output}");
    assert!(
        !output.contains(SUBMITTED),
        "submitted definition leaked: {output}"
    );
}
