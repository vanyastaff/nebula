//! The demo's simulated executor. A run is planned in full when it starts: nodes follow their
//! connections in dependency order, each node's output comes from the stand-in for its core action,
//! ports route data as `core.if` and `core.switch` do, a failure stops the run unless an `error`
//! connection handles it, and every step gets a place on a timeline. Reading the run at an instant
//! replays that timeline, so a watched run advances node by node exactly in the shape the engine
//! reports one.

use super::eval::{self, OUT};
use crate::clock;
use nebula_api_contract::v1::execution::{
    ExecutionAttempt, ExecutionDetailResponse, ExecutionFailure, ExecutionNode,
    ExecutionNodeOutput, ExecutionNodeStatus, ExecutionStatus, ExecutionSummary,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// From a run's creation to its first dispatch.
const DISPATCH_MS: i64 = 40;
/// From a node being scheduled to its dispatch.
const START_MS: i64 = 30;
/// From a cancel request to the runtime reporting the run cancelled.
const DRAIN_MS: i64 = 250;

/// One node's place on the timeline and what it ends with.
#[derive(Debug)]
struct Step {
    node: String,
    scheduled: i64,
    /// Absent for a node that is skipped without running.
    started: Option<i64>,
    finished: i64,
    result: StepResult,
    /// The node parks on a timer before finishing, as `core.delay` does.
    parks: bool,
}

#[derive(Debug)]
enum StepResult {
    Completed(Value),
    Failed(ExecutionFailure),
    Skipped,
    /// Still running when the run stopped.
    Cancelled,
}

/// A started run of the simulated executor.
#[derive(Debug)]
pub(super) struct Run {
    pub(super) id: String,
    pub(super) workflow_id: String,
    pub(super) created: i64,
    input: Option<Value>,
    steps: Vec<Step>,
    failed: bool,
    /// When a cancellation was requested, if one was.
    cancel_requested: Option<i64>,
}

/// One node of the definition, as the planner reads it.
struct Node<'a> {
    id: &'a str,
    action: String,
    enabled: bool,
    parameters: &'a Map<String, Value>,
}

/// One connection, with its source port; an absent port is `out`.
struct Edge<'a> {
    from: &'a str,
    port: &'a str,
    to: &'a str,
}

/// What a finished node handed on: when it finished and, if it produced data, the port and the
/// data. A skipped node hands nothing on.
struct Handoff {
    at: i64,
    data: Option<(String, Value)>,
}

