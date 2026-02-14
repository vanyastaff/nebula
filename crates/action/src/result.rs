use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::id::ExecutionId;

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
#[derive(Debug, Clone)]
pub enum ActionResult<T> {
    /// Successful completion -- engine passes output to dependent nodes.
    Success {
        /// The produced output value.
        output: T,
    },

    /// Skip this node -- engine skips downstream dependents.
    Skip {
        /// Human-readable reason for skipping.
        reason: String,
        /// Optional output produced before the skip decision.
        output: Option<T>,
    },

    /// Stateful iteration: not yet done, need another call.
    ///
    /// Engine saves state, optionally waits `delay`, then re-invokes.
    Continue {
        /// Intermediate output for this iteration.
        output: T,
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
        output: T,
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
        output: T,
        /// Outputs for non-selected branches (may be used for previews).
        alternatives: HashMap<BranchKey, T>,
    },

    /// Route output to a specific output port.
    Route {
        /// Target output port key.
        port: PortKey,
        /// Data to send to the port.
        data: T,
    },

    /// Fan-out to multiple output ports simultaneously.
    MultiOutput {
        /// Per-port output data.
        outputs: HashMap<PortKey, T>,
        /// Optional primary output sent to the default port.
        main_output: Option<T>,
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
        partial_output: Option<T>,
    },
}

/// Reason a stateful iteration ended.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Create a successful result.
    pub fn success(output: T) -> Self {
        Self::Success { output }
    }

    /// Create a skip result.
    pub fn skip(reason: impl Into<String>) -> Self {
        Self::Skip {
            reason: reason.into(),
            output: None,
        }
    }

    /// Create a skip result carrying an output.
    pub fn skip_with_output(reason: impl Into<String>, output: T) -> Self {
        Self::Skip {
            reason: reason.into(),
            output: Some(output),
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

    /// Transform the output value in every variant, preserving flow-control semantics.
    ///
    /// This is used by adapters to convert typed outputs to JSON and vice-versa.
    pub fn map_output<U>(self, mut f: impl FnMut(T) -> U) -> ActionResult<U> {
        match self {
            Self::Success { output } => ActionResult::Success { output: f(output) },
            Self::Skip { reason, output } => ActionResult::Skip {
                reason,
                output: output.map(&mut f),
            },
            Self::Continue {
                output,
                progress,
                delay,
            } => ActionResult::Continue {
                output: f(output),
                progress,
                delay,
            },
            Self::Break { output, reason } => ActionResult::Break {
                output: f(output),
                reason,
            },
            Self::Branch {
                selected,
                output,
                alternatives,
            } => ActionResult::Branch {
                selected,
                output: f(output),
                alternatives: alternatives.into_iter().map(|(k, v)| (k, f(v))).collect(),
            },
            Self::Route { port, data } => ActionResult::Route {
                port,
                data: f(data),
            },
            Self::MultiOutput {
                outputs,
                main_output,
            } => ActionResult::MultiOutput {
                outputs: outputs.into_iter().map(|(k, v)| (k, f(v))).collect(),
                main_output: main_output.map(&mut f),
            },
            Self::Wait {
                condition,
                timeout,
                partial_output,
            } => ActionResult::Wait {
                condition,
                timeout,
                partial_output: partial_output.map(&mut f),
            },
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
                assert_eq!(output.as_ref().unwrap(), &vec![1, 2, 3]);
            }
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn continue_result() {
        let result: ActionResult<String> = ActionResult::Continue {
            output: "partial".into(),
            progress: Some(0.5),
            delay: Some(Duration::from_secs(1)),
        };
        assert!(result.is_continue());
        assert!(!result.is_success());
    }

    #[test]
    fn break_result() {
        let result: ActionResult<i32> = ActionResult::Break {
            output: 100,
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
        alts.insert("true".into(), "yes");
        alts.insert("false".into(), "no");
        let result = ActionResult::Branch {
            selected: "true".into(),
            output: "yes",
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
            data: "something failed",
        };
        assert!(!result.is_success());
    }

    #[test]
    fn multi_output_result() {
        let mut outputs = HashMap::new();
        outputs.insert("main".into(), 1);
        outputs.insert("audit".into(), 2);
        let result = ActionResult::MultiOutput {
            outputs,
            main_output: Some(1),
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
            ActionResult::Success { output } => assert_eq!(output, 10),
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
                assert_eq!(output.as_deref(), Some("3"));
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
            output: 7,
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
                assert_eq!(output, 8);
                assert_eq!(progress, Some(0.5));
                assert_eq!(delay, Some(Duration::from_secs(1)));
            }
            _ => panic!("expected Continue"),
        }
    }

    #[test]
    fn map_output_break() {
        let r: ActionResult<i32> = ActionResult::Break {
            output: 42,
            reason: BreakReason::Completed,
        };
        let mapped = r.map_output(|n| format!("result:{n}"));
        match mapped {
            ActionResult::Break { output, reason } => {
                assert_eq!(output, "result:42");
                assert_eq!(reason, BreakReason::Completed);
            }
            _ => panic!("expected Break"),
        }
    }

    #[test]
    fn map_output_branch() {
        let mut alts = HashMap::new();
        alts.insert("a".into(), 1);
        alts.insert("b".into(), 2);
        let r = ActionResult::Branch {
            selected: "a".into(),
            output: 10,
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
                assert_eq!(output, 100);
                assert_eq!(alternatives.get("a"), Some(&10));
                assert_eq!(alternatives.get("b"), Some(&20));
            }
            _ => panic!("expected Branch"),
        }
    }

    #[test]
    fn map_output_route() {
        let r = ActionResult::Route {
            port: "out".into(),
            data: 99,
        };
        let mapped = r.map_output(|n| n as f64);
        match mapped {
            ActionResult::Route { port, data } => {
                assert_eq!(port, "out");
                assert_eq!(data, 99.0);
            }
            _ => panic!("expected Route"),
        }
    }

    #[test]
    fn map_output_multi_output() {
        let mut outputs = HashMap::new();
        outputs.insert("x".into(), 1);
        let r = ActionResult::MultiOutput {
            outputs,
            main_output: Some(0),
        };
        let mapped = r.map_output(|n| n + 100);
        match mapped {
            ActionResult::MultiOutput {
                outputs,
                main_output,
            } => {
                assert_eq!(outputs.get("x"), Some(&101));
                assert_eq!(main_output, Some(100));
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
            partial_output: Some("partial".into()),
        };
        let mapped = r.map_output(|s| s.len());
        match mapped {
            ActionResult::Wait {
                partial_output,
                timeout,
                ..
            } => {
                assert_eq!(partial_output, Some(7));
                assert_eq!(timeout, Some(Duration::from_secs(300)));
            }
            _ => panic!("expected Wait"),
        }
    }
}
