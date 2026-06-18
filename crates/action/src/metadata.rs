use nebula_core::ActionKey;
use nebula_metadata::{BaseMetadata, Metadata};
use nebula_schema::ValidSchema;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::port::{self, InputPort, OutputPort};

/// How isolated this action's execution should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IsolationLevel {
    /// No isolation. Action runs directly in engine process.
    #[default]
    None,
    /// Capability-gated in-process. In-process capability checks against
    /// declared deps.
    ///
    /// The `isolated` alias preserves backward-compatibility: metadata
    /// persisted by an older version with `"isolation_level":"isolated"`
    /// (the retired out-of-process variant, ADR-0091) deserializes here —
    /// the old `Isolated` path already routed through the same in-process
    /// runner as `CapabilityGated`, so the semantics are unchanged.
    #[serde(alias = "isolated")]
    CapabilityGated,
}

/// The kind of node an action is — its place in the workflow taxonomy.
///
/// This is the authoritative classification an action carries. UI editors,
/// the workflow validator, and the audit log read it to group, render, and
/// reason about nodes. Runtime dispatch does **not** branch on this field:
/// the engine routes structurally on the handle the factory produces, and
/// the factory (or DX adapter) stamps the matching `ActionKind` onto the
/// metadata, so the stored kind and the dispatched handle cannot drift.
///
/// The eight kinds group into four families — executors
/// ([`Stateless`](Self::Stateless), [`Stateful`](Self::Stateful),
/// [`Stream`](Self::Stream), [`Agent`](Self::Agent),
/// [`Interactive`](Self::Interactive)), flow ([`Control`](Self::Control)),
/// entry ([`Trigger`](Self::Trigger)), and provider
/// ([`Resource`](Self::Resource)) — derived from the kind rather than stored
/// as a separate field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActionKind {
    /// Pure executor: runs input to output with no state kept between
    /// executions. The engine may run instances in parallel.
    Stateless,
    /// Iterative executor with persistent state the engine checkpoints
    /// between calls — pagination, long-running loops, multi-step processing.
    Stateful,
    /// Executor that produces its output as a stream of chunks (LLM tokens,
    /// byte streams, event deltas) rather than a single value.
    Stream,
    /// Autonomous executor with an internal reasoning loop that selects and
    /// calls tools across turns under a budget.
    Agent,
    /// Executor that pauses for external input (human approval, a callback)
    /// and resumes when it arrives.
    Interactive,
    /// Flow-control node that routes, branches, filters, gates, or terminates
    /// without transforming data.
    Control,
    /// Entry node that lives outside the execution graph and starts new
    /// executions.
    Trigger,
    /// Provider node that supplies a scoped capability (DB pool, HTTP client,
    /// browser session) to downstream nodes in a branch.
    Resource,
}

/// How often the engine checkpoints an action's progress.
///
/// Orthogonal to [`ActionKind`]: any kind can request a checkpointing
/// cadence. The default, [`Inherit`](Self::Inherit), defers to the engine's
/// execution-wide policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckpointPolicy {
    /// Defer to the engine's execution-wide checkpoint cadence.
    #[default]
    Inherit,
    /// Checkpoint once, after the action completes.
    OnePass,
    /// Checkpoint after every step or iteration — maximum durability at the
    /// cost of write volume.
    Stepwise,
    /// Force a durable checkpoint and hand the execution back to the
    /// scheduler, used when an action parks waiting for an external event.
    ForcedHandoff,
}

/// Default [`ActionKind`] for metadata that predates the field or is built
/// without a kind set. The factory or DX adapter stamps the real kind.
fn default_action_kind() -> ActionKind {
    ActionKind::Stateless
}

/// Compatibility validation errors for action metadata evolution.
///
/// Wraps [`nebula_metadata::BaseCompatError`] (shared catalog-entity rules)
/// and layers the action-specific port-change rule on top.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<ActionKey>),

    /// Input or output ports changed without a major version bump.
    #[error("action ports changed without a major version bump")]
    PortsChangeWithoutMajorBump,
}

