use std::time::Duration;

use nebula_parameter::collection::ParameterCollection;
use serde::{Deserialize, Serialize};

use crate::capability::{Capability, IsolationLevel};
use crate::port::{self, InputPort, OutputPort};

// Re-export from core so downstream code can continue using `nebula_action::InterfaceVersion`.
pub use nebula_core::InterfaceVersion;

/// Static metadata describing an action type.
///
/// Used by the engine for action discovery, capability checks, schema
/// validation, and interface versioning.
#[derive(Debug, Clone)]
pub struct ActionMetadata {
    /// Unique key identifying this action type (e.g. `"http.request"`).
    pub key: String,
    /// Human-readable display name (e.g. `"HTTP Request"`).
    pub name: String,
    /// Short description of what this action does.
    pub description: String,
    /// Interface version — changes only when input/output schema changes.
    pub version: InterfaceVersion,
    /// Capabilities this action requires from the runtime.
    pub capabilities: Vec<Capability>,
    /// Required isolation level.
    pub isolation_level: IsolationLevel,
    /// Whether inputs/outputs are strongly typed or dynamic JSON.
    pub execution_mode: ExecutionMode,
    /// JSON Schema for input validation (optional).
    pub input_schema: Option<serde_json::Value>,
    /// JSON Schema for output validation (optional).
    pub output_schema: Option<serde_json::Value>,
    /// User-facing configuration parameters for this action.
    /// Describes the form fields shown in the workflow editor when configuring this node.
    /// Validation of values against this collection is the engine's responsibility.
    pub parameters: Option<ParameterCollection>,
    /// Credential type this action requires, referenced by key.
    /// The engine resolves this to an actual credential at runtime.
    pub credential: Option<String>,
    /// Declarative retry policy for this action.
    /// When set, the engine uses this to decide how to retry failed executions.
    pub retry_policy: Option<RetryPolicy>,
    /// Timeout policy for this action.
    /// The engine enforces these timeouts and cancels the action via its cancellation token.
    pub timeout_policy: Option<TimeoutPolicy>,
    /// The kind of action this metadata describes.
    /// Used as a default when implementing [`Action::action_type`](crate::Action::action_type).
    pub action_type: ActionType,
    /// Input ports this action accepts.
    /// Defaults to a single flow input `"in"`.
    pub inputs: Vec<InputPort>,
    /// Output ports this action produces.
    /// Defaults to a single main flow output `"out"`.
    pub outputs: Vec<OutputPort>,
}

impl ActionMetadata {
    /// Create metadata with the minimum required fields.
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            description: description.into(),
            version: InterfaceVersion::new(1, 0),
            capabilities: Vec::new(),
            isolation_level: IsolationLevel::default(),
            execution_mode: ExecutionMode::Dynamic,
            input_schema: None,
            output_schema: None,
            parameters: None,
            credential: None,
            retry_policy: None,
            timeout_policy: None,
            action_type: ActionType::Process,
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
        }
    }

    /// Set the interface version (major, minor).
    pub fn with_version(mut self, major: u32, minor: u32) -> Self {
        self.version = InterfaceVersion::new(major, minor);
        self
    }

    /// Add a required capability.
    pub fn with_capability(mut self, cap: Capability) -> Self {
        self.capabilities.push(cap);
        self
    }

    /// Set the required isolation level.
    pub fn with_isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    /// Set the execution mode (typed or dynamic).
    pub fn with_execution_mode(mut self, mode: ExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Set the JSON Schema for input validation.
    pub fn with_input_schema(mut self, schema: serde_json::Value) -> Self {
        self.input_schema = Some(schema);
        self
    }

    /// Set the JSON Schema for output validation.
    pub fn with_output_schema(mut self, schema: serde_json::Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Set user-facing configuration parameters for this action.
    pub fn with_parameters(mut self, parameters: ParameterCollection) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Set the credential type this action requires.
    pub fn with_credential(mut self, credential_key: impl Into<String>) -> Self {
        self.credential = Some(credential_key.into());
        self
    }

    /// Set the retry policy for this action.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Set the timeout policy for this action.
    pub fn with_timeout_policy(mut self, policy: TimeoutPolicy) -> Self {
        self.timeout_policy = Some(policy);
        self
    }

    /// Set the action type discriminant.
    pub fn with_action_type(mut self, action_type: ActionType) -> Self {
        self.action_type = action_type;
        self
    }

    /// Set the input port definitions for this action.
    pub fn with_inputs(mut self, inputs: Vec<InputPort>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Set the output port definitions for this action.
    pub fn with_outputs(mut self, outputs: Vec<OutputPort>) -> Self {
        self.outputs = outputs;
        self
    }
}

/// Discriminant for the action type hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ActionType {
    /// Stateless single-execution action.
    Process,
    /// Iterative action with a persistent state.
    Stateful,
    /// Event source that starts workflows.
    Trigger,
    /// Continuous stream producer.
    Streaming,
    /// Distributed transaction participant (saga pattern).
    Transactional,
    /// Human-in-the-loop interaction.
    Interactive,
}

/// Whether action I/O is strongly typed or dynamic JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Input/Output implement `Serialize + Deserialize` with compile-time checks.
    Typed,
    /// `serde_json::Value` with runtime JSON Schema validation.
    Dynamic,
}

