//! Stand-ins for the release's core actions. Each turns a node's input and parameters into the
//! output and port the real action would produce for the same configuration, close enough to show a
//! workflow's data moving through it. Conditions follow `core`'s condition model: a leaf
//! `{"field", "op", "value"}` over the input object, or `all` / `any` / `not` of conditions.

use crate::clock;
use nebula_api_contract::v1::execution::ExecutionFailure;
use serde_json::{Map, Value, json};
use std::cmp::Ordering;

/// The output port of every action that does not route.
pub(super) const OUT: &str = "out";

/// Longest pause a demo `core.delay` takes, so a long configured wait still finishes while watched.
pub(super) const DELAY_CAP_MS: i64 = 4_000;

/// What one node produced.
#[derive(Debug, PartialEq)]
pub(super) struct Produced {
    pub(super) output: Value,
    /// The port whose connections carry the output on.
    pub(super) port: String,
    /// How long the node waits before finishing, for `core.delay`.
    pub(super) wait_ms: Option<i64>,
}

impl Produced {
    fn on(port: &str, output: Value) -> Self {
        Self {
            output,
            port: port.to_owned(),
            wait_ms: None,
        }
    }
}

/// A failure the engine would record for a node: a typed code and a bounded message.
pub(super) fn failure(code: &str, message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure {
        code: format!("core.{code}"),
        category: "action".to_owned(),
        retryable: false,
        message: Some(message.into()),
        source_codes: Vec::new(),
    }
}

/// Runs `action` (`core.sort`) over `input` with resolved `parameters`. An explicit `data`
/// parameter replaces the input, as it does for the real actions.
pub(super) fn run_action(
    action: &str,
    parameters: &Map<String, Value>,
    input: Value,
    now_ms: i64,
) -> Result<Produced, ExecutionFailure> {
    let data = parameters.get("data").cloned().unwrap_or(input);
    let list = |name: &str| parameters.get(name).and_then(Value::as_array);
    match action {
        "core.json_transform" => {
            let object = object(&data)?;
            Ok(Produced::on(OUT, transform(object, list("operations"))?))
        },
        "core.map" => {
            let items = array(&data)?;
            let mapped = items
                .iter()
                .map(|item| transform(object(item)?, list("operations")))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Produced::on(OUT, Value::Array(mapped)))
        },
        "core.set_fields" => {
            let mut object = match &data {
                Value::Null => Map::new(),
                other => object(other)?,
            };
            for assignment in list("assignments").into_iter().flatten() {
                let name = assignment["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        failure("invalid_parameters", "Each assignment needs a name.")
                    })?;
                object.insert(name.to_owned(), assignment["value"].clone());
            }
            Ok(Produced::on(OUT, Value::Object(object)))
        },
        "core.filter" => {
            let condition = condition(parameters)?;
            let mut kept = Vec::new();
            for item in array(&data)? {
                if holds(condition, item)? {
                    kept.push(item.clone());
                }
            }
            Ok(Produced::on(OUT, Value::Array(kept)))
        },
        "core.if" => {
            let port = if holds(condition(parameters)?, &data)? {
                "true"
            } else {
                "false"
            };
            Ok(Produced::on(port, data))
        },
        "core.switch" => {
            for case in list("cases").into_iter().flatten() {
                if holds(&case["condition"], &data)? {
                    let port = case["port"].as_str().unwrap_or("default");
                    return Ok(Produced::on(port, data));
                }
            }
            Ok(Produced::on("default", data))
        },
        "core.sort" => {
            let keys = list("keys")
                .filter(|keys| !keys.is_empty())
                .ok_or_else(|| failure("invalid_parameters", "Sort needs at least one key."))?;
            let mut items = array(&data)?.clone();
            items.sort_by(|left, right| {
                keys.iter()
                    .map(|key| {
                        let field = key["field"].as_str().unwrap_or_default();
                        let (left, right) = (&left[field], &right[field]);
                        let order = compare(left, right);
                        // Nulls stay last whichever way the order runs.
                        if key["order"] == "desc" && !left.is_null() && !right.is_null() {
                            order.reverse()
                        } else {
                            order
                        }
                    })
                    .find(|order| order.is_ne())
                    .unwrap_or(Ordering::Equal)
            });
            Ok(Produced::on(OUT, Value::Array(items)))
        },
        "core.dedupe" => {
            let keys: Vec<&str> = list("keys")
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut seen = Vec::new();
            let mut kept = Vec::new();
            for item in array(&data)? {
                let identity: Vec<Value> = keys.iter().map(|key| item[*key].clone()).collect();
                if !seen.contains(&identity) {
                    seen.push(identity);
                    kept.push(item.clone());
                }
            }
            Ok(Produced::on(OUT, Value::Array(kept)))
        },
        "core.array" => {
            let mut items = array(&data)?.clone();
            for operation in list("operations").into_iter().flatten() {
                let count = |name: &str| {
                    usize::try_from(operation[name].as_u64().unwrap_or(0)).unwrap_or(usize::MAX)
                };
                items = match operation["op"].as_str() {
                    Some("take") => items.into_iter().take(count("count")).collect(),
                    Some("skip") => items.into_iter().skip(count("count")).collect(),
                    Some("chunk") => items
                        .chunks(count("size").max(1))
                        .map(|chunk| Value::Array(chunk.to_vec()))
                        .collect(),
                    Some("flatten") => items
                        .into_iter()
                        .flat_map(|item| match item {
                            Value::Array(inner) => inner,
                            other => vec![other],
                        })
                        .collect(),
                    _ => return Err(failure("invalid_parameters", "Unknown array operation.")),
                };
            }
            Ok(Produced::on(OUT, Value::Array(items)))
        },
        "core.aggregate" => Ok(Produced::on(
            OUT,
            aggregate(array(&data)?, list("group_by"), list("aggregations"))?,
        )),
        "core.delay" => {
            let wait = if param(parameters, "mode").as_str() == Some("until") {
                param(parameters, "datetime")
                    .as_str()
                    .and_then(clock::parse_rfc3339)
                    .map_or(0, |until| until - now_ms)
            } else {
                let amount = param(parameters, "amount").as_f64().unwrap_or(0.0);
                unit_ms(param(parameters, "unit").as_str(), amount)
            };
            Ok(Produced {
                output: data,
                port: OUT.to_owned(),
                wait_ms: Some(wait.clamp(0, DELAY_CAP_MS)),
            })
        },
        "core.datetime" => Ok(Produced::on(OUT, datetime(parameters, now_ms)?)),
        other => Err(failure(
            "unknown_action",
            format!("The demo has no stand-in for `{other}`."),
        )),
    }
}