impl Run {
    /// Plans a run of `definition` created at `created` (Unix milliseconds) with `input`.
    pub(super) fn plan(
        id: String,
        workflow_id: String,
        definition: &Value,
        input: Option<Value>,
        created: i64,
    ) -> Self {
        let empty = Map::new();
        let nodes: Vec<Node<'_>> = definition["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|node| {
                Some(Node {
                    id: node["id"].as_str()?,
                    action: crate::document::catalog_key(node),
                    enabled: node["enabled"].as_bool().unwrap_or(true),
                    parameters: node["parameters"].as_object().unwrap_or(&empty),
                })
            })
            .collect();
        let edges: Vec<Edge<'_>> = definition["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|connection| {
                Some(Edge {
                    from: connection["from_node"].as_str()?,
                    port: connection["from_port"].as_str().unwrap_or(OUT),
                    to: connection["to_node"].as_str()?,
                })
            })
            .collect();

        let start = created + DISPATCH_MS;
        let root_input = input.clone().unwrap_or_else(|| json!({}));
        let mut handed: BTreeMap<&str, Handoff> = BTreeMap::new();
        let mut steps = Vec::new();
        let mut failure_at: Option<i64> = None;

        for node in order(&nodes, &edges) {
            let incoming: Vec<&Edge<'_>> = edges.iter().filter(|edge| edge.to == node.id).collect();
            let (ready, input) = if incoming.is_empty() {
                (start, Some(root_input.clone()))
            } else {
                let ready = incoming
                    .iter()
                    .filter_map(|edge| handed.get(edge.from).map(|handoff| handoff.at))
                    .max()
                    .unwrap_or(start);
                let input = incoming.iter().find_map(|edge| {
                    let (port, data) = handed.get(edge.from)?.data.as_ref()?;
                    (port == edge.port).then(|| data.clone())
                });
                (ready, input)
            };
            // Fail-fast: once a failure stops the run, nothing new is scheduled.
            if failure_at.is_some_and(|failed| ready >= failed) {
                continue;
            }
            let Some(input) = input.filter(|_| node.enabled) else {
                handed.insert(
                    node.id,
                    Handoff {
                        at: ready,
                        data: None,
                    },
                );
                steps.push(Step {
                    node: node.id.to_owned(),
                    scheduled: ready,
                    started: None,
                    finished: ready,
                    result: StepResult::Skipped,
                    parks: false,
                });
                continue;
            };
            let started = ready + START_MS;
            let parameters = resolve(node.parameters, &input);
            let base = base_duration(node.id, &node.action);
            let (finished, result, handed_on, parks) =
                match eval::run_action(&node.action, &parameters, input, started) {
                    Ok(produced) => {
                        let parks = produced.wait_ms.is_some();
                        let finished = started + produced.wait_ms.unwrap_or(base);
                        (
                            finished,
                            StepResult::Completed(produced.output.clone()),
                            Some((produced.port, produced.output)),
                            parks,
                        )
                    },
                    Err(error) => {
                        let finished = started + base / 2;
                        let handled = edges
                            .iter()
                            .any(|edge| edge.from == node.id && edge.port == "error");
                        let handed_on = if handled {
                            Some((
                                "error".to_owned(),
                                json!({"error": {"code": error.code, "message": error.message}}),
                            ))
                        } else {
                            failure_at = Some(failure_at.map_or(finished, |at| at.min(finished)));
                            None
                        };
                        (finished, StepResult::Failed(error), handed_on, false)
                    },
                };
            handed.insert(
                node.id,
                Handoff {
                    at: finished,
                    data: handed_on,
                },
            );
            steps.push(Step {
                node: node.id.to_owned(),
                scheduled: ready,
                started: Some(started),
                finished,
                result,
                parks,
            });
        }

        // Fail-fast also stops what is still running when the failure lands.
        if let Some(failed) = failure_at {
            for step in &mut steps {
                if matches!(step.result, StepResult::Completed(_)) && step.finished > failed {
                    step.finished = failed;
                    step.result = StepResult::Cancelled;
                }
            }
        }
        Self {
            id,
            workflow_id,
            created,
            input,
            steps,
            failed: failure_at.is_some(),
            cancel_requested: None,
        }
    }

    /// When the run reaches its terminal status, as planned or as a cancellation cuts it short.
    fn end(&self) -> i64 {
        let natural = self
            .steps
            .iter()
            .map(|step| step.finished)
            .max()
            .unwrap_or(self.created + DISPATCH_MS)
            + 20;
        match self.cancel_requested {
            Some(requested) if requested < natural => requested + DRAIN_MS,
            _ => natural,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel_requested
            .is_some_and(|requested| requested + DRAIN_MS == self.end())
    }

    pub(super) fn is_terminal(&self, at: i64) -> bool {
        at >= self.end()
    }

    /// Requests cancellation at `at`. A run that has already ended is left as it is.
    pub(super) fn cancel(&mut self, at: i64) -> bool {
        if self.is_terminal(at) || self.cancel_requested.is_some() {
            return false;
        }
        self.cancel_requested = Some(at);
        true
    }

    pub(super) fn status(&self, at: i64) -> ExecutionStatus {
        if at >= self.end() {
            if self.cancelled() {
                ExecutionStatus::Cancelled
            } else if self.failed {
                ExecutionStatus::Failed
            } else {
                ExecutionStatus::Completed
            }
        } else if self
            .cancel_requested
            .is_some_and(|requested| at >= requested)
        {
            ExecutionStatus::Cancelling
        } else if at < self.created + DISPATCH_MS {
            ExecutionStatus::Created
        } else {
            ExecutionStatus::Running
        }
    }

    pub(super) fn summary(&self, at: i64) -> ExecutionSummary {
        let status = self.status(at);
        let started = self.created + DISPATCH_MS;
        let end = self.end();
        let updated = self
            .events()
            .filter(|event| *event <= at)
            .max()
            .unwrap_or(self.created);
        ExecutionSummary {
            id: self.id.clone(),
            workflow_id: self.workflow_id.clone(),
            status,
            created_at: clock::rfc3339(self.created),
            started_at: (at >= started).then(|| clock::rfc3339(started)),
            finished_at: (at >= end).then(|| clock::rfc3339(end)),
            updated_at: clock::rfc3339(updated),
        }
    }

    /// Every instant at which the run's reported state changes.
    fn events(&self) -> impl Iterator<Item = i64> + '_ {
        let cutoff = self.end();
        self.steps
            .iter()
            .flat_map(|step| [Some(step.scheduled), step.started, Some(step.finished)])
            .flatten()
            .filter(move |event| *event <= cutoff)
            .chain([self.created, self.created + DISPATCH_MS, cutoff])
            .chain(self.cancel_requested)
    }

    /// The run as the engine reports it at `at`.
    pub(super) fn detail(&self, at: i64) -> ExecutionDetailResponse {
        let cutoff = self.end();
        let stopped_early = self.cancelled();
        let mut nodes = BTreeMap::new();
        let mut output_bytes = 0;
        for step in &self.steps {
            // A cancellation stops scheduling at the request; nothing after it appears.
            if step.scheduled > at
                || (stopped_early && self.cancel_requested.is_some_and(|c| step.scheduled > c))
            {
                continue;
            }
            let node = self.node_at(step, at, cutoff, stopped_early);
            if let Some(attempt) = node.attempts.first() {
                output_bytes += attempt.output_bytes;
            }
            nodes.insert(step.node.clone(), node);
        }
        ExecutionDetailResponse {
            execution: self.summary(at),
            snapshot_version: self.events().filter(|event| *event <= at).count() as u64,
            input: self.input.clone(),
            nodes,
            total_retries: 0,
            total_output_bytes: output_bytes,
        }
    }

    fn node_at(&self, step: &Step, at: i64, cutoff: i64, stopped_early: bool) -> ExecutionNode {
        let time = |instant: i64| Some(clock::rfc3339(instant));
        let mut node = ExecutionNode {
            status: ExecutionNodeStatus::Ready,
            scheduled_at: time(step.scheduled),
            started_at: None,
            finished_at: None,
            next_attempt_at: None,
            attempts: Vec::new(),
            output: None,
            error: None,
        };
        // A cancellation ends whatever is still running at the cutoff.
        let (finished, result) = if stopped_early && step.finished > cutoff {
            (cutoff, &StepResult::Cancelled)
        } else {
            (step.finished, &step.result)
        };
        if let Some(started) = step.started.filter(|started| *started <= at) {
            node.started_at = time(started);
            node.status = if step.parks {
                node.next_attempt_at = time(finished);
                ExecutionNodeStatus::Waiting
            } else {
                ExecutionNodeStatus::Running
            };
        }
        if at < finished {
            return node;
        }
        node.finished_at = time(finished);
        node.next_attempt_at = None;
        match result {
            StepResult::Completed(value) => {
                let output = ExecutionNodeOutput::Inline {
                    value: value.clone(),
                };
                node.status = ExecutionNodeStatus::Completed;
                node.attempts.push(ExecutionAttempt {
                    attempt_number: 1,
                    recorded_at: clock::rfc3339(finished),
                    finished_at: time(finished),
                    output: Some(output.clone()),
                    error: None,
                    output_bytes: value.to_string().len() as u64,
                });
                node.output = Some(output);
            },
            StepResult::Failed(error) => {
                node.status = ExecutionNodeStatus::Failed;
                node.attempts.push(ExecutionAttempt {
                    attempt_number: 1,
                    recorded_at: clock::rfc3339(finished),
                    finished_at: time(finished),
                    output: None,
                    error: Some(error.clone()),
                    output_bytes: 0,
                });
                node.error = Some(error.clone());
            },
            StepResult::Skipped => node.status = ExecutionNodeStatus::Skipped,
            StepResult::Cancelled => node.status = ExecutionNodeStatus::Cancelled,
        }
        node
    }
}

