use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::id::ExecutionId;
use serde::{Deserialize, Serialize};

use crate::output::{ActionOutput, BinaryData, DataReference, DeferredOutput};

/// Type alias for workflow branch keys (e.g. `"true"`, `"false"`, `"case_1"`).
pub type BranchKey = String;

/// Type alias for action output port keys (e.g. `"main"`, `"error"`, `"filtered"`).
///
/// Canonical definition lives in [`crate::port`]; re-exported here for convenience.
pub use crate::port::PortKey;

/// Result of an action execution, carrying both data and flow-control intent.
///
/// The engine matches on this enum to decide what happens next in the workflow:
/// - `Success` → pass output to dependent nodes
/// - `Skip` → skip downstream processing (whole subgraph)
/// - `Drop` → drop this item without stopping the branch
/// - `Continue` → re-enqueue for next iteration (stateful actions)
/// - `Break` → finalize iteration (stateful actions)
/// - `Branch` → activate a specific branch path
/// - `Route` / `MultiOutput` → fan-out to output ports
/// - `Wait` → pause until external event, timer, or approval
/// - `Retry` → reserved for a future engine retry scheduler; gated behind the
///   `unstable-retry-scheduler` feature and **not** honored end-to-end (canon §11.2). The canonical
///   retry surface today is the `nebula-resilience` pipeline composed inside an action around
///   outbound calls.
/// - `Terminate` → end the whole execution explicitly (Stop / Fail nodes)
///
/// All output fields are wrapped in [`ActionOutput<T>`] to support binary,
/// reference, and stream data alongside structured values.
#[cfg_attr(
    not(feature = "unstable-retry-scheduler"),
    doc = r#"

# Feature gating

The `ActionResult::Retry` variant is hidden behind the default-off
`unstable-retry-scheduler` feature flag (canon §11.2). On default features,
consumers cannot name the variant — the following fails to compile:

```compile_fail
use nebula_action::ActionResult;
let _: ActionResult<()> = ActionResult::Retry {
    after: std::time::Duration::from_secs(1),
    reason: "gated".into(),
};
```
"#
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum ActionResult<T> {
    /// Successful completion -- engine passes output to dependent nodes.
    Success {
        /// The produced output value.
        output: ActionOutput<T>,
    },

    /// Skip this node -- engine skips downstream dependents.
    ///
    /// The *entire downstream subgraph* reachable from this node is marked
    /// skipped. Use [`Drop`](Self::Drop) if you want to discard a single item
    /// while leaving downstream processing alive for other items.
    Skip {
        /// Human-readable reason for skipping.
        reason: String,
        /// Optional output produced before the skip decision.
        output: Option<ActionOutput<T>>,
    },

    /// Drop this item from the flow without stopping downstream processing.
    ///
    /// Unlike [`Skip`](Self::Skip), which marks the entire downstream subgraph
    /// as skipped, `Drop` means "this particular item did not produce output
    /// on the main port." Downstream execution continues normally for any
    /// parallel branches, and subsequent items in a stateful iteration are
    /// processed as usual.
    ///
    /// Used by filter-style nodes that remove items without terminating
    /// the branch (n8n Filter, Node-RED `rbe`, Airflow `ShortCircuit`,
    /// Pipedream "continue workflow on condition").
    Drop {
        /// Optional human-readable reason for dropping this item.
        reason: Option<String>,
    },

    /// Stateful iteration: not yet done, need another call.
    ///
    /// Engine saves state, optionally waits `delay`, then re-invokes.
    Continue {
        /// Intermediate output for this iteration.
        output: ActionOutput<T>,
        /// Progress indicator in `0.0..=1.0` range.
        progress: Option<f64>,
        /// Optional delay before next iteration (e.g. rate limiting).
        #[serde(default, with = "duration_opt_ms")]
        delay: Option<Duration>,
    },

    /// Stateful iteration: complete.
    ///
    /// Engine finalizes state and passes output downstream.
    Break {
        /// Final output of the iteration.
        output: ActionOutput<T>,
        /// Why the iteration ended.
        reason: BreakReason,
    },

    /// Choose a workflow branch (if/else, switch).
    ///
    /// Engine activates connections matching `selected` key.
    Branch {
        /// Key of the chosen branch.
        selected: BranchKey,
        /// Output for the selected branch.
        output: ActionOutput<T>,
        /// Outputs for non-selected branches (may be used for previews).
        alternatives: HashMap<BranchKey, ActionOutput<T>>,
    },

    /// Route output to a specific output port.
    Route {
        /// Target output port key.
        port: PortKey,
        /// Data to send to the port.
        data: ActionOutput<T>,
    },

    /// Fan-out to multiple output ports simultaneously.
    ///
    /// # Downstream join semantics
    ///
    /// Downstream nodes with multiple upstream edges fire when **all** emitted
    /// output ports carry data. A port absent from the `outputs` map means
    /// "not emitted" and does not block downstream nodes connected to other
    /// emitted ports (same rule as `trigger_rule: all_success`).
    ///
    /// Authors wanting first-match-only routing should return
    /// [`Branch`](Self::Branch) or [`Route`](Self::Route) instead;
    /// `MultiOutput` expresses "multiple ports fired with data in the same
    /// dispatch."
    MultiOutput {
        /// Per-port output data.
        outputs: HashMap<PortKey, ActionOutput<T>>,
        /// Optional primary output sent to the default port.
        main_output: Option<ActionOutput<T>>,
    },

    /// Pause execution until an external condition is met.
    ///
    /// Engine persists state and resumes when the condition triggers.
    Wait {
        /// The condition that must be satisfied to resume.
        condition: WaitCondition,
        /// Maximum time to wait before the engine cancels.
        #[serde(default, with = "duration_opt_ms")]
        timeout: Option<Duration>,
        /// Partial output produced before pausing.
        partial_output: Option<ActionOutput<T>>,
    },

    /// **Unstable.** Reserved for a future engine retry scheduler.
    ///
    /// Gated behind the `unstable-retry-scheduler` feature flag. The engine
    /// does **not** honor this variant end-to-end today: there is no persisted
    /// attempt accounting, no CAS-protected counter bump, and no consumer wired
    /// through `ExecutionRepo`. Per canon §11.2 / §4.5 this variant is a
    /// `planned` capability that must be hidden until the scheduler lands.
    ///
    /// Returning this variant from a stable handler is a **logic error**: the
    /// variant is only reachable when the crate is compiled with the
    /// `unstable-retry-scheduler` feature, which is opt-in and not part of the
    /// public contract. For retry semantics today, compose
    /// [`nebula-resilience`](https://docs.rs/nebula-resilience) inside the
    /// action around the outbound call.
    ///
    /// Unlike `ActionError::Retryable`, this would be a *successful* signal
    /// that the action wants to be re-executed (e.g. upstream data not ready,
    /// rate-limit cooldown). Once the scheduler lands, the engine will
    /// re-enqueue the node after `after` elapses.
    #[cfg(feature = "unstable-retry-scheduler")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable-retry-scheduler")))]
    Retry {
        /// Suggested delay before re-execution.
        #[serde(with = "duration_ms")]
        after: Duration,
        /// Human-readable reason for requesting the retry.
        reason: String,
    },

    /// Terminate this node and signal that the execution should stop.
    ///
    /// Used by explicit termination nodes (n8n "Stop And Error",
    /// Kestra `Fail`, AWS Step Functions `Succeed`/`Fail` states,
    /// Pipedream "Exit Workflow", Make `Rollback`). Plugin authors should
    /// return `Terminate` today when they want "no more work from this
    /// branch" semantics.
    ///
    /// # v1 behaviour
    ///
    /// The engine's `evaluate_edge` treats `Terminate` the same as
    /// [`Skip`](Self::Skip): downstream edges from this node do **not**
    /// fire. That is the entirety of the current engine-side wiring.
    ///
    /// Full scheduler integration — cancelling sibling branches still in
    /// flight, propagating the [`TerminationReason`] into the audit log
    /// as `ExecutionTerminationReason::ExplicitStop` /
    /// `ExecutionTerminationReason::ExplicitFail`, and driving
    /// `determine_final_status` off the terminate signal — is tracked as
    /// Phase 3 of the ControlAction plan and is **not yet wired**. Do not
    /// rely on `Terminate` in v1 to cancel sibling branches; it only
    /// gates the local subgraph downstream of the terminating node.
    Terminate {
        /// Why the execution is ending.
        reason: TerminationReason,
    },
}

