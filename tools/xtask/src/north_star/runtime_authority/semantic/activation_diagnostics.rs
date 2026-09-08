//! Activation-diagnostic policy. Provenance and bounded decoding precede this predicate.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use thiserror::Error;

#[cfg(test)]
#[path = "activation_diagnostics/tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn fixture() -> Value {
    super::super::json::decode(include_bytes!("activation_diagnostics/observed.json"))
        .expect("the committed producer observation is valid bounded JSON")
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ActivationDiagnosticError {
    #[error("activation diagnostic observation has missing, extra, or invalid fields")]
    Shape,
    #[error("activation diagnostic producer or inventory version is unsupported")]
    Version,
    #[error("activation diagnostic scenario inventory is missing, duplicated, or unexpected")]
    Inventory,
    #[error("activation diagnostic diagnostic event order, boundary, status, or code is invalid")]
    Event,
    #[error(
        "activation diagnostic diagnostic field is missing, unbounded, or contains a fixture secret"
    )]
    Diagnostic,
    #[error("activation diagnostic HTTP diagnostics differ from compiler diagnostics")]
    HttpParity,
    #[error("activation diagnostic diagnostic exclusions differ from the trusted policy")]
    Exclusions,
}

#[derive(Clone, Copy)]
enum Boundary {
    Compiler,
    Plan,
    Flavor,
    Compatibility,
    Workflow,
}

impl Boundary {
    fn label(self) -> &'static str {
        match self {
            Self::Compiler => "compiler",
            Self::Plan => "plan_integrity",
            Self::Flavor => "flavor_integrity",
            Self::Compatibility => "registry_compatibility",
            Self::Workflow => "workflow_validation",
        }
    }

    fn namespace(self) -> &'static str {
        match self {
            Self::Compiler => "PLUGIN_PLAN_GRAPH_V1:",
            Self::Plan => "PLUGIN_PLAN_INTEGRITY:",
            Self::Flavor => "PLUGIN_FLAVOR_INTEGRITY:",
            Self::Compatibility => "PLUGIN_PLAN_COMPATIBILITY:",
            Self::Workflow => "WORKFLOW:",
        }
    }
}

struct ScenarioRule {
    name: &'static str,
    boundary: Boundary,
    codes: &'static [&'static str],
}

const fn rule(
    name: &'static str,
    boundary: Boundary,
    codes: &'static [&'static str],
) -> ScenarioRule {
    ScenarioRule {
        name,
        boundary,
        codes,
    }
}

