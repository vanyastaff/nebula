//! Mock data for the Live Monitor page.

/// Execution status for monitor list.
#[derive(Clone, Copy, PartialEq)]
pub enum MonitorStatus {
    Running,
    Queued,
    Completed,
    Failed,
}

/// Single execution in the monitor list.
#[derive(Clone)]
pub struct MonitorExecution {
    pub id: String,
    pub workflow: String,
    pub started: String,
    pub duration: String,
    /// Duration in ms when completed/failed; None when running/queued.
    pub duration_ms: Option<u32>,
    pub trigger: String,
    pub status: MonitorStatus,
    pub error: Option<String>,
    /// e.g. "6/6"
    pub nodes: String,
    pub input_size: String,
    pub output_size: Option<String>,
    pub retries: u32,
}

/// Summary counts for monitor.
#[derive(Clone)]
pub struct MonitorSummary {
    pub running: u32,
    pub queued: u32,
    pub done_1m: u32,
    pub failed_1m: u32,
    /// Success rate 0–100 (e.g. 98)
    pub success_rate_pct: u32,
    /// Avg latency ms
    pub avg_latency_ms: u32,
    /// Throughput per minute
    pub throughput_per_min: u32,
    /// P50, P90, P95, P99 in ms
    pub percentiles_ms: (u32, u32, u32, u32),
    /// Sparkline values (e.g. last 18 points)
    pub success_sparkline: Vec<u32>,
    /// SLO current % and target %
    pub slo_current_pct: f32,
    pub slo_target_pct: f32,
    pub workers_active: u32,
    pub workers_total: u32,
    pub cluster_nodes: u32,
    pub uptime_pct: f32,
}

/// Step/node status for coloring (ok, warn, error).
#[derive(Clone, Copy, PartialEq, Default)]
pub enum StepStatus {
    #[default]
    Ok,
    Warn,
    Error,
}

/// Trace step type.
#[derive(Clone, Copy, PartialEq)]
pub enum TraceStepType {
    Trigger,
    Action,
    Condition,
    Http,
    Db,
}

/// Single step in execution trace.
#[derive(Clone)]
pub struct TraceStep {
    pub id: usize,
    pub name: String,
    pub step_type: TraceStepType,
    pub start_offset_ms: u32,
    pub duration_ms: u32,
    pub status: StepStatus,
    /// Optional input for expandable details.
    pub input: Option<String>,
    /// Optional output for expandable details.
    pub output: Option<String>,
}

/// Per-node log line (for node detail panel).
#[derive(Clone)]
pub struct NodeLogEntry {
    pub t_ms: u32,
    pub level: LogLevel,
    pub message: String,
}

/// Retry attempt for a node.
#[derive(Clone)]
pub struct NodeRetry {
    pub attempt: u32,
    pub at: String,
    pub error: String,
    pub dur: String,
}

/// Rich per-node detail (for node detail panel).
#[derive(Clone)]
pub struct NodeDetail {
    pub id: usize,
    pub name: String,
    pub step_type: TraceStepType,
    pub status: StepStatus,
    pub start_ms: u32,
    pub dur_ms: u32,
    pub input: Option<String>,
    pub output: Option<String>,
    pub meta: Vec<(String, String)>,
    pub logs: Vec<NodeLogEntry>,
    pub retries: Vec<NodeRetry>,
    pub error: Option<String>,
    pub stack_trace: Option<String>,
}

/// Event history entry (WorkflowExecutionStarted, ActivityTaskScheduled, etc.).
#[derive(Clone)]
pub struct EventHistoryEntry {
    pub id: u32,
    pub event_type: String,
    pub ts: String,
    pub data_json: String,
}

/// Worker row for right panel.
#[derive(Clone)]
pub struct WorkerInfo {
    pub id: String,
    pub status: String,
    pub queue_len: u32,
    pub cpu: String,
}

/// Log level.
#[derive(Clone, Copy, PartialEq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Single log entry.
#[derive(Clone)]
pub struct LogEntry {
    pub timestamp_ms: u32,
    pub level: LogLevel,
    pub message: String,
}

/// Execution details for the right panel.
#[derive(Clone)]
pub struct ExecutionDetails {
    pub id: String,
    pub workflow: String,
    pub tenant: String,
    pub trigger: String,
    pub input_size: String,
    pub nodes_done: (u32, u32),
}

