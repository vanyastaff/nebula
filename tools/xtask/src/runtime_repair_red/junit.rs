use std::collections::{BTreeMap, BTreeSet};

use quick_xml::{
    Reader, XmlVersion,
    encoding::Decoder,
    events::{BytesCData, BytesRef, BytesStart, BytesText, Event},
};

use super::{ExpectedFailure, VerificationError};

const MAX_SUITES: usize = 64;
const MAX_TEST_CASES: usize = 512;
const MAX_XML_DEPTH: usize = 16;
const MAX_CASE_OUTPUT_BYTES: usize = 1024 * 1024;

pub(super) fn verify(
    source: &str,
    expected_classname: &str,
    expected_failures: &[ExpectedFailure],
) -> Result<usize, VerificationError> {
    let report = JunitParser::parse(source)?;
    let expected_count = expected_failures.len();
    if report.root_counts.tests != expected_count
        || report.root_counts.failures != expected_count
        || report.root_counts.errors != 0
        || report.root_counts.disabled != 0
    {
        return invalid_junit(format!(
            "root aggregate must report tests={expected_count}, failures={expected_count}, errors=0, disabled=0"
        ));
    }
    if report.suite_names.as_slice() != [expected_classname] {
        return invalid_junit(format!(
            "report must contain exactly one suite named `{expected_classname}`"
        ));
    }
    if report.cases.len() != expected_count {
        return invalid_junit(format!(
            "report contains {} testcases; expected {expected_count}",
            report.cases.len()
        ));
    }

    let expected_by_name = expected_failures
        .iter()
        .map(|expected| (expected.test_name.as_str(), expected.reason_code.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut observed_names = BTreeSet::new();
    for case in &report.cases {
        if case.classname != expected_classname {
            return invalid_junit(format!(
                "test `{}` has classname `{}`; expected `{expected_classname}`",
                case.name, case.classname
            ));
        }
        if !observed_names.insert(case.name.as_str()) {
            return invalid_junit(format!("test identity `{}` is duplicated", case.name));
        }
        let expected_reason = expected_by_name.get(case.name.as_str()).ok_or_else(|| {
            VerificationError::InvalidJunit {
                detail: format!("unexpected test identity `{}`", case.name),
            }
        })?;
        if case.failure_elements != 1 {
            return invalid_junit(format!(
                "test `{}` must contain exactly one failure element",
                case.name
            ));
        }
        if case.error_elements != 0 || case.skipped_elements != 0 {
            return invalid_junit(format!(
                "test `{}` is skipped or errored instead of an expected failure",
                case.name
            ));
        }
        if case.timed_out {
            return invalid_junit(format!("test `{}` was terminated by a timeout", case.name));
        }
        validate_marker(case, expected_reason)?;
    }

    let missing = expected_by_name
        .keys()
        .copied()
        .filter(|test_name| !observed_names.contains(test_name))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return invalid_junit(format!(
            "expected test identities are missing: {}",
            missing.join(", ")
        ));
    }
    Ok(report.cases.len())
}

fn validate_marker(case: &ParsedCase, expected_reason: &str) -> Result<(), VerificationError> {
    let expected_marker = format!("EXPECTED_RED:{expected_reason}");
    let markers = case
        .captured_stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("EXPECTED_RED:"))
        .collect::<Vec<_>>();
    if markers.as_slice() == [expected_marker.as_str()] {
        Ok(())
    } else {
        invalid_junit(format!(
            "test `{}` must emit exactly `{expected_marker}` on its own line",
            case.name
        ))
    }
}

struct JunitParser {
    stack: Vec<Element>,
    root_counts: Option<Counts>,
    suite: Option<SuiteState>,
    suite_names: Vec<String>,
    current_case: Option<CaseState>,
    cases: Vec<ParsedCase>,
}