// Scenario names identify executed inputs, not test names or submitted pass flags.
// Repeated codes are intentional: the real workflow validator has both a
// structural and a graph pass. Their independently raised events must survive.
const SCENARIOS: &[ScenarioRule] = &[
    rule("missing_action", Boundary::Compiler, &["MISSING_ACTION"]),
    rule("missing_plugin", Boundary::Compiler, &["MISSING_PLUGIN"]),
    rule(
        "unsupported_schema",
        Boundary::Compiler,
        &["UNSUPPORTED_WORKFLOW_SCHEMA"],
    ),
    rule("duplicate_node", Boundary::Compiler, &["DUPLICATE_NODE"]),
    rule(
        "missing_endpoint",
        Boundary::Compiler,
        &["MISSING_CONNECTION_ENDPOINT"],
    ),
    rule(
        "duplicate_connection",
        Boundary::Compiler,
        &["DUPLICATE_CONNECTION"],
    ),
    rule("graph_cycle", Boundary::Compiler, &["GRAPH_CYCLE"]),
    rule(
        "undeclared_effects",
        Boundary::Compiler,
        &["UNDECLARED_EFFECTS"],
    ),
    rule(
        "unsupported_effect_kind",
        Boundary::Compiler,
        &["UNSUPPORTED_EFFECT_KIND"],
    ),
    rule(
        "unsupported_node_kind",
        Boundary::Compiler,
        &["UNSUPPORTED_NODE_KIND"],
    ),
    rule(
        "action_version_mismatch",
        Boundary::Compiler,
        &["ACTION_VERSION_MISMATCH"],
    ),
    rule(
        "disabled_node_edge",
        Boundary::Compiler,
        &["DISABLED_NODE_EDGE"],
    ),
    rule(
        "unsupported_source_port",
        Boundary::Compiler,
        &["UNSUPPORTED_SOURCE_PORT"],
    ),
    rule(
        "unsupported_target_port",
        Boundary::Compiler,
        &["UNSUPPORTED_TARGET_PORT"],
    ),
    rule(
        "trigger_kind_mismatch",
        Boundary::Compiler,
        &["TRIGGER_KIND_MISMATCH"],
    ),
    rule(
        "duplicate_trigger",
        Boundary::Compiler,
        &["DUPLICATE_TRIGGER", "TRIGGER_KIND_MISMATCH"],
    ),
    rule(
        "invalid_reference_path",
        Boundary::Compiler,
        &["INVALID_REFERENCE_PATH"],
    ),
    rule(
        "unknown_slot_override",
        Boundary::Compiler,
        &["UNKNOWN_SLOT_OVERRIDE"],
    ),
    rule(
        "invalid_parameter_contract",
        Boundary::Compiler,
        &["INVALID_PARAMETER_CONTRACT"],
    ),
    rule(
        "invalid_reference_contract",
        Boundary::Compiler,
        &["INVALID_REFERENCE_CONTRACT"],
    ),
    rule(
        "invalid_trigger_configuration",
        Boundary::Compiler,
        &["INVALID_TRIGGER_CONFIGURATION"],
    ),
    rule(
        "missing_resource_contract",
        Boundary::Compiler,
        &["MISSING_RESOURCE_CONTRACT", "MISSING_RESOURCE_CONTRACT"],
    ),
    rule(
        "missing_credential_contract",
        Boundary::Compiler,
        &["MISSING_CREDENTIAL_CONTRACT", "MISSING_CREDENTIAL_CONTRACT"],
    ),
    rule(
        "empty_selector",
        Boundary::Compiler,
        &["EMPTY_SELECTOR", "MISSING_RESOURCE_CONTRACT"],
    ),
    rule(
        "slot_kind_mismatch",
        Boundary::Compiler,
        &["MISSING_RESOURCE_CONTRACT", "SLOT_KIND_MISMATCH"],
    ),
    rule(
        "tag_filter",
        Boundary::Compiler,
        &["UNSUPPORTED_TAG_FILTER"],
    ),
    rule(
        "node_filter",
        Boundary::Compiler,
        &["NODE_TYPE_FILTER_REJECTED"],
    ),
    rule(
        "support_required",
        Boundary::Compiler,
        &["SUPPORT_CARDINALITY"],
    ),
    rule(
        "invalid_projection",
        Boundary::Compiler,
        &["UNSUPPORTED_CONTRACT_PROJECTION"],
    ),
    rule(
        "wrong_dependency_type",
        Boundary::Compiler,
        &["DEPENDENCY_TYPE_MISMATCH"],
    ),
    rule(
        "undeclared_dependency",
        Boundary::Compiler,
        &["UNDECLARED_PLUGIN_DEPENDENCY"],
    ),
    rule(
        "schema_incompatible",
        Boundary::Compiler,
        &["SCHEMA_INCOMPATIBLE"],
    ),
    rule(
        "compiled_record_depth",
        Boundary::Compiler,
        &["INVALID_COMPILED_RECORD"],
    ),
    rule(
        "registry_compatibility.legacy_compiler_1",
        Boundary::Compatibility,
        &["UNSUPPORTED_EFFECT_PROTOCOL"],
    ),
    rule(
        "plan_integrity.unsupported_compiler_2",
        Boundary::Plan,
        &["UNSUPPORTED_FORMAT"],
    ),
    rule(
        "registry_compatibility.plugin_set_mismatch",
        Boundary::Compatibility,
        &["PLUGIN_SET_MISMATCH"],
    ),
    rule(
        "registry_compatibility.worker_flavor_mismatch",
        Boundary::Compatibility,
        &["WORKER_FLAVOR_MISMATCH"],
    ),
    rule(
        "registry_compatibility.contract_mismatch",
        Boundary::Compatibility,
        &["CONTRACT_MISMATCH"],
    ),
    rule(
        "plan_integrity.unsupported_format",
        Boundary::Plan,
        &["UNSUPPORTED_FORMAT"],
    ),
    rule(
        "plan_integrity.noncanonical",
        Boundary::Plan,
        &["NON_CANONICAL"],
    ),
    rule(
        "plan_integrity.converters_unsupported",
        Boundary::Plan,
        &["CONVERTERS_UNSUPPORTED"],
    ),
    rule(
        "plan_integrity.revision_mismatch",
        Boundary::Plan,
        &["REVISION_ID_MISMATCH"],
    ),
    rule(
        "plan_integrity.unknown_capability",
        Boundary::Plan,
        &["UNKNOWN_CAPABILITY"],
    ),
    rule(
        "plan_integrity.canonical_encoding_depth",
        Boundary::Plan,
        &["CANONICAL_ENCODING"],
    ),
    rule(
        "flavor_integrity.unsupported_record_version",
        Boundary::Flavor,
        &["UNSUPPORTED_RECORD_VERSION"],
    ),
    rule(
        "flavor_integrity.unsupported_hash_version",
        Boundary::Flavor,
        &["UNSUPPORTED_CANONICAL_HASH_VERSION"],
    ),
    rule(
        "flavor_integrity.revision_mismatch",
        Boundary::Flavor,
        &["REVISION_ID_MISMATCH"],
    ),
    rule(
        "workflow_validation.empty_name",
        Boundary::Workflow,
        &["EMPTY_NAME"],
    ),
    rule(
        "workflow_validation.no_nodes",
        Boundary::Workflow,
        &["NO_NODES"],
    ),
    rule(
        "workflow_validation.self_loop",
        Boundary::Workflow,
        &["SELF_LOOP", "SELF_LOOP"],
    ),
    rule(
        "workflow_validation.duplicate_node",
        Boundary::Workflow,
        &["DUPLICATE_NODE_KEY", "DUPLICATE_NODE_KEY"],
    ),
    rule(
        "workflow_validation.missing_endpoint",
        Boundary::Workflow,
        &["UNKNOWN_NODE", "UNKNOWN_NODE"],
    ),
    rule(
        "workflow_validation.duplicate_connection",
        Boundary::Workflow,
        &["DUPLICATE_CONNECTION"],
    ),
    rule(
        "workflow_validation.graph_cycle",
        Boundary::Workflow,
        &["CYCLE_DETECTED", "NO_ENTRY_NODES"],
    ),
    rule(
        "workflow_validation.invalid_reference_contract",
        Boundary::Workflow,
        &["INVALID_PARAM_REF"],
    ),
    rule(
        "workflow_validation.unsupported_schema",
        Boundary::Workflow,
        &["UNSUPPORTED_SCHEMA"],
    ),
    rule(
        "workflow_validation.duplicate_trigger",
        Boundary::Workflow,
        &["INVALID_TRIGGER"],
    ),
    rule(
        "workflow_validation.reference_without_connection",
        Boundary::Workflow,
        &["REFERENCE_WITHOUT_CONNECTION"],
    ),
    rule(
        "workflow_validation.invalid_retry",
        Boundary::Workflow,
        &["INVALID_RETRY_CONFIG"],
    ),
    rule(
        "workflow_validation.invalid_action_key",
        Boundary::Workflow,
        &["INVALID_ACTION_KEY"],
    ),
    rule(
        "workflow_validation.invalid_plugin_key",
        Boundary::Workflow,
        &["INVALID_PLUGIN_KEY"],
    ),
    rule(
        "workflow_validation.invalid_owner",
        Boundary::Workflow,
        &["INVALID_OWNER_ID"],
    ),
    rule(
        "workflow_validation.port_incompatible",
        Boundary::Workflow,
        &["PORT_SCHEMA_INCOMPATIBLE"],
    ),
    rule(
        "workflow_validation.port_undecidable",
        Boundary::Workflow,
        &["PORT_SCHEMA_UNDECIDABLE"],
    ),
    rule(
        "workflow_validation.reference_path",
        Boundary::Workflow,
        &["REFERENCE_PATH_UNRESOLVED"],
    ),
    rule(
        "workflow_validation.reference_incompatible",
        Boundary::Workflow,
        &["PORT_SCHEMA_INCOMPATIBLE", "REFERENCE_TYPE_INCOMPATIBLE"],
    ),
    rule(
        "workflow_validation.reference_undecidable",
        Boundary::Workflow,
        &["PORT_SCHEMA_INCOMPATIBLE", "REFERENCE_TYPE_UNDECIDABLE"],
    ),
];