/// Declarative retry configuration for an action.
///
/// Inspired by Temporal's retry policy. When attached to [`ActionMetadata`],
/// the engine uses this to decide how to retry failed executions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of attempts (1 = no retry, 2 = one retry, etc.).
    pub max_attempts: u32,
    /// Initial delay between retries.
    pub initial_interval: Duration,
    /// Multiplier applied to the interval after each attempt.
    pub backoff_coefficient: f64,
    /// Upper bound on the delay between retries.
    pub max_interval: Option<Duration>,
    /// Error type names that should NOT be retried, even if marked retryable.
    pub non_retryable_errors: Vec<String>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_interval: Duration::from_secs(1),
            backoff_coefficient: 2.0,
            max_interval: Some(Duration::from_secs(60)),
            non_retryable_errors: Vec::new(),
        }
    }
}

impl RetryPolicy {
    /// Create a policy that never retries.
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            ..Default::default()
        }
    }

    /// Set the maximum number of attempts.
    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n;
        self
    }

    /// Set the initial interval between retries.
    pub fn with_initial_interval(mut self, interval: Duration) -> Self {
        self.initial_interval = interval;
        self
    }

    /// Set the backoff coefficient.
    pub fn with_backoff_coefficient(mut self, coeff: f64) -> Self {
        self.backoff_coefficient = coeff;
        self
    }

    /// Set the maximum interval between retries.
    pub fn with_max_interval(mut self, interval: Duration) -> Self {
        self.max_interval = Some(interval);
        self
    }
}

/// Timeout taxonomy for action execution.
///
/// Inspired by Temporal's activity timeouts. The engine enforces these
/// timeouts and cancels the action via its cancellation token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeoutPolicy {
    /// Maximum time from schedule to start (queue wait time).
    pub schedule_to_start: Option<Duration>,
    /// Maximum execution time once started.
    pub start_to_close: Option<Duration>,
    /// Maximum total time from schedule to completion.
    pub schedule_to_close: Option<Duration>,
    /// Expected interval between heartbeats; action is cancelled if a heartbeat is missed.
    pub heartbeat: Option<Duration>,
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            schedule_to_start: None,
            start_to_close: Some(Duration::from_secs(30)),
            schedule_to_close: None,
            heartbeat: None,
        }
    }
}

impl TimeoutPolicy {
    /// Set the maximum execution time.
    pub fn with_start_to_close(mut self, timeout: Duration) -> Self {
        self.start_to_close = Some(timeout);
        self
    }

    /// Set the schedule-to-start timeout.
    pub fn with_schedule_to_start(mut self, timeout: Duration) -> Self {
        self.schedule_to_start = Some(timeout);
        self
    }

    /// Set the overall schedule-to-close timeout.
    pub fn with_schedule_to_close(mut self, timeout: Duration) -> Self {
        self.schedule_to_close = Some(timeout);
        self
    }