/// Nodes in dependency order (Kahn's algorithm, ties in definition order). Nodes on a cycle never
/// become ready and are left out; publication rejects such graphs before they can run.
fn order<'a, 'n>(nodes: &'a [Node<'n>], edges: &[Edge<'_>]) -> Vec<&'a Node<'n>> {
    let mut waiting: Vec<usize> = nodes
        .iter()
        .map(|node| edges.iter().filter(|edge| edge.to == node.id).count())
        .collect();
    let mut placed = vec![false; nodes.len()];
    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some(index) = (0..nodes.len()).find(|index| !placed[*index] && waiting[*index] == 0) {
        placed[index] = true;
        let id = nodes[index].id;
        ordered.push(&nodes[index]);
        for edge in edges.iter().filter(|edge| edge.from == id) {
            if let Some(target) = nodes.iter().position(|node| node.id == edge.to) {
                waiting[target] = waiting[target].saturating_sub(1);
            }
        }
    }
    ordered
}

/// How long a node runs, steady for the same node so runs of a workflow look alike.
fn base_duration(node: &str, action: &str) -> i64 {
    let hash = node
        .bytes()
        .chain(action.bytes())
        .fold(17_i64, |hash, byte| {
            (hash * 31 + i64::from(byte)) % 1_000_003
        });
    280 + hash % 620
}

