//! Test utilities for action authors.
//!
//! Provides [`TestContextBuilder`] for constructing [`ActionContext`] in tests
//! without needing real credential/resource providers.
//!
//! Also provides [`StatefulTestHarness`] and [`TriggerTestHarness`] for
//! stepping through stateful and trigger actions in isolation.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::{
    id::{ExecutionId, WorkflowId},
    node_key,
};
use nebula_credential::{CredentialAccessError, CredentialAccessor, CredentialSnapshot};
use tokio_util::sync::CancellationToken;

use crate::{
    capability::{
        ActionLogLevel, ActionLogger, ExecutionEmitter, ResourceAccessor, TriggerScheduler,
    },
    context::{ActionContext, TriggerContext},
    error::ActionError,
    result::ActionResult,
    stateful::StatefulAction,
    trigger::TriggerAction,
};

/// Factory that produces a fresh boxed resource on each call.
///
/// Stored instead of a raw `Box<dyn Any>` so `acquire()` can hand out
/// independent instances on every call without consuming the slot —
/// matches the semantics of production resource accessors where the
/// same key can be acquired many times.
type ResourceFactory = Arc<dyn Fn() -> Box<dyn Any + Send + Sync> + Send + Sync>;

/// Builder for creating test [`ActionContext`] instances.
///
/// Supports credential snapshots (string-keyed), type-based credentials,
/// resources, input data, and spy logging.
///
/// # Examples
///
/// ```rust,no_run
/// # use nebula_credential::CredentialSnapshot;
/// # use nebula_action::testing::TestContextBuilder;
/// # let snapshot: CredentialSnapshot = todo!();
/// let ctx = TestContextBuilder::new()
///     .with_credential_snapshot("api_key", snapshot)
///     .build();
///
/// // Minimal context with no configuration:
/// let ctx = TestContextBuilder::minimal().build();
/// ```
pub struct TestContextBuilder {
    credentials: HashMap<String, CredentialSnapshot>,
    typed_credentials: HashMap<TypeId, CredentialSnapshot>,
    resources: HashMap<String, ResourceFactory>,
    input: Option<serde_json::Value>,
    logs: Arc<SpyLogger>,
}

