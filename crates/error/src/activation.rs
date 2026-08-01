//! Structured activation diagnostics.
//!
//! An activation rejection has to say five things a caller can act on: which
//! rule fired ([`ActivationDiagnostic::code`]), where
//! ([`ActivationDiagnostic::path`]), what the contract required
//! ([`ActivationDiagnostic::expected`]), what was found
//! ([`ActivationDiagnostic::actual`]), and what to do about it
//! ([`ActivationDiagnostic::remediation`]). Flattening those into one prose
//! sentence loses every one of them: a caller cannot match on a sentence, and a
//! UI cannot point at the offending node.
//!
//! The type lives here, in the cross-cutting error crate, because both the
//! Core-tier workflow validator and the Business-tier plan compiler reject
//! activations and must produce the same shape. Owning it in either of those
//! would force the other into an upward dependency.

use core::fmt;

/// Longest a single diagnostic field may be, in bytes.
///
/// Diagnostics travel into logs and HTTP responses, and several fields embed
/// author-supplied text: a `path` names a node key, an `actual` reports the
/// contract that was observed. Neither is bounded by anything upstream, so a
/// workflow carrying a megabyte-long key would push a megabyte per diagnostic
/// into every log line and error body that mentions it. The bound is generous
/// enough that no honest identifier reaches it.
pub const MAX_DIAGNOSTIC_FIELD_BYTES: usize = 512;

/// Marker appended to a field cut to fit [`MAX_DIAGNOSTIC_FIELD_BYTES`].
///
/// A truncated value must be visibly truncated: a consumer comparing `actual`
/// against a known contract has to be able to tell "this differs" from "this is
/// the first 512 bytes of something that differs".
pub const TRUNCATION_MARKER: &str = "…";

/// Cut `field` to the byte bound, splitting only on a `char` boundary.
///
/// Slicing a `String` mid-codepoint panics, and these fields carry
/// author-supplied text, so the split point is walked back to the nearest
/// boundary rather than assumed.
fn bounded_field(field: String) -> String {
    if field.len() <= MAX_DIAGNOSTIC_FIELD_BYTES {
        return field;
    }
    let mut end = MAX_DIAGNOSTIC_FIELD_BYTES;
    while end > 0 && !field.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = String::with_capacity(end + TRUNCATION_MARKER.len());
    truncated.push_str(&field[..end]);
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

/// One stable, secret-free activation diagnostic.
///
/// Construct through [`ActivationDiagnostic::new`], which bounds every field
/// and refuses a diagnostic that would be missing one — a caller that receives
/// this type can rely on all five being present and non-empty.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActivationDiagnostic {
    code: String,
    path: String,
    expected: String,
    actual: String,
    remediation: String,
}

impl ActivationDiagnostic {
    /// Build one complete diagnostic, or `None` when the contract cannot hold.
    ///
    /// Returns `None` when any field is empty or blank, or when `code` exceeds
    /// [`MAX_DIAGNOSTIC_FIELD_BYTES`]. `code` is checked against the bound
    /// rather than cut to fit: it is a stable machine-readable contract, and a
    /// truncated code would silently name a different diagnostic. The remaining
    /// four fields are truncated with [`TRUNCATION_MARKER`].
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        path: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Option<Self> {
        let code = code.into();
        if code.len() > MAX_DIAGNOSTIC_FIELD_BYTES {
            return None;
        }

        let value = Self {
            code,
            path: bounded_field(path.into()),
            expected: bounded_field(expected.into()),
            actual: bounded_field(actual.into()),
            remediation: bounded_field(remediation.into()),
        };

        [
            value.code.as_str(),
            value.path.as_str(),
            value.expected.as_str(),
            value.actual.as_str(),
            value.remediation.as_str(),
        ]
        .iter()
        .all(|field| !field.trim().is_empty())
        .then_some(value)
    }

    /// Stable machine-readable diagnostic code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Canonical path to the incompatible workflow or registry element.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Secret-free description of the required contract.
    #[must_use]
    pub fn expected(&self) -> &str {
        &self.expected
    }

    /// Secret-free description of the observed contract, or a safe sentinel.
    #[must_use]
    pub fn actual(&self) -> &str {
        &self.actual
    }

    /// Stable, actionable remediation guidance.
    #[must_use]
    pub fn remediation(&self) -> &str {
        &self.remediation
    }
}

impl fmt::Debug for ActivationDiagnostic {
    /// Shows only `code` and `path`.
    ///
    /// `expected` and `actual` describe contract content, which is exactly
    /// where a credential default or a parameter value could reach a log line.
    /// A caller that wants them asks for them by name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationDiagnostic")
            .field("code", &self.code)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ActivationDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {}", self.code, self.path)
    }
}

