//! TUI application state and main loop.

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use nebula_core::id::NodeId;
use tokio::sync::mpsc;

use super::event::{LogLevel, TuiEvent};
use super::render;

/// Status of a single node in the TUI.
#[derive(Clone)]
pub struct NodeInfo {
    pub name: String,
    pub action_key: String,
    pub status: NodeStatus,
    pub elapsed: Option<Duration>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum NodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    #[allow(dead_code)]
    Skipped,
}

/// A log entry displayed in the bottom panel.
#[derive(Clone)]
pub struct LogEntry {
    pub offset: Duration,
    pub level: LogLevel,
    pub message: String,
}

/// The TUI application state.
pub struct App {
    pub workflow_name: String,
    pub execution_id: String,
    pub started_at: Instant,
    pub nodes: Vec<(NodeId, NodeInfo)>,
    pub node_index: HashMap<NodeId, usize>,
    pub selected_node: usize,
    pub logs: Vec<LogEntry>,
    #[allow(dead_code)]
    pub log_scroll: u16,
    pub done: bool,
    pub success: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(
        workflow_name: String,
        execution_id: String,
        node_order: Vec<(NodeId, String, String)>,
    ) -> Self {
        let mut nodes = Vec::with_capacity(node_order.len());
        let mut node_index = HashMap::new();

        for (i, (id, name, action_key)) in node_order.into_iter().enumerate() {
            node_index.insert(id, i);
            nodes.push((
                id,
                NodeInfo {
                    name,
                    action_key,
                    status: NodeStatus::Pending,
                    elapsed: None,
                    output: None,
                    error: None,
                },
            ));
        }

        Self {
            workflow_name,
            execution_id,
            started_at: Instant::now(),
            nodes,
            node_index,
            selected_node: 0,
            logs: Vec::new(),
            log_scroll: 0,
            done: false,
            success: false,
            should_quit: false,
        }
    }

    pub fn total_elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn completed_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|(_, n)| n.status == NodeStatus::Completed)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|(_, n)| n.status == NodeStatus::Failed)
            .count()
    }

    pub fn selected_info(&self) -> Option<&NodeInfo> {
        self.nodes.get(self.selected_node).map(|(_, info)| info)
    }

    fn handle_tui_event(&mut self, evt: TuiEvent) {
        match evt {
            TuiEvent::Key(key) => {
                if key.kind == KeyEventKind::Press {
                    self.handle_key(key.code);
                }
            }
            TuiEvent::NodeStarted {
                node_id,
                name: _,
                action_key: _,
            } => {
                if let Some(&idx) = self.node_index.get(&node_id) {
                    self.nodes[idx].1.status = NodeStatus::Running;
                    self.push_log(
                        LogLevel::Info,
                        format!("node \"{}\" started", self.nodes[idx].1.name),
                    );
                }
            }
            TuiEvent::NodeCompleted {
                node_id,
                elapsed,
                output,
            } => {
                if let Some(&idx) = self.node_index.get(&node_id) {
                    let name = self.nodes[idx].1.name.clone();
                    let info = &mut self.nodes[idx].1;
                    info.status = NodeStatus::Completed;
                    info.elapsed = Some(elapsed);
                    info.output = Some(output);
                    self.push_log(
                        LogLevel::Info,
                        format!("node \"{name}\" completed ({elapsed:?})"),
                    );
                }
            }
            TuiEvent::NodeFailed {
                node_id,
                elapsed,
                error,
            } => {
                if let Some(&idx) = self.node_index.get(&node_id) {
                    let name = self.nodes[idx].1.name.clone();
                    let info = &mut self.nodes[idx].1;
                    info.status = NodeStatus::Failed;
                    info.elapsed = Some(elapsed);
                    info.error = Some(error.clone());
                    self.push_log(LogLevel::Error, format!("node \"{name}\" failed: {error}"));
                }
            }
            TuiEvent::WorkflowDone { success, .. } => {
                self.done = true;
                self.success = success;
                let status = if success { "completed" } else { "failed" };
                self.push_log(LogLevel::Info, format!("execution {status}"));
            }
            TuiEvent::Log { level, message } => {
                self.push_log(level, message);
            }
            TuiEvent::Tick | TuiEvent::Resize(..) => {}
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_node > 0 {
                    self.selected_node -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_node + 1 < self.nodes.len() {
                    self.selected_node += 1;
                }
            }
            _ => {}
        }
    }

    fn push_log(&mut self, level: LogLevel, message: String) {
        self.logs.push(LogEntry {
            offset: self.started_at.elapsed(),
            level,
            message,
        });
    }
}

/// Run the TUI event loop.
///
/// Takes an `mpsc::UnboundedReceiver<TuiEvent>` that receives engine events
/// and terminal events are polled via crossterm.
pub async fn run_tui(mut rx: mpsc::UnboundedReceiver<TuiEvent>, mut app: App) -> io::Result<()> {
    let mut terminal = ratatui::init();
    terminal.clear()?;

    let tick_rate = Duration::from_millis(100);

    loop {
        terminal.draw(|frame| render::draw(frame, &app))?;

        // Poll terminal events with a short timeout.
        if event::poll(tick_rate)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key.code);
        }

        // Drain all pending engine events.
        while let Ok(evt) = rx.try_recv() {
            app.handle_tui_event(evt);
        }

        if app.should_quit {
            break;
        }
    }

    ratatui::restore();
    Ok(())
}