/// Check semantics only; callers must separately authenticate provenance and decode bounded JSON.
pub(crate) fn verify(value: &Value) -> Result<(), ActivationDiagnosticError> {
    let root = object(
        value,
        &[
            "producer_version",
            "contract",
            "scenario_inventory_version",
            "excluded_diagnostics",
            "scenarios",
        ],
    )?;
    if root["producer_version"].as_u64() != Some(2)
        || root["scenario_inventory_version"].as_u64() != Some(1)
        || root["contract"].as_str() != Some("activation-diagnostics")
    {
        return Err(ActivationDiagnosticError::Version);
    }
    let excluded = root["excluded_diagnostics"]
        .as_array()
        .ok_or(ActivationDiagnosticError::Exclusions)?;
    if excluded.len() != 1 {
        return Err(ActivationDiagnosticError::Exclusions);
    }
    let exclusion = object(&excluded[0], &["code", "reason"])?;
    if exclusion["code"].as_str() != Some("WORKFLOW:GRAPH_ERROR")
        || exclusion["reason"].as_str()
            != Some(
                "No reachable producer beyond error conversion plumbing in the supported workflow validator.",
            )
    {
        return Err(ActivationDiagnosticError::Exclusions);
    }
    let scenarios = root["scenarios"]
        .as_array()
        .ok_or(ActivationDiagnosticError::Inventory)?;
    if scenarios.len() != SCENARIOS.len() {
        return Err(ActivationDiagnosticError::Inventory);
    }
    let mut seen = BTreeSet::new();
    for scenario in scenarios {
        let fields = object(scenario, &["scenario", "input_sha256", "events"])?;
        let name = fields["scenario"]
            .as_str()
            .ok_or(ActivationDiagnosticError::Inventory)?;
        let rule = SCENARIOS
            .iter()
            .find(|rule| rule.name == name)
            .ok_or(ActivationDiagnosticError::Inventory)?;
        if !seen.insert(name) {
            return Err(ActivationDiagnosticError::Inventory);
        }
        let hash = fields["input_sha256"]
            .as_str()
            .ok_or(ActivationDiagnosticError::Shape)?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ActivationDiagnosticError::Shape);
        }
        verify_events(&fields["events"], rule)?;
    }
    Ok(())
}