/// Produce this rejection's diagnostics in a canonical order.
///
/// Every activation rejection implements this, so a caller that renders an
/// error does not need to know which layer refused it. Implementations return
/// at least one diagnostic: a rejection with nothing to say is not actionable,
/// and the whole point of the contract is that a caller can act.
pub trait ActivationDiagnostics {
    /// Canonically sorted, duplicate-free diagnostics for this rejection.
    fn activation_diagnostics(&self) -> Vec<ActivationDiagnostic>;
}

/// Sort and deduplicate diagnostics so equivalent input reports identically.
///
/// Ordering is by `(code, path, expected, actual, remediation)` through the
/// derived `Ord`, which makes the report reproducible: two runs over the same
/// workflow emit the same sequence, so a snapshot test is meaningful and a
/// caller diffing two reports sees only real changes.
#[must_use]
pub fn canonical_diagnostics(
    mut diagnostics: Vec<ActivationDiagnostic>,
) -> Vec<ActivationDiagnostic> {
    diagnostics.sort();
    diagnostics.dedup();
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(path: &str, actual: &str) -> ActivationDiagnostic {
        ActivationDiagnostic::new("E001", path, "a contract", actual, "fix it")
            .expect("the fixture uses non-empty diagnostic fields")
    }

    #[test]
    fn a_diagnostic_missing_any_field_is_refused() {
        assert!(ActivationDiagnostic::new("", "/workflow", "a", "b", "fix").is_none());
        assert!(ActivationDiagnostic::new("E1", "/workflow", "a", "", "fix").is_none());
        assert!(ActivationDiagnostic::new("E1", "   ", "a", "b", "fix").is_none());
    }

    /// Author-supplied text reaches `path` and `actual`, so an unbounded
    /// workflow value would push its whole length into every log line and error
    /// body that names the diagnostic.
    #[test]
    fn author_supplied_fields_are_bounded_and_visibly_truncated() {
        let overlong = "n".repeat(MAX_DIAGNOSTIC_FIELD_BYTES * 4);
        let bounded = diagnostic(&overlong, &overlong);

        for field in [bounded.path(), bounded.actual()] {
            assert!(
                field.len() <= MAX_DIAGNOSTIC_FIELD_BYTES + TRUNCATION_MARKER.len(),
                "a diagnostic field must not carry an unbounded workflow value"
            );
            assert!(
                field.ends_with(TRUNCATION_MARKER),
                "a truncated value must be visibly truncated, so a consumer cannot \
                 mistake a prefix for the whole contract"
            );
        }
        assert_eq!(
            bounded.expected(),
            "a contract",
            "a short field is left as-is"
        );
    }

    /// The bound splits on a `char` boundary: these fields carry author-supplied
    /// text, and slicing a `String` mid-codepoint panics.
    #[test]
    fn truncation_splits_on_a_char_boundary() {
        let multibyte = "日".repeat(MAX_DIAGNOSTIC_FIELD_BYTES);
        let bounded = diagnostic(&multibyte, &multibyte);

        assert!(bounded.path().ends_with(TRUNCATION_MARKER));
        assert!(
            bounded.path().len() <= MAX_DIAGNOSTIC_FIELD_BYTES + TRUNCATION_MARKER.len(),
            "walking back to a boundary must not push the value over the bound"
        );
        assert!(
            bounded
                .path()
                .trim_end_matches(TRUNCATION_MARKER)
                .chars()
                .all(|character| character == '日'),
            "truncation must not produce a partial codepoint"
        );
    }

    /// A code is a stable machine-readable contract, so it is rejected rather
    /// than cut: a truncated code would silently name a different diagnostic.
    #[test]
    fn an_overlong_code_is_rejected_rather_than_truncated() {
        let overlong_code = "E".repeat(MAX_DIAGNOSTIC_FIELD_BYTES + 1);
        assert!(
            ActivationDiagnostic::new(&overlong_code, "/workflow", "a", "b", "fix").is_none(),
            "a code that does not fit its own contract is a construction bug"
        );
    }

    #[test]
    fn contract_content_stays_out_of_debug_and_display() {
        let secret = "credential-value-that-must-not-leak";
        let rendered = diagnostic("/nodes/a", secret);
        assert!(!format!("{rendered}").contains(secret));
        assert!(!format!("{rendered:?}").contains(secret));
    }

    #[test]
    fn equivalent_input_produces_one_stable_order() {
        let later = diagnostic("/nodes/b", "second");
        let earlier = diagnostic("/nodes/a", "first");

        let canonical =
            canonical_diagnostics(vec![later.clone(), earlier.clone(), earlier.clone()]);
        assert_eq!(
            canonical,
            vec![earlier, later],
            "equivalent input must produce a stable, duplicate-free order"
        );
    }
}

