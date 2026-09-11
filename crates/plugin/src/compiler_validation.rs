//! Pure semantic checks shared by executable-plan compilation stages.

use std::collections::{BTreeMap, HashMap, HashSet};

use nebula_workflow::Connection;

use crate::plan::{
    PlanEpoch, RecordedActionV1, RecordedBindingContractV1, RecordedBindingSiteV1,
    RecordedBindingV1, RecordedConnectionV1, RecordedInputPortV1, RecordedNodeV1,
    RecordedParameterValueV1, validate_reference_contract,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordedReferenceViolationReason {
    MissingSource,
    IncompatibleSchema,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedReferenceViolation {
    pub(super) consumer_node: String,
    pub(super) parameter_key: String,
    pub(super) reason: RecordedReferenceViolationReason,
}

pub(super) fn validate_recorded_references(
    nodes: &BTreeMap<String, RecordedNodeV1>,
    actions: &BTreeMap<String, RecordedActionV1>,
    connections: &[RecordedConnectionV1],
) -> Vec<RecordedReferenceViolation> {
    let actions_by_key = actions
        .values()
        .map(|action| (action.key.as_str(), action))
        .collect::<HashMap<_, _>>();
    let mut violations = Vec::new();
    for consumer in nodes.values() {
        for parameter in &consumer.parameters {
            let RecordedParameterValueV1::Reference {
                node_key,
                output_path,
            } = &parameter.value
            else {
                continue;
            };
            let Some(source) = nodes.get(node_key) else {
                violations.push(RecordedReferenceViolation {
                    consumer_node: consumer.id.clone(),
                    parameter_key: parameter.key.clone(),
                    reason: RecordedReferenceViolationReason::MissingSource,
                });
                continue;
            };
            let has_incoming_source = connections.iter().any(|connection| {
                connection.from_node == *node_key && connection.to_node == consumer.id
            });
            let reason = if !has_incoming_source {
                Some(RecordedReferenceViolationReason::MissingSource)
            } else if validate_reference_contract(
                parameter,
                output_path,
                source,
                consumer,
                &actions_by_key,
                connections,
                PlanEpoch::CURRENT,
            )
            .is_err()
            {
                Some(RecordedReferenceViolationReason::IncompatibleSchema)
            } else {
                None
            };
            if let Some(reason) = reason {
                violations.push(RecordedReferenceViolation {
                    consumer_node: consumer.id.clone(),
                    parameter_key: parameter.key.clone(),
                    reason,
                });
            }
        }
    }
    violations
}

pub(super) fn support_cardinality_violations(
    nodes: &BTreeMap<String, RecordedNodeV1>,
    actions: &BTreeMap<String, RecordedActionV1>,
    connections: &[RecordedConnectionV1],
) -> Vec<(String, String)> {
    let mut violations = Vec::new();
    for node in nodes.values() {
        let Some(action) = actions.get(&node.action_key) else {
            continue;
        };
        for input in &action.inputs {
            let RecordedInputPortV1::Support {
                key,
                required,
                multi,
                ..
            } = input
            else {
                continue;
            };
            let incoming_count = connections
                .iter()
                .filter(|connection| {
                    connection.to_node == node.id
                        && connection.to_port.as_deref() == Some(key.as_str())
                })
                .count();
            if (*required && incoming_count == 0) || (!*multi && incoming_count > 1) {
                violations.push((node.id.clone(), key.clone()));
            }
        }
    }
    violations
}

pub(super) fn authored_connection_key(
    connection: &Connection,
) -> (String, String, String, Option<String>) {
    (
        connection.from_node.to_string(),
        connection.effective_from_port().to_string(),
        connection.to_node.to_string(),
        connection.to_port.as_ref().map(ToString::to_string),
    )
}

pub(super) fn connection_record_key(
    connection: &RecordedConnectionV1,
) -> (String, String, String, Option<String>) {
    (
        connection.from_node.clone(),
        connection.from_port.clone(),
        connection.to_node.clone(),
        connection.to_port.clone(),
    )
}

pub(super) fn graph_has_cycle<'a>(
    nodes: impl Iterator<Item = &'a String>,
    adjacency: &HashMap<String, Vec<String>>,
) -> bool {
    let nodes = nodes.map(String::as_str).collect::<Vec<_>>();
    let node_set = nodes.iter().copied().collect::<HashSet<_>>();
    let mut indegree = nodes
        .iter()
        .copied()
        .map(|node| (node, 0_usize))
        .collect::<HashMap<_, _>>();
    for targets in adjacency.values() {
        for target in targets {
            if node_set.contains(target.as_str())
                && let Some(count) = indegree.get_mut(target.as_str())
            {
                *count = count.saturating_add(1);
            }
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(node, count)| (*count == 0).then_some(*node))
        .collect::<Vec<_>>();
    let mut visited = 0_usize;
    while let Some(node) = ready.pop() {
        visited = visited.saturating_add(1);
        if let Some(targets) = adjacency.get(node) {
            for target in targets {
                if let Some(count) = indegree.get_mut(target.as_str()) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        ready.push(target);
                    }
                }
            }
        }
    }
    visited != nodes.len()
}

pub(super) fn binding_sort_key(binding: &RecordedBindingV1) -> (u8, &str, &str, u8) {
    let (site_tag, site) = match &binding.site {
        RecordedBindingSiteV1::Node(node) => (0, node.as_str()),
        RecordedBindingSiteV1::Trigger(trigger) => (1, trigger.as_str()),
    };
    let contract_tag = match binding.contract {
        RecordedBindingContractV1::Resource { .. } => 0,
        RecordedBindingContractV1::Credential { .. } => 1,
    };
    (site_tag, site, binding.slot_key.as_str(), contract_tag)
}