/// Why a workflow execution was explicitly terminated by a node.
///
/// Delivered via [`ActionResult::Terminate`] and recorded in the execution
/// audit log so that explicit termination is distinguishable from crashes
/// or natural completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum TerminationReason {
    /// Successful early termination — the node intentionally ended the
    /// workflow with a success outcome.
    Success {
        /// Optional note explaining why the node chose to terminate early.
        note: Option<String>,
    },
    /// Error termination — the node intentionally ended the workflow
    /// with a failure outcome.
    Failure {
        /// Opaque error code identifier.
        ///
        /// See [`TerminationCode`] — currently a thin wrapper over
        /// `Arc<str>`, will be swapped to the structured `ErrorCode`
        /// in Phase 10 of the action-v2 roadmap without changing this
        /// public shape or the wire format (the newtype is
        /// `#[serde(transparent)]`).
        code: TerminationCode,
        /// Human-readable error message.
        message: String,
    },
}

/// Opaque identifier for a termination error.
///
/// Currently backed by `Arc<str>` and serialised as a bare JSON string
/// via `#[serde(transparent)]`. Phase 10 of the action-v2 roadmap will
/// swap the inner representation to a structured `ErrorCode` type
/// (namespace, code, metadata) without changing this public API or the
/// wire format, so existing persisted `TerminationCode` values will
/// continue to deserialise.
///
/// Construct from any string-ish source via `From`:
///
/// ```
/// use nebula_action::TerminationCode;
///
/// let from_str: TerminationCode = "E_BAD".into();
/// let from_owned: TerminationCode = String::from("E_BAD").into();
/// assert_eq!(from_str.as_str(), "E_BAD");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TerminationCode(Arc<str>);

impl TerminationCode {
    /// Construct a new `TerminationCode` from anything convertible to
    /// `Arc<str>`.
    #[must_use]
    pub fn new(code: impl Into<Arc<str>>) -> Self {
        Self(code.into())
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TerminationCode {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for TerminationCode {
    fn from(s: String) -> Self {
        Self(Arc::from(s))
    }
}

impl From<Arc<str>> for TerminationCode {
    fn from(a: Arc<str>) -> Self {
        Self(a)
    }
}

impl std::fmt::Display for TerminationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Reason a stateful iteration ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BreakReason {
    /// All work completed naturally.
    Completed,
    /// Reached the configured iteration limit.
    MaxIterations,
    /// A user-defined stop condition was satisfied.
    ConditionMet,
    /// Custom reason with description.
    Custom(String),
}

/// Condition that must be met before a waiting action resumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum WaitCondition {
    /// Wait for an inbound HTTP callback.
    Webhook {
        /// Unique ID the external caller must include in the callback.
        callback_id: String,
    },
    /// Wait until a specific point in time.
    Until {
        /// The UTC datetime to resume at.
        datetime: DateTime<Utc>,
    },
    /// Wait for a fixed duration.
    Duration {
        /// How long to wait before resuming.
        #[serde(with = "duration_ms")]
        duration: Duration,
    },
    /// Wait for human approval.
    Approval {
        /// Identifier of the person who must approve.
        approver: String,
        /// Message shown to the approver.
        message: String,
    },
    /// Wait for another execution to complete.
    Execution {
        /// The execution to wait on.
        execution_id: ExecutionId,
    },
}

/// Normalize a progress fraction to the valid `0.0..=1.0` range.
///
/// - `NaN` → `0.0` (downstream progress bars / ETAs divide by zero or render nonsense otherwise)
/// - negative → `0.0`
/// - values above 1.0 → `1.0`
///
/// Applied inside `continue_with` / `continue_with_delay` so action
/// authors cannot accidentally poison downstream consumers with
/// malformed progress data.
fn sanitize_fraction(x: f64) -> f64 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Saturating cast from `u128` millis to `u64`. Durations longer
    /// than `u64::MAX` ms (~584 million years) saturate to `u64::MAX`
    /// instead of silently wrapping via `as u64`. Not reachable with
    /// legitimate inputs, but honest about the boundary.
    fn millis_saturating(d: &Duration) -> u64 {
        u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
    }

    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        millis_saturating(duration).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(Duration::from_millis(millis))
    }
}

