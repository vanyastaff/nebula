//! Scheduler projection of an integrity-checked recorded plan.

use std::{collections::HashMap, fmt, str::FromStr, time::Duration};

use nebula_core::{ExecutablePlanRevisionId, PortKey, WorkerFlavorRevisionId};
use nebula_workflow::{
    CheckpointingConfig, Connection, ErrorStrategy, NodeDefinition, ParamValue, RateLimit,
    RetryConfig, WorkflowConfig,
};

use crate::plan::{
    RecordedDurationV1, RecordedErrorStrategyV1, RecordedNodeV1, RecordedParameterValueV1,
    RecordedRetryV1, RecordedWorkflowConfigV1,
};
use crate::{ExecutablePlanRevision, PlanBindingRequirement};

/// A recorded plan cannot be represented by the scheduler's typed graph.
///
/// Diagnostics deliberately contain no recorded values or selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutionGraphProjectionError {
    /// An integrity-checked key no longer fits its runtime key type.
    #[error("recorded execution graph contains an invalid runtime key")]
    InvalidKey,
    /// An exact recorded action version cannot be reconstructed.
    #[error("recorded execution graph contains an invalid action version")]
    InvalidActionVersion,
    /// Recorded parallelism exceeds the target platform's address width.
    #[error("recorded execution graph parallelism exceeds platform capacity")]
    ParallelismOverflow,
    /// A recorded duration cannot be represented by the runtime.
    #[error("recorded execution graph contains an invalid duration")]
    InvalidDuration,
}

/// Immutable scheduler input projected only from an integrity-checked plan.
///
/// It carries no tenant authority. Bindings retain their full abstract contract
/// and require owner-scoped resolution before use. They are deliberately not
/// turned into `NodeDefinition::slot_bindings`, whose IDs imply concrete selection.
/// Node labels use their canonical node keys because authoring/UI labels are not
/// recorded execution semantics. Trigger and converter contracts remain in the
/// original plan and are outside this scheduler projection.
pub struct ExecutableGraph {
    plan_revision_id: ExecutablePlanRevisionId,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    nodes: Box<[NodeDefinition]>,
    connections: Box<[Connection]>,
    config: WorkflowConfig,
    variables: HashMap<String, serde_json::Value>,
    bindings: Box<[PlanBindingRequirement]>,
}

impl ExecutableGraph {
    pub(crate) fn project(
        plan: &ExecutablePlanRevision,
    ) -> Result<Self, ExecutionGraphProjectionError> {
        let content = &plan.recorded().content;
        let nodes = content
            .nodes
            .iter()
            .map(project_node)
            .collect::<Result<_, _>>()?;
        let connections = content
            .connections
            .iter()
            .map(|connection| {
                Ok(Connection {
                    from_node: parse_key(&connection.from_node)?,
                    to_node: parse_key(&connection.to_node)?,
                    from_port: Some(
                        PortKey::new(&connection.from_port)
                            .map_err(|_| ExecutionGraphProjectionError::InvalidKey)?,
                    ),
                    to_port: connection
                        .to_port
                        .as_deref()
                        .map(PortKey::new)
                        .transpose()
                        .map_err(|_| ExecutionGraphProjectionError::InvalidKey)?,
                })
            })
            .collect::<Result<_, ExecutionGraphProjectionError>>()?;
        Ok(Self {
            plan_revision_id: plan.id(),
            worker_flavor_revision_id: plan.worker_flavor_revision_id(),
            nodes,
            connections,
            config: project_config(&content.workflow_config)?,
            variables: content
                .variables
                .iter()
                .map(|variable| (variable.name.clone(), variable.value.clone()))
                .collect(),
            bindings: plan.bindings().into(),
        })
    }

    /// Exact recorded plan identity supplying every projected field.
    #[must_use]
    pub const fn plan_revision_id(&self) -> ExecutablePlanRevisionId {
        self.plan_revision_id
    }

    /// Exact frozen worker flavor required by the source plan.
    #[must_use]
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        self.worker_flavor_revision_id
    }

    /// Canonically ordered nodes, with fully qualified action keys and exact versions.
    #[must_use]
    pub fn nodes(&self) -> &[NodeDefinition] {
        &self.nodes
    }

    /// Recorded directed connections with canonical ports.
    #[must_use]
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Recorded scheduler configuration, including full-precision durations and retry bits.
    #[must_use]
    pub const fn config(&self) -> &WorkflowConfig {
        &self.config
    }

    /// Recorded workflow variables; values may be sensitive.
    #[must_use]
    pub const fn variables(&self) -> &HashMap<String, serde_json::Value> {
        &self.variables
    }

    /// Full authority-free node and trigger binding requirements.
    #[must_use]
    pub fn bindings(&self) -> &[PlanBindingRequirement] {
        &self.bindings
    }
}

