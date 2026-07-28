// budget-justified: expected-failure policy, raw nextest status, and bounded JUnit reconciliation form one fail-closed evidence boundary.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod junit;

const MANIFEST_PATH: &str = "tools/xtask/gates/runtime-repair-red-v1.toml";
const MANIFEST_VERSION: u16 = 1;
const NEXTEST_TEST_RUN_FAILED: u8 = 100;
const EXPECTED_PROFILE: &str = "runtime-repair-red";
const EXPECTED_PACKAGE: &str = "nebula-server";
const EXPECTED_FEATURE: &str = "runtime-repair-red";
const EXPECTED_TEST_BINARY: &str = "runtime_repair_red_scenarios";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_JUNIT_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXPECTED_FAILURES: usize = 128;

#[derive(Debug, Serialize)]
pub(crate) struct ManifestSummary {
    manifest_version: u16,
    profile: &'static str,
    expected_failure_count: usize,
    status: &'static str,
}

impl ManifestSummary {
    pub(crate) fn to_json_line(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut output = serde_json::to_vec(self)?;
        output.push(b'\n');
        Ok(output)
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct VerificationSummary {
    manifest_version: u16,
    profile: &'static str,
    expected_failure_count: usize,
    verified_failure_count: usize,
    status: &'static str,
}

impl VerificationSummary {
    pub(crate) fn to_json_line(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut output = serde_json::to_vec(self)?;
        output.push(b'\n');
        Ok(output)
    }
}

pub(crate) fn validate_manifest(
    workspace_root: &Path,
) -> Result<ManifestSummary, VerificationError> {
    let manifest = load_manifest(workspace_root)?;
    Ok(ManifestSummary {
        manifest_version: manifest.manifest_version,
        profile: EXPECTED_PROFILE,
        expected_failure_count: manifest.expected_failures.len(),
        status: "valid",
    })
}

pub(crate) fn verify(
    workspace_root: &Path,
    nextest_exit_code: u8,
    junit_path: &Path,
) -> Result<VerificationSummary, VerificationError> {
    let manifest = load_manifest(workspace_root)?;
    verify_file(&manifest, nextest_exit_code, junit_path)
}

fn verify_file(
    manifest: &ValidatedManifest,
    nextest_exit_code: u8,
    junit_path: &Path,
) -> Result<VerificationSummary, VerificationError> {
    validate_verification_preconditions(manifest, nextest_exit_code)?;
    let junit_bytes = fs::read(junit_path).map_err(|source| VerificationError::JunitRead {
        path: junit_path.to_path_buf(),
        source,
    })?;
    if junit_bytes.len() > MAX_JUNIT_BYTES {
        return Err(VerificationError::JunitTooLarge {
            size: junit_bytes.len(),
            maximum: MAX_JUNIT_BYTES,
        });
    }
    let junit_source = std::str::from_utf8(&junit_bytes).map_err(VerificationError::JunitUtf8)?;
    verify_junit_source(manifest, nextest_exit_code, junit_source)
}

#[cfg(test)]
fn verify_documents(
    manifest_source: &str,
    nextest_exit_code: u8,
    junit_source: &str,
) -> Result<VerificationSummary, VerificationError> {
    let manifest = validate_manifest_source(manifest_source)?;
    verify_junit_source(&manifest, nextest_exit_code, junit_source)
}

fn verify_junit_source(
    manifest: &ValidatedManifest,
    nextest_exit_code: u8,
    junit_source: &str,
) -> Result<VerificationSummary, VerificationError> {
    validate_verification_preconditions(manifest, nextest_exit_code)?;
    if junit_source.len() > MAX_JUNIT_BYTES {
        return Err(VerificationError::JunitTooLarge {
            size: junit_source.len(),
            maximum: MAX_JUNIT_BYTES,
        });
    }
    let verified_failure_count = junit::verify(
        junit_source,
        &manifest.expected_classname,
        &manifest.expected_failures,
    )?;
    Ok(VerificationSummary {
        manifest_version: manifest.manifest_version,
        profile: EXPECTED_PROFILE,
        expected_failure_count: manifest.expected_failures.len(),
        verified_failure_count,
        status: "expected-red-verified",
    })
}

fn validate_verification_preconditions(
    manifest: &ValidatedManifest,
    nextest_exit_code: u8,
) -> Result<(), VerificationError> {
    if nextest_exit_code != NEXTEST_TEST_RUN_FAILED {
        return Err(VerificationError::UnexpectedNextestExit {
            actual: nextest_exit_code,
        });
    }
    if manifest.expected_failures.is_empty() {
        return Err(VerificationError::EmptyExpectedFailureSet);
    }
    Ok(())
}

fn load_manifest(workspace_root: &Path) -> Result<ValidatedManifest, VerificationError> {
    let manifest_path = workspace_root.join(MANIFEST_PATH);
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|source| VerificationError::ManifestRead {
            path: manifest_path,
            source,
        })?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(VerificationError::ManifestTooLarge {
            size: manifest_bytes.len(),
            maximum: MAX_MANIFEST_BYTES,
        });
    }
    let manifest_source =
        std::str::from_utf8(&manifest_bytes).map_err(VerificationError::ManifestUtf8)?;
    validate_manifest_source(manifest_source)
}

