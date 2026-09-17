use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::id::ExecutionId;
use serde::{Deserialize, Serialize};

use crate::branch_key::BranchKey;
use crate::output::{ActionOutput, BinaryData, DataReference, DeferredOutput};
use crate::port_key::PortKey;

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
/// - `Terminate` → end the whole execution explicitly (Stop / Fail nodes)
///
/// `ActionResult` does not trigger action retry: re-execution from a result
/// variant is not part of the engine contract. Operator-declared engine retry
/// is driven by `retry_policy` plus retryable failures; in-action retry around
/// outbound calls is composed with `nebula-resilience`.
///
/// All output fields are wrapped in [`ActionOutput<T>`] to support binary,
/// reference, and stream data alongside structured values.
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

    /// Terminate this node and signal that the execution should stop.
    ///
    /// Used by explicit termination nodes (n8n "Stop And Error",
    /// Kestra `Fail`, AWS Step Functions `Succeed`/`Fail` states,
    /// Pipedream "Exit Workflow", Make `Rollback`). Plugin authors should
    /// return `Terminate` today when they want "no more work from this
    /// branch" semantics.
    ///
    /// # Engine wiring
    ///
    /// The engine's `evaluate_edge` gates downstream edges from this
    /// node off the moment the action returns (same shape as
    /// [`Skip`](Self::Skip)).
    ///
    /// In addition, the frontier loop maps the [`TerminationReason`]
    /// into `ExecutionTerminationReason::ExplicitStop` /
    /// `ExecutionTerminationReason::ExplicitFail` and records it on
    /// `ExecutionState.terminated_by` (first-write-wins) **before** the
    /// next checkpoint, so the signal is durable across crashes. After
    /// the persist succeeds, the engine signals its `cancel_token` —
    /// sibling branches still in flight tear down cleanly. The signal
    /// drives `determine_final_status` (which prioritises explicit
    /// termination over `failed_node` and external cancel) and is
    /// surfaced on `ExecutionResult.termination_reason` and
    /// `ExecutionEvent::ExecutionFinished.termination_reason` so audit
    /// / API / webhook consumers can distinguish ExplicitFail from a
    /// system-driven failure and ExplicitStop from natural completion.
    ///
    /// See ROADMAP §M0.3 for the wiring contract and for
    /// the operational-honesty rule this closed.
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

    pub(super) fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        millis_saturating(duration).serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(Duration::from_millis(millis))
    }
}

mod duration_opt_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => u64::try_from(d.as_millis())
                .unwrap_or(u64::MAX)
                .serialize(s),
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Duration>, D::Error> {
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
    /// ```rust
    /// use nebula_action::ActionResult;
    ///
    /// let page_data = vec![1, 2, 3];
    /// let result = ActionResult::continue_with(page_data, Some(0.5));
    /// assert!(result.is_continue());
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
    /// ```rust
    /// use std::time::Duration;
    /// use nebula_action::ActionResult;
    ///
    /// let result = ActionResult::continue_with_delay("partial", Some(0.8), Duration::from_secs(5));
    /// assert!(result.is_continue());
    /// assert_eq!(result.into_primary_output().and_then(|o| o.into_value()), Some("partial"));
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
    /// ```rust
    /// use nebula_action::{ActionResult, BreakReason};
    ///
    /// let result = ActionResult::break_completed("final output");
    /// match result {
    ///     ActionResult::Break { reason, .. } => assert_eq!(reason, BreakReason::Completed),
    ///     _ => panic!("expected Break"),
    /// }
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
    /// ```rust
    /// use nebula_action::{ActionResult, BreakReason};
    ///
    /// let result = ActionResult::break_with_reason("truncated", BreakReason::MaxIterations);
    /// match result {
    ///     ActionResult::Break { reason, .. } => assert_eq!(reason, BreakReason::MaxIterations),
    ///     _ => panic!("expected Break"),
    /// }
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
            Self::Drop { reason } => Ok(ActionResult::Drop { reason }),
            Self::Terminate { reason } => Ok(ActionResult::Terminate { reason }),
        }
    }

    /// Extract the primary output, consuming `self`.
    ///
    /// Returns `Some(ActionOutput<T>)` for variants that carry a primary output.
    /// Returns `None` for `Skip` without output, `Wait` without partial
    /// output, and `MultiOutput` without main output.
    ///
    /// To extract the inner `T` directly, chain with [`ActionOutput::into_value`]:
    ///
    /// ```rust
    /// use nebula_action::ActionResult;
    ///
    /// let result = ActionResult::success(42);
    /// let value: Option<i32> = result.into_primary_output().and_then(|o| o.into_value());
    /// assert_eq!(value, Some(42));
    ///
    /// // A dropped item carries no primary output.
    /// let dropped: ActionResult<i32> = ActionResult::drop_item();
    /// assert!(dropped.into_primary_output().is_none());
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
            Self::Drop { .. } => None,
            Self::Terminate { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