/// Version of the NS14 diagnostic-contract report shape.
///
/// Bumped when the report gains or loses a field, so a consumer that stored an
/// older bundle can tell it is reading a different shape rather than silently
/// missing an entry.
pub const DIAGNOSTIC_CONTRACT_REPORT_VERSION: u16 = 1;

/// One rejection's compliance with the five-field contract.
///
/// Records the observed values rather than a bare boolean: a reviewer reading
/// the bundle can see *what* each rejection reports, not merely that it
/// reported something. That distinction is what makes the report evidence
/// instead of a checkbox.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiagnosticContractEntry {
    /// Rust path of the rejection variant this entry covers.
    pub rejection: String,
    /// Stable machine-readable code the rejection reports.
    pub code: String,
    /// Logical path the rejection points at.
    pub path: String,
    /// Whether the contract the rejection required is reported.
    pub reports_expected: bool,
    /// Whether what was found is reported.
    pub reports_actual: bool,
    /// Whether actionable guidance is reported.
    pub reports_remediation: bool,
}

impl DiagnosticContractEntry {
    /// Record what `diagnostic` reports for the rejection named `rejection`.
    #[must_use]
    pub fn observed(rejection: impl Into<String>, diagnostic: &ActivationDiagnostic) -> Self {
        Self {
            rejection: rejection.into(),
            code: diagnostic.code().to_owned(),
            path: diagnostic.path().to_owned(),
            reports_expected: !diagnostic.expected().trim().is_empty(),
            reports_actual: !diagnostic.actual().trim().is_empty(),
            reports_remediation: !diagnostic.remediation().trim().is_empty(),
        }
    }

    /// Whether this rejection satisfies the whole five-field contract.
    ///
    /// `code` and `path` are non-empty by construction — [`ActivationDiagnostic::new`]
    /// refuses a diagnostic without them — so completeness reduces to the three
    /// fields a producer could still leave blank.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.reports_expected && self.reports_actual && self.reports_remediation
    }
}

/// The versioned NS14 diagnostic-contract report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiagnosticContractReport {
    /// Shape version of this report.
    pub report_version: u16,
    /// Contract this report is evidence for.
    pub contract: String,
    /// One entry per rejection variant, ordered by `(rejection, code)`.
    pub entries: Vec<DiagnosticContractEntry>,
}

impl DiagnosticContractReport {
    /// Build a report over `entries`, ordered so two runs produce one document.
    #[must_use]
    pub fn new(mut entries: Vec<DiagnosticContractEntry>) -> Self {
        entries.sort_by(|left, right| {
            (&left.rejection, &left.code).cmp(&(&right.rejection, &right.code))
        });
        Self {
            report_version: DIAGNOSTIC_CONTRACT_REPORT_VERSION,
            contract: "ns14".to_owned(),
            entries,
        }
    }

    /// Rejections that do not satisfy the five-field contract.
    ///
    /// An empty slice is the only passing state; a caller that wants a gate
    /// asserts on this rather than on a count, so a failure names the
    /// rejections rather than reporting an arithmetic mismatch.
    #[must_use]
    pub fn incomplete(&self) -> Vec<&DiagnosticContractEntry> {
        self.entries
            .iter()
            .filter(|entry| !entry.is_complete())
            .collect()
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;

    fn diagnostic() -> ActivationDiagnostic {
        ActivationDiagnostic::new("E001", "/nodes/a", "a contract", "something else", "fix it")
            .expect("the fixture uses non-empty diagnostic fields")
    }

    #[test]
    fn a_complete_rejection_leaves_the_report_empty_of_gaps() {
        let report = DiagnosticContractReport::new(vec![DiagnosticContractEntry::observed(
            "WorkflowError::EmptyName",
            &diagnostic(),
        )]);

        assert_eq!(report.report_version, DIAGNOSTIC_CONTRACT_REPORT_VERSION);
        assert_eq!(report.contract, "ns14");
        assert!(report.incomplete().is_empty());
    }

    /// The gate has to be able to fail, so a blank field must surface as a gap
    /// rather than be smoothed over by the constructor.
    #[test]
    fn a_blank_field_surfaces_as_an_incomplete_entry() {
        let mut entry = DiagnosticContractEntry::observed("WorkflowError::NoNodes", &diagnostic());
        entry.reports_remediation = false;

        let report = DiagnosticContractReport::new(vec![entry]);
        let gaps = report.incomplete();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].rejection, "WorkflowError::NoNodes");
    }

    #[test]
    fn entries_are_ordered_so_two_runs_produce_one_document() {
        let later = DiagnosticContractEntry::observed("B", &diagnostic());
        let earlier = DiagnosticContractEntry::observed("A", &diagnostic());

        let report = DiagnosticContractReport::new(vec![later, earlier]);
        assert_eq!(report.entries[0].rejection, "A");
        assert_eq!(report.entries[1].rejection, "B");
    }
}