mod duration_opt_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(duration: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => u64::try_from(d.as_millis())
                .unwrap_or(u64::MAX)
                .serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt: Option<u64> = Option::deserialize(d)?;
        Ok(opt.map(Duration::from_millis))
    }
}

// ── Convenience constructors ────────────────────────────────────────────────

impl<T> ActionResult<T> {
    /// Create a successful result wrapping the output in [`ActionOutput::Value`].
    #[must_use]
    pub fn success(output: T) -> Self {
        Self::Success {
            output: ActionOutput::Value(output),
        }
    }

    /// Create a successful result with binary data.
    #[must_use]
    pub fn success_binary(data: BinaryData) -> Self {
        Self::Success {
            output: ActionOutput::Binary(data),
        }
    }

    /// Create a successful result with a data reference.
    #[must_use]
    pub fn success_reference(reference: DataReference) -> Self {
        Self::Success {
            output: ActionOutput::Reference(reference),
        }
    }

    /// Create a successful result with no output.
    #[must_use]
    pub fn success_empty() -> Self {
        Self::Success {
            output: ActionOutput::Empty,
        }
    }

    /// Create a successful result with a pre-built `ActionOutput`.
    #[must_use]
    pub fn success_output(output: ActionOutput<T>) -> Self {
        Self::Success { output }
    }

    /// Create a successful result with a deferred output.
    #[must_use]
    pub fn success_deferred(deferred: DeferredOutput) -> Self {
        Self::Success {
            output: ActionOutput::Deferred(Box::new(deferred)),
        }
    }

    /// Create a skip result.
    #[must_use]
    pub fn skip(reason: impl Into<String>) -> Self {
        Self::Skip {
            reason: reason.into(),
            output: None,
        }
    }

    /// Create a skip result carrying a value output.
    #[must_use]
    pub fn skip_with_output(reason: impl Into<String>, output: T) -> Self {
        Self::Skip {
            reason: reason.into(),
            output: Some(ActionOutput::Value(output)),
        }
    }

    /// Create a `Drop` result without a reason.
    ///
    /// Drops the current item from the main output; downstream branches
    /// continue processing subsequent items normally.
    #[must_use]
    pub fn drop_item() -> Self {
        Self::Drop { reason: None }
    }

    /// Create a `Drop` result with a human-readable reason.
    #[must_use]
    pub fn drop_with_reason(reason: impl Into<String>) -> Self {
        Self::Drop {
            reason: Some(reason.into()),
        }
    }

    /// Create a `Terminate` result that ends the execution successfully.
    #[must_use]
    pub fn terminate_success(note: Option<String>) -> Self {
        Self::Terminate {
            reason: TerminationReason::Success { note },
        }
    }

    /// Create a `Terminate` result that ends the execution with a failure.
    #[must_use]
    pub fn terminate_failure(code: impl Into<TerminationCode>, message: impl Into<String>) -> Self {
        Self::Terminate {
            reason: TerminationReason::Failure {
                code: code.into(),
                message: message.into(),
            },
        }
    }

    /// Create a `Continue` result for stateful action iteration.
    ///
    /// Wraps `output` in [`ActionOutput::Value`] with optional progress.
    /// No delay between iterations.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(ActionResult::continue_with(page_data, Some(0.5)))
    /// ```
    #[must_use]
    pub fn continue_with(output: T, progress: Option<f64>) -> Self {
        Self::Continue {
            output: ActionOutput::Value(output),
            progress: progress.map(sanitize_fraction),
            delay: None,
        }
    }

    /// Create a `Continue` result with a delay before the next iteration.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(ActionResult::continue_with_delay(data, Some(0.8), Duration::from_secs(5)))
    /// ```
    #[must_use]
    pub fn continue_with_delay(output: T, progress: Option<f64>, delay: Duration) -> Self {
        Self::Continue {
            output: ActionOutput::Value(output),
            progress: progress.map(sanitize_fraction),
            delay: Some(delay),
        }
    }

    /// Create a `Break` result indicating natural completion.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(ActionResult::break_completed(final_output))
    /// ```
    #[must_use]
    pub fn break_completed(output: T) -> Self {
        Self::Break {
            output: ActionOutput::Value(output),
            reason: BreakReason::Completed,
        }
    }

    /// Create a `Break` result with a specific reason.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// Ok(ActionResult::break_with_reason(output, BreakReason::MaxIterations))
    /// ```
    #[must_use]
    pub fn break_with_reason(output: T, reason: BreakReason) -> Self {
        Self::Break {
            output: ActionOutput::Value(output),
            reason,
        }
    }

    /// Returns `true` if the result indicates successful completion.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Returns `true` if the action wants to continue iterating.
    #[must_use]
    pub fn is_continue(&self) -> bool {
        matches!(self, Self::Continue { .. })
    }

