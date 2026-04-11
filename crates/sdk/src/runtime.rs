//! In-process test runtime for driving a single action end-to-end.
//!
//! [`TestRuntime`] mirrors the shape of [`nebula_runtime::ActionRegistry`]'s
//! `register_*` API: one method per action kind. Instead of building a full
//! registry/executor stack, it wraps the action in the matching adapter,
//! constructs the context through [`TestContextBuilder`], and drives the full
//! lifecycle — a single `execute` for stateless, an iteration loop for
//! stateful, a spawn-cancel window for poll triggers, or a single fake event
//! dispatch for webhook triggers.
//!
//! # Example
//!
//! ```rust,no_run
//! # use nebula_sdk::runtime::TestRuntime;
//! # use nebula_action::testing::TestContextBuilder;
//! # use serde_json::json;
//! # async fn demo<A>(action: A) -> anyhow::Result<()>
//! # where A: nebula_action::stateful::StatefulAction + Send + Sync + 'static,
//! #       A::Input: serde::de::DeserializeOwned + Send + Sync,
//! #       A::Output: serde::Serialize + Send + Sync,
//! #       A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
//! # {
//! let ctx = TestContextBuilder::new().with_input(json!({"limit": 30}));
//! let report = TestRuntime::new(ctx).run_stateful(action).await?;
//! println!("{} iterations, note = {:?}", report.iterations, report.note);
//! # Ok(()) }
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use nebula_action::{
    ActionError, ActionResult, BreakReason, IncomingEvent, PollAction, PollTriggerAdapter,
    StatefulAction, StatefulActionAdapter, StatefulHandler, StatelessAction,
    StatelessActionAdapter, StatelessHandler, TestContextBuilder, TriggerHandler, WebhookAction,
    WebhookTriggerAdapter,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Default safety cap on stateful iteration loops.
const DEFAULT_STATEFUL_CAP: u32 = 1000;

/// Default window for trigger runs if not explicitly set.
const DEFAULT_TRIGGER_WINDOW: Duration = Duration::from_secs(2);

/// Grace period for `start()` to exit after cancellation in poll runs.
const TRIGGER_STOP_GRACE: Duration = Duration::from_secs(5);

/// Structured outcome of a [`TestRuntime`] run.
///
/// All six fields are populated for every run; unused ones are empty rather
/// than absent so example/test code can format uniformly.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// Which kind of action was run: `"stateless"`, `"stateful"`,
    /// `"trigger:poll"`, or `"trigger:webhook"`.
    pub kind: &'static str,
    /// Primary output — for stateless/stateful this is the final `ActionResult`
    /// output; for triggers this is a JSON array of emitted execution payloads.
    pub output: Value,
    /// Number of iterations (1 for stateless, loop count for stateful,
    /// emitted count for triggers).
    pub iterations: u32,
    /// Wall-clock duration of the run.
    pub duration: Duration,
    /// Raw emitted execution payloads captured from triggers (empty for
    /// non-trigger runs).
    pub emitted: Vec<Value>,
    /// Optional note (break reason, cap hit, trigger start error, etc.).
    pub note: Option<String>,
}

/// In-process harness for running a single action through its full lifecycle.
///
/// Owns a [`TestContextBuilder`] (which carries input, credentials, resources,
/// and spy logger) plus two knobs specific to harness execution —
/// `stateful_cap` and `trigger_window`.
///
/// Each `run_*` method is a terminal operation: it consumes `self`, builds the
/// context, wraps the action in the matching adapter, and drives the lifecycle.
pub struct TestRuntime {
    ctx: TestContextBuilder,
    stateful_cap: u32,
    trigger_window: Duration,
}

impl TestRuntime {
    /// Create a new runtime from a prepared [`TestContextBuilder`].
    #[must_use]
    pub fn new(ctx: TestContextBuilder) -> Self {
        Self {
            ctx,
            stateful_cap: DEFAULT_STATEFUL_CAP,
            trigger_window: DEFAULT_TRIGGER_WINDOW,
        }
    }

    /// Override the max iteration count for stateful runs (default: 1000).
    #[must_use]
    pub fn with_stateful_cap(mut self, cap: u32) -> Self {
        self.stateful_cap = cap.max(1);
        self
    }

    /// Override how long poll triggers run before cancellation (default: 2s).
    #[must_use]
    pub fn with_trigger_window(mut self, window: Duration) -> Self {
        self.trigger_window = window;
        self
    }

    /// Run a stateless action: one `execute` call, return its output.
    ///
    /// # Errors
    ///
    /// Propagates [`ActionError`] from the action itself.
    pub async fn run_stateless<A>(self, action: A) -> Result<RunReport, ActionError>
    where
        A: StatelessAction + Send + Sync + 'static,
        A::Input: DeserializeOwned + Send + Sync,
        A::Output: Serialize + Send + Sync,
    {
        let input = self.ctx.input().cloned().unwrap_or(Value::Null);
        let ctx = self.ctx.build();
        let handler = StatelessActionAdapter::new(action);
        let start = Instant::now();
        let result = handler.execute(input, &ctx).await?;
        Ok(RunReport {
            kind: "stateless",
            output: extract_output(&result),
            iterations: 1,
            duration: start.elapsed(),
            emitted: Vec::new(),
            note: None,
        })
    }