impl TestContextBuilder {
    /// Create a new test context builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            credentials: HashMap::new(),
            typed_credentials: HashMap::new(),
            resources: HashMap::new(),
            input: None,
            logs: Arc::new(SpyLogger::new()),
        }
    }

    /// Create a minimal context with no configuration needed.
    ///
    /// Equivalent to [`Self::new()`], provided for discoverability.
    #[must_use]
    pub fn minimal() -> Self {
        Self::new()
    }

    /// Add a typed credential snapshot for testing.
    ///
    /// The credential is stored as a [`CredentialSnapshot`] and returned
    /// by the test credential accessor when requested by `key`.
    #[must_use]
    pub fn with_credential_snapshot(
        mut self,
        key: impl Into<String>,
        snapshot: CredentialSnapshot,
    ) -> Self {
        self.credentials.insert(key.into(), snapshot);
        self
    }

    /// Add a type-based credential for testing.
    ///
    /// The scheme is stored by [`TypeId`] and returned when
    /// `ActionContext::credential_typed` is called with the same type.
    #[must_use]
    pub fn with_credential<S>(mut self, scheme: S) -> Self
    where
        S: nebula_core::AuthScheme + Clone + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<S>();
        let snapshot = CredentialSnapshot::new(
            std::any::type_name::<S>(),
            nebula_credential::CredentialRecord::new(),
            scheme,
        );
        self.typed_credentials.insert(type_id, snapshot);
        self
    }

    /// Add a resource by key.
    ///
    /// The resource value is stored as a cloneable factory. Each call to
    /// [`ActionContext::resource`] for this key returns a fresh `Box<R>`
    /// — this matches production `ResourceAccessor` semantics where the
    /// same key can be acquired multiple times, and prevents tests from
    /// passing against the harness then failing in production on a
    /// second acquire.
    ///
    /// `R` must be `Clone + Send + Sync + 'static`.
    #[must_use]
    pub fn with_resource<R>(mut self, key: impl Into<String>, resource: R) -> Self
    where
        R: Clone + Send + Sync + 'static,
    {
        let factory: ResourceFactory =
            Arc::new(move || Box::new(resource.clone()) as Box<dyn Any + Send + Sync>);
        self.resources.insert(key.into(), factory);
        self
    }

    /// Set input data for the test.
    ///
    /// Input is stored on the builder and available via [`Self::input()`].
    /// Call this before [`Self::build()`], then pass the input separately
    /// to the action's `execute` method — input is **not** embedded in the
    /// built [`ActionContext`].
    #[must_use]
    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = Some(input);
        self
    }

    /// Get the configured input data, if any.
    #[must_use]
    pub fn input(&self) -> Option<&serde_json::Value> {
        self.input.as_ref()
    }

    /// Get the spy logger for checking logged messages after execution.
    #[must_use]
    pub fn spy_logger(&self) -> Arc<SpyLogger> {
        Arc::clone(&self.logs)
    }

    /// Build the test context.
    #[must_use]
    pub fn build(self) -> ActionContext {
        ActionContext::new(
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor {
            credentials: self.credentials,
            typed_credentials: self.typed_credentials,
        }))
        .with_resources(Arc::new(TestResourceAccessor {
            resources: Arc::new(parking_lot::Mutex::new(self.resources)),
        }))
        .with_logger(self.logs)
    }

    /// Build a [`TriggerContext`] for testing trigger actions.
    ///
    /// Returns the context plus [`SpyEmitter`] and [`SpyScheduler`] for
    /// inspecting emitted executions and scheduled delays.
    #[must_use]
    pub fn build_trigger(self) -> (TriggerContext, Arc<SpyEmitter>, Arc<SpyScheduler>) {
        let emitter = Arc::new(SpyEmitter::new());
        let scheduler = Arc::new(SpyScheduler::new());
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
            CancellationToken::new(),
        )
        .with_credentials(Arc::new(TestCredentialAccessor {
            credentials: self.credentials,
            typed_credentials: self.typed_credentials,
        }))
        .with_emitter(Arc::clone(&emitter) as Arc<dyn ExecutionEmitter>)
        .with_scheduler(Arc::clone(&scheduler) as Arc<dyn TriggerScheduler>)
        .with_logger(self.logs);
        (ctx, emitter, scheduler)
    }
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Logger that captures log entries for test assertions.
pub struct SpyLogger {
    entries: parking_lot::Mutex<Vec<(ActionLogLevel, String)>>,
}

impl SpyLogger {
    /// Create a new spy logger.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get all logged messages (level and text).
    #[must_use]
    pub fn entries(&self) -> Vec<(ActionLogLevel, String)> {
        self.entries.lock().clone()
    }

    /// Get only the message strings.
    #[must_use]
    pub fn messages(&self) -> Vec<String> {
        self.entries.lock().iter().map(|(_, m)| m.clone()).collect()
    }

    /// Check if any entry contains the given substring.
    #[must_use]
    pub fn contains(&self, substring: &str) -> bool {
        self.entries
            .lock()
            .iter()
            .any(|(_, m)| m.contains(substring))
    }

    /// Number of log entries.
    #[must_use]
    pub fn count(&self) -> usize {
        self.entries.lock().len()
    }
}

impl Default for SpyLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SpyLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpyLogger")
            .field("count", &self.count())
            .finish()
    }
}

impl ActionLogger for SpyLogger {
    fn log(&self, level: ActionLogLevel, message: &str) {
        self.entries.lock().push((level, message.to_owned()));
    }
}

struct TestCredentialAccessor {
    credentials: HashMap<String, CredentialSnapshot>,
    typed_credentials: HashMap<TypeId, CredentialSnapshot>,
}

#[async_trait]
impl CredentialAccessor for TestCredentialAccessor {
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
        self.credentials.get(id).cloned().ok_or_else(|| {
            CredentialAccessError::NotFound(format!("credential `{id}` not found in test context"))
        })
    }

    async fn has(&self, id: &str) -> bool {
        self.credentials.contains_key(id)
    }

    async fn get_by_type(
        &self,
        type_id: TypeId,
        type_name: &str,
    ) -> Result<CredentialSnapshot, CredentialAccessError> {
        self.typed_credentials
            .get(&type_id)
            .cloned()
            .ok_or_else(|| {
                CredentialAccessError::NotFound(format!(
                    "no typed credential for `{type_name}` in test context"
                ))
            })
    }
}