fn verify_events(value: &Value, rule: &ScenarioRule) -> Result<(), ActivationDiagnosticError> {
    let events = value.as_array().ok_or(ActivationDiagnosticError::Event)?;
    let compiler = matches!(rule.boundary, Boundary::Compiler);
    let expected_count = rule.codes.len() * if compiler { 2 } else { 1 };
    if events.len() != expected_count {
        return Err(ActivationDiagnosticError::Event);
    }
    for (sequence, value) in events.iter().enumerate() {
        let http = compiler && sequence >= rule.codes.len();
        let fields = event(
            value,
            sequence,
            if http {
                "http_activation"
            } else {
                rule.boundary.label()
            },
            http,
        )?;
        if fields[0].strip_prefix(rule.boundary.namespace())
            != Some(rule.codes[sequence % rule.codes.len()])
        {
            return Err(ActivationDiagnosticError::Event);
        }
        if http {
            let original = event(
                &events[sequence - rule.codes.len()],
                sequence - rule.codes.len(),
                "compiler",
                false,
            )?;
            if fields != original {
                return Err(ActivationDiagnosticError::HttpParity);
            }
        }
    }
    Ok(())
}

fn event<'a>(
    value: &'a Value,
    sequence: usize,
    boundary: &str,
    http: bool,
) -> Result<[&'a str; 5], ActivationDiagnosticError> {
    let fields = object(
        value,
        &[
            "sequence",
            "kind",
            "boundary",
            "http_status",
            "code",
            "path",
            "expected",
            "actual",
            "remediation",
        ],
    )?;
    if fields["sequence"].as_u64() != u64::try_from(sequence).ok()
        || fields["kind"].as_str() != Some("diagnostic_raised")
        || fields["boundary"].as_str() != Some(boundary)
        || if http {
            fields["http_status"].as_u64() != Some(422)
        } else {
            !fields["http_status"].is_null()
        }
    {
        return Err(ActivationDiagnosticError::Event);
    }
    let mut diagnostic = [""; 5];
    for (index, name) in ["code", "path", "expected", "actual", "remediation"]
        .iter()
        .enumerate()
    {
        let text = fields[*name]
            .as_str()
            .ok_or(ActivationDiagnosticError::Diagnostic)?;
        if text.trim().is_empty()
            || text.len() > 4096
            || text.contains("activation-private-description-canary")
        {
            return Err(ActivationDiagnosticError::Diagnostic);
        }
        diagnostic[index] = text;
    }
    if !diagnostic[1].starts_with('/') {
        return Err(ActivationDiagnosticError::Diagnostic);
    }
    Ok(diagnostic)
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, ActivationDiagnosticError> {
    let fields = value.as_object().ok_or(ActivationDiagnosticError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(ActivationDiagnosticError::Shape);
    }
    Ok(fields)
}
