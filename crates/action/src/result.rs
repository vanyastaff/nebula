use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::id::ExecutionId;

use crate::output::{ActionOutput, BinaryData, DataReference};

/// Type alias for workflow branch keys (e.g. `"true"`, `"false"`, `"case_1"`).
pub type BranchKey = String;

/// Type alias for action output port keys (e.g. `"main"`, `"error"`, `"filtered"`).
pub type PortKey = String;

/// Result of an action execution, carrying both data and flow-control intent.
///
/// The engine matches on this enum to decide what happens next in the workflow:
/// - `Success` → pass output to dependent nodes
/// - `Skip` → skip downstream processing
/// - `Continue` → re-enqueue for next iteration (stateful actions)
/// - `Break` → finalize iteration (stateful actions)
/// - `Branch` → activate a specific branch path
/// - `Route` / `MultiOutput` → fan-out to output ports
/// - `Wait` → pause until external event, timer, or approval
///
/// All output fields are wrapped in [`ActionOutput<T>`] to support binary,
/// reference, and stream data alongside structured values.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ActionResult<T> {
    /// Successful completion -- engine passes output to dependent nodes.
    Success {
        /// The produced output value.
        output: ActionOutput<T>,
    },

    /// Skip this node -- engine skips downstream dependents.
    Skip {
        /// Human-readable reason for skipping.
        reason: String,
        /// Optional output produced before the skip decision.
        output: Option<ActionOutput<T>>,
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
        timeout: Option<Duration>,
        /// Partial output produced before pausing.
        partial_output: Option<ActionOutput<T>>,
    },

    /// Request a retry after a delay.
    ///
    /// Unlike `ActionError::Retryable`, this is a *successful* signal that the
    /// action wants to be re-executed (e.g. upstream data not ready, rate-limit
    /// cooldown). The engine re-enqueues the node after `after` elapses.
    Retry {
        /// Suggested delay before re-execution.
        after: Duration,
        /// Human-readable reason for requesting the retry.
        reason: String,
    },
}

/// Reason a stateful iteration ended.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
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

// ── Convenience constructors ────────────────────────────────────────────────

impl<T> ActionResult<T> {
    /// Create a successful result wrapping the output in [`ActionOutput::Value`].
    pub fn success(output: T) -> Self {
        Self::Success {
            output: ActionOutput::Value(output),
        }
    }

    /// Create a successful result with binary data.
    pub fn success_binary(data: BinaryData) -> Self {
        Self::Success {
            output: ActionOutput::Binary(data),
        }
    }

    /// Create a successful result with a data reference.
    pub fn success_reference(reference: DataReference) -> Self {
        Self::Success {
            output: ActionOutput::Reference(reference),
        }
    }

    /// Create a successful result with no output.
    pub fn success_empty() -> Self {
        Self::Success {
            output: ActionOutput::Empty,
        }
    }

    /// Create a skip result.
    pub fn skip(reason: impl Into<String>) -> Self {
        Self::Skip {
            reason: reason.into(),
            output: None,
        }
    }

    /// Create a skip result carrying a value output.
    pub fn skip_with_output(reason: impl Into<String>, output: T) -> Self {
        Self::Skip {
            reason: reason.into(),
            output: Some(ActionOutput::Value(output)),
        }
    }

    /// Returns `true` if the result indicates successful completion.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Returns `true` if the action wants to continue iterating.
    pub fn is_continue(&self) -> bool {
        matches!(self, Self::Continue { .. })
    }

    /// Returns `true` if the action is waiting for an external event.
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::Wait { .. })
    }

    /// Returns `true` if the action is requesting a retry.
    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
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
            Self::Retry { after, reason } => ActionResult::Retry { after, reason },
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
            }
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
            }
            Self::Wait {
                condition,
                timeout,
                partial_output,
            } => Ok(ActionResult::Wait {
                condition,
                timeout,
                partial_output: partial_output.map(|o| o.try_map(&mut f)).transpose()?,
            }),
            Self::Retry { after, reason } => Ok(ActionResult::Retry { after, reason }),
        }
    }

    /// Extract the primary output, consuming `self`.
    ///
    /// Returns `Some(ActionOutput<T>)` for variants that carry a primary output.
    /// Returns `None` for `Skip` without output, `Wait` without partial
    /// output, `MultiOutput` without main output, and `Retry`.
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
            Self::Retry { .. } => None,
        }
    }

    /// Extract the primary value `T` from the output, consuming `self`.
    ///
    /// Equivalent to `self.into_primary_output().and_then(|o| o.into_value())`.
    /// Returns `None` for non-value outputs (binary, reference, stream, empty)
    /// and for variants with no output.
    pub fn into_primary_value(self) -> Option<T> {
        self.into_primary_output().and_then(|o| o.into_value())
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
            }
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
            }
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
            }
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
            }
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
            }
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
            }
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
            }
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
            }
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
            }
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
            }
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
            }
            _ => panic!("expected Wait"),
        }
    }

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
            }
            _ => panic!("expected Retry"),
        }
    }

    // ── retry tests ──────────────────────────────────────────────────

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
            }
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
            }
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

    // ── into_primary_value tests ─────────────────────────────────────

    #[test]
    fn into_primary_value_success() {
        let r = ActionResult::success(42);
        assert_eq!(r.into_primary_value(), Some(42));
    }

    #[test]
    fn into_primary_value_empty() {
        let r: ActionResult<i32> = ActionResult::success_empty();
        assert_eq!(r.into_primary_value(), None);
    }

    #[test]
    fn into_primary_value_retry() {
        let r: ActionResult<i32> = ActionResult::Retry {
            after: Duration::from_secs(1),
            reason: "wait".into(),
        };
        assert_eq!(r.into_primary_value(), None);
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
}
