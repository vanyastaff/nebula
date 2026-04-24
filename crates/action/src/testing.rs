//! Test utilities for action authors.
#![allow(missing_docs)]

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use nebula_core::{
    CoreError, CredentialKey, ResourceKey,
    accessor::{CredentialAccessor, Logger, ResourceAccessor},
    node_key,
};
use nebula_credential::CredentialSnapshot;
use tokio_util::sync::CancellationToken;

use crate::{
    capability::{ExecutionEmitter, TriggerScheduler},
    error::ActionError,
    result::ActionResult,
    stateful::StatefulAction,
    trigger::TriggerAction,
};

type ResourceFactory = Arc<dyn Fn() -> Box<dyn Any + Send + Sync> + Send + Sync>;
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// Test contexts are type aliases over the concrete runtime contexts so action
// authors test against the same `ActionRuntimeContext` / `TriggerRuntimeContext`
// shapes the engine supplies in production. Duplicating a parallel "test"
// concrete type drifts — callers can still set identity, scope, and
// capabilities via `TestContextBuilder` or the `.with_*` builders.
pub type TestActionContext = crate::context::ActionRuntimeContext;
pub type TestTriggerContext = crate::context::TriggerRuntimeContext;

pub struct TestContextBuilder {
    credentials: HashMap<String, CredentialSnapshot>,
    typed_credentials: HashMap<TypeId, CredentialSnapshot>,
    resources: HashMap<String, ResourceFactory>,
    input: Option<serde_json::Value>,
    logs: Arc<SpyLogger>,
}

impl TestContextBuilder {
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

    #[must_use]
    pub fn minimal() -> Self {
        Self::new()
    }

    #[must_use]
    pub fn with_credential_snapshot(
        mut self,
        key: impl Into<String>,
        snapshot: CredentialSnapshot,
    ) -> Self {
        self.credentials.insert(key.into(), snapshot);
        self
    }

    #[must_use]
    pub fn with_credential<S>(mut self, scheme: S) -> Self
    where
        S: nebula_credential::AuthScheme + Clone + Send + Sync + 'static,
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

    #[must_use]
    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = Some(input);
        self
    }

    #[must_use]
    pub fn input(&self) -> Option<&serde_json::Value> {
        self.input.as_ref()
    }

    #[must_use]
    pub fn spy_logger(&self) -> Arc<SpyLogger> {
        Arc::clone(&self.logs)
    }

    #[must_use]
    pub fn build(self) -> crate::context::ActionRuntimeContext {
        use nebula_core::id::{ExecutionId, WorkflowId};
        let base = Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(CancellationToken::new())
                .build(),
        );
        crate::context::ActionRuntimeContext::new(
            base,
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        )
        .with_resources(Arc::new(TestResourceAccessor {
            resources: Arc::new(parking_lot::Mutex::new(self.resources)),
        }))
        .with_credentials(Arc::new(TestCredentialAccessor {
            credentials: self.credentials,
            typed_credentials: self.typed_credentials,
        }))
        .with_logger(self.logs)
    }

    #[must_use]
    pub fn build_trigger(
        self,
    ) -> (
        crate::context::TriggerRuntimeContext,
        Arc<SpyEmitter>,
        Arc<SpyScheduler>,
    ) {
        use nebula_core::id::WorkflowId;
        let emitter = Arc::new(SpyEmitter::new());
        let scheduler = Arc::new(SpyScheduler::new());
        let base = Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(CancellationToken::new())
                .build(),
        );
        let ctx =
            crate::context::TriggerRuntimeContext::new(base, WorkflowId::new(), node_key!("test"))
                .with_resources(Arc::new(TestResourceAccessor {
                    resources: Arc::new(parking_lot::Mutex::new(self.resources)),
                }))
                .with_credentials(Arc::new(TestCredentialAccessor {
                    credentials: self.credentials,
                    typed_credentials: self.typed_credentials,
                }))
                .with_logger(self.logs)
                .with_scheduler(Arc::clone(&scheduler) as Arc<dyn TriggerScheduler>)
                .with_emitter(Arc::clone(&emitter) as Arc<dyn ExecutionEmitter>);
        (ctx, emitter, scheduler)
    }
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SpyLogger {
    entries: parking_lot::Mutex<Vec<(nebula_core::accessor::LogLevel, String)>>,
}

