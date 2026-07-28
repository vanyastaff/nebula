use std::collections::BTreeSet;

use serde_json::Value;

use super::ValidationError;

const REQUIRED_SCHEMA_FIELDS: [&str; 10] = [
    "gate_id",
    "gate_version",
    "registry_version",
    "evidence_schema_version",
    "input",
    "runs",
    "aggregation",
    "denominator",
    "threshold_evaluation",
    "result_status",
];

pub(super) fn validate(schema: &Value) -> Result<(), ValidationError> {
    jsonschema::draft202012::meta::validate(schema).map_err(|source| {
        ValidationError::InvalidSchema {
            detail: format!("Draft 2020-12 meta-schema validation failed: {source}"),
        }
    })?;

    let root = schema
        .as_object()
        .ok_or_else(|| ValidationError::InvalidSchema {
            detail: "root must be an object".to_owned(),
        })?;
    if root.get("$schema").and_then(Value::as_str)
        != Some("https://json-schema.org/draft/2020-12/schema")
    {
        return invalid_schema("root must declare JSON Schema draft 2020-12");
    }
    if root.get("type").and_then(Value::as_str) != Some("object")
        || root.get("additionalProperties").and_then(Value::as_bool) != Some(false)
    {
        return invalid_schema("root must be a closed object schema");
    }
    require_fields(schema, "", &REQUIRED_SCHEMA_FIELDS)?;
    for field in REQUIRED_SCHEMA_FIELDS {
        if schema.pointer(&format!("/properties/{field}")).is_none() {
            return invalid_schema(&format!("root property `{field}` is missing"));
        }
    }
    require_contract_fields(schema)?;
    validate_bounds(schema, "$")?;

    jsonschema::draft202012::new(schema).map_err(|source| ValidationError::InvalidSchema {
        detail: format!("Draft 2020-12 schema compilation failed: {source}"),
    })?;
    Ok(())
}

pub(super) fn validate_instance(schema: &Value, evidence: &Value) -> Result<(), ValidationError> {
    let validator =
        jsonschema::draft202012::new(schema).map_err(|source| ValidationError::InvalidSchema {
            detail: format!("Draft 2020-12 schema compilation failed: {source}"),
        })?;
    validator
        .validate(evidence)
        .map_err(|source| ValidationError::InvalidEvidence {
            detail: source.to_string(),
        })
}

