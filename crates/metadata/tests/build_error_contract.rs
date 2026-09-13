//! Definition admission retains typed causes without logging authored payloads.

use std::error::Error as _;

use nebula_error::{Classify, ErrorCategory};
use nebula_metadata::{MetadataBuildError, MetadataError};
use nebula_schema::{ValidationError, ValidationReport};

#[test]
fn schema_failure_keeps_report_but_redacts_diagnostics() {
    let report = ValidationReport::from(
        ValidationError::builder("private_code")
            .message("private_schema_payload")
            .build(),
    );
    let error = MetadataBuildError::from(report);
    let retained = error
        .source()
        .expect("schema cause")
        .downcast_ref::<ValidationReport>()
        .expect("typed report");
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained.iter().next().unwrap().message(),
        "private_schema_payload"
    );
    assert_eq!(error.code().as_str(), "METADATA:INVALID_SCHEMA");
    assert_eq!(error.category(), ErrorCategory::Validation);
    let diagnostic = format!("{error}: {error:?}");
    assert!(!diagnostic.contains("private_schema_payload"));
    assert!(!diagnostic.contains("private_code"));
}

#[test]
fn intrinsic_failure_keeps_typed_cause() {
    let error = MetadataBuildError::from(MetadataError::BlankName);
    assert_eq!(
        error.source().unwrap().downcast_ref::<MetadataError>(),
        Some(&MetadataError::BlankName),
    );
    assert_eq!(error.code().as_str(), "METADATA:INVALID_DEFINITION");
}