fn validate_manifest_source(source: &str) -> Result<ValidatedManifest, VerificationError> {
    let manifest: ExpectedFailureManifest =
        toml::from_str(source).map_err(VerificationError::ManifestParse)?;
    validate_fixed_field(
        "manifest_version",
        manifest.manifest_version,
        MANIFEST_VERSION,
    )?;
    validate_fixed_text("profile", &manifest.profile, EXPECTED_PROFILE)?;
    validate_fixed_text("package", &manifest.package, EXPECTED_PACKAGE)?;
    validate_fixed_text("feature", &manifest.feature, EXPECTED_FEATURE)?;
    validate_fixed_text("test_binary", &manifest.test_binary, EXPECTED_TEST_BINARY)?;
    if manifest.expected_failures.len() > MAX_EXPECTED_FAILURES {
        return invalid_manifest(format!(
            "expected_failures contains {} entries; maximum is {MAX_EXPECTED_FAILURES}",
            manifest.expected_failures.len()
        ));
    }

    let mut test_names = BTreeSet::new();
    let mut previous_test_name: Option<&str> = None;
    for expected_failure in &manifest.expected_failures {
        validate_test_name(&expected_failure.test_name)?;
        validate_reason_code(&expected_failure.reason_code)?;
        if !test_names.insert(expected_failure.test_name.as_str()) {
            return invalid_manifest(format!(
                "test identity `{}` is duplicated",
                expected_failure.test_name
            ));
        }
        if previous_test_name
            .is_some_and(|previous| previous >= expected_failure.test_name.as_str())
        {
            return invalid_manifest(
                "expected_failures must be strictly sorted by test_name".to_owned(),
            );
        }
        previous_test_name = Some(&expected_failure.test_name);
    }

    Ok(ValidatedManifest {
        manifest_version: manifest.manifest_version,
        expected_classname: format!("{EXPECTED_PACKAGE}::{EXPECTED_TEST_BINARY}"),
        expected_failures: manifest.expected_failures,
    })
}

fn validate_fixed_field(field: &str, actual: u16, expected: u16) -> Result<(), VerificationError> {
    if actual == expected {
        Ok(())
    } else {
        invalid_manifest(format!("{field} must be {expected}, found {actual}"))
    }
}

fn validate_fixed_text(field: &str, actual: &str, expected: &str) -> Result<(), VerificationError> {
    if actual == expected {
        Ok(())
    } else {
        invalid_manifest(format!("{field} must be `{expected}`, found `{actual}`"))
    }
}

fn validate_test_name(test_name: &str) -> Result<(), VerificationError> {
    let is_valid = !test_name.is_empty()
        && test_name.len() <= 256
        && test_name.split("::").all(|segment| {
            let mut bytes = segment.bytes();
            bytes.next().is_some_and(|first| {
                (first.is_ascii_lowercase() || first == b'_')
                    && bytes.all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
        });
    if is_valid {
        Ok(())
    } else {
        invalid_manifest(format!(
            "test_name `{test_name}` must be a bounded canonical lowercase Rust test path"
        ))
    }
}

fn validate_reason_code(reason_code: &str) -> Result<(), VerificationError> {
    let is_valid = !reason_code.is_empty()
        && reason_code.len() <= 96
        && !reason_code.starts_with('-')
        && !reason_code.ends_with('-')
        && !reason_code.contains("--")
        && reason_code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if is_valid {
        Ok(())
    } else {
        invalid_manifest(format!(
            "reason_code `{reason_code}` must be bounded lowercase kebab-case"
        ))
    }
}

fn invalid_manifest<T>(detail: String) -> Result<T, VerificationError> {
    Err(VerificationError::InvalidManifest { detail })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFailureManifest {
    manifest_version: u16,
    profile: String,
    package: String,
    feature: String,
    test_binary: String,
    expected_failures: Vec<ExpectedFailure>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ExpectedFailure {
    pub(super) test_name: String,
    pub(super) reason_code: String,
}

#[derive(Debug)]
struct ValidatedManifest {
    manifest_version: u16,
    expected_classname: String,
    expected_failures: Vec<ExpectedFailure>,
}

#[derive(Debug, Error)]
pub(crate) enum VerificationError {
    #[error("cannot read runtime-repair expected-case manifest `{path}`: {source}")]
    ManifestRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("runtime-repair expected-case manifest is {size} bytes; maximum is {maximum}")]
    ManifestTooLarge { size: usize, maximum: usize },
    #[error("runtime-repair expected-case manifest is not UTF-8: {0}")]
    ManifestUtf8(#[source] std::str::Utf8Error),
    #[error("runtime-repair expected-case manifest TOML is invalid: {0}")]
    ManifestParse(#[source] toml::de::Error),
    #[error("runtime-repair expected-case manifest is invalid: {detail}")]
    InvalidManifest { detail: String },
    #[error(
        "cargo-nextest exit code must be 100 (TEST_RUN_FAILED), found {actual}; build, setup, metadata, configuration, and successful exits are rejected"
    )]
    UnexpectedNextestExit { actual: u8 },
    #[error("runtime-repair JUnit verification requires at least one active expected failure")]
    EmptyExpectedFailureSet,
    #[error("cannot read runtime-repair JUnit report `{path}`: {source}")]
    JunitRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("runtime-repair JUnit report is {size} bytes; maximum is {maximum}")]
    JunitTooLarge { size: usize, maximum: usize },
    #[error("runtime-repair JUnit report is not UTF-8: {0}")]
    JunitUtf8(#[source] std::str::Utf8Error),
    #[error("runtime-repair JUnit report is invalid: {detail}")]
    InvalidJunit { detail: String },
}

#[cfg(test)]
mod tests;