fn require_contract_fields(schema: &Value) -> Result<(), ValidationError> {
    let required_objects = [
        (
            "/properties/input",
            "fixture_revision,workload_revision,configuration_revision",
        ),
        (
            "/$defs/runEvidence",
            "measurement_run_id,source_revision,environment,sampling,cache_policy,interval,fairness,raw_observations,query_plans,ci",
        ),
        (
            "/$defs/environment",
            "hardware,operating_system,rust,runtime,database,topology",
        ),
        (
            "/$defs/hardware",
            "provider,machine_model,cpu_architecture,cpu_model,logical_cpu_count,memory_bytes",
        ),
        ("/$defs/operatingSystem", "name,version,kernel_version"),
        ("/$defs/rustEnvironment", "toolchain,rustc_version,target"),
        ("/$defs/runtimeEnvironment", "name,version,settings"),
        (
            "/$defs/databaseEnvironment/oneOf/0",
            "applicable,engine,version,settings,topology",
        ),
        (
            "/$defs/databaseEnvironment/oneOf/1",
            "applicable,non_applicability_reason",
        ),
        ("/$defs/topology", "processes,workers,regions,description"),
        (
            "/$defs/sampling",
            "minimum_sample_size,sample_count,run_count,invalid_sample_count,retry_count,invalid_sample_policy,retry_policy",
        ),
        (
            "/$defs/cachePolicy",
            "mode,cold_definition,warm_definition,cache_reset_procedure",
        ),
        ("/$defs/interval", "start_event,stop_event"),
        (
            "/$defs/fairness/oneOf/0",
            "applicable,metric,configured_target_ratio,allowed_deviation_ratio,configuration_revision",
        ),
        (
            "/$defs/fairness/oneOf/1",
            "applicable,non_applicability_reason",
        ),
        ("/$defs/queryPlans/oneOf/0", "applicable,artifacts"),
        (
            "/$defs/queryPlans/oneOf/1",
            "applicable,non_applicability_reason",
        ),
        (
            "/$defs/ciIdentity",
            "workflow_path,job_id,run_id,run_attempt,run_url,source_revision",
        ),
        (
            "/properties/aggregation",
            "code_repository,code_revision,entrypoint,artifact_digest",
        ),
        (
            "/properties/denominator",
            "eligible_count,included_count,excluded_count,skipped_count,exclusion_reasons,skip_reasons",
        ),
        (
            "/properties/threshold_evaluation",
            "threshold_kind,metric_results,passed",
        ),
        (
            "/$defs/observationArtifact",
            "path,sha256,format,observation_count,contains_secrets,contains_raw_payloads",
        ),
    ];
    for (pointer, required_csv) in required_objects {
        let required = required_csv.split(',').collect::<Vec<_>>();
        require_fields(schema, pointer, &required)?;
    }

    for pointer in [
        "/$defs/databaseEnvironment",
        "/$defs/fairness",
        "/$defs/queryPlans",
    ] {
        let alternatives = schema
            .pointer(pointer)
            .and_then(|node| node.get("oneOf"))
            .and_then(Value::as_array)
            .ok_or_else(|| ValidationError::InvalidSchema {
                detail: format!("`{pointer}` must declare applicability with oneOf"),
            })?;
        if alternatives.len() != 2 {
            return invalid_schema(&format!("`{pointer}` must have two applicability branches"));
        }
    }

    for pointer in [
        "/properties/runs",
        "/$defs/runEvidence/properties/raw_observations",
        "/properties/threshold_evaluation/properties/metric_results",
    ] {
        require_nonempty_bounded_array(schema, pointer)?;
    }
    require_non_overlapping_scalars(schema)?;
    require_result_consistency(schema)?;
    require_closed_sampling_policies(schema)?;

    for field in ["contains_secrets", "contains_raw_payloads"] {
        if schema
            .pointer(&format!(
                "/$defs/observationArtifact/properties/{field}/const"
            ))
            .and_then(Value::as_bool)
            != Some(false)
        {
            return invalid_schema(&format!("observation `{field}` must be const false"));
        }
    }
    Ok(())
}

fn require_nonempty_bounded_array(schema: &Value, pointer: &str) -> Result<(), ValidationError> {
    let array = schema
        .pointer(pointer)
        .and_then(Value::as_object)
        .ok_or_else(|| ValidationError::InvalidSchema {
            detail: format!("`{pointer}` must be an array schema"),
        })?;
    if array.get("type").and_then(Value::as_str) != Some("array")
        || array.get("minItems").and_then(Value::as_u64) != Some(1)
        || array.get("maxItems").and_then(Value::as_u64).is_none()
    {
        return invalid_schema(&format!("`{pointer}` must be nonempty and bounded"));
    }
    Ok(())
}