struct TestResourceAccessor {
    resources: Arc<parking_lot::Mutex<HashMap<String, ResourceFactory>>>,
}

#[async_trait]
impl ResourceAccessor for TestResourceAccessor {
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        // Factory is cloned out under the lock (cheap Arc clone) and
        // invoked outside to keep the lock critical section tiny and
        // side-effect-free. Multiple acquires for the same key each
        // get a fresh Box — matches production accessor semantics.
        let factory = self.resources.lock().get(key).cloned().ok_or_else(|| {
            ActionError::fatal(format!("resource `{key}` not found in test context"))
        })?;
        Ok(factory())
    }

    async fn exists(&self, key: &str) -> bool {
        self.resources.lock().contains_key(key)
    }
}

// ── Spy capabilities for trigger testing ────────────────────────────────────

/// Spy emitter that captures emitted executions for test assertions.
pub struct SpyEmitter {
    emitted: parking_lot::Mutex<Vec<serde_json::Value>>,
}

impl SpyEmitter {
    /// Create a new spy emitter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            emitted: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get all emitted inputs.
    #[must_use]
    pub fn emitted(&self) -> Vec<serde_json::Value> {
        self.emitted.lock().clone()
    }

    /// Number of emitted executions.
    #[must_use]
    pub fn count(&self) -> usize {
        self.emitted.lock().len()
    }
}

impl Default for SpyEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SpyEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpyEmitter")
            .field("count", &self.count())
            .finish()
    }
}

#[async_trait]
impl ExecutionEmitter for SpyEmitter {
    async fn emit(&self, input: serde_json::Value) -> Result<ExecutionId, ActionError> {
        self.emitted.lock().push(input);
        Ok(ExecutionId::new())
    }
}

/// Spy scheduler that captures scheduled delays for test assertions.
pub struct SpyScheduler {
    scheduled: parking_lot::Mutex<Vec<Duration>>,
}

impl SpyScheduler {
    /// Create a new spy scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scheduled: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get all scheduled delays.
    #[must_use]
    pub fn scheduled(&self) -> Vec<Duration> {
        self.scheduled.lock().clone()
    }

    /// Number of scheduled delays.
    #[must_use]
    pub fn count(&self) -> usize {
        self.scheduled.lock().len()
    }
}

impl Default for SpyScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SpyScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpyScheduler")
            .field("count", &self.count())
            .finish()
    }
}

#[async_trait]
impl TriggerScheduler for SpyScheduler {
    async fn schedule_after(&self, delay: Duration) -> Result<(), ActionError> {
        self.scheduled.lock().push(delay);
        Ok(())
    }
}

// ── StatefulTestHarness ─────────────────────────────────────────────────────

/// Test harness for stepping through [`StatefulAction`] iterations.
///
/// Manages state serialization/deserialization between iterations so
/// tests can inspect intermediate state and step one iteration at a time.
///
/// # Examples
///
/// ```rust,ignore
/// let harness = StatefulTestHarness::new(my_action, ctx)?;
/// let result = harness.step(input).await?;
/// assert_eq!(harness.iterations(), 1);
/// ```
pub struct StatefulTestHarness<A: StatefulAction> {
    action: A,
    state: serde_json::Value,
    ctx: ActionContext,
    iterations: u32,
}

impl<A> StatefulTestHarness<A>
where
    A: StatefulAction + Send + Sync + 'static,
    A::Input: Send + Sync,
    A::Output: Send + Sync,
    A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
{
    /// Create a new harness with initial state from the action.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] if `init_state()` produces a value
    /// that cannot be serialized to JSON.
    pub fn new(action: A, ctx: ActionContext) -> Result<Self, ActionError> {
        let state = serde_json::to_value(action.init_state())
            .map_err(|e| ActionError::fatal(format!("init_state serialize: {e}")))?;
        Ok(Self {
            action,
            state,
            ctx,
            iterations: 0,
        })
    }

    /// Run one iteration. Returns the result and updates internal state.
    ///
    /// # Errors
    ///
    /// - Returns [`ActionError::Fatal`] if state (de)serialization fails.
    /// - Propagates any error from the action's `execute` method.
    pub async fn step(&mut self, input: A::Input) -> Result<ActionResult<A::Output>, ActionError> {
        self.iterations = self.iterations.saturating_add(1);
        let mut typed_state: A::State = serde_json::from_value(self.state.clone())
            .map_err(|e| ActionError::fatal(format!("state deserialize: {e}")))?;
        let result = self
            .action
            .execute(input, &mut typed_state, &self.ctx)
            .await?;
        self.state = serde_json::to_value(&typed_state)
            .map_err(|e| ActionError::fatal(format!("state serialize: {e}")))?;
        Ok(result)
    }

    /// Get current state as a typed value.
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if the internal JSON state cannot be
    /// deserialized into `S`.
    pub fn state<S: serde::de::DeserializeOwned>(&self) -> Result<S, serde_json::Error> {
        serde_json::from_value(self.state.clone())
    }

    /// Get current state as raw JSON.
    #[must_use]
    pub fn state_json(&self) -> &serde_json::Value {
        &self.state
    }

    /// Number of iterations executed so far.
    #[must_use]
    pub fn iterations(&self) -> u32 {
        self.iterations
    }
}