    /// Set the heartbeat interval.
    pub fn with_heartbeat(mut self, interval: Duration) -> Self {
        self.heartbeat = Some(interval);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_builder() {
        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
            .with_version(2, 1)
            .with_execution_mode(ExecutionMode::Typed);

        assert_eq!(meta.key, "http.request");
        assert_eq!(meta.name, "HTTP Request");
        assert_eq!(meta.version, InterfaceVersion::new(2, 1));
        assert_eq!(meta.execution_mode, ExecutionMode::Typed);
    }

    #[test]
    fn interface_version_compatibility() {
        let v1_0 = InterfaceVersion::new(1, 0);
        let v1_2 = InterfaceVersion::new(1, 2);
        let v2_0 = InterfaceVersion::new(2, 0);

        // v1.2 is compatible with v1.0 requirement
        assert!(v1_0.is_compatible_with(&v1_2));
        // v1.0 is NOT compatible with v1.2 requirement (minor too low)
        assert!(!v1_2.is_compatible_with(&v1_0));
        // Different major = incompatible
        assert!(!v1_0.is_compatible_with(&v2_0));
    }

    #[test]
    fn interface_version_display() {
        assert_eq!(InterfaceVersion::new(1, 3).to_string(), "1.3");
    }

    #[test]
    fn default_metadata_values() {
        let meta = ActionMetadata::new("test", "Test", "A test action");
        assert_eq!(meta.version, InterfaceVersion::new(1, 0));
        assert_eq!(meta.isolation_level, IsolationLevel::default());
        assert_eq!(meta.execution_mode, ExecutionMode::Dynamic);
        assert!(meta.capabilities.is_empty());
        assert!(meta.input_schema.is_none());
        assert!(meta.output_schema.is_none());
        assert!(meta.parameters.is_none());
        assert!(meta.credential.is_none());
        assert!(meta.retry_policy.is_none());
        assert!(meta.timeout_policy.is_none());
        // Default ports
        assert_eq!(meta.inputs.len(), 1);
        assert!(meta.inputs[0].is_flow());
        assert_eq!(meta.inputs[0].key(), "in");
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_flow());
        assert_eq!(meta.outputs[0].key(), "out");
    }