impl JunitParser {
    fn parse(source: &str) -> Result<ParsedReport, VerificationError> {
        let mut reader = Reader::from_str(source);
        reader.config_mut().enable_all_checks(true);
        let mut parser = Self {
            stack: Vec::new(),
            root_counts: None,
            suite: None,
            suite_names: Vec::new(),
            current_case: None,
            cases: Vec::new(),
        };

        loop {
            match reader.read_event() {
                Ok(Event::Start(start)) => {
                    parser.start_element(&start, reader.decoder())?;
                },
                Ok(Event::Empty(start)) => {
                    let element = parser.start_element(&start, reader.decoder())?;
                    parser.end_element(element)?;
                },
                Ok(Event::End(end)) => {
                    let element = Element::from_name(end.name().as_ref())?;
                    parser.end_element(element)?;
                },
                Ok(Event::Text(text)) => parser.capture_text(&text)?,
                Ok(Event::CData(cdata)) => parser.capture_cdata(&cdata)?,
                Ok(Event::GeneralRef(reference)) => parser.capture_reference(&reference)?,
                Ok(Event::Decl(_)) if parser.stack.is_empty() && parser.root_counts.is_none() => {},
                Ok(Event::Comment(_)) => {},
                Ok(Event::Eof) => break,
                Ok(Event::Decl(_) | Event::PI(_) | Event::DocType(_)) => {
                    return invalid_junit(
                        "unexpected XML declaration, processing instruction, or doctype".to_owned(),
                    );
                },
                Err(source) => {
                    return invalid_junit(format!("XML parse failed: {source}"));
                },
            }
        }

        parser.finish()
    }

    fn start_element(
        &mut self,
        start: &BytesStart<'_>,
        decoder: Decoder,
    ) -> Result<Element, VerificationError> {
        if self.stack.len() >= MAX_XML_DEPTH {
            return invalid_junit(format!("XML nesting exceeds {MAX_XML_DEPTH} elements"));
        }
        let element = Element::from_name(start.name().as_ref())?;
        let parent = self.stack.last().copied();
        if !element.allows_parent(parent) {
            return invalid_junit(format!(
                "element `{}` is not valid beneath {}",
                element.name(),
                parent.map_or("the document root", Element::name)
            ));
        }
        let attributes = decode_attributes(start, decoder)?;
        element.validate_attributes(&attributes)?;
        if element.captures_marker_output() {
            let case = self.case_mut(element)?;
            if !case.captured_stderr.is_empty() && !case.captured_stderr.ends_with('\n') {
                case.captured_stderr.push('\n');
            }
        }

        match element {
            Element::Testsuites => {
                if self.root_counts.is_some() {
                    return invalid_junit("report contains multiple root elements".to_owned());
                }
                self.root_counts = Some(Counts::parse(&attributes, false)?);
            },
            Element::Testsuite => {
                if self.suite.is_some() {
                    return invalid_junit("test suites cannot be nested".to_owned());
                }
                if self.suite_names.len() >= MAX_SUITES {
                    return invalid_junit(format!("report contains more than {MAX_SUITES} suites"));
                }
                let name = required_attribute(&attributes, "name", element)?;
                self.suite_names.push(name.to_owned());
                self.suite = Some(SuiteState {
                    name: name.to_owned(),
                    counts: Counts::parse(&attributes, true)?,
                    case_start: self.cases.len(),
                });
            },
            Element::Testcase => {
                if self.current_case.is_some() {
                    return invalid_junit("testcases cannot be nested".to_owned());
                }
                if self.cases.len() >= MAX_TEST_CASES {
                    return invalid_junit(format!(
                        "report contains more than {MAX_TEST_CASES} testcases"
                    ));
                }
                let suite_name = self
                    .suite
                    .as_ref()
                    .ok_or_else(|| VerificationError::InvalidJunit {
                        detail: "testcase is outside a testsuite".to_owned(),
                    })?
                    .name
                    .as_str();
                let name = required_attribute(&attributes, "name", element)?;
                let classname = required_attribute(&attributes, "classname", element)?;
                if classname != suite_name {
                    return invalid_junit(format!(
                        "testcase `{name}` classname `{classname}` does not match suite `{suite_name}`"
                    ));
                }
                self.current_case = Some(CaseState {
                    name: name.to_owned(),
                    classname: classname.to_owned(),
                    failure_elements: 0,
                    error_elements: 0,
                    skipped_elements: 0,
                    timed_out: false,
                    captured_text_bytes: 0,
                    captured_stderr: String::new(),
                });
            },
            Element::Failure => {
                let case = self.case_mut(element)?;
                case.failure_elements += 1;
                if attributes_indicate_timeout(&attributes) {
                    case.timed_out = true;
                }
            },
            Element::Error => {
                let case = self.case_mut(element)?;
                case.error_elements += 1;
                if attributes_indicate_timeout(&attributes) {
                    case.timed_out = true;
                }
            },
            Element::Skipped => self.case_mut(element)?.skipped_elements += 1,
            Element::SystemOut
            | Element::SystemErr
            | Element::Description
            | Element::Properties
            | Element::Property => {},
        }

        self.stack.push(element);
        Ok(element)
    }

