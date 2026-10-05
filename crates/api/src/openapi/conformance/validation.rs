//! Response validation and denominator accounting for actual served operations.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

/// One served operation, keyed exactly as the OpenAPI document names it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    /// Lower-case HTTP method.
    pub method: String,
    /// Path template, including the `/api/v1` prefix.
    pub path: String,
    /// The document's `operationId`.
    pub operation_id: String,
}

/// One drift finding: an operation, a fixed kind code and a schema location.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// The operation the finding belongs to.
    pub operation: Operation,
    /// The produced (or, for coverage findings, the unreached) status.
    pub status: u16,
    /// Fixed finding code, e.g. `undocumented-status`.
    pub kind: String,
    /// A declared schema location; never a raw response or request value.
    pub schema_path: String,
}

/// One observed live response and the findings it produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    /// The matched operation.
    pub operation: Operation,
    /// The produced status code.
    pub status: u16,
    /// False when the fixture did not consume the entire response stream.
    pub body_complete: bool,
    /// Findings for this response; empty when it conforms.
    pub findings: Vec<Finding>,
}

/// A checked-in waiver.
///
/// With `operation_id` and no `status` the whole operation leaves the coverage
/// denominator; with `status` only that documented success status does. Either
/// way the operation's observed responses are still validated. Without
/// `operation_id` the entry waives a served route that is deliberately absent
/// from the document (an internal or transport route), matched by method and
/// path template.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exclusion {
    method: String,
    path: String,
    #[serde(default)]
    operation_id: Option<String>,
    #[serde(default)]
    status: Option<u16>,
    reason: String,
}

/// The media type without parameters (`application/json; charset=utf-8` →
/// `application/json`); an absent header is the empty string.
fn essence(content_type: Option<&str>) -> &str {
    content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
}

fn response_pointer(operation: &Operation, status: Option<u16>) -> String {
    let base = format!(
        "/paths/{}/{}",
        operation.path.replace('~', "~0").replace('/', "~1"),
        operation.method
    );
    match status {
        Some(status) => format!("{base}/responses/{status}"),
        None => base,
    }
}

/// Every served operation with its documented success (2xx/3xx) statuses.
///
/// # Errors
///
/// [`super::ConformanceError::InvalidSpec`] when the document has no
/// operations or an operation lacks an `operationId` or `responses`.
pub fn inventory(
    spec: &Value,
) -> Result<BTreeMap<Operation, BTreeSet<u16>>, super::ConformanceError> {
    let mut result = BTreeMap::new();
    for (path, item) in spec["paths"]
        .as_object()
        .ok_or(super::ConformanceError::InvalidSpec)?
    {
        for method in METHODS {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let operation_id = operation["operationId"]
                .as_str()
                .ok_or(super::ConformanceError::InvalidSpec)?;
            let successful = operation["responses"]
                .as_object()
                .ok_or(super::ConformanceError::InvalidSpec)?
                .keys()
                .filter_map(|status| status.parse::<u16>().ok())
                .filter(|status| (200..400).contains(status))
                .collect();
            result.insert(
                Operation {
                    method: (*method).to_owned(),
                    path: path.clone(),
                    operation_id: operation_id.to_owned(),
                },
                successful,
            );
        }
    }
    if result.is_empty() {
        return Err(super::ConformanceError::InvalidSpec);
    }
    Ok(result)
}

/// Validate one fully consumed live response against the served document:
/// documented status, documented media type, JSON Schema 2020-12 conformance
/// and decoding into the shared `nebula-api-contract` wire type.
pub fn validate_response(
    spec: &Value,
    operation: &Operation,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> Observation {
    validate_response_with_length(spec, operation, status, content_type, body, body.len())
}

pub(super) fn validate_response_with_length(
    spec: &Value,
    operation: &Operation,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
    body_length: usize,
) -> Observation {
    let mut observation = Observation {
        operation: operation.clone(),
        status,
        body_complete: true,
        findings: Vec::new(),
    };
    let response_path = response_pointer(operation, Some(status));
    let mut finding = |kind: &str| {
        observation.findings.push(Finding {
            operation: operation.clone(),
            status,
            kind: kind.to_owned(),
            schema_path: response_path.clone(),
        });
    };
    let Some(response) = spec.pointer(&response_path) else {
        finding("undocumented-status");
        return observation;
    };
    let Some(content) = response.get("content").and_then(Value::as_object) else {
        if body_length != 0 {
            finding("unexpected-response-body");
        }
        return observation;
    };
    let Some(media_schema) = content.get(essence(content_type)) else {
        finding("undocumented-content-type");
        return observation;
    };
    let Some(schema) = media_schema.get("schema") else {
        finding("missing-response-schema");
        return observation;
    };
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        finding("invalid-response-json");
        return observation;
    };
    // Preserve local component refs and full JSON Schema 2020-12 semantics.
    // With network resolvers disabled, an unresolved or remote ref fails closed.
    let document = json!({ "allOf": [schema], "components": spec["components"] });
    match jsonschema::draft202012::new(&document) {
        Ok(validator) if validator.is_valid(&value) => {},
        Ok(_) => finding("response-schema-mismatch"),
        Err(_) => finding("invalid-response-schema"),
    }
    match schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
    {
        Some(name) => match super::wire::decode(name, value) {
            Some(true) => {},
            Some(false) => finding("contract-decode-failure"),
            None => finding("unmapped-contract-type"),
        },
        None => finding("unmapped-contract-type"),
    }
    observation
}