static NULL: Value = Value::Null;

/// A parameter by name. A missing one reads as null, as indexing a `Value` does; indexing the map
/// itself would panic.
fn param<'a>(parameters: &'a Map<String, Value>, name: &str) -> &'a Value {
    parameters.get(name).unwrap_or(&NULL)
}

fn object(value: &Value) -> Result<Map<String, Value>, ExecutionFailure> {
    value.as_object().cloned().ok_or_else(|| {
        failure(
            "invalid_input",
            format!("Expected an object for `data`, got {}.", kind(value)),
        )
    })
}

fn array(value: &Value) -> Result<&Vec<Value>, ExecutionFailure> {
    value.as_array().ok_or_else(|| {
        failure(
            "invalid_input",
            format!("Expected an array for `data`, got {}.", kind(value)),
        )
    })
}

const fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// `pick` / `omit` / `rename` / `flatten` over one object, in order.
fn transform(
    mut object: Map<String, Value>,
    operations: Option<&Vec<Value>>,
) -> Result<Value, ExecutionFailure> {
    for operation in operations.into_iter().flatten() {
        let fields: Vec<&str> = operation["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        match operation["op"].as_str() {
            Some("pick") => object.retain(|key, _| fields.contains(&key.as_str())),
            Some("omit") => object.retain(|key, _| !fields.contains(&key.as_str())),
            Some("rename") => {
                let (Some(from), Some(to)) = (operation["from"].as_str(), operation["to"].as_str())
                else {
                    return Err(failure(
                        "invalid_parameters",
                        "Rename needs `from` and `to`.",
                    ));
                };
                if let Some(value) = object.remove(from) {
                    object.insert(to.to_owned(), value);
                }
            },
            Some("flatten") => {
                let separator = operation["separator"].as_str().unwrap_or(".");
                let mut flat = Map::new();
                flatten_into(&mut flat, "", &Value::Object(object), separator);
                object = flat;
            },
            _ => {
                return Err(failure(
                    "invalid_parameters",
                    "Unknown transform operation.",
                ));
            },
        }
    }
    Ok(Value::Object(object))
}

fn flatten_into(flat: &mut Map<String, Value>, prefix: &str, value: &Value, separator: &str) {
    match value {
        Value::Object(object) if !object.is_empty() => {
            for (key, inner) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}{separator}{key}")
                };
                flatten_into(flat, &path, inner, separator);
            }
        },
        other => {
            flat.insert(prefix.to_owned(), other.clone());
        },
    }
}