impl<A> std::fmt::Debug for StatefulTestHarness<A>
where
    A: StatefulAction + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatefulTestHarness")
            .field("action", &self.action)
            .field("state", &self.state)
            .field("iterations", &self.iterations)
            .finish_non_exhaustive()
    }
}

// ── TriggerTestHarness ──────────────────────────────────────────────────────

/// Test harness for [`TriggerAction`] — captures emitted executions and
/// scheduled delays.
///
/// Wraps a trigger action with spy emitter/scheduler so tests can inspect
/// what the trigger did during `start`/`stop`.
///
/// # Examples
///
/// ```rust,ignore
/// let harness = TriggerTestHarness::new(my_trigger, ctx);
/// harness.start().await?;
/// assert_eq!(harness.emitted().len(), 1);
/// harness.stop().await?;
/// ```
pub struct TriggerTestHarness<A: TriggerAction> {
    action: A,
    ctx: TriggerContext,
    emitter: Arc<SpyEmitter>,
    scheduler: Arc<SpyScheduler>,
}

impl<A> TriggerTestHarness<A>
where
    A: TriggerAction + Send + Sync + 'static,
{
    /// Create a new trigger test harness from a builder.
    ///
    /// Uses [`TestContextBuilder::build_trigger`] internally to wire
    /// spy capabilities.
    #[must_use]
    pub fn new(action: A, builder: TestContextBuilder) -> Self {
        let (ctx, emitter, scheduler) = builder.build_trigger();
        Self {
            action,
            ctx,
            emitter,
            scheduler,
        }
    }

    /// Start the trigger.
    ///
    /// # Errors
    ///
    /// Propagates any error from the trigger's `start` method.
    pub async fn start(&self) -> Result<(), ActionError> {
        self.action.start(&self.ctx).await
    }

    /// Stop the trigger.
    ///
    /// # Errors
    ///
    /// Propagates any error from the trigger's `stop` method.
    pub async fn stop(&self) -> Result<(), ActionError> {
        self.action.stop(&self.ctx).await
    }

    /// Get all emitted execution inputs.
    #[must_use]
    pub fn emitted(&self) -> Vec<serde_json::Value> {
        self.emitter.emitted()
    }

    /// Get all scheduled delays.
    #[must_use]
    pub fn scheduled(&self) -> Vec<Duration> {
        self.scheduler.scheduled()
    }

    /// Number of emitted executions.
    #[must_use]
    pub fn emit_count(&self) -> usize {
        self.emitter.count()
    }

    /// Number of scheduled delays.
    #[must_use]
    pub fn schedule_count(&self) -> usize {
        self.scheduler.count()
    }

    /// Get a reference to the underlying trigger context.
    #[must_use]
    pub fn context(&self) -> &TriggerContext {
        &self.ctx
    }
}