    fn end_element(&mut self, element: Element) -> Result<(), VerificationError> {
        if self.stack.last() != Some(&element) {
            return invalid_junit(format!(
                "closing element `{}` does not match parser state",
                element.name()
            ));
        }
        match element {
            Element::Testcase => {
                let case =
                    self.current_case
                        .take()
                        .ok_or_else(|| VerificationError::InvalidJunit {
                            detail: "testcase closed without an active testcase".to_owned(),
                        })?;
                self.cases.push(case.finish());
            },
            Element::Testsuite => self.finish_suite()?,
            _ => {},
        }
        self.stack.pop();
        Ok(())
    }

    fn capture_text(&mut self, text: &BytesText<'_>) -> Result<(), VerificationError> {
        let decoded = text
            .xml_content(XmlVersion::Implicit1_0)
            .map_err(|source| VerificationError::InvalidJunit {
                detail: format!("cannot decode XML text: {source}"),
            })?;
        self.capture_decoded_text(&decoded)
    }

    fn capture_cdata(&mut self, cdata: &BytesCData<'_>) -> Result<(), VerificationError> {
        let decoded = cdata
            .xml_content(XmlVersion::Implicit1_0)
            .map_err(|source| VerificationError::InvalidJunit {
                detail: format!("cannot decode XML CDATA: {source}"),
            })?;
        self.capture_decoded_text(&decoded)
    }

    fn capture_reference(&mut self, reference: &BytesRef<'_>) -> Result<(), VerificationError> {
        let resolved = if let Some(character) =
            reference
                .resolve_char_ref()
                .map_err(|source| VerificationError::InvalidJunit {
                    detail: format!("cannot resolve XML character reference: {source}"),
                })? {
            character.to_string()
        } else {
            let name = reference
                .decode()
                .map_err(|source| VerificationError::InvalidJunit {
                    detail: format!("cannot decode XML entity reference: {source}"),
                })?;
            match name.as_ref() {
                "amp" => "&",
                "apos" => "'",
                "gt" => ">",
                "lt" => "<",
                "quot" => "\"",
                unknown => {
                    return invalid_junit(format!("unknown XML entity reference `&{unknown};`"));
                },
            }
            .to_owned()
        };
        self.capture_decoded_text(&resolved)
    }

    fn capture_decoded_text(&mut self, text: &str) -> Result<(), VerificationError> {
        let captures_marker_output = self
            .stack
            .iter()
            .any(|element| element.captures_marker_output());
        let accepts_case_text = self.stack.iter().any(|element| element.accepts_case_text());
        if accepts_case_text {
            let case =
                self.current_case
                    .as_mut()
                    .ok_or_else(|| VerificationError::InvalidJunit {
                        detail: "captured output is outside a testcase".to_owned(),
                    })?;
            let new_length = case
                .captured_text_bytes
                .checked_add(text.len())
                .ok_or_else(|| VerificationError::InvalidJunit {
                    detail: "testcase output length overflowed".to_owned(),
                })?;
            if new_length > MAX_CASE_OUTPUT_BYTES {
                return invalid_junit(format!(
                    "testcase output exceeds {MAX_CASE_OUTPUT_BYTES} bytes"
                ));
            }
            case.captured_text_bytes = new_length;
            if captures_marker_output {
                case.captured_stderr.push_str(text);
            }
        } else if !text.trim().is_empty() {
            return invalid_junit("unexpected character data outside testcase output".to_owned());
        }
        Ok(())
    }

    fn case_mut(&mut self, element: Element) -> Result<&mut CaseState, VerificationError> {
        self.current_case
            .as_mut()
            .ok_or_else(|| VerificationError::InvalidJunit {
                detail: format!("element `{}` is outside a testcase", element.name()),
            })
    }

    fn finish_suite(&mut self) -> Result<(), VerificationError> {
        let suite = self
            .suite
            .take()
            .ok_or_else(|| VerificationError::InvalidJunit {
                detail: "testsuite closed without active suite state".to_owned(),
            })?;
        let suite_cases = &self.cases[suite.case_start..];
        let actual = Counts::from_cases(suite_cases);
        if actual != suite.counts {
            return invalid_junit(format!(
                "suite `{}` aggregate {:?} does not match testcase counts {:?}",
                suite.name, suite.counts, actual
            ));
        }
        Ok(())
    }