impl SpyLogger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: parking_lot::Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn entries(&self) -> Vec<(nebula_core::accessor::LogLevel, String)> {
        self.entries.lock().clone()
    }

    #[must_use]
    pub fn messages(&self) -> Vec<String> {
        self.entries
            .lock()
            .iter()
            .map(|(_, msg)| msg.clone())
            .collect()
    }

    #[must_use]
    pub fn contains(&self, substring: &str) -> bool {
        self.entries
            .lock()
            .iter()
            .any(|(_, message)| message.contains(substring))
    }

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

impl Logger for SpyLogger {
    fn log(&self, level: nebula_core::accessor::LogLevel, message: &str) {
        self.entries.lock().push((level, message.to_owned()));
    }

    fn log_with_fields(
        &self,
        level: nebula_core::accessor::LogLevel,
        message: &str,
        fields: &[(&str, &str)],
    ) {
        if fields.is_empty() {
            self.log(level, message);
            return;
        }
        let suffix = fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        self.log(level, &format!("{message} {suffix}"));
    }
}

struct TestCredentialAccessor {
    credentials: HashMap<String, CredentialSnapshot>,
    typed_credentials: HashMap<TypeId, CredentialSnapshot>,
}

impl CredentialAccessor for TestCredentialAccessor {
    fn has(&self, key: &CredentialKey) -> bool {
        self.credentials.contains_key(key.as_str())
            || self.typed_credentials.values().any(|snapshot| {
                let scheme_lower = snapshot.scheme_pattern().to_lowercase();
                key.as_str() == scheme_lower
            })
    }

    fn resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
        let key_str = key.as_str();
        if let Some(snapshot) = self.credentials.get(key_str) {
            let snapshot = snapshot.clone();
            return Box::pin(async move { Ok(Box::new(snapshot) as Box<dyn Any + Send + Sync>) });
        }
        for snapshot in self.typed_credentials.values() {
            let scheme_lower = snapshot.scheme_pattern().to_lowercase();
            if key_str == scheme_lower || key_str.ends_with(&scheme_lower) {
                let snapshot = snapshot.clone();
                return Box::pin(
                    async move { Ok(Box::new(snapshot) as Box<dyn Any + Send + Sync>) },
                );
            }
        }
        let key_owned = key_str.to_owned();
        Box::pin(async move { Err(CoreError::CredentialNotFound { key: key_owned }) })
    }

    fn try_resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        let key_str = key.as_str();
        if let Some(snapshot) = self.credentials.get(key_str) {
            let snapshot = snapshot.clone();
            return Box::pin(
                async move { Ok(Some(Box::new(snapshot) as Box<dyn Any + Send + Sync>)) },
            );
        }
        for snapshot in self.typed_credentials.values() {
            let scheme_lower = snapshot.scheme_pattern().to_lowercase();
            if key_str == scheme_lower || key_str.ends_with(&scheme_lower) {
                let snapshot = snapshot.clone();
                return Box::pin(async move {
                    Ok(Some(Box::new(snapshot) as Box<dyn Any + Send + Sync>))
                });
            }
        }
        Box::pin(async { Ok(None) })
    }
}

struct TestResourceAccessor {
    resources: Arc<parking_lot::Mutex<HashMap<String, ResourceFactory>>>,
}