    #[test]
    fn with_parameters_builder() {
        use nebula_parameter::prelude::*;

        let params = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("url", "URL")))
            .with(ParameterDef::Select(SelectParameter::new(
                "method", "Method",
            )));

        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
            .with_parameters(params);

        let params = meta.parameters.expect("parameters should be Some");
        assert_eq!(params.len(), 2);
        assert_eq!(params.get_by_key("url").unwrap().key(), "url");
        assert_eq!(params.get_by_key("method").unwrap().key(), "method");
    }

    #[test]
    fn with_credential_builder() {
        let meta = ActionMetadata::new("slack.send", "Slack Send", "Send a Slack message")
            .with_credential("slack_oauth");
        assert_eq!(meta.credential.as_deref(), Some("slack_oauth"));
    }

    #[test]
    fn builder_chaining_all_new_fields() {
        use nebula_parameter::prelude::*;

        let params = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("channel", "Channel")));

        let meta = ActionMetadata::new("slack.send", "Slack Send", "Send message")
            .with_parameters(params)
            .with_credential("slack_oauth")
            .with_execution_mode(ExecutionMode::Dynamic);

        assert!(meta.parameters.is_some());
        assert_eq!(meta.credential.as_deref(), Some("slack_oauth"));
        assert_eq!(meta.execution_mode, ExecutionMode::Dynamic);
    }

    #[test]
    fn parameters_none_by_default() {
        let meta = ActionMetadata::new("noop", "No-Op", "Does nothing");
        assert!(meta.parameters.is_none());
        assert!(meta.credential.is_none());
        // Existing fields still have their defaults
        assert!(meta.input_schema.is_none());
        assert!(meta.output_schema.is_none());
    }

    #[test]
    fn retry_policy_default() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.initial_interval, Duration::from_secs(1));
        assert_eq!(policy.backoff_coefficient, 2.0);
        assert_eq!(policy.max_interval, Some(Duration::from_secs(60)));
        assert!(policy.non_retryable_errors.is_empty());
    }

    #[test]
    fn retry_policy_no_retry() {
        let policy = RetryPolicy::no_retry();
        assert_eq!(policy.max_attempts, 1);
    }

    #[test]
    fn retry_policy_builder() {
        let policy = RetryPolicy::default()
            .with_max_attempts(5)
            .with_initial_interval(Duration::from_millis(500))
            .with_backoff_coefficient(1.5)
            .with_max_interval(Duration::from_secs(30));
        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.initial_interval, Duration::from_millis(500));
        assert_eq!(policy.backoff_coefficient, 1.5);
        assert_eq!(policy.max_interval, Some(Duration::from_secs(30)));
    }

    #[test]
    fn timeout_policy_default() {
        let policy = TimeoutPolicy::default();
        assert!(policy.schedule_to_start.is_none());
        assert_eq!(policy.start_to_close, Some(Duration::from_secs(30)));
        assert!(policy.schedule_to_close.is_none());
        assert!(policy.heartbeat.is_none());
    }

    #[test]
    fn timeout_policy_builder() {
        let policy = TimeoutPolicy::default()
            .with_start_to_close(Duration::from_secs(60))
            .with_schedule_to_start(Duration::from_secs(10))
            .with_heartbeat(Duration::from_secs(5));
        assert_eq!(policy.start_to_close, Some(Duration::from_secs(60)));
        assert_eq!(policy.schedule_to_start, Some(Duration::from_secs(10)));
        assert_eq!(policy.heartbeat, Some(Duration::from_secs(5)));
        assert!(policy.schedule_to_close.is_none());
    }

    #[test]
    fn metadata_with_retry_and_timeout() {
        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
            .with_retry_policy(
                RetryPolicy::default()
                    .with_max_attempts(5)
                    .with_initial_interval(Duration::from_secs(2)),
            )
            .with_timeout_policy(
                TimeoutPolicy::default()
                    .with_start_to_close(Duration::from_secs(10))
                    .with_heartbeat(Duration::from_secs(3)),
            );

        let retry = meta.retry_policy.unwrap();
        assert_eq!(retry.max_attempts, 5);
        assert_eq!(retry.initial_interval, Duration::from_secs(2));

        let timeout = meta.timeout_policy.unwrap();
        assert_eq!(timeout.start_to_close, Some(Duration::from_secs(10)));
        assert_eq!(timeout.heartbeat, Some(Duration::from_secs(3)));
    }

    // ── Port builder tests ──────────────────────────────────────────

    #[test]
    fn with_inputs_builder() {
        let meta = ActionMetadata::new("ai.agent", "AI Agent", "Run agent").with_inputs(vec![
            InputPort::flow("in"),
            InputPort::support("model", "AI Model", "Language model"),
        ]);
        assert_eq!(meta.inputs.len(), 2);
        assert!(meta.inputs[0].is_flow());
        assert!(meta.inputs[1].is_support());
        assert_eq!(meta.inputs[1].key(), "model");
    }

    #[test]
    fn with_outputs_builder() {
        use crate::port::FlowKind;

        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make calls")
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);
        assert_eq!(meta.outputs.len(), 2);
        if let OutputPort::Flow { kind, .. } = &meta.outputs[0] {
            assert_eq!(*kind, FlowKind::Main);
        }
        if let OutputPort::Flow { kind, .. } = &meta.outputs[1] {
            assert_eq!(*kind, FlowKind::Error);
        }
    }

    #[test]
    fn with_dynamic_output() {
        let meta = ActionMetadata::new("flow.switch", "Switch", "Route by conditions")
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::dynamic("rule", "rules")]);
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_dynamic());
        assert_eq!(meta.outputs[0].key(), "rule");
    }

    #[test]
    fn with_support_input_full_config() {
        use crate::port::{ConnectionFilter, SupportPort};

        let meta = ActionMetadata::new("ai.agent", "AI Agent", "Run agent").with_inputs(vec![
            InputPort::flow("in"),
            InputPort::Support(SupportPort {
                key: "tools".into(),
                name: "Tools".into(),
                description: "Agent tools".into(),
                required: false,
                multi: true,
                filter: ConnectionFilter::new().with_allowed_tags(vec!["langchain_tool".into()]),
            }),
        ]);
        assert_eq!(meta.inputs.len(), 2);
        if let InputPort::Support(s) = &meta.inputs[1] {
            assert!(s.multi);
            assert!(!s.required);
            assert!(!s.filter.is_empty());
        } else {
            panic!("expected Support port");
        }
    }

    #[test]
    fn builder_chaining_with_ports() {
        let meta = ActionMetadata::new("test", "Test", "desc")
            .with_version(2, 0)
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")])
            .with_execution_mode(ExecutionMode::Typed);

        assert_eq!(meta.version, InterfaceVersion::new(2, 0));
        assert_eq!(meta.inputs.len(), 1);
        assert_eq!(meta.outputs.len(), 2);
        assert_eq!(meta.execution_mode, ExecutionMode::Typed);
    }
}