/// Static metadata describing an action type.
///
/// Used by the engine for action discovery, capability checks, schema
/// validation, and interface versioning.
///
/// The shared catalog prefix (`key`, `name`, `description`, `schema`, `icon`,
/// `documentation_url`, `tags`, `maturity`, `deprecation`) lives on the
/// composed [`BaseMetadata`]. Entity-specific
/// fields (`version`, ports, `isolation_level`, `kind`,
/// `checkpoint_policy`) stay on this struct.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionMetadata {
    /// Shared catalog prefix — see [`BaseMetadata`]. Carries `version`
    /// (bumped when schema/ports change; exact-match dispatch in the engine).
    #[serde(flatten)]
    pub base: BaseMetadata<ActionKey>,
    /// Input ports this action accepts.
    /// Defaults to a single flow input `"in"`.
    pub inputs: Vec<InputPort>,
    /// Output ports this action produces.
    /// Defaults to a single main flow output `"out"`.
    pub outputs: Vec<OutputPort>,
    /// Isolation level for this action's execution.
    pub isolation_level: IsolationLevel,
    /// Node-taxonomy classification of this action. Authoritative for UI
    /// grouping, validation, and audit; runtime dispatch is structural and
    /// does not read it.
    ///
    /// The factory or DX adapter that produces the dispatch handle stamps the
    /// matching kind, so it cannot drift from the dispatched handle. Defaults
    /// to [`ActionKind::Stateless`] for metadata that predates the field.
    #[serde(default = "default_action_kind")]
    pub kind: ActionKind,
    /// Checkpointing cadence requested for this action.
    ///
    /// Defaults to [`CheckpointPolicy::Inherit`], which defers to the engine's
    /// execution-wide policy.
    #[serde(default)]
    pub checkpoint_policy: CheckpointPolicy,
    /// Per-action concurrency throttle hint — **persisted hint, not yet
    /// enforced** as of П1.
    ///
    /// The П1 surface lands the field so action authors and registries
    /// can carry it through serialization round-trips, but the engine
    /// scheduler does not currently read it: setting `Some(n)` does
    /// **not** bound in-flight executions today. Enforcement lands in
    /// the engine cluster-mode cascade. Until then, treat
    /// this as a stable storage shape, not a runtime guarantee.
    ///
    /// Future contract (when enforced): `None` — engine-global throttle
    /// still applies, but no per-action limit. `Some(n)` — at most `n`
    /// in-flight executions of this action across the engine.
    ///
    /// Per Tech Spec §15.12 F9 + PRODUCT_ backpressure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<core::num::NonZeroU32>,
    /// Schema describing the type this action produces as output.
    ///
    /// Stamped by the factory or DX adapter from `<A::Output as HasSchema>::schema()`
    /// — the single writer is the factory, mirroring how `kind` is stamped.
    /// TypeDAG edge checks (`T3+`) read this field to validate that the
    /// producer's output is assignable to the consumer's input.
    ///
    /// Defaults to [`ValidSchema::empty`] for metadata persisted before this
    /// field existed (back-compat), and for actions with an untyped output.
    #[serde(default = "ValidSchema::empty")]
    pub output_schema: ValidSchema,
}

impl Metadata for ActionMetadata {
    type Key = ActionKey;
    fn base(&self) -> &BaseMetadata<ActionKey> {
        &self.base
    }
}