impl<A> std::fmt::Debug for TriggerTestHarness<A>
where
    A: TriggerAction + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerTestHarness")
            .field("action", &self.action)
            .field("emitter", &self.emitter)
            .field("scheduler", &self.scheduler)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nebula_core::action_key;
    use nebula_credential::{CredentialRecord, SecretString, SecretToken};

    use super::*;
    #[cfg(feature = "unstable-retry-scheduler")]
    use crate::assert_retry;
    use crate::{
        action::Action,
        assert_branch, assert_break, assert_cancelled, assert_continue, assert_fatal,
        assert_retryable, assert_skip, assert_success, assert_validation_error, assert_wait,
        context::{Context, CredentialContextExt},
        dependency::ActionDependencies,
        metadata::ActionMetadata,
        output::ActionOutput,
    };

    #[test]
    fn test_context_builder_defaults() {
        let builder = TestContextBuilder::new();
        let ctx = builder.build();
        let _ = ctx.execution_id;
        let _ = ctx.node_key;
    }

    #[test]
    fn minimal_is_equivalent_to_new() {
        let ctx = TestContextBuilder::minimal().build();
        let _ = ctx.execution_id;
    }

    #[tokio::test]
    async fn test_context_builder_with_credential_snapshot() {
        let snapshot = CredentialSnapshot::new(
            "api_key",
            CredentialRecord::new(),
            SecretToken::new(SecretString::new("test-secret")),
        );

        let ctx = TestContextBuilder::new()
            .with_credential_snapshot("my_cred", snapshot)
            .build();

        assert!(ctx.has_credential_id("my_cred").await);
        assert!(!ctx.has_credential_id("other").await);

        let snap = ctx.credential_by_id("my_cred").await.unwrap();
        assert_eq!(snap.scheme_pattern(), "SecretToken");
    }

    #[tokio::test]
    async fn test_context_builder_missing_credential_returns_error() {
        let ctx = TestContextBuilder::new().build();
        let result = ctx.credential_by_id("missing").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn with_resource_provides_resource() {
        let ctx = TestContextBuilder::new()
            .with_resource("db", 42_i32)
            .build();

        assert!(ctx.has_resource("db").await);
        assert!(!ctx.has_resource("cache").await);

        let resource = ctx.resource("db").await.unwrap();
        let val = resource.downcast_ref::<i32>().unwrap();
        assert_eq!(*val, 42);
    }

    #[tokio::test]
    async fn with_resource_allows_multiple_acquires_for_same_key() {
        // Before M3 the test accessor removed the resource on first
        // acquire — a second call returned NotFound even though
        // production accessors typically hand out the same handle
        // many times. Factory-backed storage fixes this.
        let ctx = TestContextBuilder::new()
            .with_resource("db", 42_i32)
            .build();

        assert_eq!(
            *ctx.resource("db")
                .await
                .unwrap()
                .downcast_ref::<i32>()
                .unwrap(),
            42
        );
        assert_eq!(
            *ctx.resource("db")
                .await
                .unwrap()
                .downcast_ref::<i32>()
                .unwrap(),
            42
        );
        assert!(
            ctx.has_resource("db").await,
            "key must remain after first acquire"
        );
    }

    #[tokio::test]
    async fn with_resource_missing_returns_error() {
        let ctx = TestContextBuilder::minimal().build();
        let result = ctx.resource("missing").await;
        assert!(result.is_err());
    }

    #[test]
    fn with_input_stores_and_retrieves() {
        let builder = TestContextBuilder::new().with_input(serde_json::json!({"key": "value"}));
        let input = builder.input().cloned().unwrap();
        assert_eq!(input, serde_json::json!({"key": "value"}));
    }

    #[test]
    fn spy_logger_captures_messages() {
        let logger = SpyLogger::new();
        logger.log(ActionLogLevel::Info, "hello world");
        logger.log(ActionLogLevel::Error, "something failed");

        assert_eq!(logger.count(), 2);
        assert!(logger.contains("hello"));
        assert!(!logger.contains("missing"));

        let messages = logger.messages();
        assert_eq!(messages, vec!["hello world", "something failed"]);
    }

    #[test]
    fn spy_logger_shared_via_builder() {
        let builder = TestContextBuilder::new();
        let spy = builder.spy_logger();
        let ctx = builder.build();

        ctx.logger.log(ActionLogLevel::Info, "from action");

        assert_eq!(spy.count(), 1);
        assert!(spy.contains("from action"));
    }

    // ── Typed credential (with_credential<S>) ─────────────────────────

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct TestApiKey {
        key: String,
    }

    impl nebula_core::AuthScheme for TestApiKey {
        fn pattern() -> nebula_core::AuthPattern {
            nebula_core::AuthPattern::SecretToken
        }
    }

    impl zeroize::Zeroize for TestApiKey {
        fn zeroize(&mut self) {
            self.key.zeroize();
        }
    }

    #[tokio::test]
    async fn with_credential_type_based_access() {
        let ctx = TestContextBuilder::new()
            .with_credential(TestApiKey {
                key: "my-key".to_owned(),
            })
            .build();

        let guard = ctx.credential::<TestApiKey>().await.unwrap();
        assert_eq!(guard.key, "my-key");
    }

    #[tokio::test]
    async fn typed_credential_missing_returns_error() {
        let ctx = TestContextBuilder::minimal().build();
        let result = ctx.credential::<TestApiKey>().await;
        assert!(result.is_err());
    }

    // ── SpyEmitter / SpyScheduler ────────────────────────────────────

    #[tokio::test]
    async fn spy_emitter_captures_emissions() {
        let spy = SpyEmitter::new();
        let _ = spy.emit(serde_json::json!({"event": "a"})).await;
        let _ = spy.emit(serde_json::json!({"event": "b"})).await;

        assert_eq!(spy.count(), 2);
        let emitted = spy.emitted();
        assert_eq!(emitted[0], serde_json::json!({"event": "a"}));
        assert_eq!(emitted[1], serde_json::json!({"event": "b"}));
    }

    #[tokio::test]
    async fn spy_scheduler_captures_delays() {
        let spy = SpyScheduler::new();
        spy.schedule_after(Duration::from_secs(5)).await.unwrap();
        spy.schedule_after(Duration::from_millis(100))
            .await
            .unwrap();

        assert_eq!(spy.count(), 2);
        let scheduled = spy.scheduled();
        assert_eq!(scheduled[0], Duration::from_secs(5));
        assert_eq!(scheduled[1], Duration::from_millis(100));
    }

    // ── build_trigger ────────────────────────────────────────────────

    #[tokio::test]
    async fn build_trigger_provides_spy_capabilities() {
        let (ctx, emitter, scheduler) = TestContextBuilder::new().build_trigger();

        ctx.emit_execution(serde_json::json!({"x": 1}))
            .await
            .unwrap();
        ctx.schedule_after(Duration::from_secs(10)).await.unwrap();

        assert_eq!(emitter.count(), 1);
        assert_eq!(scheduler.count(), 1);
    }

    // ── Assertion macro tests ────────────────────────────────────────

    #[test]
    fn assert_success_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::success(42));
        assert_success!(result);
    }

    #[test]
    fn assert_success_macro_with_value() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::success(42));
        assert_success!(result, 42);
    }

    #[test]
    #[should_panic(expected = "expected ActionResult::Success")]
    fn assert_success_macro_panics_on_skip() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::skip("nope"));
        assert_success!(result);
    }

    #[test]
    fn assert_branch_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::Branch {
            selected: "true".into(),
            output: ActionOutput::Value(1),
            alternatives: HashMap::new(),
        });
        assert_branch!(result, "true");
    }

    #[test]
    #[should_panic(expected = "expected branch key")]
    fn assert_branch_macro_wrong_key() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::Branch {
            selected: "false".into(),
            output: ActionOutput::Value(0),
            alternatives: HashMap::new(),
        });
        assert_branch!(result, "true");
    }

    #[test]
    fn assert_continue_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::Continue {
            output: ActionOutput::Value(1),
            progress: None,
            delay: None,
        });
        assert_continue!(result);
    }

    #[test]
    fn assert_break_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::Break {
            output: ActionOutput::Value(1),
            reason: crate::result::BreakReason::Completed,
        });
        assert_break!(result);
    }

    #[test]
    fn assert_skip_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::skip("reason"));
        assert_skip!(result);
    }

    #[test]
    fn assert_wait_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::Wait {
            condition: crate::result::WaitCondition::Duration {
                duration: Duration::from_mins(1),
            },
            timeout: None,
            partial_output: None,
        });
        assert_wait!(result);
    }

    #[cfg(feature = "unstable-retry-scheduler")]
    #[test]
    fn assert_retry_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::Retry {
            after: Duration::from_secs(5),
            reason: "not ready".into(),
        });
        assert_retry!(result);
    }

    #[test]
    fn assert_retryable_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Err(ActionError::retryable("timeout"));
        assert_retryable!(result);
    }

    #[test]
    fn assert_fatal_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Err(ActionError::fatal("bad input"));
        assert_fatal!(result);
    }

    #[test]
    fn assert_validation_error_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Err(ActionError::validation(
            "email",
            crate::error::ValidationReason::MissingField,
            None::<String>,
        ));
        assert_validation_error!(result);
    }

    #[test]
    fn assert_cancelled_macro_ok() {
        let result: Result<ActionResult<i32>, ActionError> = Err(ActionError::Cancelled);
        assert_cancelled!(result);
    }

    // ── StatefulTestHarness tests ────────────────────────────────────

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct CounterState {
        count: u32,
    }

    #[derive(Debug)]
    struct CounterAction {
        meta: ActionMetadata,
        max: u32,
    }

    impl ActionDependencies for CounterAction {}

    impl Action for CounterAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatefulAction for CounterAction {
        type Input = ();
        type Output = u32;
        type State = CounterState;

        fn init_state(&self) -> Self::State {
            CounterState { count: 0 }
        }

        fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &impl Context,
        ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
            state.count += 1;
            let count = state.count;
            let max = self.max;
            async move {
                if count >= max {
                    Ok(ActionResult::Break {
                        output: ActionOutput::Value(count),
                        reason: crate::result::BreakReason::Completed,
                    })
                } else {
                    Ok(ActionResult::Continue {
                        output: ActionOutput::Value(count),
                        progress: Some(f64::from(count) / f64::from(max)),
                        delay: None,
                    })
                }
            }
        }
    }

    #[tokio::test]
    async fn stateful_harness_steps_through_iterations() {
        let action = CounterAction {
            meta: ActionMetadata::new(action_key!("test.counter"), "Counter", "Test counter"),
            max: 3,
        };
        let ctx = TestContextBuilder::minimal().build();
        let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

        assert_eq!(harness.iterations(), 0);

        let r1 = harness.step(()).await.unwrap();
        assert!(r1.is_continue());
        assert_eq!(harness.iterations(), 1);
        let s: CounterState = harness.state().unwrap();
        assert_eq!(s.count, 1);

        let r2 = harness.step(()).await.unwrap();
        assert!(r2.is_continue());
        assert_eq!(harness.iterations(), 2);

        let r3 = harness.step(()).await.unwrap();
        assert!(!r3.is_continue());
        assert_eq!(harness.iterations(), 3);
        let s: CounterState = harness.state().unwrap();
        assert_eq!(s.count, 3);
    }

    #[tokio::test]
    async fn stateful_harness_state_json() {
        let action = CounterAction {
            meta: ActionMetadata::new(action_key!("test.counter"), "Counter", "Test counter"),
            max: 10,
        };
        let ctx = TestContextBuilder::minimal().build();
        let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

        harness.step(()).await.unwrap();
        let json = harness.state_json();
        assert_eq!(json["count"], 1);
    }

    // ── TriggerTestHarness tests ─────────────────────────────────────

    #[derive(Debug)]
    struct TickTrigger {
        meta: ActionMetadata,
    }

    impl ActionDependencies for TickTrigger {}

    impl Action for TickTrigger {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl TriggerAction for TickTrigger {
        fn start(
            &self,
            ctx: &TriggerContext,
        ) -> impl Future<Output = Result<(), ActionError>> + Send {
            let emitter = Arc::clone(&ctx.emitter);
            let scheduler = Arc::clone(&ctx.scheduler);
            async move {
                emitter.emit(serde_json::json!({"tick": 1})).await?;
                scheduler.schedule_after(Duration::from_mins(1)).await?;
                Ok(())
            }
        }

        async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn trigger_harness_captures_emissions_and_schedules() {
        let trigger = TickTrigger {
            meta: ActionMetadata::new(action_key!("test.tick"), "Tick", "Test tick trigger"),
        };
        let harness = TriggerTestHarness::new(trigger, TestContextBuilder::minimal());

        harness.start().await.unwrap();

        assert_eq!(harness.emit_count(), 1);
        assert_eq!(harness.emitted()[0], serde_json::json!({"tick": 1}));
        assert_eq!(harness.schedule_count(), 1);
        assert_eq!(harness.scheduled()[0], Duration::from_mins(1));

        harness.stop().await.unwrap();
    }

    #[tokio::test]
    async fn trigger_harness_context_accessible() {
        let trigger = TickTrigger {
            meta: ActionMetadata::new(action_key!("test.tick"), "Tick", "Test tick trigger"),
        };
        let harness = TriggerTestHarness::new(trigger, TestContextBuilder::minimal());
        let _ = harness.context().workflow_id;
    }
}
