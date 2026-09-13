use std::fmt;

use crate::ValidationReport;

use super::SchemaGraphDocument;

/// A semantic graph document failed admission.
///
/// Formatting is deliberately payload-free. Inspect [`Self::report`] for
/// stable diagnostics or recover the original document with
/// [`Self::into_document`].
pub struct SchemaAdmissionError {
    document: SchemaGraphDocument,
    report: ValidationReport,
}

impl SchemaAdmissionError {
    pub(super) const fn new(document: SchemaGraphDocument, report: ValidationReport) -> Self {
        Self { document, report }
    }

    /// Borrows the original lossless document.
    #[must_use]
    pub const fn document(&self) -> &SchemaGraphDocument {
        &self.document
    }

    /// Borrows the standard schema validation report.
    #[must_use]
    pub const fn report(&self) -> &ValidationReport {
        &self.report
    }

    /// Recovers the original lossless document.
    #[must_use]
    pub fn into_document(self) -> SchemaGraphDocument {
        self.document
    }
}

impl fmt::Debug for SchemaAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SchemaAdmissionError")
            .field("diagnostic_count", &self.report.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for SchemaAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "schema graph admission failed with {} diagnostic(s)",
            self.report.len()
        )
    }
}

impl std::error::Error for SchemaAdmissionError {}