impl ActionMetadata {
    /// Create metadata with the minimum required fields.
    #[must_use]
    pub fn new(key: ActionKey, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            base: BaseMetadata::new(key, name, description, ValidSchema::empty()),
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            isolation_level: IsolationLevel::None,
            kind: ActionKind::Stateless,
            checkpoint_policy: CheckpointPolicy::Inherit,
            max_concurrent: None,
            output_schema: ValidSchema::empty(),
        }
    }

    /// Create metadata from a key alone — name defaults to the key string and
    /// description is empty.
    ///
    /// Convenience constructor symmetric with `ResourceMetadata::from_key`,
    /// useful for fixtures and placeholder catalog entries where only the key
    /// is meaningful. (`CredentialMetadata` has no `from_key` — its required
    /// `AuthPattern` has no meaningful default.)
    #[must_use]
    pub fn from_key(key: &ActionKey) -> Self {
        Self::new(key.clone(), key.to_string(), String::new())
    }

    /// Begin building metadata with the required catalog fields seeded.
    ///
    /// Symmetric entry point with `ResourceMetadata::builder` /
    /// `CredentialMetadata::builder`. `ActionMetadata` uses a consuming fluent
    /// builder — the `with_*` methods take and return `Self` — so this returns
    /// the value directly; chain `with_*` and finish with
    /// [`build`](Self::build), or use the value as-is.
    #[must_use]
    pub fn builder(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self::new(key, name, description)
    }

    /// Create metadata whose `parameters` schema is auto-derived from the
    /// action's [`Input`](crate::Action::Input) type.
    ///
    /// Symmetric with `CredentialMetadata::for_credential` and
    /// `ResourceMetadata::for_resource`. Prefer this over
    /// [`ActionMetadata::new`] + [`ActionMetadata::with_schema`] when the
    /// author owns a typed `Input` struct — the compiler then enforces that
    /// the declared `Input` type agrees with the schema the engine, UI, and
    /// validator will see. Works for any action kind (stateless, stateful,
    /// paginated, batch, trigger, …) because every [`Action`](crate::Action)
    /// has an `Input: HasSchema`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use nebula_action::ActionMetadata;
    /// use nebula_core::action_key;
    ///
    /// let meta = ActionMetadata::for_action::<MyAction>(
    ///     action_key!("my.action"), "My Action", "desc",
    /// );
    /// ```
    #[must_use]
    pub fn for_action<A>(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self
    where
        A: crate::action::Action,
    {
        Self::new(key, name, description)
            .with_schema(<A::Input as nebula_schema::HasSchema>::schema())
            .with_output_schema(<A::Output as nebula_schema::HasSchema>::schema())
    }

    /// Set the interface version from `(major, minor)` components.
    ///
    /// Equivalent to `with_version_full(Version::new(major, minor, 0))`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version(mut self, major: u64, minor: u64) -> Self {
        self.base.version = Version::new(major, minor, 0);
        self
    }

    /// Set the full interface version, including patch and pre-release data.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version_full(mut self, version: Version) -> Self {
        self.base.version = version;
        self
    }

    /// Set the input port definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_inputs(mut self, inputs: Vec<InputPort>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Append a single input port, preserving already-declared ones.
    #[must_use = "builder methods must be chained or built"]
    pub fn add_input(mut self, port: InputPort) -> Self {
        self.inputs.push(port);
        self
    }

    /// Set the output port definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_outputs(mut self, outputs: Vec<OutputPort>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Append a single output port, preserving already-declared ones.
    #[must_use = "builder methods must be chained or built"]
    pub fn add_output(mut self, port: OutputPort) -> Self {
        self.outputs.push(port);
        self
    }

    /// Bump the major version to `new_major`, zeroing minor and patch.
    #[must_use = "builder methods must be chained or built"]
    pub fn bump_major(mut self, new_major: u64) -> Self {
        self.base.version = Version::new(new_major, 0, 0);
        self
    }

    /// Bump the minor version to `new_minor`, zeroing patch.
    #[must_use = "builder methods must be chained or built"]
    pub fn bump_minor(mut self, new_minor: u64) -> Self {
        self.base.version = Version::new(self.base.version.major, new_minor, 0);
        self
    }

    /// Bump the patch version to `new_patch`.
    #[must_use = "builder methods must be chained or built"]
    pub fn bump_patch(mut self, new_patch: u64) -> Self {
        self.base.version =
            Version::new(self.base.version.major, self.base.version.minor, new_patch);
        self
    }

    /// Set the parameter schema for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_schema(mut self, schema: ValidSchema) -> Self {
        self.base.schema = schema;
        self
    }

    /// Set the output schema for this action.
    ///
    /// The single writer is the factory or DX adapter (via
    /// `<A::Output as HasSchema>::schema()`); action authors rarely call
    /// this directly. Symmetric with [`with_schema`](Self::with_schema).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_output_schema(mut self, schema: ValidSchema) -> Self {
        self.output_schema = schema;
        self
    }

    /// The schema describing what this action produces.
    ///
    /// Stamped from `<A::Output as HasSchema>::schema()` by the factory.
    /// TypeDAG edge checks read this to validate producer→consumer
    /// assignability. Returns an empty schema for actions that predated
    /// this field or have an untyped output.
    #[must_use]
    pub fn output_schema(&self) -> &ValidSchema {
        &self.output_schema
    }

    /// Set the isolation level for dispatch routing.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_isolation_level(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    /// Set the node-taxonomy kind for this action.
    ///
    /// The single writer is the factory or DX adapter that produces the
    /// dispatch handle; action authors rarely call this directly.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_kind(mut self, kind: ActionKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the checkpointing cadence for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_checkpoint_policy(mut self, policy: CheckpointPolicy) -> Self {
        self.checkpoint_policy = policy;
        self
    }

    /// Set the per-action concurrency throttle hint.
    ///
    /// **Persisted hint, not yet enforced** as of П1 — see
    /// [`max_concurrent`](Self::max_concurrent) field docs for the
    /// enforcement timeline. Builder-style; chainable.
    ///
    /// Per Tech Spec §15.12 F9.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_concurrent(mut self, n: core::num::NonZeroU32) -> Self {
        self.max_concurrent = Some(n);
        self
    }

    /// Terminal builder for API consistency with other metadata types.
    #[must_use]
    pub fn build(self) -> Self {
        self
    }

    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Delegates `key immutable / version monotonic / schema-break-requires-
    /// major` to [`nebula_metadata::validate_base_compat`]; layers the action-
    /// specific port-change rule on top.
    ///
    /// The ports rule lives on this type rather than in `nebula-metadata`
    /// because no other catalog citizen exposes a typed input/output port
    /// graph — only actions declare ports that the workflow validator wires
    /// between nodes, so the rule has no meaningful shared form.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;

        let ports_changed = self.inputs != previous.inputs || self.outputs != previous.outputs;
        if ports_changed && self.base.version.major == previous.base.version.major {
            return Err(MetadataCompatibilityError::PortsChangeWithoutMajorBump);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::action_key;
    use nebula_metadata::BaseCompatError;

    use super::*;

    #[test]
    fn metadata_builder() {
        let meta = ActionMetadata::new(
            action_key!("http.request"),
            "HTTP Request",
            "Make HTTP calls",
        )
        .with_version(2, 1);

        assert_eq!(meta.base.key, action_key!("http.request"));
        assert_eq!(meta.base.name, "HTTP Request");
        assert_eq!(meta.base.version, Version::new(2, 1, 0));
    }

    #[test]
    fn metadata_partial_eq_roundtrip() {
        // Two metadata values built identically must compare equal —
        // this is the guarantee downstream registries rely on for
        // deduplication and cache-key checks.
        let a = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(1, 2)
            .with_isolation_level(IsolationLevel::CapabilityGated);
        let b = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(1, 2)
            .with_isolation_level(IsolationLevel::CapabilityGated);
        assert_eq!(a, b);

        let c = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc").with_version(1, 3);
        assert_ne!(a, c, "different minor version must break equality");
    }

    #[test]
    fn metadata_serde_roundtrip() {
        // Serialize → JSON → deserialize must round-trip cleanly.
        // Engine persistence and cross-process plugin discovery both
        // depend on this contract.
        let original = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(2, 1)
            .with_isolation_level(IsolationLevel::CapabilityGated);

        let json = serde_json::to_string(&original).expect("serialization succeeds");
        let decoded: ActionMetadata =
            serde_json::from_str(&json).expect("deserialization succeeds");

        assert_eq!(original, decoded);
    }

    #[test]
    fn retired_category_field_is_ignored_on_load() {
        // `category` was retired in favour of `kind`. Metadata persisted by an
        // older build still carries a `"category"` key; loading it must succeed
        // and ignore the stray field (no `deny_unknown_fields`), not error.
        let mut as_value: serde_json::Value = serde_json::to_value(ActionMetadata::new(
            action_key!("http.request"),
            "HTTP",
            "desc",
        ))
        .unwrap();
        as_value
            .as_object_mut()
            .unwrap()
            .insert("category".to_string(), serde_json::json!("control"));
        let json = serde_json::to_string(&as_value).unwrap();

        let decoded: ActionMetadata = serde_json::from_str(&json)
            .expect("legacy metadata carrying a retired `category` key must still load");
        assert_eq!(decoded.kind, ActionKind::Stateless);
        assert!(
            !serde_json::to_value(&decoded)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("category"),
            "re-serialized metadata must not resurrect the retired `category` field"
        );
    }

    // ── ActionKind ──────────────────────────────────────────────────

    #[test]
    fn kind_default_is_stateless() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc");
        assert_eq!(meta.kind, ActionKind::Stateless);
    }

    #[test]
    fn kind_builder() {
        let meta = ActionMetadata::new(action_key!("res"), "Resource", "Provide a pool")
            .with_kind(ActionKind::Resource);
        assert_eq!(meta.kind, ActionKind::Resource);
    }

    #[test]
    fn action_kind_serde_roundtrip() {
        // Every variant must round-trip through JSON with a stable
        // snake_case wire tag — UI, validator, and audit consumers read it.
        let cases = [
            (ActionKind::Stateless, r#""stateless""#),
            (ActionKind::Stateful, r#""stateful""#),
            (ActionKind::Stream, r#""stream""#),
            (ActionKind::Agent, r#""agent""#),
            (ActionKind::Interactive, r#""interactive""#),
            (ActionKind::Control, r#""control""#),
            (ActionKind::Trigger, r#""trigger""#),
            (ActionKind::Resource, r#""resource""#),
        ];
        for (kind, wire) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, wire, "ActionKind wire tag changed for {kind:?}");
            let back: ActionKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn kind_metadata_serde_roundtrip() {
        let original = ActionMetadata::new(action_key!("trig"), "Trigger", "desc")
            .with_kind(ActionKind::Trigger);
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.kind, ActionKind::Trigger);
        assert_eq!(original, decoded);
    }

    #[test]
    fn kind_backward_compat_without_field() {
        // Metadata serialized before `kind` existed must still deserialize,
        // defaulting to `Stateless`.
        let legacy = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc");
        let mut as_value: serde_json::Value = serde_json::to_value(&legacy).unwrap();
        as_value
            .as_object_mut()
            .unwrap()
            .remove("kind")
            .expect("kind field must be present after serialize");
        let json_string = serde_json::to_string(&as_value).unwrap();
        let decoded: ActionMetadata = serde_json::from_str(&json_string)
            .expect("legacy metadata without kind must deserialize");
        assert_eq!(
            decoded.kind,
            ActionKind::Stateless,
            "missing kind field should default to Stateless"
        );
    }

    // ── CheckpointPolicy ────────────────────────────────────────────

    #[test]
    fn checkpoint_policy_default_is_inherit() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc");
        assert_eq!(meta.checkpoint_policy, CheckpointPolicy::Inherit);
        assert_eq!(CheckpointPolicy::default(), CheckpointPolicy::Inherit);
    }

    #[test]
    fn checkpoint_policy_builder() {
        let meta = ActionMetadata::new(action_key!("loop"), "Loop", "Iterate")
            .with_checkpoint_policy(CheckpointPolicy::Stepwise);
        assert_eq!(meta.checkpoint_policy, CheckpointPolicy::Stepwise);
    }

    #[test]
    fn checkpoint_policy_serde_roundtrip() {
        let cases = [
            (CheckpointPolicy::Inherit, r#""inherit""#),
            (CheckpointPolicy::OnePass, r#""one_pass""#),
            (CheckpointPolicy::Stepwise, r#""stepwise""#),
            (CheckpointPolicy::ForcedHandoff, r#""forced_handoff""#),
        ];
        for (policy, wire) in cases {
            let json = serde_json::to_string(&policy).unwrap();
            assert_eq!(
                json, wire,
                "CheckpointPolicy wire tag changed for {policy:?}"
            );
            let back: CheckpointPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(back, policy);
        }
    }

    #[test]
    fn checkpoint_policy_backward_compat_without_field() {
        let legacy = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc");
        let mut as_value: serde_json::Value = serde_json::to_value(&legacy).unwrap();
        as_value
            .as_object_mut()
            .unwrap()
            .remove("checkpoint_policy")
            .expect("checkpoint_policy field must be present after serialize");
        let json_string = serde_json::to_string(&as_value).unwrap();
        let decoded: ActionMetadata = serde_json::from_str(&json_string)
            .expect("legacy metadata without checkpoint_policy must deserialize");
        assert_eq!(decoded.checkpoint_policy, CheckpointPolicy::Inherit);
    }

    #[test]
    fn default_metadata_values() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "A test action");
        assert_eq!(meta.base.version, Version::new(1, 0, 0));
        // Default ports
        assert_eq!(meta.inputs.len(), 1);
        assert!(meta.inputs[0].is_flow());
        assert_eq!(meta.inputs[0].key(), "in");
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_flow());
        assert_eq!(meta.outputs[0].key(), "out");
        // Default parameters
        assert!(meta.base.schema.fields().is_empty());
    }

    // ── Port builder tests ──────────────────────────────────────────

    #[test]
    fn with_inputs_builder() {
        let meta = ActionMetadata::new(action_key!("ai.agent"), "AI Agent", "Run agent")
            .with_inputs(vec![
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

        let meta = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "Make calls")
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
        let meta = ActionMetadata::new(action_key!("flow.switch"), "Switch", "Route by conditions")
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::dynamic("rule", "rules")]);
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_dynamic());
        assert_eq!(meta.outputs[0].key(), "rule");
    }

    #[test]
    fn with_support_input_full_config() {
        use crate::port::{ConnectionFilter, SupportPort};

        let meta = ActionMetadata::new(action_key!("ai.agent"), "AI Agent", "Run agent")
            .with_inputs(vec![
                InputPort::flow("in"),
                InputPort::Support(SupportPort {
                    key: "tools".into(),
                    name: "Tools".into(),
                    description: "Agent tools".into(),
                    required: false,
                    multi: true,
                    filter: ConnectionFilter::new()
                        .with_allowed_tags(vec!["langchain_tool".into()]),
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
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc")
            .with_version(2, 0)
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        assert_eq!(meta.base.version, Version::new(2, 0, 0));
        assert_eq!(meta.inputs.len(), 1);
        assert_eq!(meta.outputs.len(), 2);
    }

    #[test]
    fn ports_change_requires_major_bump() {
        let prev = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 0)
            .with_outputs(vec![OutputPort::flow("out")]);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 1)
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(err, MetadataCompatibilityError::PortsChangeWithoutMajorBump);
    }

    #[test]
    fn schema_field_change_requires_major_bump() {
        use nebula_schema::{FieldCollector, Schema, field_key};

        let prev =
            ActionMetadata::new(action_key!("http.request"), "HTTP", "desc").with_version(1, 0);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(1, 1)
            .with_schema(
                Schema::builder()
                    .string(field_key!("added"), |s| s)
                    .build()
                    .unwrap(),
            );

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::SchemaChangeWithoutMajorBump)
        );
    }

    #[test]
    fn schema_change_with_major_bump_is_valid() {
        let prev = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 0)
            .with_outputs(vec![OutputPort::flow("out")]);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(2, 0)
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn key_change_is_rejected() {
        let prev = ActionMetadata::new(action_key!("a.one"), "A", "desc").with_version(1, 0);
        let next = ActionMetadata::new(action_key!("a.two"), "A", "desc").with_version(2, 0);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::KeyChanged { .. })
        ));
    }

    #[test]
    fn isolation_level_defaults_to_none() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "A test action");
        assert_eq!(meta.isolation_level, IsolationLevel::None);
        assert_eq!(IsolationLevel::default(), IsolationLevel::None);
    }

    #[test]
    fn with_isolation_level_builder() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc")
            .with_isolation_level(IsolationLevel::CapabilityGated);
        assert_eq!(meta.isolation_level, IsolationLevel::CapabilityGated);
    }

    #[test]
    fn legacy_isolated_value_deserializes_as_capability_gated() {
        // Backward-compat (ADR-0091): metadata persisted before the retired
        // `Isolated` variant was removed must still load — the wire value
        // `"isolated"` maps onto `CapabilityGated`, not a hard parse error.
        let level: IsolationLevel =
            serde_json::from_str("\"isolated\"").expect("legacy value loads");
        assert_eq!(level, IsolationLevel::CapabilityGated);
        // New serialization uses the current name.
        assert_eq!(
            serde_json::to_string(&IsolationLevel::CapabilityGated).unwrap(),
            "\"capability_gated\""
        );
    }

    #[test]
    fn version_regression_is_rejected() {
        let prev = ActionMetadata::new(action_key!("a.one"), "A", "desc").with_version(2, 1);
        let next = ActionMetadata::new(action_key!("a.one"), "A", "desc").with_version(2, 0);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::VersionRegressed { .. })
        ));
    }

    // ── output_schema field ─────────────────────────────────────────────

    /// Fixture output type with a named field so the schema is non-empty and
    /// the assertion `field_present` is non-vacuous (not just `is_empty()==false`).
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct EchoOutput {
        message: String,
    }

    impl nebula_schema::HasSchema for EchoOutput {
        fn schema() -> ValidSchema {
            use nebula_schema::{FieldCollector, Schema, field_key};
            Schema::builder()
                .string(
                    field_key!("message"),
                    nebula_schema::StringBuilder::required,
                )
                .build()
                .expect("EchoOutput schema is valid")
        }
    }

    // Minimal Action impl used only to drive `for_action` / factory tests.
    struct EchoAction;

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct EchoInput {
        text: String,
    }

    impl nebula_schema::HasSchema for EchoInput {
        fn schema() -> ValidSchema {
            use nebula_schema::{FieldCollector, Schema, field_key};
            Schema::builder()
                .string(field_key!("text"), nebula_schema::StringBuilder::required)
                .build()
                .expect("EchoInput schema is valid")
        }
    }

    impl crate::action::Action for EchoAction {
        type Input = EchoInput;
        type Output = EchoOutput;

        fn metadata() -> ActionMetadata {
            ActionMetadata::for_action::<Self>(action_key!("test.echo"), "Echo", "Echoes input")
        }

        fn dependencies() -> &'static nebula_core::Dependencies {
            use std::sync::OnceLock;
            static DEPS: OnceLock<nebula_core::Dependencies> = OnceLock::new();
            DEPS.get_or_init(nebula_core::Dependencies::new)
        }
    }

    #[test]
    fn new_initialises_output_schema_to_empty() {
        let meta = ActionMetadata::new(action_key!("test"), "T", "d");
        assert!(
            meta.output_schema.fields().is_empty(),
            "output_schema must default to empty"
        );
    }

    #[test]
    fn with_output_schema_builder_stores_schema() {
        use nebula_schema::{FieldCollector, Schema, field_key};
        let schema = Schema::builder()
            .string(
                field_key!("message"),
                nebula_schema::StringBuilder::required,
            )
            .build()
            .unwrap();
        let meta = ActionMetadata::new(action_key!("test"), "T", "d").with_output_schema(schema);
        assert!(
            !meta.output_schema.fields().is_empty(),
            "output_schema must store the provided schema"
        );
        assert!(
            meta.output_schema
                .fields()
                .iter()
                .any(|f| f.key().as_str() == "message"),
            "output_schema must contain the `message` field"
        );
    }

    #[test]
    fn for_action_stamps_output_schema_from_action_output_type() {
        let meta = ActionMetadata::for_action::<EchoAction>(
            action_key!("test.echo"),
            "Echo",
            "Echoes input",
        );
        // Non-vacuous: assert the `message` field is present in the output schema.
        assert!(
            meta.output_schema
                .fields()
                .iter()
                .any(|f| f.key().as_str() == "message"),
            "for_action must stamp output_schema from A::Output — `message` field missing"
        );
    }

    #[test]
    fn output_schema_back_compat_missing_field_deserializes_to_empty() {
        // Metadata serialized before `output_schema` existed must still load,
        // defaulting the field to an empty schema — mirrors kind_backward_compat_without_field.
        let legacy = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc");
        let mut as_value: serde_json::Value = serde_json::to_value(&legacy).unwrap();
        as_value
            .as_object_mut()
            .unwrap()
            .remove("output_schema")
            .expect("output_schema must be present after serialize");
        let json_string = serde_json::to_string(&as_value).unwrap();
        let decoded: ActionMetadata = serde_json::from_str(&json_string)
            .expect("legacy metadata without output_schema must deserialize");
        assert!(
            decoded.output_schema.fields().is_empty(),
            "missing output_schema field should default to empty"
        );
    }

    #[test]
    fn output_schema_serde_roundtrip() {
        use nebula_schema::{FieldCollector, Schema, field_key};
        let schema = Schema::builder()
            .string(field_key!("result"), nebula_schema::StringBuilder::required)
            .build()
            .unwrap();
        let original =
            ActionMetadata::new(action_key!("test"), "T", "d").with_output_schema(schema);
        let json = serde_json::to_string(&original).expect("serialize succeeds");
        let decoded: ActionMetadata = serde_json::from_str(&json).expect("deserialize succeeds");
        assert_eq!(original, decoded);
    }
}