    fn finish(self) -> Result<ParsedReport, VerificationError> {
        if !self.stack.is_empty() || self.suite.is_some() || self.current_case.is_some() {
            return invalid_junit("JUnit document ended with unclosed elements".to_owned());
        }
        let root_counts = self
            .root_counts
            .ok_or_else(|| VerificationError::InvalidJunit {
                detail: "JUnit document is missing a testsuites root".to_owned(),
            })?;
        let actual = Counts::from_cases(&self.cases);
        if actual != root_counts {
            return invalid_junit(format!(
                "root aggregate {root_counts:?} does not match testcase counts {actual:?}"
            ));
        }
        Ok(ParsedReport {
            root_counts,
            suite_names: self.suite_names,
            cases: self.cases,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Element {
    Testsuites,
    Testsuite,
    Testcase,
    Failure,
    Error,
    Skipped,
    SystemOut,
    SystemErr,
    Description,
    Properties,
    Property,
}

impl Element {
    fn from_name(name: &[u8]) -> Result<Self, VerificationError> {
        match name {
            b"testsuites" => Ok(Self::Testsuites),
            b"testsuite" => Ok(Self::Testsuite),
            b"testcase" => Ok(Self::Testcase),
            b"failure" => Ok(Self::Failure),
            b"error" => Ok(Self::Error),
            b"skipped" => Ok(Self::Skipped),
            b"system-out" => Ok(Self::SystemOut),
            b"system-err" => Ok(Self::SystemErr),
            b"description" => Ok(Self::Description),
            b"properties" => Ok(Self::Properties),
            b"property" => Ok(Self::Property),
            b"retry" | b"rerunFailure" | b"rerunError" | b"flakyFailure" | b"flakyError" => {
                invalid_junit(format!(
                    "retry, rerun, and flaky element `{}` is forbidden",
                    String::from_utf8_lossy(name)
                ))
            },
            _ => invalid_junit(format!(
                "unsupported JUnit element `{}`",
                String::from_utf8_lossy(name)
            )),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Testsuites => "testsuites",
            Self::Testsuite => "testsuite",
            Self::Testcase => "testcase",
            Self::Failure => "failure",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::SystemOut => "system-out",
            Self::SystemErr => "system-err",
            Self::Description => "description",
            Self::Properties => "properties",
            Self::Property => "property",
        }
    }

    const fn allows_parent(self, parent: Option<Self>) -> bool {
        match self {
            Self::Testsuites => parent.is_none(),
            Self::Testsuite => matches!(parent, Some(Self::Testsuites)),
            Self::Testcase => matches!(parent, Some(Self::Testsuite)),
            Self::Failure | Self::Error | Self::Skipped => {
                matches!(parent, Some(Self::Testcase))
            },
            Self::SystemOut | Self::SystemErr | Self::Description => {
                matches!(parent, Some(Self::Testcase | Self::Failure | Self::Error))
            },
            Self::Properties => matches!(parent, Some(Self::Testsuite | Self::Testcase)),
            Self::Property => matches!(parent, Some(Self::Properties)),
        }
    }

    const fn captures_marker_output(self) -> bool {
        matches!(self, Self::SystemErr)
    }

    const fn accepts_case_text(self) -> bool {
        matches!(
            self,
            Self::Failure | Self::Error | Self::SystemOut | Self::SystemErr | Self::Description
        )
    }

    fn validate_attributes(
        self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<(), VerificationError> {
        let allowed: &[&str] = match self {
            Self::Testsuites => &[
                "name",
                "tests",
                "failures",
                "errors",
                "disabled",
                "skipped",
                "uuid",
                "timestamp",
                "time",
            ],
            Self::Testsuite => &[
                "name",
                "tests",
                "failures",
                "errors",
                "disabled",
                "skipped",
                "timestamp",
                "time",
                "hostname",
            ],
            Self::Testcase => &["name", "classname", "timestamp", "time"],
            Self::Failure | Self::Error => &["type", "message", "timestamp", "time"],
            Self::Skipped => &["message"],
            Self::Property => &["name", "value"],
            Self::SystemOut | Self::SystemErr | Self::Description | Self::Properties => &[],
        };
        if let Some(unknown) = attributes
            .keys()
            .find(|name| !allowed.contains(&name.as_str()))
        {
            return invalid_junit(format!(
                "element `{}` has unsupported attribute `{unknown}`",
                self.name()
            ));
        }
        Ok(())
    }
}

fn decode_attributes(
    start: &BytesStart<'_>,
    decoder: Decoder,
) -> Result<BTreeMap<String, String>, VerificationError> {
    let mut decoded = BTreeMap::new();
    for attribute in start.attributes().with_checks(true) {
        let attribute = attribute.map_err(|source| VerificationError::InvalidJunit {
            detail: format!("invalid XML attribute: {source}"),
        })?;
        let name = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|source| VerificationError::InvalidJunit {
                detail: format!("XML attribute name is not UTF-8: {source}"),
            })?
            .to_owned();
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
            .map_err(|source| VerificationError::InvalidJunit {
                detail: format!("cannot decode XML attribute `{name}`: {source}"),
            })?
            .into_owned();
        if decoded.insert(name.clone(), value).is_some() {
            return invalid_junit(format!("XML attribute `{name}` is duplicated"));
        }
    }
    Ok(decoded)
}

fn required_attribute<'a>(
    attributes: &'a BTreeMap<String, String>,
    name: &str,
    element: Element,
) -> Result<&'a str, VerificationError> {
    attributes
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| VerificationError::InvalidJunit {
            detail: format!("element `{}` requires attribute `{name}`", element.name()),
        })
}

fn attributes_indicate_timeout(attributes: &BTreeMap<String, String>) -> bool {
    attributes
        .iter()
        .filter(|(name, _)| matches!(name.as_str(), "type" | "message"))
        .any(|(_, value)| {
            let normalized = value.to_ascii_lowercase();
            normalized.contains("timeout") || normalized.contains("timed out")
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counts {
    tests: usize,
    failures: usize,
    errors: usize,
    disabled: usize,
}

impl Counts {
    fn parse(
        attributes: &BTreeMap<String, String>,
        require_disabled: bool,
    ) -> Result<Self, VerificationError> {
        let tests = parse_count(attributes, "tests")?;
        let failures = parse_count(attributes, "failures")?;
        let errors = parse_count(attributes, "errors")?;
        let disabled = match (
            attributes.get("disabled"),
            attributes.get("skipped"),
            require_disabled,
        ) {
            (Some(disabled), None, _) => parse_count_value("disabled", disabled)?,
            (None, Some(skipped), _) => parse_count_value("skipped", skipped)?,
            (None, None, false) => 0,
            (None, None, true) => {
                return invalid_junit("testsuite requires a disabled count".to_owned());
            },
            (Some(_), Some(_), _) => {
                return invalid_junit(
                    "aggregate cannot define both disabled and skipped counts".to_owned(),
                );
            },
        };
        Ok(Self {
            tests,
            failures,
            errors,
            disabled,
        })
    }

    fn from_cases(cases: &[ParsedCase]) -> Self {
        Self {
            tests: cases.len(),
            failures: cases
                .iter()
                .filter(|case| case.failure_elements > 0)
                .count(),
            errors: cases.iter().filter(|case| case.error_elements > 0).count(),
            disabled: cases
                .iter()
                .filter(|case| case.skipped_elements > 0)
                .count(),
        }
    }
}

fn parse_count(
    attributes: &BTreeMap<String, String>,
    name: &str,
) -> Result<usize, VerificationError> {
    let value = attributes
        .get(name)
        .ok_or_else(|| VerificationError::InvalidJunit {
            detail: format!("aggregate requires `{name}` count"),
        })?;
    parse_count_value(name, value)
}

fn parse_count_value(name: &str, value: &str) -> Result<usize, VerificationError> {
    value
        .parse::<usize>()
        .map_err(|_| VerificationError::InvalidJunit {
            detail: format!("aggregate `{name}` count `{value}` is not an unsigned integer"),
        })
}

struct SuiteState {
    name: String,
    counts: Counts,
    case_start: usize,
}

struct CaseState {
    name: String,
    classname: String,
    failure_elements: usize,
    error_elements: usize,
    skipped_elements: usize,
    timed_out: bool,
    captured_text_bytes: usize,
    captured_stderr: String,
}

impl CaseState {
    fn finish(self) -> ParsedCase {
        ParsedCase {
            name: self.name,
            classname: self.classname,
            failure_elements: self.failure_elements,
            error_elements: self.error_elements,
            skipped_elements: self.skipped_elements,
            timed_out: self.timed_out,
            captured_stderr: self.captured_stderr,
        }
    }
}

struct ParsedReport {
    root_counts: Counts,
    suite_names: Vec<String>,
    cases: Vec<ParsedCase>,
}

struct ParsedCase {
    name: String,
    classname: String,
    failure_elements: usize,
    error_elements: usize,
    skipped_elements: usize,
    timed_out: bool,
    captured_stderr: String,
}

fn invalid_junit<T>(detail: String) -> Result<T, VerificationError> {
    Err(VerificationError::InvalidJunit { detail })
}