/// Parameter entries as the action sees them: literals as written, expressions such as
/// `{{ $input.customer.name }}` read from the node's input.
fn resolve(entries: &Map<String, Value>, input: &Value) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, entry)| {
            let value = match entry["type"].as_str() {
                Some("literal") => entry["value"].clone(),
                Some("expression") => evaluate(entry["expr"].as_str().unwrap_or_default(), input),
                _ => Value::Null,
            };
            (key.clone(), value)
        })
        .collect()
}

/// A whole-string `{{ path }}` yields the value itself; templates inside text are interpolated.
fn evaluate(expression: &str, input: &Value) -> Value {
    let trimmed = expression.trim();
    if let Some(inner) = trimmed
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .filter(|inner| !inner.contains("{{"))
    {
        return lookup(inner.trim(), input);
    }
    let mut text = String::new();
    let mut rest = expression;
    while let Some(open) = rest.find("{{") {
        text.push_str(&rest[..open]);
        let Some(close) = rest[open..].find("}}") else {
            break;
        };
        let path = &rest[open + 2..open + close];
        text.push_str(&crate::schema::display(&lookup(path.trim(), input)));
        rest = &rest[open + close + 2..];
    }
    text.push_str(rest);
    Value::String(text)
}

/// `$input.a.b` (or `$json.a.b`) as a path into the input; a missing step reads as null.
fn lookup(path: &str, input: &Value) -> Value {
    let Some(rest) = path
        .strip_prefix("$input")
        .or_else(|| path.strip_prefix("$json"))
    else {
        return Value::Null;
    };
    rest.split('.')
        .filter(|step| !step.is_empty())
        .try_fold(input, |value, step| match value {
            Value::Array(items) => step
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index)),
            other => other.get(step),
        })
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_791_491_680_000;

    fn node(id: &str, action: &str, parameters: Value) -> Value {
        json!({"id": id, "plugin_key": "core", "action_key": action, "parameters": parameters})
    }

    fn literal(value: Value) -> Value {
        json!({"type": "literal", "value": value})
    }

    fn plan(nodes: Value, connections: Value, input: Value) -> Run {
        let definition = json!({"nodes": nodes, "connections": connections});
        Run::plan("exe_1".into(), "wf_1".into(), &definition, Some(input), T0)
    }

    fn statuses(detail: &ExecutionDetailResponse) -> Vec<(&str, ExecutionNodeStatus)> {
        detail
            .nodes
            .iter()
            .map(|(id, node)| (id.as_str(), node.status))
            .collect()
    }

    #[test]
    fn a_chain_advances_node_by_node_and_hands_data_on() {
        let run = plan(
            json!([
                node(
                    "paid",
                    "filter",
                    json!({
                        "data": {"type": "expression", "expr": "{{ $input.orders }}"},
                        "condition": literal(json!({"field": "paid", "op": "truthy"}))
                    })
                ),
                node(
                    "count",
                    "aggregate",
                    json!({"aggregations": literal(json!([{"fn": "count", "field": "id", "out": "n"}]))})
                )
            ]),
            json!([{"from_node": "paid", "to_node": "count"}]),
            json!({"orders": [{"id": 1, "paid": true}, {"id": 2, "paid": false}]}),
        );
        assert_eq!(run.status(T0), ExecutionStatus::Created);
        assert!(run.detail(T0).nodes.is_empty());

        let scheduled = run.detail(T0 + DISPATCH_MS + 10);
        assert_eq!(scheduled.execution.status, ExecutionStatus::Running);
        assert_eq!(
            statuses(&scheduled),
            vec![("paid", ExecutionNodeStatus::Ready)]
        );
        let early = run.detail(T0 + DISPATCH_MS + START_MS + 10);
        assert_eq!(
            statuses(&early),
            vec![("paid", ExecutionNodeStatus::Running)]
        );

        let end = run.end();
        let done = run.detail(end);
        assert_eq!(done.execution.status, ExecutionStatus::Completed);
        assert!(done.execution.finished_at.is_some());
        assert_eq!(
            statuses(&done),
            vec![
                ("count", ExecutionNodeStatus::Completed),
                ("paid", ExecutionNodeStatus::Completed)
            ]
        );
        let ExecutionNodeOutput::Inline { value } = done.nodes["count"].output.clone().unwrap()
        else {
            panic!("inline output");
        };
        assert_eq!(value, json!({"n": 1}));
        // Every reported change moves the snapshot version on.
        assert!(done.snapshot_version > early.snapshot_version);
    }

    #[test]
    fn the_untaken_branch_of_an_if_is_skipped() {
        let run = plan(
            json!([
                node(
                    "check",
                    "if",
                    json!({"condition": literal(json!({"field": "total", "op": "gte", "value": 100}))})
                ),
                node("big", "set_fields", json!({})),
                node("small", "set_fields", json!({}))
            ]),
            json!([
                {"from_node": "check", "from_port": "true", "to_node": "big"},
                {"from_node": "check", "from_port": "false", "to_node": "small"}
            ]),
            json!({"total": 20}),
        );
        let done = run.detail(run.end());
        assert_eq!(done.nodes["big"].status, ExecutionNodeStatus::Skipped);
        assert_eq!(done.nodes["small"].status, ExecutionNodeStatus::Completed);
        assert!(done.nodes["big"].started_at.is_none());
    }

    #[test]
    fn a_failure_stops_the_run_and_nothing_after_it_is_scheduled() {
        let run = plan(
            json!([
                node(
                    "bad",
                    "filter",
                    json!({"condition": literal(json!({"field": "a", "op": "exists"}))})
                ),
                node("after", "set_fields", json!({}))
            ]),
            json!([{"from_node": "bad", "to_node": "after"}]),
            json!({"not": "a list"}),
        );
        let done = run.detail(run.end());
        assert_eq!(done.execution.status, ExecutionStatus::Failed);
        assert_eq!(done.nodes["bad"].status, ExecutionNodeStatus::Failed);
        assert_eq!(
            done.nodes["bad"]
                .error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("core.invalid_input")
        );
        assert!(!done.nodes.contains_key("after"));
    }

    #[test]
    fn an_error_connection_handles_a_failure_and_the_run_completes() {
        let run = plan(
            json!([
                node(
                    "bad",
                    "filter",
                    json!({"condition": literal(json!({"field": "a", "op": "exists"}))})
                ),
                node(
                    "recover",
                    "set_fields",
                    json!({"assignments": literal(json!([{"name": "handled", "value": true}]))})
                )
            ]),
            json!([{"from_node": "bad", "from_port": "error", "to_node": "recover"}]),
            json!({}),
        );
        let done = run.detail(run.end());
        assert_eq!(done.execution.status, ExecutionStatus::Completed);
        assert_eq!(done.nodes["recover"].status, ExecutionNodeStatus::Completed);
    }

    #[test]
    fn a_cancellation_drains_and_cancels_what_is_still_running() {
        let mut run = plan(
            json!([
                node(
                    "wait",
                    "delay",
                    json!({"mode": literal(json!("for")), "amount": literal(json!(3)), "unit": literal(json!("seconds"))})
                ),
                node("after", "set_fields", json!({}))
            ]),
            json!([{"from_node": "wait", "to_node": "after"}]),
            json!({}),
        );
        let waiting = run.detail(T0 + 500);
        assert_eq!(waiting.nodes["wait"].status, ExecutionNodeStatus::Waiting);
        assert!(waiting.nodes["wait"].next_attempt_at.is_some());

        assert!(run.cancel(T0 + 600));
        assert!(!run.cancel(T0 + 700), "a second request changes nothing");
        assert_eq!(run.status(T0 + 650), ExecutionStatus::Cancelling);
        let cancelled = run.detail(T0 + 600 + DRAIN_MS);
        assert_eq!(cancelled.execution.status, ExecutionStatus::Cancelled);
        assert_eq!(
            cancelled.nodes["wait"].status,
            ExecutionNodeStatus::Cancelled
        );
        assert!(!cancelled.nodes.contains_key("after"));
        // A finished run cannot be cancelled.
        assert!(!run.cancel(T0 + 60_000));
    }

    #[test]
    fn expressions_read_the_node_input() {
        let input = json!({"customer": {"name": "Ada", "tags": ["vip"]}});
        assert_eq!(evaluate("{{ $input.customer.name }}", &input), json!("Ada"));
        assert_eq!(
            evaluate("{{ $json.customer.tags.0 }}", &input),
            json!("vip")
        );
        assert_eq!(
            evaluate("Hello {{ $input.customer.name }}!", &input),
            json!("Hello Ada!")
        );
        assert_eq!(evaluate("{{ $input.missing }}", &input), Value::Null);
    }
}