fn condition(parameters: &Map<String, Value>) -> Result<&Value, ExecutionFailure> {
    parameters
        .get("condition")
        .filter(|condition| condition.is_object())
        .ok_or_else(|| failure("invalid_parameters", "A condition object is required."))
}

/// Evaluates a core condition against one object.
pub(super) fn holds(condition: &Value, data: &Value) -> Result<bool, ExecutionFailure> {
    if let Some(all) = condition.get("all").and_then(Value::as_array) {
        for child in all {
            if !holds(child, data)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if let Some(any) = condition.get("any").and_then(Value::as_array) {
        for child in any {
            if holds(child, data)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if let Some(inner) = condition.get("not") {
        return holds(inner, data).map(|inner| !inner);
    }
    let field = condition["field"].as_str().ok_or_else(|| {
        failure(
            "invalid_condition",
            "A condition needs a `field` and an `op`.",
        )
    })?;
    let actual = data.get(field);
    let expected = &condition["value"];
    Ok(match condition["op"].as_str() {
        Some("eq") => actual == Some(expected),
        Some("ne") => actual != Some(expected),
        Some("exists") => actual.is_some_and(|value| !value.is_null()),
        Some("not_exists") => actual.is_none_or(Value::is_null),
        Some("truthy") => actual.is_some_and(truthy),
        Some(op @ ("gt" | "gte" | "lt" | "lte")) => {
            let Some(actual) = actual else {
                return Ok(false);
            };
            let order = match (actual, expected) {
                (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) => {
                    compare(actual, expected)
                },
                _ => {
                    return Err(failure(
                        "invalid_condition",
                        format!("`{field}` and the compared value are of different kinds."),
                    ));
                },
            };
            match op {
                "gt" => order.is_gt(),
                "gte" => order.is_ge(),
                "lt" => order.is_lt(),
                _ => order.is_le(),
            }
        },
        _ => {
            return Err(failure(
                "invalid_condition",
                "Unknown condition operator; use eq, ne, gt, gte, lt, lte, exists, not_exists or truthy.",
            ));
        },
    })
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    }
}

/// Orders numbers numerically and strings lexically; nulls and absent values sort last.
fn compare(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Greater,
        (_, Value::Null) => Ordering::Less,
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&right.as_f64().unwrap_or(0.0)),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        _ => left.to_string().cmp(&right.to_string()),
    }
}

fn aggregate(
    items: &[Value],
    group_by: Option<&Vec<Value>>,
    aggregations: Option<&Vec<Value>>,
) -> Result<Value, ExecutionFailure> {
    let groups: Vec<&str> = group_by
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let aggregations = aggregations
        .filter(|aggregations| !aggregations.is_empty())
        .ok_or_else(|| {
            failure(
                "invalid_parameters",
                "Aggregate needs at least one aggregation.",
            )
        })?;
    let mut buckets: Vec<(Vec<Value>, Vec<&Value>)> = Vec::new();
    for item in items {
        let identity: Vec<Value> = groups.iter().map(|group| item[*group].clone()).collect();
        match buckets.iter_mut().find(|(key, _)| *key == identity) {
            Some((_, members)) => members.push(item),
            None => buckets.push((identity, vec![item])),
        }
    }
    let summarize = |identity: &[Value], members: &[&Value]| -> Result<Value, ExecutionFailure> {
        let mut row = Map::new();
        for (group, value) in groups.iter().zip(identity) {
            row.insert((*group).to_owned(), value.clone());
        }
        for aggregation in aggregations {
            let function = aggregation["fn"].as_str().unwrap_or("count");
            let field = aggregation["field"].as_str().unwrap_or_default();
            let out = aggregation["out"]
                .as_str()
                .map_or_else(|| format!("{function}_{field}"), str::to_owned);
            let values: Vec<&Value> = members.iter().map(|member| &member[field]).collect();
            let numbers: Vec<f64> = values.iter().filter_map(|value| value.as_f64()).collect();
            let result = match function {
                "count" => json!(members.len()),
                "count_distinct" => {
                    let mut distinct: Vec<&Value> = Vec::new();
                    for value in &values {
                        if !distinct.contains(value) {
                            distinct.push(value);
                        }
                    }
                    json!(distinct.len())
                },
                "sum" => json!(round(numbers.iter().sum())),
                "avg" if numbers.is_empty() => Value::Null,
                "avg" => json!(round(numbers.iter().sum::<f64>() / numbers.len() as f64)),
                "min" => numbers
                    .iter()
                    .copied()
                    .reduce(f64::min)
                    .map_or(Value::Null, |n| json!(n)),
                "max" => numbers
                    .iter()
                    .copied()
                    .reduce(f64::max)
                    .map_or(Value::Null, |n| json!(n)),
                "collect" => Value::Array(values.into_iter().cloned().collect()),
                "join" => {
                    let separator = aggregation["sep"].as_str().unwrap_or(", ");
                    let texts: Vec<String> = values
                        .iter()
                        .map(|value| crate::schema::display(value))
                        .collect();
                    json!(texts.join(separator))
                },
                other => {
                    return Err(failure(
                        "invalid_parameters",
                        format!("Unknown aggregation `{other}`."),
                    ));
                },
            };
            row.insert(out, result);
        }
        Ok(Value::Object(row))
    };
    if groups.is_empty() {
        let all: Vec<&Value> = items.iter().collect();
        return summarize(&[], &all);
    }
    buckets
        .iter()
        .map(|(identity, members)| summarize(identity, members))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

/// Two decimals, as money and averages read.
fn round(number: f64) -> f64 {
    (number * 100.0).round() / 100.0
}

fn unit_ms(unit: Option<&str>, amount: f64) -> i64 {
    let factor = match unit {
        Some("milliseconds") => 1.0,
        Some("minutes") => 60_000.0,
        Some("hours") => 3_600_000.0,
        Some("days") => 86_400_000.0,
        Some("weeks") => 604_800_000.0,
        _ => 1_000.0,
    };
    // A float-to-integer `as` saturates, and callers clamp a demo wait far below the limits.
    (amount * factor) as i64
}

fn datetime(parameters: &Map<String, Value>, now_ms: i64) -> Result<Value, ExecutionFailure> {
    let instant = |name: &str| -> Result<i64, ExecutionFailure> {
        match parameters.get(name).and_then(Value::as_str) {
            None | Some("") => Ok(now_ms),
            Some(text) => clock::parse_rfc3339(text).ok_or_else(|| {
                failure(
                    "invalid_parameters",
                    format!("`{name}` is not an RFC 3339 timestamp."),
                )
            }),
        }
    };
    let amount = param(parameters, "amount").as_f64().unwrap_or(0.0);
    let unit = param(parameters, "unit").as_str();
    match param(parameters, "op").as_str() {
        Some("format" | "parse") => Ok(json!({"value": clock::rfc3339(instant("input")?)})),
        Some("add") => {
            Ok(json!({"value": clock::rfc3339(instant("input")? + unit_ms(unit, amount))}))
        },
        Some("subtract") => {
            Ok(json!({"value": clock::rfc3339(instant("input")? - unit_ms(unit, amount))}))
        },
        Some("diff") => {
            let elapsed = instant("to")? - instant("from")?;
            let per_unit = unit_ms(unit, 1.0).max(1);
            Ok(json!({"value": elapsed / per_unit, "unit": unit.unwrap_or("seconds")}))
        },
        _ => Err(failure(
            "invalid_parameters",
            "DateTime needs an operation: format, parse, add, subtract or diff.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(action: &str, parameters: Value, input: Value) -> Result<Produced, ExecutionFailure> {
        let parameters = parameters.as_object().cloned().unwrap_or_default();
        run_action(action, &parameters, input, 0)
    }

    fn output(action: &str, parameters: Value, input: Value) -> Value {
        run(action, parameters, input).unwrap().output
    }

    #[test]
    fn transforms_pick_omit_rename_and_flatten_an_object() {
        let input = json!({"id": 1, "name": "Ada", "meta": {"tier": "gold"}, "secret": "x"});
        assert_eq!(
            output(
                "core.json_transform",
                json!({"operations": [{"op": "omit", "fields": ["secret"]}, {"op": "rename", "from": "name", "to": "who"}, {"op": "flatten"}]}),
                input.clone()
            ),
            json!({"id": 1, "who": "Ada", "meta.tier": "gold"})
        );
        assert_eq!(
            output(
                "core.map",
                json!({"operations": [{"op": "pick", "fields": ["id"]}]}),
                json!([input])
            ),
            json!([{"id": 1}])
        );
    }

    #[test]
    fn an_explicit_data_parameter_replaces_the_input() {
        assert_eq!(
            output(
                "core.set_fields",
                json!({"data": {"a": 1}, "assignments": [{"name": "b", "value": 2}]}),
                json!({"ignored": true})
            ),
            json!({"a": 1, "b": 2})
        );
    }

    #[test]
    fn conditions_filter_and_route_as_core_evaluates_them() {
        let orders =
            json!([{"status": "paid", "total": 5}, {"status": "open", "total": 9}, {"total": 1}]);
        assert_eq!(
            output(
                "core.filter",
                json!({"condition": {"field": "status", "op": "eq", "value": "paid"}}),
                orders.clone()
            ),
            json!([{"status": "paid", "total": 5}])
        );
        // A missing field never equals, and is never greater; `ne` holds for it.
        assert_eq!(
            output(
                "core.filter",
                json!({"condition": {"any": [{"field": "total", "op": "gt", "value": 8}, {"field": "status", "op": "not_exists"}]}}),
                orders
            ),
            json!([{"status": "open", "total": 9}, {"total": 1}])
        );
        let routed = run(
            "core.if",
            json!({"condition": {"not": {"field": "vip", "op": "truthy"}}}),
            json!({"vip": false}),
        )
        .unwrap();
        assert_eq!(routed.port, "true");
        let switched = run(
            "core.switch",
            json!({"cases": [{"condition": {"field": "type", "op": "eq", "value": "a"}, "port": "first"}]}),
            json!({"type": "b"}),
        )
        .unwrap();
        assert_eq!(switched.port, "default");
    }

    #[test]
    fn comparing_values_of_different_kinds_is_a_configuration_failure() {
        let failure = run(
            "core.if",
            json!({"condition": {"field": "total", "op": "gt", "value": "10"}}),
            json!({"total": 3}),
        )
        .unwrap_err();
        assert_eq!(failure.code, "core.invalid_condition");
    }

    #[test]
    fn sorts_dedupes_shapes_and_aggregates_lists() {
        let rows = json!([
            {"id": "a", "region": "EU", "total": 10.0},
            {"id": "b", "region": "US", "total": null},
            {"id": "c", "region": "EU", "total": 30.5},
            {"id": "a", "region": "EU", "total": 10.0}
        ]);
        assert_eq!(
            output(
                "core.sort",
                json!({"keys": [{"field": "total", "order": "desc"}]}),
                rows.clone()
            )[0]["id"],
            "c"
        );
        // Nulls sort last whichever way the order runs, as the core sort's default places them.
        assert_eq!(
            output(
                "core.sort",
                json!({"keys": [{"field": "total"}]}),
                rows.clone()
            )[3]["id"],
            "b"
        );
        assert_eq!(
            output("core.dedupe", json!({"keys": ["id"]}), rows.clone())
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            output(
                "core.array",
                json!({"operations": [{"op": "chunk", "size": 3}, {"op": "take", "count": 1}]}),
                rows.clone()
            ),
            json!([[rows[0], rows[1], rows[2]]])
        );
        assert_eq!(
            output(
                "core.aggregate",
                json!({"group_by": ["region"], "aggregations": [{"fn": "sum", "field": "total", "out": "revenue"}, {"fn": "count", "field": "id", "out": "orders"}]}),
                rows
            ),
            json!([{"region": "EU", "revenue": 50.5, "orders": 3}, {"region": "US", "revenue": 0.0, "orders": 1}])
        );
    }

    #[test]
    fn the_wrong_shape_of_input_fails_with_a_readable_reason() {
        let failure = run(
            "core.filter",
            json!({"condition": {"field": "a", "op": "exists"}}),
            json!({"a": 1}),
        )
        .unwrap_err();
        assert_eq!(failure.code, "core.invalid_input");
        assert_eq!(
            failure.message.as_deref(),
            Some("Expected an array for `data`, got an object.")
        );
    }

    #[test]
    fn a_delay_waits_no_longer_than_the_demo_cap() {
        let short = run(
            "core.delay",
            json!({"mode": "for", "amount": 2, "unit": "seconds"}),
            json!({}),
        )
        .unwrap();
        assert_eq!(short.wait_ms, Some(2_000));
        let long = run(
            "core.delay",
            json!({"mode": "for", "amount": 3, "unit": "hours"}),
            json!({}),
        )
        .unwrap();
        assert_eq!(long.wait_ms, Some(DELAY_CAP_MS));
    }

    #[test]
    fn datetime_formats_and_shifts_instants() {
        let now = clock::parse_rfc3339("2026-10-08T12:00:00Z").unwrap();
        let parameters =
            json!({"op": "add", "input": "2026-10-08T12:00:00Z", "amount": 90, "unit": "minutes"});
        let shifted = run_action(
            "core.datetime",
            parameters.as_object().unwrap(),
            Value::Null,
            now,
        )
        .unwrap();
        assert_eq!(shifted.output["value"], "2026-10-08T13:30:00.000000Z");
    }
}