fn require_non_overlapping_scalars(schema: &Value) -> Result<(), ValidationError> {
    let alternatives = schema
        .pointer("/$defs/scalarValue/oneOf")
        .and_then(Value::as_array)
        .ok_or_else(|| ValidationError::InvalidSchema {
            detail: "`/$defs/scalarValue` must declare oneOf".to_owned(),
        })?;
    let actual = alternatives
        .iter()
        .filter_map(|alternative| alternative.get("type").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let expected = ["boolean", "number", "string"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    if alternatives.len() != expected.len() || actual != expected {
        return invalid_schema(
            "`/$defs/scalarValue` alternatives must be the disjoint boolean, number, and string types",
        );
    }
    Ok(())
}

fn require_result_consistency(schema: &Value) -> Result<(), ValidationError> {
    let expected_values = [
        (
            "/allOf/0/if/properties/result_status/const",
            Value::String("passed".to_owned()),
        ),
        (
            "/allOf/0/then/properties/threshold_evaluation/properties/passed/const",
            Value::Bool(true),
        ),
        (
            "/allOf/0/then/properties/threshold_evaluation/properties/metric_results/items/properties/passed/const",
            Value::Bool(true),
        ),
        (
            "/allOf/0/else/properties/threshold_evaluation/properties/passed/const",
            Value::Bool(false),
        ),
    ];
    for (pointer, expected) in expected_values {
        if schema.pointer(pointer) != Some(&expected) {
            return invalid_schema(
                "result consistency must use one passed/non-passed conditional and reject failed metrics under a passed result",
            );
        }
    }
    Ok(())
}

fn require_closed_sampling_policies(schema: &Value) -> Result<(), ValidationError> {
    for (field, expected_policy) in [
        ("invalid_sample_policy", "fail-run"),
        ("retry_policy", "no-retry"),
    ] {
        let pointer = format!("/$defs/sampling/properties/{field}/enum");
        let is_exact_policy = schema
            .pointer(&pointer)
            .and_then(Value::as_array)
            .is_some_and(|policies| {
                policies.len() == 1 && policies[0].as_str() == Some(expected_policy)
            });
        if !is_exact_policy {
            return invalid_schema(&format!(
                "sampling `{field}` must be the closed policy `{expected_policy}`"
            ));
        }
    }
    Ok(())
}

fn require_fields(schema: &Value, pointer: &str, expected: &[&str]) -> Result<(), ValidationError> {
    let required = schema
        .pointer(pointer)
        .and_then(|node| node.get("required"))
        .and_then(Value::as_array)
        .ok_or_else(|| ValidationError::InvalidSchema {
            detail: format!("`{pointer}` must declare required fields"),
        })?;
    let actual = required
        .iter()
        .map(|field| {
            field
                .as_str()
                .ok_or_else(|| ValidationError::InvalidSchema {
                    detail: format!("`{pointer}` required fields must be strings"),
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual != expected.iter().copied().collect::<BTreeSet<_>>() {
        return invalid_schema(&format!("`{pointer}` required field set is incomplete"));
    }
    Ok(())
}

fn validate_bounds(node: &Value, pointer: &str) -> Result<(), ValidationError> {
    if let Some(object) = node.as_object() {
        match object.get("type").and_then(Value::as_str) {
            Some("object")
                if object.get("additionalProperties").and_then(Value::as_bool) != Some(false) =>
            {
                return invalid_schema(&format!("`{pointer}` must reject unknown fields"));
            },
            Some("string")
                if !object.contains_key("maxLength")
                    && !object.contains_key("enum")
                    && !object.contains_key("const") =>
            {
                return invalid_schema(&format!("`{pointer}` string is not length-bounded"));
            },
            Some("array") if !object.contains_key("maxItems") => {
                return invalid_schema(&format!("`{pointer}` array is not length-bounded"));
            },
            Some("integer" | "number")
                if !object.contains_key("const")
                    && (!object.contains_key("minimum") || !object.contains_key("maximum")) =>
            {
                return invalid_schema(&format!("`{pointer}` number is not bounded"));
            },
            _ => {},
        }
        for (key, child) in object {
            validate_bounds(child, &format!("{pointer}/{key}"))?;
        }
    } else if let Some(array) = node.as_array() {
        for (index, child) in array.iter().enumerate() {
            validate_bounds(child, &format!("{pointer}/{index}"))?;
        }
    }
    Ok(())
}

fn invalid_schema<T>(detail: &str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidSchema {
        detail: detail.to_owned(),
    })
}