impl fmt::Debug for ExecutableGraph {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutableGraph")
            .field("plan_revision_id", &self.plan_revision_id)
            .field("worker_flavor_revision_id", &self.worker_flavor_revision_id)
            .field("node_count", &self.nodes.len())
            .field("connection_count", &self.connections.len())
            .field("binding_count", &self.bindings.len())
            .finish_non_exhaustive()
    }
}

fn parse_key<T: FromStr>(key: &str) -> Result<T, ExecutionGraphProjectionError> {
    key.parse()
        .map_err(|_| ExecutionGraphProjectionError::InvalidKey)
}

fn project_node(
    recorded: &RecordedNodeV1,
) -> Result<NodeDefinition, ExecutionGraphProjectionError> {
    let mut node = NodeDefinition::new(
        parse_key(&recorded.id)?,
        &recorded.id,
        &recorded.plugin_key,
        &recorded.action_key,
    )
    .map_err(|_| ExecutionGraphProjectionError::InvalidKey)?;
    node.interface_version = Some(
        semver::Version::try_from(&recorded.action_version)
            .map_err(|_| ExecutionGraphProjectionError::InvalidActionVersion)?,
    );
    node.parameters = recorded
        .parameters
        .iter()
        .map(|parameter| {
            let value = match &parameter.value {
                RecordedParameterValueV1::Literal { value } => ParamValue::literal(value.clone()),
                RecordedParameterValueV1::Expression { expression } => {
                    ParamValue::expression(expression)
                },
                RecordedParameterValueV1::Template { template } => ParamValue::template(template),
                RecordedParameterValueV1::Reference {
                    node_key,
                    output_path,
                } => ParamValue::reference(parse_key(node_key)?, output_path),
            };
            Ok((parameter.key.clone(), value))
        })
        .collect::<Result<_, ExecutionGraphProjectionError>>()?;
    node.retry_policy = recorded.retry_policy.as_ref().map(project_retry);
    node.timeout = recorded
        .timeout
        .as_ref()
        .map(project_duration)
        .transpose()?;
    node.rate_limit = recorded.rate_limit.as_ref().map(|limit| RateLimit {
        max_requests: limit.max_requests,
        window_secs: limit.window_seconds,
    });
    node.enabled = recorded.enabled;
    Ok(node)
}

fn project_retry(retry: &RecordedRetryV1) -> RetryConfig {
    RetryConfig {
        max_attempts: retry.max_attempts,
        initial_delay_ms: retry.initial_delay_ms,
        max_delay_ms: retry.max_delay_ms,
        backoff_multiplier: f64::from_bits(retry.backoff_multiplier_bits),
    }
}

fn project_duration(
    duration: &RecordedDurationV1,
) -> Result<Duration, ExecutionGraphProjectionError> {
    if duration.nanoseconds >= 1_000_000_000 {
        return Err(ExecutionGraphProjectionError::InvalidDuration);
    }
    Ok(Duration::new(duration.seconds, duration.nanoseconds))
}

fn project_config(
    config: &RecordedWorkflowConfigV1,
) -> Result<WorkflowConfig, ExecutionGraphProjectionError> {
    Ok(WorkflowConfig {
        timeout: config.timeout.as_ref().map(project_duration).transpose()?,
        max_parallel_nodes: usize::try_from(config.max_parallel_nodes)
            .map_err(|_| ExecutionGraphProjectionError::ParallelismOverflow)?,
        checkpointing: CheckpointingConfig {
            enabled: config.checkpointing.enabled,
            interval: config
                .checkpointing
                .interval
                .as_ref()
                .map(project_duration)
                .transpose()?,
        },
        retry_policy: config.retry_policy.as_ref().map(project_retry),
        error_strategy: match config.error_strategy {
            RecordedErrorStrategyV1::FailFast => ErrorStrategy::FailFast,
            RecordedErrorStrategyV1::ContinueOnError => ErrorStrategy::ContinueOnError,
            RecordedErrorStrategyV1::IgnoreErrors => ErrorStrategy::IgnoreErrors,
        },
    })
}