pub fn monitor_summary() -> MonitorSummary {
    MonitorSummary {
        running: 3,
        queued: 1,
        done_1m: 112,
        failed_1m: 2,
        success_rate_pct: 98,
        avg_latency_ms: 347,
        throughput_per_min: 115,
        percentiles_ms: (247, 612, 891, 2100),
        success_sparkline: vec![
            88, 91, 89, 94, 92, 96, 94, 97, 95, 98, 96, 98, 97, 99, 98, 97, 99, 98,
        ],
        slo_current_pct: 98.2,
        slo_target_pct: 99.5,
        workers_active: 4,
        workers_total: 4,
        cluster_nodes: 1,
        uptime_pct: 99.2,
    }
}

pub fn monitor_executions() -> Vec<MonitorExecution> {
    vec![
        MonitorExecution {
            id: "ex_f3a".into(),
            workflow: "Order Processing".into(),
            started: "2s ago".into(),
            duration: "0.4s".into(),
            duration_ms: Some(400),
            trigger: "webhook".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "6/6".into(),
            input_size: "0.8kb".into(),
            output_size: Some("1.2kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_f51".into(),
            workflow: "User Sync".into(),
            started: "3m ago".into(),
            duration: "0.8s".into(),
            duration_ms: Some(770),
            trigger: "cron".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "4/4".into(),
            input_size: "0.4kb".into(),
            output_size: Some("0.6kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_f68".into(),
            workflow: "Email Campaign".into(),
            started: "5m ago".into(),
            duration: "1.1s".into(),
            duration_ms: Some(1100),
            trigger: "manual".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "5/5".into(),
            input_size: "1.2kb".into(),
            output_size: Some("1.4kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_f71".into(),
            workflow: "Invoice Generator".into(),
            started: "5m ago".into(),
            duration: "1.5s".into(),
            duration_ms: Some(1500),
            trigger: "webhook".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "7/7".into(),
            input_size: "0.9kb".into(),
            output_size: Some("3.2kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_fad".into(),
            workflow: "Order Processing".into(),
            started: "6m ago".into(),
            duration: "1.9s".into(),
            duration_ms: Some(1900),
            trigger: "cron".into(),
            status: MonitorStatus::Failed,
            error: Some("DatabaseError: connection timeout".into()),
            nodes: "3/6".into(),
            input_size: "0.9kb".into(),
            output_size: None,
            retries: 2,
        },
        MonitorExecution {
            id: "ex_fc4".into(),
            workflow: "User Sync".into(),
            started: "9m ago".into(),
            duration: "2.3s".into(),
            duration_ms: Some(2300),
            trigger: "manual".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "4/4".into(),
            input_size: "0.4kb".into(),
            output_size: Some("0.7kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_f68_run".into(),
            workflow: "Email Campaign".into(),
            started: "10m ago".into(),
            duration: "—".into(),
            duration_ms: None,
            trigger: "webhook".into(),
            status: MonitorStatus::Running,
            error: None,
            nodes: "2/5".into(),
            input_size: "1.2kb".into(),
            output_size: None,
            retries: 0,
        },
        MonitorExecution {
            id: "ex_ff2".into(),
            workflow: "Invoice Generator".into(),
            started: "11m ago".into(),
            duration: "3.0s".into(),
            duration_ms: Some(3000),
            trigger: "cron".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "7/7".into(),
            input_size: "1.0kb".into(),
            output_size: Some("2.9kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_1020".into(),
            workflow: "Order Processing".into(),
            started: "12m ago".into(),
            duration: "3.4s".into(),
            duration_ms: Some(3400),
            trigger: "manual".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "6/6".into(),
            input_size: "0.8kb".into(),
            output_size: Some("1.1kb".into()),
            retries: 0,
        },
        MonitorExecution {
            id: "ex_fdb".into(),
            workflow: "User Sync".into(),
            started: "15m ago".into(),
            duration: "3.7s".into(),
            duration_ms: Some(3700),
            trigger: "webhook".into(),
            status: MonitorStatus::Completed,
            error: None,
            nodes: "4/4".into(),
            input_size: "0.4kb".into(),
            output_size: Some("0.7kb".into()),
            retries: 0,
        },
    ]
}

pub fn trace_steps(exec_id: &str) -> Vec<TraceStep> {
    if exec_id == "ex_f3a" {
        vec![
            TraceStep {
                id: 1,
                name: "Webhook Trigger".into(),
                step_type: TraceStepType::Trigger,
                start_offset_ms: 0,
                duration_ms: 12,
                status: StepStatus::Ok,
                input: None,
                output: Some(r#"{"event":"order.created","payload":{"orderId":"ord_1048","userId":"usr_4829"}}"#.into()),
            },
            TraceStep {
                id: 2,
                name: "Fetch User".into(),
                step_type: TraceStepType::Http,
                start_offset_ms: 12,
                duration_ms: 236,
                status: StepStatus::Ok,
                input: Some(r#"{"userId":"usr_4829"}"#.into()),
                output: Some(r#"{"id":"usr_4829","plan":"premium","tier":"gold"}"#.into()),
            },
            TraceStep {
                id: 3,
                name: "Check Premium".into(),
                step_type: TraceStepType::Condition,
                start_offset_ms: 248,
                duration_ms: 1,
                status: StepStatus::Ok,
                input: Some(r#"{"user":{"plan":"premium","tier":"gold"}}"#.into()),
                output: Some(r#"{"result":true,"branch":"insert_order"}"#.into()),
            },
            TraceStep {
                id: 4,
                name: "Insert Order".into(),
                step_type: TraceStepType::Db,
                start_offset_ms: 249,
                duration_ms: 189,
                status: StepStatus::Warn,
                input: Some(r#"{"userId":"usr_4829","amount":149.99,"currency":"USD"}"#.into()),
                output: Some(r#"{"orderId":"ord_1048","status":"created"}"#.into()),
            },
            TraceStep {
                id: 5,
                name: "Send Email".into(),
                step_type: TraceStepType::Action,
                start_offset_ms: 438,
                duration_ms: 30,
                status: StepStatus::Ok,
                input: Some(r#"{"to":"alex@acme.com","template":"order_confirmation"}"#.into()),
                output: Some(r#"{"messageId":"msg_8f3a2","queued":true}"#.into()),
            },
            TraceStep {
                id: 6,
                name: "Notify Slack".into(),
                step_type: TraceStepType::Action,
                start_offset_ms: 468,
                duration_ms: 102,
                status: StepStatus::Ok,
                input: Some(r##"{"channel":"#orders","text":"New order ord_1048"}"##.into()),
                output: Some(r#"{"ok":true,"ts":"1708606981.123456"}"#.into()),
            },
        ]
    } else if exec_id == "ex_fad" {
        vec![
            TraceStep {
                id: 1,
                name: "Cron Trigger".into(),
                step_type: TraceStepType::Trigger,
                start_offset_ms: 0,
                duration_ms: 5,
                status: StepStatus::Ok,
                input: None,
                output: Some(
                    r#"{"schedule":"@every 5m","fireTime":"2024-02-22T14:14:22Z"}"#.into(),
                ),
            },
            TraceStep {
                id: 2,
                name: "Fetch Orders".into(),
                step_type: TraceStepType::Http,
                start_offset_ms: 5,
                duration_ms: 210,
                status: StepStatus::Ok,
                input: Some(r#"{"status":"pending","since":"2024-02-22T14:09:22Z"}"#.into()),
                output: Some(
                    r#"{"orders":[{"id":"ord_2001"},{"id":"ord_2002"}],"total":48}"#.into(),
                ),
            },
            TraceStep {
                id: 3,
                name: "Validate Schema".into(),
                step_type: TraceStepType::Condition,
                start_offset_ms: 215,
                duration_ms: 2,
                status: StepStatus::Ok,
                input: Some(r#"{"count":48}"#.into()),
                output: Some(r#"{"valid":true,"passed":48}"#.into()),
            },
            TraceStep {
                id: 4,
                name: "Insert Batch".into(),
                step_type: TraceStepType::Db,
                start_offset_ms: 217,
                duration_ms: 2433,
                status: StepStatus::Error,
                input: Some(r#"{"orders":[...],"count":48}"#.into()),
                output: None,
            },
        ]
    } else {
        vec![]
    }
}

/// Rich node detail for the selected step (exec_id + step index or step id).
pub fn node_detail(exec_id: &str, step_id: usize) -> Option<NodeDetail> {
    if exec_id == "ex_f3a" && step_id == 4 {
        return Some(NodeDetail {
            id: 4,
            name: "Insert Order".into(),
            step_type: TraceStepType::Db,
            status: StepStatus::Warn,
            start_ms: 249,
            dur_ms: 189,
            input: Some(r#"{"userId":"usr_4829","amount":149.99,"currency":"USD"}"#.into()),
            output: Some(r#"{"orderId":"ord_1098","status":"created"}"#.into()),
            meta: vec![
                ("query".into(), "INSERT INTO orders ...".into()),
                ("table".into(), "orders".into()),
                ("rows".into(), "1".into()),
                ("poolLatency".into(), "284ms".into()),
                ("threshold".into(), "200ms".into()),
            ],
            logs: vec![
                NodeLogEntry {
                    t_ms: 249,
                    level: LogLevel::Info,
                    message: "Acquiring DB connection from pool".into(),
                },
                NodeLogEntry {
                    t_ms: 254,
                    level: LogLevel::Warn,
                    message: "Pool latency elevated: 284ms RTT (threshold: 200ms)".into(),
                },
                NodeLogEntry {
                    t_ms: 438,
                    level: LogLevel::Info,
                    message: "INSERT executed — 1 row affected".into(),
                },
            ],
            retries: vec![],
            error: None,
            stack_trace: None,
        });
    }
    if exec_id == "ex_fad" && step_id == 4 {
        return Some(NodeDetail {
            id: 4,
            name: "Insert Batch".into(),
            step_type: TraceStepType::Db,
            status: StepStatus::Error,
            start_ms: 217,
            dur_ms: 2433,
            input: Some(r#"{"orders":[{"id":"ord_2001"},{"id":"ord_2002"}],"count":48}"#.into()),
            output: None,
            meta: vec![
                ("query".into(), "INSERT INTO orders_batch ...".into()),
                ("table".into(), "orders_batch".into()),
                ("attempt".into(), "3".into()),
                ("maxAttempts".into(), "3".into()),
            ],
            logs: vec![
                NodeLogEntry { t_ms: 217, level: LogLevel::Info, message: "Acquiring DB connection".into() },
                NodeLogEntry { t_ms: 217, level: LogLevel::Warn, message: "Pool latency elevated: 380ms RTT".into() },
                NodeLogEntry { t_ms: 890, level: LogLevel::Warn, message: "Retry 1/3 — connection reset by peer".into() },
                NodeLogEntry { t_ms: 1600, level: LogLevel::Warn, message: "Retry 2/3 — connection reset by peer".into() },
                NodeLogEntry { t_ms: 2310, level: LogLevel::Error, message: "Retry 3/3 — connection timeout (5000ms)".into() },
                NodeLogEntry { t_ms: 2433, level: LogLevel::Error, message: "Activity failed: DatabaseError: connection timeout".into() },
            ],
            retries: vec![
                NodeRetry { attempt: 1, at: "14:14:23.107".into(), error: "connection reset by peer".into(), dur: "710ms".into() },
                NodeRetry { attempt: 2, at: "14:14:23.817".into(), error: "connection reset by peer".into(), dur: "710ms".into() },
                NodeRetry { attempt: 3, at: "14:14:24.527".into(), error: "connection timeout (5000ms)".into(), dur: "710ms".into() },
            ],
            error: Some("nebula::resource::DatabaseError: connection timeout after 3 retries".into()),
            stack_trace: Some(
                "nebula::resource::DatabaseError: connection timeout\n  at DbPool::acquire (nebula-db/src/pool.rs:142)\n  at Action::execute (nebula-core/src/action.rs:87)".into(),
            ),
        });
    }
    // Fallback: build from trace step if available
    let steps = trace_steps(exec_id);
    steps.into_iter().find(|s| s.id == step_id).map(|s| {
        let name = s.name.clone();
        NodeDetail {
            id: s.id,
            name: s.name,
            step_type: s.step_type,
            status: s.status,
            start_ms: s.start_offset_ms,
            dur_ms: s.duration_ms,
            input: s.input,
            output: s.output,
            meta: vec![],
            logs: vec![
                NodeLogEntry {
                    t_ms: s.start_offset_ms,
                    level: LogLevel::Info,
                    message: format!("{} started", name),
                },
                NodeLogEntry {
                    t_ms: s.start_offset_ms + s.duration_ms,
                    level: if s.status == StepStatus::Error {
                        LogLevel::Error
                    } else {
                        LogLevel::Info
                    },
                    message: format!(
                        "{} {}",
                        name,
                        if s.status == StepStatus::Ok {
                            "completed"
                        } else {
                            "failed"
                        }
                    ),
                },
            ],
            retries: vec![],
            error: None,
            stack_trace: None,
        }
    })
}

/// Event history for an execution (for Events tab).
pub fn event_history(exec_id: &str) -> Vec<EventHistoryEntry> {
    if exec_id == "ex_f3a" {
        vec![
            EventHistoryEntry {
                id: 1,
                event_type: "WorkflowExecutionStarted".into(),
                ts: "14:23:01.000".into(),
                data_json: r#"{"workflowId":"ex_f3a"}"#.into(),
            },
            EventHistoryEntry {
                id: 2,
                event_type: "WorkflowTaskScheduled".into(),
                ts: "14:23:01.001".into(),
                data_json: "{}".into(),
            },
            EventHistoryEntry {
                id: 3,
                event_type: "ActivityTaskScheduled".into(),
                ts: "14:23:01.012".into(),
                data_json: r#"{"activityType":"FetchUser","input":{"userId":"usr_4829"}}"#.into(),
            },
            EventHistoryEntry {
                id: 4,
                event_type: "ActivityTaskCompleted".into(),
                ts: "14:23:01.248".into(),
                data_json: r#"{"result":{"plan":"premium","tier":"gold"}}"#.into(),
            },
            EventHistoryEntry {
                id: 5,
                event_type: "ActivityTaskScheduled".into(),
                ts: "14:23:01.249".into(),
                data_json: r#"{"activityType":"InsertOrder"}"#.into(),
            },
            EventHistoryEntry {
                id: 6,
                event_type: "ActivityTaskCompleted".into(),
                ts: "14:23:01.438".into(),
                data_json: r#"{"result":{"orderId":"ord_1098"}}"#.into(),
            },
            EventHistoryEntry {
                id: 7,
                event_type: "WorkflowExecutionCompleted".into(),
                ts: "14:23:01.300".into(),
                data_json: r#"{"output":{"status":"ok"}}"#.into(),
            },
        ]
    } else if exec_id == "ex_fad" {
        vec![
            EventHistoryEntry {
                id: 1,
                event_type: "WorkflowExecutionStarted".into(),
                ts: "14:14:22.000".into(),
                data_json: r#"{"workflowId":"ex_fad"}"#.into(),
            },
            EventHistoryEntry {
                id: 2,
                event_type: "ActivityTaskScheduled".into(),
                ts: "14:14:22.005".into(),
                data_json: r#"{"activityType":"FetchOrders"}"#.into(),
            },
            EventHistoryEntry {
                id: 3,
                event_type: "ActivityTaskCompleted".into(),
                ts: "14:14:22.215".into(),
                data_json: r#"{"result":{"count":48}}"#.into(),
            },
            EventHistoryEntry {
                id: 4,
                event_type: "ActivityTaskScheduled".into(),
                ts: "14:14:22.217".into(),
                data_json: r#"{"activityType":"InsertBatch","retryPolicy":{"maxAttempts":3}}"#
                    .into(),
            },
            EventHistoryEntry {
                id: 5,
                event_type: "ActivityTaskTimedOut".into(),
                ts: "14:14:23.107".into(),
                data_json: r#"{"timeoutType":"ScheduleToClose"}"#.into(),
            },
            EventHistoryEntry {
                id: 6,
                event_type: "ActivityTaskFailed".into(),
                ts: "14:14:24.527".into(),
                data_json: r#"{"failure":{"message":"DatabaseError: connection timeout"}}"#.into(),
            },
            EventHistoryEntry {
                id: 7,
                event_type: "WorkflowExecutionFailed".into(),
                ts: "14:14:24.650".into(),
                data_json: r#"{"failure":{"message":"nebula::resource::DatabaseError"}}"#.into(),
            },
        ]
    } else {
        vec![]
    }
}

/// Latency heatmap: rows x cols, value 0.0–1.0 (for 24h distribution).
pub fn heatmap_data() -> Vec<Vec<f32>> {
    let rows = 8;
    let cols = 24;
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let b = if r < 3 {
                        0.7
                    } else if r < 5 {
                        0.4
                    } else {
                        0.1
                    };
                    let spike = if (c == 8 || c == 17) && r > 4 {
                        0.5
                    } else {
                        0.0
                    };
                    (b + spike + (c as f32 * 0.02).sin() * 0.2).clamp(0.0, 1.0)
                })
                .collect()
        })
        .collect()
}

/// Workers for right panel.
pub fn workers() -> Vec<WorkerInfo> {
    vec![
        WorkerInfo {
            id: "wrk-1".into(),
            status: "active".into(),
            queue_len: 3,
            cpu: "18%".into(),
        },
        WorkerInfo {
            id: "wrk-2".into(),
            status: "active".into(),
            queue_len: 1,
            cpu: "7%".into(),
        },
        WorkerInfo {
            id: "wrk-3".into(),
            status: "active".into(),
            queue_len: 0,
            cpu: "3%".into(),
        },
        WorkerInfo {
            id: "wrk-4".into(),
            status: "idle".into(),
            queue_len: 0,
            cpu: "0%".into(),
        },
    ]
}

/// Related executions (same workflow, other runs).
pub fn related_executions(exec_id: &str, workflow: &str, limit: usize) -> Vec<MonitorExecution> {
    monitor_executions()
        .into_iter()
        .filter(|e| e.workflow == workflow && e.id != exec_id)
        .take(limit)
        .collect()
}

pub fn execution_log(exec_id: &str) -> Vec<LogEntry> {
    if exec_id == "ex_f3a" {
        vec![
            LogEntry {
                timestamp_ms: 0,
                level: LogLevel::Info,
                message: "Execution started".into(),
            },
            LogEntry {
                timestamp_ms: 2,
                level: LogLevel::Info,
                message: "Webhook payload validated — 284b".into(),
            },
            LogEntry {
                timestamp_ms: 12,
                level: LogLevel::Info,
                message: "Fetching user ID: usr_4829".into(),
            },
            LogEntry {
                timestamp_ms: 248,
                level: LogLevel::Info,
                message: "User found: plan=premium tier=gold".into(),
            },
            LogEntry {
                timestamp_ms: 251,
                level: LogLevel::Info,
                message: "Condition: user.tier==='premium' → true".into(),
            },
            LogEntry {
                timestamp_ms: 252,
                level: LogLevel::Info,
                message: "Routing → Insert Order".into(),
            },
            LogEntry {
                timestamp_ms: 254,
                level: LogLevel::Warn,
                message: "DB connection slow (284ms RTT)".into(),
            },
            LogEntry {
                timestamp_ms: 443,
                level: LogLevel::Info,
                message: "Order inserted: ord_1048".into(),
            },
            LogEntry {
                timestamp_ms: 445,
                level: LogLevel::Info,
                message: "Slack notified: #orders".into(),
            },
            LogEntry {
                timestamp_ms: 547,
                level: LogLevel::Info,
                message: "Execution completed — 547ms".into(),
            },
        ]
    } else if exec_id == "ex_fad" {
        vec![
            LogEntry {
                timestamp_ms: 0,
                level: LogLevel::Info,
                message: "Execution started".into(),
            },
            LogEntry {
                timestamp_ms: 5,
                level: LogLevel::Info,
                message: "Cron trigger fired — run #42".into(),
            },
            LogEntry {
                timestamp_ms: 100,
                level: LogLevel::Warn,
                message: "DB connection pool exhausted".into(),
            },
            LogEntry {
                timestamp_ms: 1900,
                level: LogLevel::Error,
                message: "DatabaseError: connection timeout after 1900ms".into(),
            },
        ]
    } else {
        vec![]
    }
}

pub fn execution_details(exec_id: &str) -> Option<ExecutionDetails> {
    match exec_id {
        "ex_f3a" => Some(ExecutionDetails {
            id: "ex_f3a".into(),
            workflow: "Order Processing".into(),
            tenant: "default".into(),
            trigger: "webhook".into(),
            input_size: "1.0kb".into(),
            nodes_done: (6, 6),
        }),
        "ex_fad" => Some(ExecutionDetails {
            id: "ex_fad".into(),
            workflow: "Order Processing".into(),
            tenant: "default".into(),
            trigger: "cron".into(),
            input_size: "0.2kb".into(),
            nodes_done: (2, 5),
        }),
        "ex_f51" => Some(ExecutionDetails {
            id: "ex_f51".into(),
            workflow: "User Sync".into(),
            tenant: "default".into(),
            trigger: "cron".into(),
            input_size: "0.4kb".into(),
            nodes_done: (5, 5),
        }),
        "ex_f68" => Some(ExecutionDetails {
            id: "ex_f68".into(),
            workflow: "Email Campaign".into(),
            tenant: "default".into(),
            trigger: "manual".into(),
            input_size: "2.1kb".into(),
            nodes_done: (8, 8),
        }),
        _ => None,
    }
}

pub fn total_duration_ms(exec_id: &str) -> u32 {
    match exec_id {
        "ex_f3a" => 547,
        "ex_fad" => 1900,
        "ex_f51" => 800,
        "ex_f68" => 1100,
        _ => 0,
    }
}