    /// Returns `true` if the action is waiting for an external event.
    #[must_use]
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::Wait { .. })
    }

    /// Returns `true` if the action is requesting a retry.
    ///
    /// The `Retry` variant itself is gated behind the
    /// `unstable-retry-scheduler` feature (canon §11.2), but this predicate
    /// is **always available** so that consumers can ask the question in a
    /// feature-unification-safe way. Without the feature the variant cannot
    /// be constructed, so this always returns `false`; with the feature, it
    /// returns `true` iff the result is `Retry`.
    ///
    /// The engine uses this method as a runtime guard to keep `Retry` out of
    /// the normal success path even when Cargo feature unification lands the
    /// variant in `nebula-action` without enabling the mirror feature in
    /// `nebula-engine`.
    #[must_use]
    pub fn is_retry(&self) -> bool {
        #[cfg(feature = "unstable-retry-scheduler")]
        {
            matches!(self, Self::Retry { .. })
        }
        #[cfg(not(feature = "unstable-retry-scheduler"))]
        {
            let _ = self;
            false
        }
    }

    /// Returns `true` if the action dropped its item without stopping the branch.
    #[must_use]
    pub fn is_drop(&self) -> bool {
        matches!(self, Self::Drop { .. })
    }

    /// Returns `true` if the action is requesting explicit execution termination.
    #[must_use]
    pub fn is_terminate(&self) -> bool {
        matches!(self, Self::Terminate { .. })
    }

    /// Transform the output value in every variant, preserving flow-control semantics.
    ///
    /// Delegates to [`ActionOutput::map`] for each output field.
    pub fn map_output<U>(self, mut f: impl FnMut(T) -> U) -> ActionResult<U> {
        match self {
            Self::Success { output } => ActionResult::Success {
                output: output.map(&mut f),
            },
            Self::Skip { reason, output } => ActionResult::Skip {
                reason,
                output: output.map(|o| o.map(&mut f)),
            },
            Self::Continue {
                output,
                progress,
                delay,
            } => ActionResult::Continue {
                output: output.map(&mut f),
                progress,
                delay,
            },
            Self::Break { output, reason } => ActionResult::Break {
                output: output.map(&mut f),
                reason,
            },
            Self::Branch {
                selected,
                output,
                alternatives,
            } => ActionResult::Branch {
                selected,
                output: output.map(&mut f),
                alternatives: alternatives
                    .into_iter()
                    .map(|(k, v)| (k, v.map(&mut f)))
                    .collect(),
            },
            Self::Route { port, data } => ActionResult::Route {
                port,
                data: data.map(&mut f),
            },
            Self::MultiOutput {
                outputs,
                main_output,
            } => ActionResult::MultiOutput {
                outputs: outputs
                    .into_iter()
                    .map(|(k, v)| (k, v.map(&mut f)))
                    .collect(),
                main_output: main_output.map(|o| o.map(&mut f)),
            },
            Self::Wait {
                condition,
                timeout,
                partial_output,
            } => ActionResult::Wait {
                condition,
                timeout,
                partial_output: partial_output.map(|o| o.map(&mut f)),
            },
            #[cfg(feature = "unstable-retry-scheduler")]
            Self::Retry { after, reason } => ActionResult::Retry { after, reason },
            Self::Drop { reason } => ActionResult::Drop { reason },
            Self::Terminate { reason } => ActionResult::Terminate { reason },
        }
    }

    /// Fallible version of [`map_output`](Self::map_output).
    ///
    /// Delegates to [`ActionOutput::try_map`] for each output field.
    pub fn try_map_output<U, E>(
        self,
        mut f: impl FnMut(T) -> Result<U, E>,
    ) -> Result<ActionResult<U>, E> {
        match self {
            Self::Success { output } => Ok(ActionResult::Success {
                output: output.try_map(&mut f)?,
            }),
            Self::Skip { reason, output } => Ok(ActionResult::Skip {
                reason,
                output: output.map(|o| o.try_map(&mut f)).transpose()?,
            }),
            Self::Continue {
                output,
                progress,
                delay,
            } => Ok(ActionResult::Continue {
                output: output.try_map(&mut f)?,
                progress,
                delay,
            }),
            Self::Break { output, reason } => Ok(ActionResult::Break {
                output: output.try_map(&mut f)?,
                reason,
            }),
            Self::Branch {
                selected,
                output,
                alternatives,
            } => {
                let mapped_output = output.try_map(&mut f)?;
                let mapped_alts = alternatives
                    .into_iter()
                    .map(|(k, v)| Ok((k, v.try_map(&mut f)?)))
                    .collect::<Result<HashMap<_, _>, E>>()?;
                Ok(ActionResult::Branch {
                    selected,
                    output: mapped_output,
                    alternatives: mapped_alts,
                })
            },
            Self::Route { port, data } => Ok(ActionResult::Route {
                port,
                data: data.try_map(&mut f)?,
            }),
            Self::MultiOutput {
                outputs,
                main_output,
            } => {
                let mapped_outputs = outputs
                    .into_iter()
                    .map(|(k, v)| Ok((k, v.try_map(&mut f)?)))
                    .collect::<Result<HashMap<_, _>, E>>()?;
                Ok(ActionResult::MultiOutput {
                    outputs: mapped_outputs,
                    main_output: main_output.map(|o| o.try_map(&mut f)).transpose()?,
                })
            },
            Self::Wait {
                condition,
                timeout,
                partial_output,
            } => Ok(ActionResult::Wait {
                condition,
                timeout,
                partial_output: partial_output.map(|o| o.try_map(&mut f)).transpose()?,
            }),
            #[cfg(feature = "unstable-retry-scheduler")]
            Self::Retry { after, reason } => Ok(ActionResult::Retry { after, reason }),
            Self::Drop { reason } => Ok(ActionResult::Drop { reason }),
            Self::Terminate { reason } => Ok(ActionResult::Terminate { reason }),
        }
    }

    /// Extract the primary output, consuming `self`.
    ///
    /// Returns `Some(ActionOutput<T>)` for variants that carry a primary output.
    /// Returns `None` for `Skip` without output, `Wait` without partial
    /// output, `MultiOutput` without main output, and `Retry` (when the
    /// `unstable-retry-scheduler` feature is enabled).
    ///
    /// To extract the inner `T` directly, chain with [`ActionOutput::into_value`]:
    ///
    /// ```rust,ignore
    /// let value: Option<T> = result.into_primary_output().and_then(|o| o.into_value());
    /// ```
    #[must_use]
    pub fn into_primary_output(self) -> Option<ActionOutput<T>> {
        match self {
            Self::Success { output } => Some(output),
            Self::Skip { output, .. } => output,
            Self::Continue { output, .. } => Some(output),
            Self::Break { output, .. } => Some(output),
            Self::Branch { output, .. } => Some(output),
            Self::Route { data, .. } => Some(data),
            Self::MultiOutput { main_output, .. } => main_output,
            Self::Wait { partial_output, .. } => partial_output,
            #[cfg(feature = "unstable-retry-scheduler")]
            Self::Retry { .. } => None,
            Self::Drop { .. } => None,
            Self::Terminate { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_result() {
        let result = ActionResult::success(42);
        assert!(result.is_success());
        assert!(!result.is_continue());
        assert!(!result.is_waiting());
    }

    #[test]
    fn skip_result() {
        let result: ActionResult<()> = ActionResult::skip("no data");
        match &result {
            ActionResult::Skip { reason, output } => {
                assert_eq!(reason, "no data");
                assert!(output.is_none());
            },
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn skip_with_output() {
        let result = ActionResult::skip_with_output("filtered", vec![1, 2, 3]);
        match &result {
            ActionResult::Skip { reason, output } => {
                assert_eq!(reason, "filtered");
                assert_eq!(output.as_ref().unwrap().as_value().unwrap(), &vec![1, 2, 3]);
            },
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn continue_result() {
        let result: ActionResult<String> = ActionResult::Continue {
            output: ActionOutput::Value("partial".into()),
            progress: Some(0.5),
            delay: Some(Duration::from_secs(1)),
        };
        assert!(result.is_continue());
        assert!(!result.is_success());
    }

    #[test]
    fn break_result() {
        let result: ActionResult<i32> = ActionResult::Break {
            output: ActionOutput::Value(100),
            reason: BreakReason::MaxIterations,
        };
        assert!(!result.is_continue());
        match &result {
            ActionResult::Break { reason, .. } => {
                assert_eq!(reason, &BreakReason::MaxIterations);
            },
            _ => panic!("expected Break"),
        }
    }

    #[test]
    fn branch_result() {
        let mut alts = HashMap::new();
        alts.insert("true".into(), ActionOutput::Value("yes"));
        alts.insert("false".into(), ActionOutput::Value("no"));
        let result = ActionResult::Branch {
            selected: "true".into(),
            output: ActionOutput::Value("yes"),
            alternatives: alts,
        };
        match result {
            ActionResult::Branch {
                selected,
                alternatives,
                ..
            } => {
                assert_eq!(selected, "true");
                assert_eq!(alternatives.len(), 2);
            },
            _ => panic!("expected Branch"),
        }
    }

    #[test]
    fn route_result() {
        let result = ActionResult::Route {
            port: "error".into(),
            data: ActionOutput::Value("something failed"),
        };
        assert!(!result.is_success());
    }

    #[test]
    fn multi_output_result() {
        let mut outputs = HashMap::new();
        outputs.insert("main".into(), ActionOutput::Value(1));
        outputs.insert("audit".into(), ActionOutput::Value(2));
        let result = ActionResult::MultiOutput {
            outputs,
            main_output: Some(ActionOutput::Value(1)),
        };
        assert!(!result.is_success());
    }

    #[test]
    fn wait_result() {
        let result: ActionResult<()> = ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: Duration::from_secs(60),
            },
            timeout: Some(Duration::from_secs(300)),
            partial_output: None,
        };
        assert!(result.is_waiting());
    }

    #[test]
    fn break_reason_equality() {
        assert_eq!(BreakReason::Completed, BreakReason::Completed);
        assert_ne!(BreakReason::Completed, BreakReason::MaxIterations);
        assert_eq!(
            BreakReason::Custom("done".into()),
            BreakReason::Custom("done".into())
        );
    }

    // ── map_output tests ────────────────────────────────────────────

    #[test]
    fn map_output_success() {
        let r = ActionResult::success(5);
        let mapped = r.map_output(|n| n * 2);
        match mapped {
            ActionResult::Success { output } => assert_eq!(output.into_value(), Some(10)),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn map_output_skip() {
        let r = ActionResult::skip_with_output("skip", 3);
        let mapped = r.map_output(|n| n.to_string());
        match mapped {
            ActionResult::Skip { reason, output } => {
                assert_eq!(reason, "skip");
                assert_eq!(output.unwrap().as_value().map(|s| s.as_str()), Some("3"));
            },
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn map_output_skip_none() {
        let r: ActionResult<i32> = ActionResult::skip("no output");
        let mapped = r.map_output(|n| n.to_string());
        match mapped {
            ActionResult::Skip { output, .. } => assert!(output.is_none()),
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn map_output_continue() {
        let r: ActionResult<i32> = ActionResult::Continue {
            output: ActionOutput::Value(7),
            progress: Some(0.5),
            delay: Some(Duration::from_secs(1)),
        };
        let mapped = r.map_output(|n| n + 1);
        match mapped {
            ActionResult::Continue {
                output,
                progress,
                delay,
            } => {
                assert_eq!(output.into_value(), Some(8));
                assert_eq!(progress, Some(0.5));
                assert_eq!(delay, Some(Duration::from_secs(1)));
            },
            _ => panic!("expected Continue"),
        }
    }

    #[test]
    fn map_output_break() {
        let r: ActionResult<i32> = ActionResult::Break {
            output: ActionOutput::Value(42),
            reason: BreakReason::Completed,
        };
        let mapped = r.map_output(|n| format!("result:{n}"));
        match mapped {
            ActionResult::Break { output, reason } => {
                assert_eq!(output.as_value().map(|s| s.as_str()), Some("result:42"));
                assert_eq!(reason, BreakReason::Completed);
            },
            _ => panic!("expected Break"),
        }
    }

    #[test]
    fn map_output_branch() {
        let mut alts = HashMap::new();
        alts.insert("a".into(), ActionOutput::Value(1));
        alts.insert("b".into(), ActionOutput::Value(2));
        let r = ActionResult::Branch {
            selected: "a".into(),
            output: ActionOutput::Value(10),
            alternatives: alts,
        };
        let mapped = r.map_output(|n| n * 10);
        match mapped {
            ActionResult::Branch {
                selected,
                output,
                alternatives,
            } => {
                assert_eq!(selected, "a");
                assert_eq!(output.into_value(), Some(100));
                assert_eq!(alternatives.get("a").unwrap().as_value(), Some(&10));
                assert_eq!(alternatives.get("b").unwrap().as_value(), Some(&20));
            },
            _ => panic!("expected Branch"),
        }
    }

    #[test]
    fn map_output_route() {
        let r = ActionResult::Route {
            port: "out".into(),
            data: ActionOutput::Value(99),
        };
        let mapped = r.map_output(|n| n as f64);
        match mapped {
            ActionResult::Route { port, data } => {
                assert_eq!(port, "out");
                assert_eq!(data.into_value(), Some(99.0));
            },
            _ => panic!("expected Route"),
        }
    }

    #[test]
    fn map_output_multi_output() {
        let mut outputs = HashMap::new();
        outputs.insert("x".into(), ActionOutput::Value(1));
        let r = ActionResult::MultiOutput {
            outputs,
            main_output: Some(ActionOutput::Value(0)),
        };
        let mapped = r.map_output(|n| n + 100);
        match mapped {
            ActionResult::MultiOutput {
                outputs,
                main_output,
            } => {
                assert_eq!(outputs.get("x").unwrap().as_value(), Some(&101));
                assert_eq!(main_output.unwrap().into_value(), Some(100));
            },
            _ => panic!("expected MultiOutput"),
        }
    }

    #[test]
    fn map_output_wait() {
        let r: ActionResult<String> = ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: Duration::from_secs(60),
            },
            timeout: Some(Duration::from_secs(300)),
            partial_output: Some(ActionOutput::Value("partial".into())),
        };
        let mapped = r.map_output(|s| s.len());
        match mapped {
            ActionResult::Wait {
                partial_output,
                timeout,
                ..
            } => {
                assert_eq!(partial_output.unwrap().into_value(), Some(7));
                assert_eq!(timeout, Some(Duration::from_secs(300)));
            },
            _ => panic!("expected Wait"),
        }
    }

    #[cfg(feature = "unstable-retry-scheduler")]
    #[test]
    fn map_output_retry() {
        let r: ActionResult<i32> = ActionResult::Retry {
            after: Duration::from_secs(5),
            reason: "rate limited".into(),
        };
        let mapped = r.map_output(|n| n * 2);
        match mapped {
            ActionResult::Retry { after, reason } => {
                assert_eq!(after, Duration::from_secs(5));
                assert_eq!(reason, "rate limited");
            },
            _ => panic!("expected Retry"),
        }
    }

    // ── retry tests ──────────────────────────────────────────────────

    #[cfg(feature = "unstable-retry-scheduler")]
    #[test]
    fn retry_result() {
        let result: ActionResult<()> = ActionResult::Retry {
            after: Duration::from_secs(10),
            reason: "upstream not ready".into(),
        };
        assert!(result.is_retry());
        assert!(!result.is_success());
        assert!(!result.is_continue());
        assert!(!result.is_waiting());
    }

    // ── try_map_output tests ─────────────────────────────────────────

    #[test]
    fn try_map_output_success_ok() {
        let r = ActionResult::success(5);
        let mapped = r.try_map_output(|n| Ok::<_, String>(n * 2));
        match mapped.unwrap() {
            ActionResult::Success { output } => assert_eq!(output.into_value(), Some(10)),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn try_map_output_success_err() {
        let r = ActionResult::success(5);
        let mapped = r.try_map_output(|_| Err::<i32, _>("serialization failed"));
        assert_eq!(mapped.unwrap_err(), "serialization failed");
    }

    #[test]
    fn try_map_output_skip_with_output() {
        let r = ActionResult::skip_with_output("filtered", 3);
        let mapped = r.try_map_output(|n| Ok::<_, String>(n.to_string()));
        match mapped.unwrap() {
            ActionResult::Skip { reason, output } => {
                assert_eq!(reason, "filtered");
                assert_eq!(output.unwrap().as_value().map(|s| s.as_str()), Some("3"));
            },
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn try_map_output_skip_none() {
        let r: ActionResult<i32> = ActionResult::skip("no output");
        let mapped = r.try_map_output(|n| Ok::<_, String>(n.to_string()));
        match mapped.unwrap() {
            ActionResult::Skip { output, .. } => assert!(output.is_none()),
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn try_map_output_branch_partial_failure() {
        let mut alts = HashMap::new();
        alts.insert("a".into(), ActionOutput::Value(1));
        alts.insert("b".into(), ActionOutput::Value(2));
        let r = ActionResult::Branch {
            selected: "a".into(),
            output: ActionOutput::Value(10),
            alternatives: alts,
        };
        // Fail on value 2 to test short-circuit
        let mapped = r.try_map_output(|n| if n == 2 { Err("bad value") } else { Ok(n * 10) });
        assert_eq!(mapped.unwrap_err(), "bad value");
    }

    #[cfg(feature = "unstable-retry-scheduler")]
    #[test]
    fn try_map_output_retry() {
        let r: ActionResult<i32> = ActionResult::Retry {
            after: Duration::from_secs(5),
            reason: "retry".into(),
        };
        let mapped = r.try_map_output(|_| Err::<String, _>("should not be called"));
        match mapped.unwrap() {
            ActionResult::Retry { after, reason } => {
                assert_eq!(after, Duration::from_secs(5));
                assert_eq!(reason, "retry");
            },
            _ => panic!("expected Retry"),
        }
    }

    // ── into_primary_output tests ────────────────────────────────────

    #[test]
    fn into_primary_output_success() {
        let r = ActionResult::success(42);
        let out = r.into_primary_output().unwrap();
        assert_eq!(out.into_value(), Some(42));
    }

    #[test]
    fn into_primary_output_skip_some() {
        let r = ActionResult::skip_with_output("reason", 7);
        let out = r.into_primary_output().unwrap();
        assert_eq!(out.into_value(), Some(7));
    }

    #[test]
    fn into_primary_output_skip_none() {
        let r: ActionResult<i32> = ActionResult::skip("no data");
        assert!(r.into_primary_output().is_none());
    }

    #[test]
    fn into_primary_output_continue() {
        let r: ActionResult<i32> = ActionResult::Continue {
            output: ActionOutput::Value(99),
            progress: Some(0.5),
            delay: None,
        };
        let out = r.into_primary_output().unwrap();
        assert_eq!(out.into_value(), Some(99));
    }

    #[test]
    fn into_primary_output_branch() {
        let r = ActionResult::Branch {
            selected: "a".into(),
            output: ActionOutput::Value(10),
            alternatives: HashMap::new(),
        };
        let out = r.into_primary_output().unwrap();
        assert_eq!(out.into_value(), Some(10));
    }

    #[test]
    fn into_primary_output_route() {
        let r = ActionResult::Route {
            port: "out".into(),
            data: ActionOutput::Value(55),
        };
        let out = r.into_primary_output().unwrap();
        assert_eq!(out.into_value(), Some(55));
    }

    #[cfg(feature = "unstable-retry-scheduler")]
    #[test]
    fn into_primary_output_retry() {
        let r: ActionResult<i32> = ActionResult::Retry {
            after: Duration::from_secs(1),
            reason: "wait".into(),
        };
        assert!(r.into_primary_output().is_none());
    }

    #[test]
    fn into_primary_output_wait_none() {
        let r: ActionResult<i32> = ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: Duration::from_secs(60),
            },
            timeout: None,
            partial_output: None,
        };
        assert!(r.into_primary_output().is_none());
    }

    #[test]
    fn into_primary_output_wait_some() {
        let r: ActionResult<i32> = ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: Duration::from_secs(60),
            },
            timeout: None,
            partial_output: Some(ActionOutput::Value(33)),
        };
        let out = r.into_primary_output().unwrap();
        assert_eq!(out.into_value(), Some(33));
    }

    // ── success_binary / success_reference / success_empty tests ─────

    #[test]
    fn success_binary_result() {
        use crate::output::{BinaryData, BinaryStorage};
        let r: ActionResult<i32> = ActionResult::success_binary(BinaryData {
            content_type: "image/png".into(),
            data: BinaryStorage::Inline(vec![1, 2, 3]),
            size: 3,
            metadata: None,
        });
        assert!(r.is_success());
        match r {
            ActionResult::Success { output } => assert!(output.is_binary()),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn success_reference_result() {
        use crate::output::DataReference;
        let r: ActionResult<i32> = ActionResult::success_reference(DataReference {
            storage_type: "s3".into(),
            path: "bucket/key".into(),
            size: Some(1024),
            content_type: None,
        });
        assert!(r.is_success());
        match r {
            ActionResult::Success { output } => assert!(output.is_reference()),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn success_empty_result() {
        let r: ActionResult<i32> = ActionResult::success_empty();
        assert!(r.is_success());
        match r {
            ActionResult::Success { output } => assert!(output.is_empty()),
            _ => panic!("expected Success"),
        }
    }

    // ── success_output / success_deferred tests ─────────────────────

    #[test]
    fn success_output_result() {
        use crate::output::{DeferredOutput, ExpectedOutput, Producer, ProducerKind, Resolution};
        let deferred = ActionOutput::<serde_json::Value>::Deferred(Box::new(DeferredOutput {
            handle_id: "h-1".into(),
            resolution: Resolution::Await {
                channel_id: "ch".into(),
            },
            expected: ExpectedOutput::Dynamic,
            progress: None,
            producer: Producer {
                kind: ProducerKind::AiModel,
                name: None,
                version: None,
            },
            retry: None,
            timeout: None,
        }));
        let r = ActionResult::success_output(deferred);
        assert!(r.is_success());
        match r {
            ActionResult::Success { output } => assert!(output.is_deferred()),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn success_deferred_result() {
        use crate::output::{DeferredOutput, ExpectedOutput, Producer, ProducerKind, Resolution};
        let r: ActionResult<serde_json::Value> = ActionResult::success_deferred(DeferredOutput {
            handle_id: "h-2".into(),
            resolution: Resolution::Callback {
                endpoint: "https://example.com".into(),
                token: "tok".into(),
            },
            expected: ExpectedOutput::Value { schema: None },
            progress: None,
            producer: Producer {
                kind: ProducerKind::ExternalApi,
                name: None,
                version: None,
            },
            retry: None,
            timeout: None,
        });
        assert!(r.is_success());
        match r {
            ActionResult::Success { output } => {
                assert!(output.is_deferred());
                assert!(output.needs_resolution());
            },
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn map_output_with_deferred() {
        use crate::output::{DeferredOutput, ExpectedOutput, Producer, ProducerKind, Resolution};
        let r: ActionResult<i32> = ActionResult::Success {
            output: ActionOutput::Deferred(Box::new(DeferredOutput {
                handle_id: "h".into(),
                resolution: Resolution::Await {
                    channel_id: "ch".into(),
                },
                expected: ExpectedOutput::Dynamic,
                progress: None,
                producer: Producer {
                    kind: ProducerKind::LocalCompute,
                    name: None,
                    version: None,
                },
                retry: None,
                timeout: None,
            })),
        };
        let mapped = r.map_output(|n| n.to_string());
        match mapped {
            ActionResult::Success { output } => assert!(output.is_deferred()),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn map_output_with_streaming() {
        use crate::output::{ExpectedOutput, StreamMode, StreamOutput, StreamState};
        let r: ActionResult<i32> = ActionResult::Success {
            output: ActionOutput::Streaming(StreamOutput {
                stream_id: "s".into(),
                mode: StreamMode::Events,
                expected: ExpectedOutput::Dynamic,
                state: StreamState::Pending,
                buffer: None,
            }),
        };
        let mapped = r.map_output(|n| n * 2);
        match mapped {
            ActionResult::Success { output } => assert!(output.is_streaming()),
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn map_output_with_collection() {
        let r: ActionResult<i32> = ActionResult::Success {
            output: ActionOutput::Collection(vec![ActionOutput::Value(1), ActionOutput::Value(2)]),
        };
        let mapped = r.map_output(|n| n * 10);
        match mapped {
            ActionResult::Success { output } => match output {
                ActionOutput::Collection(items) => {
                    assert_eq!(items[0].as_value(), Some(&10));
                    assert_eq!(items[1].as_value(), Some(&20));
                },
                _ => panic!("expected Collection"),
            },
            _ => panic!("expected Success"),
        }
    }

    // ── continue/break constructor tests ────────────────────────────

    #[test]
    fn continue_with_constructor() {
        let result = ActionResult::continue_with(42, Some(0.5));
        assert!(result.is_continue());
        match result {
            ActionResult::Continue {
                output,
                progress,
                delay,
            } => {
                assert_eq!(output.as_value(), Some(&42));
                assert_eq!(progress, Some(0.5));
                assert!(delay.is_none());
            },
            _ => panic!("expected Continue"),
        }
    }

    #[test]
    fn continue_with_delay_constructor() {
        let result = ActionResult::continue_with_delay(7, Some(0.8), Duration::from_secs(5));
        assert!(result.is_continue());
        match result {
            ActionResult::Continue {
                output,
                progress,
                delay,
            } => {
                assert_eq!(output.as_value(), Some(&7));
                assert_eq!(progress, Some(0.8));
                assert_eq!(delay, Some(Duration::from_secs(5)));
            },
            _ => panic!("expected Continue"),
        }
    }

    #[test]
    fn break_completed_constructor() {
        let result = ActionResult::break_completed(String::from("done"));
        assert!(!result.is_continue());
        match result {
            ActionResult::Break { output, reason } => {
                assert_eq!(output.as_value().map(|s| s.as_str()), Some("done"));
                assert_eq!(reason, BreakReason::Completed);
            },
            _ => panic!("expected Break"),
        }
    }

    #[test]
    fn break_with_reason_constructor() {
        let result = ActionResult::break_with_reason(99, BreakReason::MaxIterations);
        assert!(!result.is_continue());
        match result {
            ActionResult::Break { output, reason } => {
                assert_eq!(output.as_value(), Some(&99));
                assert_eq!(reason, BreakReason::MaxIterations);
            },
            _ => panic!("expected Break"),
        }
    }

    // ── Drop variant ────────────────────────────────────────────────

    #[test]
    fn drop_item_constructor() {
        let r: ActionResult<()> = ActionResult::drop_item();
        assert!(r.is_drop());
        match r {
            ActionResult::Drop { reason } => assert!(reason.is_none()),
            _ => panic!("expected Drop"),
        }
    }

    #[test]
    fn drop_with_reason_constructor() {
        let r: ActionResult<()> = ActionResult::drop_with_reason("rate limit exceeded");
        assert!(r.is_drop());
        match r {
            ActionResult::Drop { reason } => {
                assert_eq!(reason.as_deref(), Some("rate limit exceeded"));
            },
            _ => panic!("expected Drop"),
        }
    }

    #[test]
    fn drop_into_primary_output_is_none() {
        let r: ActionResult<i32> = ActionResult::drop_item();
        assert!(r.into_primary_output().is_none());
    }

    #[test]
    fn drop_map_output_preserves_reason() {
        let r: ActionResult<i32> = ActionResult::drop_with_reason("bad item");
        let mapped = r.map_output(|n| n * 10);
        match mapped {
            ActionResult::Drop { reason } => {
                assert_eq!(reason.as_deref(), Some("bad item"));
            },
            _ => panic!("expected Drop"),
        }
    }

    #[test]
    fn drop_serde_round_trip() {
        let original: ActionResult<i32> = ActionResult::drop_with_reason("filtered");
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionResult<i32> = serde_json::from_str(&json).unwrap();
        match decoded {
            ActionResult::Drop { reason } => {
                assert_eq!(reason.as_deref(), Some("filtered"));
            },
            _ => panic!("expected Drop"),
        }
    }

    // ── Terminate variant ──────────────────────────────────────────

    #[test]
    fn terminate_success_constructor() {
        let r: ActionResult<()> = ActionResult::terminate_success(Some("done early".into()));
        assert!(r.is_terminate());
        match r {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Success { note } => {
                    assert_eq!(note.as_deref(), Some("done early"));
                },
                TerminationReason::Failure { .. } => panic!("expected Success"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    #[test]
    fn terminate_failure_constructor() {
        let r: ActionResult<()> =
            ActionResult::terminate_failure("INVALID_STATE", "cannot proceed from current state");
        assert!(r.is_terminate());
        match r {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Failure { code, message } => {
                    assert_eq!(code.as_str(), "INVALID_STATE");
                    assert_eq!(message, "cannot proceed from current state");
                },
                TerminationReason::Success { .. } => panic!("expected Failure"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    #[test]
    fn terminate_into_primary_output_is_none() {
        let r: ActionResult<i32> = ActionResult::terminate_success(None);
        assert!(r.into_primary_output().is_none());
    }

    #[test]
    fn terminate_map_output_preserves_reason() {
        let r: ActionResult<i32> = ActionResult::terminate_failure("CODE", "msg");
        let mapped = r.map_output(|n| n * 10);
        match mapped {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Failure { code, message } => {
                    assert_eq!(code.as_str(), "CODE");
                    assert_eq!(message, "msg");
                },
                TerminationReason::Success { .. } => panic!("expected Failure"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    #[test]
    fn terminate_success_serde_round_trip() {
        let original: ActionResult<i32> = ActionResult::terminate_success(Some("ok".into()));
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionResult<i32> = serde_json::from_str(&json).unwrap();
        match decoded {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Success { note } => assert_eq!(note.as_deref(), Some("ok")),
                TerminationReason::Failure { .. } => panic!("expected Success"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    #[test]
    fn terminate_failure_serde_round_trip() {
        let original: ActionResult<i32> =
            ActionResult::terminate_failure("E_BAD", "something broke");
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionResult<i32> = serde_json::from_str(&json).unwrap();
        match decoded {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Failure { code, message } => {
                    assert_eq!(code.as_str(), "E_BAD");
                    assert_eq!(message, "something broke");
                },
                TerminationReason::Success { .. } => panic!("expected Failure"),
            },
            _ => panic!("expected Terminate"),
        }
    }
}