pub(super) fn unconsumed_response(
    spec: &Value,
    operation: &Operation,
    status: u16,
    content_type: Option<&str>,
    body_length: usize,
) -> Observation {
    let response_path = response_pointer(operation, Some(status));
    let kind = match spec.pointer(&response_path) {
        None => Some("undocumented-status"),
        Some(response) => match response.get("content").and_then(Value::as_object) {
            None if body_length != 0 => Some("unexpected-response-body"),
            None => None,
            Some(content) if !content.contains_key(essence(content_type)) => {
                Some("undocumented-content-type")
            },
            Some(_) => None,
        },
    };
    Observation {
        operation: operation.clone(),
        status,
        body_complete: false,
        findings: kind
            .map(|kind| Finding {
                operation: operation.clone(),
                status,
                kind: kind.to_owned(),
                schema_path: response_path,
            })
            .into_iter()
            .collect(),
    }
}

/// An observation of a served route the document does not describe.
pub(super) fn undocumented_operation(method: String, path: String, status: u16) -> Observation {
    let operation = Operation {
        method,
        path,
        operation_id: String::new(),
    };
    Observation {
        findings: vec![Finding {
            operation: operation.clone(),
            status,
            kind: UNDOCUMENTED_OPERATION.to_owned(),
            schema_path: response_pointer(&operation, None),
        }],
        operation,
        status,
        body_complete: false,
    }
}

const UNDOCUMENTED_OPERATION: &str = "undocumented-operation";

/// Documented error (4xx/5xx) statuses of one served operation.
fn documented_error_statuses(spec: &Value, operation: &Operation) -> BTreeSet<u16> {
    spec.pointer(&response_pointer(operation, None))
        .and_then(|item| item["responses"].as_object())
        .into_iter()
        .flat_map(|responses| responses.keys())
        .filter_map(|status| status.parse::<u16>().ok())
        .filter(|status| *status >= 400)
        .collect()
}