impl ResourceAccessor for TestResourceAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.resources.lock().contains_key(key.as_str())
    }

    fn acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
        let Some(factory) = self.resources.lock().get(key.as_str()).cloned() else {
            let missing = key.as_str().to_owned();
            return Box::pin(async move {
                Err(CoreError::CredentialNotConfigured(format!(
                    "resource `{missing}` not found in TestResourceAccessor"
                )))
            });
        };
        Box::pin(async move { Ok(factory()) })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        let maybe_factory = self.resources.lock().get(key.as_str()).cloned();
        Box::pin(async move { Ok(maybe_factory.map(|factory| factory())) })
    }
}

pub struct SpyEmitter {
    emitted: parking_lot::Mutex<Vec<serde_json::Value>>,
}

impl SpyEmitter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            emitted: parking_lot::Mutex::new(Vec::new()),
        }
    }
    #[must_use]
    pub fn emitted(&self) -> Vec<serde_json::Value> {
        self.emitted.lock().clone()
    }
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

impl ExecutionEmitter for SpyEmitter {
    fn emit(
        &self,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<nebula_core::ExecutionId, ActionError>> + Send + '_>>
    {
        self.emitted.lock().push(input);
        Box::pin(async { Ok(nebula_core::ExecutionId::new()) })
    }
}

pub struct SpyScheduler {
    scheduled: parking_lot::Mutex<Vec<Duration>>,
}

impl SpyScheduler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            scheduled: parking_lot::Mutex::new(Vec::new()),
        }
    }
    #[must_use]
    pub fn scheduled(&self) -> Vec<Duration> {
        self.scheduled.lock().clone()
    }
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

impl TriggerScheduler for SpyScheduler {
    fn schedule_after(
        &self,
        delay: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + '_>> {
        self.scheduled.lock().push(delay);
        Box::pin(async { Ok(()) })
    }
}

pub struct StatefulTestHarness<A: StatefulAction> {
    action: A,
    state: serde_json::Value,
    ctx: TestActionContext,
    iterations: u32,
}

impl<A> StatefulTestHarness<A>
where
    A: StatefulAction + Send + Sync + 'static,
    A::Input: Send + Sync,
    A::Output: Send + Sync,
    A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
{
    pub fn new(action: A, ctx: TestActionContext) -> Result<Self, ActionError> {
        let state = serde_json::to_value(action.init_state())
            .map_err(|e| ActionError::fatal(format!("init_state serialize: {e}")))?;
        Ok(Self {
            action,
            state,
            ctx,
            iterations: 0,
        })
    }

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

    pub fn state<S: serde::de::DeserializeOwned>(&self) -> Result<S, serde_json::Error> {
        serde_json::from_value(self.state.clone())
    }

    #[must_use]
    pub fn state_json(&self) -> &serde_json::Value {
        &self.state
    }

    #[must_use]
    pub fn iterations(&self) -> u32 {
        self.iterations
    }
}

pub struct TriggerTestHarness<A: TriggerAction> {
    action: A,
    ctx: TestTriggerContext,
    emitter: Arc<SpyEmitter>,
    scheduler: Arc<SpyScheduler>,
}

impl<A> TriggerTestHarness<A>
where
    A: TriggerAction + Send + Sync + 'static,
{
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

    pub async fn start(&self) -> Result<(), ActionError> {
        self.action.start(&self.ctx).await
    }

    pub async fn stop(&self) -> Result<(), ActionError> {
        self.action.stop(&self.ctx).await
    }

    #[must_use]
    pub fn emitted(&self) -> Vec<serde_json::Value> {
        self.emitter.emitted()
    }

    #[must_use]
    pub fn scheduled(&self) -> Vec<Duration> {
        self.scheduler.scheduled()
    }

    #[must_use]
    pub fn emit_count(&self) -> usize {
        self.emitter.count()
    }

    #[must_use]
    pub fn schedule_count(&self) -> usize {
        self.scheduler.count()
    }

    #[must_use]
    pub fn context(&self) -> &TestTriggerContext {
        &self.ctx
    }
}