    /// Run a stateful action: loop `execute` until `Break` or the safety cap.
    ///
    /// # Errors
    ///
    /// Propagates [`ActionError`] from `init_state` or any iteration.
    pub async fn run_stateful<A>(self, action: A) -> Result<RunReport, ActionError>
    where
        A: StatefulAction + Send + Sync + 'static,
        A::Input: DeserializeOwned + Send + Sync,
        A::Output: Serialize + Send + Sync,
        A::State: Serialize + DeserializeOwned + Clone + Send + Sync,
    {
        let input = self.ctx.input().cloned().unwrap_or(Value::Null);
        let cap = self.stateful_cap;
        let ctx = self.ctx.build();
        let handler = StatefulActionAdapter::new(action);
        let mut state = handler.init_state()?;
        let start = Instant::now();
        let mut iterations = 0u32;

        loop {
            iterations += 1;
            let result = handler.execute(&input, &mut state, &ctx).await?;
            let output = extract_output(&result);

            match result {
                ActionResult::Continue { .. } => {
                    if iterations >= cap {
                        return Ok(RunReport {
                            kind: "stateful",
                            output,
                            iterations,
                            duration: start.elapsed(),
                            emitted: Vec::new(),
                            note: Some(format!("hit stateful cap {cap}")),
                        });
                    }
                }
                ActionResult::Break { reason, .. } => {
                    return Ok(RunReport {
                        kind: "stateful",
                        output,
                        iterations,
                        duration: start.elapsed(),
                        emitted: Vec::new(),
                        note: Some(format_break_reason(&reason)),
                    });
                }
                other => {
                    return Ok(RunReport {
                        kind: "stateful",
                        output,
                        iterations,
                        duration: start.elapsed(),
                        emitted: Vec::new(),
                        note: Some(format!("non-iterative result: {other:?}")),
                    });
                }
            }
        }
    }

    /// Run a poll trigger: spawn `start()`, sleep the configured window,
    /// cancel, and return everything captured by the spy emitter.
    ///
    /// # Errors
    ///
    /// Never returns an error from the trigger itself — instead captures any
    /// start-loop failure into the `note` field of the report.
    pub async fn run_poll<A>(self, action: A) -> Result<RunReport, ActionError>
    where
        A: PollAction + Send + Sync + 'static,
        A::Cursor: Send + Sync,
        A::Event: Send + Sync,
    {
        let window = self.trigger_window;
        let (ctx, spy, _scheduler) = self.ctx.build_trigger();
        let handler: Arc<dyn TriggerHandler> = Arc::new(PollTriggerAdapter::new(action));
        let cancel = ctx.cancellation.clone();
        let start = Instant::now();

        let start_handle = {
            let handler = handler.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move { handler.start(&ctx).await })
        };

        tokio::time::sleep(window).await;
        cancel.cancel();

        let start_outcome = tokio::time::timeout(TRIGGER_STOP_GRACE, start_handle).await;
        let _ = handler.stop(&ctx).await;

        let emitted = spy.emitted();
        let count = emitted.len() as u32;

        let note = match start_outcome {
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(e))) => Some(format!("start() returned error: {e}")),
            Ok(Err(join_err)) => Some(format!("start task panicked: {join_err}")),
            Err(_) => Some("trigger did not exit within grace period".to_owned()),
        };

        Ok(RunReport {
            kind: "trigger:poll",
            output: Value::Array(emitted.clone()),
            iterations: count,
            duration: start.elapsed(),
            emitted,
            note,
        })
    }

    /// Run a webhook trigger with a single fake incoming event.
    ///
    /// Sequence: `start()` → `handle_event(event)` → `stop()`. Returns whatever
    /// the spy emitter captured during `handle_event`.
    ///
    /// # Errors
    ///
    /// Propagates any [`ActionError`] from `start`, `handle_event`, or `stop`.
    pub async fn run_webhook<A>(
        self,
        action: A,
        event: IncomingEvent,
    ) -> Result<RunReport, ActionError>
    where
        A: WebhookAction + Send + Sync + 'static,
        <A as WebhookAction>::State: Send + Sync,
    {
        let (ctx, spy, _scheduler) = self.ctx.build_trigger();
        let handler: Arc<dyn TriggerHandler> = Arc::new(WebhookTriggerAdapter::new(action));
        let start = Instant::now();

        handler.start(&ctx).await?;
        let outcome = handler.handle_event(event, &ctx).await?;
        handler.stop(&ctx).await?;

        let emitted = spy.emitted();

        Ok(RunReport {
            kind: "trigger:webhook",
            output: serde_json::json!({
                "outcome": format!("{outcome:?}"),
                "emitted": emitted.clone(),
            }),
            iterations: 1,
            duration: start.elapsed(),
            emitted,
            note: None,
        })
    }
}

fn extract_output(result: &ActionResult<Value>) -> Value {
    match result {
        ActionResult::Success { output }
        | ActionResult::Continue { output, .. }
        | ActionResult::Break { output, .. } => output.as_value().cloned().unwrap_or(Value::Null),
        other => serde_json::json!(format!("{other:?}")),
    }
}

fn format_break_reason(reason: &BreakReason) -> String {
    match reason {
        BreakReason::Completed => "Completed".to_owned(),
        BreakReason::MaxIterations => "MaxIterations".to_owned(),
        BreakReason::ConditionMet => "ConditionMet".to_owned(),
        BreakReason::Custom(s) => format!("Custom({s})"),
        other => format!("{other:?}"),
    }
}
