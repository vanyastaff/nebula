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
}
