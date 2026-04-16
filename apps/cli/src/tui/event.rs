//! Events flowing into the TUI event loop.

use std::time::Duration;

use nebula_core::NodeKey;

/// All events the TUI handles.
#[allow(dead_code)] // Variants used when real-time engine events are wired.
pub enum TuiEvent {
    /// Terminal key press.
    Key(crossterm::event::KeyEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Periodic tick for timer refresh.
    Tick,
    /// A workflow node started executing.
    NodeStarted {
        node_key: NodeKey,
        name: String,
        action_key: String,
    },
    /// A workflow node completed successfully.
    NodeCompleted {
        node_key: NodeKey,
        elapsed: Duration,
        output: serde_json::Value,
    },
    /// A workflow node failed.
    NodeFailed {
        node_key: NodeKey,
        elapsed: Duration,
        error: String,
    },
    /// Workflow execution finished.
    WorkflowDone {
        total_elapsed: Duration,
        success: bool,
    },
    /// A log line from the execution.
    Log { level: LogLevel, message: String },
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}
