use std::borrow::Cow;

/// A single schema validation issue (error or warning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// Field key where issue was observed.
    pub key: String,
    /// Stable machine-readable issue code.
    pub code: Cow<'static, str>,
    /// Human-readable detail message.
    pub message: String,
}

impl ValidationIssue {
    /// Construct a new validation issue.
    pub fn new(
        key: impl Into<String>,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Result of schema validation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    errors: Vec<ValidationIssue>,
    warnings: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Create empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow hard errors.
    pub fn errors(&self) -> &[ValidationIssue] {
        self.errors.as_slice()
    }

    /// Borrow warnings.
    pub fn warnings(&self) -> &[ValidationIssue] {
        self.warnings.as_slice()
    }

    /// Returns true if report has at least one error.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns true if report has at least one warning.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Push hard error.
    pub fn push_error(&mut self, issue: ValidationIssue) {
        self.errors.push(issue);
    }

    /// Push warning.
    pub fn push_warning(&mut self, issue: ValidationIssue) {
        self.warnings.push(issue);
    }
}