/// Aggregate observations into the `openapi-runtime-compatibility` report.
///
/// A drift finding is one distinct `(operation, status, kind, schema path)`;
/// repeated observations of the same violation add to its `occurrences`, not to
/// the count. Coverage gaps are findings too: `unreached-operation` when no
/// fully consumed, conformant response exists for a nonexcluded operation, and
/// `unreached-success-status` for each documented, nonexcluded 2xx/3xx status
/// that no such response produced. A served route absent from the document is
/// an `undocumented-operation` finding unless a route exclusion (one without
/// `operation_id`) names its method and path template.
///
/// Issue #1010 requires every documented non-error status to be reachable.
/// Documented error statuses that no conformant response exercised are
/// reported as `unexercised_error_status_count`, not as drift.
///
/// # Errors
///
/// [`super::ConformanceError::InvalidExclusions`] for a malformed, unknown,
/// duplicate or redundant exclusion, one naming an undocumented success
/// status, or a route exclusion naming a documented route;
/// [`super::ConformanceError::InvalidObservation`] for an observation outside
/// the served inventory that is not an undocumented-route observation.
pub fn report(
    spec: &Value,
    observations: &[Observation],
    exclusions: &Value,
) -> Result<Value, super::ConformanceError> {
    use super::ConformanceError::{InvalidExclusions, InvalidObservation};

    let inventory = inventory(spec)?;
    let exclusions: Vec<Exclusion> =
        serde_json::from_value(exclusions.clone()).map_err(|_| InvalidExclusions)?;
    let mut excluded_operations = BTreeSet::new();
    let mut excluded_statuses = BTreeSet::new();
    let mut excluded_routes = BTreeSet::new();
    for exclusion in &exclusions {
        if exclusion.reason.trim().is_empty() {
            return Err(InvalidExclusions);
        }
        let route = (exclusion.method.clone(), exclusion.path.clone());
        let documented = inventory
            .keys()
            .any(|operation| operation.method == route.0 && operation.path == route.1);
        let fresh = match (&exclusion.operation_id, exclusion.status) {
            (None, None) => !documented && excluded_routes.insert(route),
            (None, Some(_)) => false,
            (Some(operation_id), status) => {
                let operation = Operation {
                    method: route.0,
                    path: route.1,
                    operation_id: operation_id.clone(),
                };
                let Some(successes) = inventory.get(&operation) else {
                    return Err(InvalidExclusions);
                };
                match status {
                    None => excluded_operations.insert(operation),
                    Some(status) => {
                        successes.contains(&status) && excluded_statuses.insert((operation, status))
                    },
                }
            },
        };
        if !fresh {
            return Err(InvalidExclusions);
        }
    }
    if excluded_statuses
        .iter()
        .any(|(operation, _)| excluded_operations.contains(operation))
    {
        return Err(InvalidExclusions);
    }

    let mut reached: BTreeMap<&Operation, BTreeSet<u16>> = BTreeMap::new();
    let mut exercised_errors: BTreeMap<&Operation, BTreeSet<u16>> = BTreeMap::new();
    let mut findings: BTreeMap<Finding, usize> = BTreeMap::new();
    let mut waived_route_cases = 0_usize;
    for observation in observations {
        if !inventory.contains_key(&observation.operation) {
            let undocumented = observation.operation.operation_id.is_empty()
                && !observation.findings.is_empty()
                && observation
                    .findings
                    .iter()
                    .all(|finding| finding.kind == UNDOCUMENTED_OPERATION);
            if !undocumented {
                return Err(InvalidObservation);
            }
            let route = (
                observation.operation.method.clone(),
                observation.operation.path.clone(),
            );
            if excluded_routes.contains(&route) {
                waived_route_cases += 1;
                continue;
            }
        }
        for finding in &observation.findings {
            *findings.entry(finding.clone()).or_default() += 1;
        }
        if !observation.findings.is_empty() {
            continue;
        }
        if observation.status >= 400 {
            exercised_errors
                .entry(&observation.operation)
                .or_default()
                .insert(observation.status);
        }
        if observation.body_complete {
            reached
                .entry(&observation.operation)
                .or_default()
                .insert(observation.status);
        }
    }
    let mut coverage_findings = Vec::new();
    let mut unexercised_errors = Vec::new();
    for (operation, successes) in &inventory {
        if excluded_operations.contains(operation) {
            continue;
        }
        let observed = reached.get(operation);
        if observed.is_none() {
            coverage_findings.push(Finding {
                operation: operation.clone(),
                status: 0,
                kind: "unreached-operation".to_owned(),
                schema_path: response_pointer(operation, None),
            });
        }
        for &status in successes {
            let waived = excluded_statuses.contains(&(operation.clone(), status));
            if !waived && !observed.is_some_and(|observed| observed.contains(&status)) {
                coverage_findings.push(Finding {
                    operation: operation.clone(),
                    status,
                    kind: "unreached-success-status".to_owned(),
                    schema_path: response_pointer(operation, Some(status)),
                });
            }
        }
        let exercised = exercised_errors.get(operation);
        for status in documented_error_statuses(spec, operation) {
            if !exercised.is_some_and(|exercised| exercised.contains(&status)) {
                unexercised_errors
                    .push(json!({ "operation_id": operation.operation_id, "status": status }));
            }
        }
    }
    for finding in coverage_findings {
        findings.insert(finding, 0);
    }

    let findings: Vec<Value> = findings
        .into_iter()
        .map(|(finding, occurrences)| {
            json!({
                "operation_id": finding.operation.operation_id,
                "method": finding.operation.method,
                "path": finding.operation.path,
                "status": finding.status,
                "kind": finding.kind,
                "schema_path": finding.schema_path,
                "occurrences": occurrences,
            })
        })
        .collect();
    let excluded: Vec<Value> = exclusions
        .iter()
        .map(|exclusion| {
            json!({
                "operation_id": exclusion.operation_id,
                "method": exclusion.method,
                "path": exclusion.path,
                "status": exclusion.status,
                "reason": exclusion.reason,
            })
        })
        .collect();
    Ok(json!({
        "report_version": 1,
        "gate_id": "NS15",
        "artifact_name": "openapi-runtime-compatibility",
        "kind": "compatibility-report",
        "served_operation_count": inventory.len(),
        "excluded_operation_count": excluded_operations.len(),
        "excluded_success_status_count": excluded_statuses.len(),
        "excluded_undocumented_route_count": excluded_routes.len(),
        "exclusions": excluded,
        "observed_case_count": observations.len(),
        "unconsumed_case_count": observations
            .iter()
            .filter(|observation| !observation.body_complete)
            .count(),
        "waived_undocumented_route_case_count": waived_route_cases,
        "conformant_operation_count": reached.len(),
        "unexercised_error_status_count": unexercised_errors.len(),
        "unexercised_error_statuses": unexercised_errors,
        "openapi_runtime_drift_finding_count": findings.len(),
        "findings": findings,
        "complete": findings.is_empty(),
    }))
}
