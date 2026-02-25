//! Fake dashboard data.

/// Hourly execution counts for the bar chart.
#[derive(Clone)]
pub struct HourlyExecution {
    pub hour: String,
    pub count: u32,
}

/// Workflow health entry with sparkline data.
#[derive(Clone)]
pub struct WorkflowHealth {
    pub name: String,
    pub success_rate: f32,
    #[allow(dead_code)]
    pub sparkline: Vec<f32>,
}

/// Execution status.
#[derive(Clone, Copy, PartialEq)]
pub enum ExecutionStatus {
    Completed,
    Failed,
    Running,
}

/// Recent execution row.
#[derive(Clone)]
pub struct RecentExecution {
    pub id: String,
    pub workflow: String,
    pub status: ExecutionStatus,
    pub progress: (u32, u32),
    pub duration: String,
    pub trigger: String,
    pub started: String,
}

/// Summary metric card data.
#[derive(Clone)]
pub struct MetricCard {
    pub title: String,
    pub value: String,
    pub subtitle: Option<String>,
    /// When true, subtitle uses success (green) color.
    pub subtitle_positive: bool,
    pub trend: Option<Trend>,
}

#[derive(Clone)]
pub struct Trend {
    pub up: bool,
    pub value: String,
    pub label: String,
}

pub fn hourly_executions() -> Vec<HourlyExecution> {
    vec![
        HourlyExecution {
            hour: "00".into(),
            count: 45,
        },
        HourlyExecution {
            hour: "02".into(),
            count: 62,
        },
        HourlyExecution {
            hour: "04".into(),
            count: 38,
        },
        HourlyExecution {
            hour: "06".into(),
            count: 89,
        },
        HourlyExecution {
            hour: "08".into(),
            count: 112,
        },
        HourlyExecution {
            hour: "10".into(),
            count: 98,
        },
        HourlyExecution {
            hour: "12".into(),
            count: 134,
        },
        HourlyExecution {
            hour: "14".into(),
            count: 87,
        },
        HourlyExecution {
            hour: "16".into(),
            count: 156,
        },
        HourlyExecution {
            hour: "18".into(),
            count: 92,
        },
        HourlyExecution {
            hour: "20".into(),
            count: 78,
        },
        HourlyExecution {
            hour: "22".into(),
            count: 109,
        },
    ]
}

pub fn workflow_health() -> Vec<WorkflowHealth> {
    vec![
        WorkflowHealth {
            name: "Order Processing".into(),
            success_rate: 99.6,
            sparkline: vec![98.0, 99.2, 99.5, 99.8, 99.6, 99.7],
        },
        WorkflowHealth {
            name: "User Sync".into(),
            success_rate: 91.7,
            sparkline: vec![92.0, 90.5, 91.0, 92.5, 91.7, 91.2],
        },
        WorkflowHealth {
            name: "Email Campaign".into(),
            success_rate: 100.0,
            sparkline: vec![100.0, 100.0, 100.0, 100.0, 100.0, 100.0],
        },
        WorkflowHealth {
            name: "Invoice Generator".into(),
            success_rate: 98.2,
            sparkline: vec![97.5, 98.0, 98.5, 98.2, 98.8, 98.2],
        },
        WorkflowHealth {
            name: "Daily Report".into(),
            success_rate: 99.1,
            sparkline: vec![98.8, 99.0, 99.2, 99.1, 99.3, 99.1],
        },
    ]
}

pub fn recent_executions() -> Vec<RecentExecution> {
    vec![
        RecentExecution {
            id: "exec_003a3f".into(),
            workflow: "Order Processing".into(),
            status: ExecutionStatus::Completed,
            progress: (4, 4),
            duration: "8.4s".into(),
            trigger: "webhook".into(),
            started: "2s ago".into(),
        },
        RecentExecution {
            id: "exec_7b2c1d".into(),
            workflow: "User Sync".into(),
            status: ExecutionStatus::Failed,
            progress: (0, 4),
            duration: "0.8s".into(),
            trigger: "cron".into(),
            started: "1m ago".into(),
        },
        RecentExecution {
            id: "exec_9f4e8a".into(),
            workflow: "Email Campaign".into(),
            status: ExecutionStatus::Running,
            progress: (2, 6),
            duration: "n/a".into(),
            trigger: "manual".into(),
            started: "3m ago".into(),
        },
        RecentExecution {
            id: "exec_1a5b2c".into(),
            workflow: "Invoice Generator".into(),
            status: ExecutionStatus::Completed,
            progress: (3, 3),
            duration: "1.1s".into(),
            trigger: "webhook".into(),
            started: "5m ago".into(),
        },
        RecentExecution {
            id: "exec_8c3d9e".into(),
            workflow: "Daily Report".into(),
            status: ExecutionStatus::Completed,
            progress: (5, 5),
            duration: "12.3s".into(),
            trigger: "cron".into(),
            started: "12m ago".into(),
        },
    ]
}

pub fn metric_cards() -> Vec<MetricCard> {
    vec![
        MetricCard {
            title: "EXECUTIONS (24H)".into(),
            value: "1,202".into(),
            subtitle: None,
            subtitle_positive: false,
            trend: Some(Trend {
                up: true,
                value: "12%".into(),
                label: "vs yesterday".into(),
            }),
        },
        MetricCard {
            title: "SUCCESS RATE".into(),
            value: "98.4%".into(),
            subtitle: None,
            subtitle_positive: false,
            trend: Some(Trend {
                up: true,
                value: "0.2%".into(),
                label: "vs yesterday".into(),
            }),
        },
        MetricCard {
            title: "ACTIVE WORKERS".into(),
            value: "4".into(),
            subtitle: Some("99.2% uptime".into()),
            subtitle_positive: true,
            trend: None,
        },
        MetricCard {
            title: "AVG DURATION PER".into(),
            value: "2.8s".into(),
            subtitle: None,
            subtitle_positive: false,
            trend: Some(Trend {
                up: false,
                value: "1%".into(),
                label: "vs yesterday".into(),
            }),
        },
        MetricCard {
            title: "FAILED (24H)".into(),
            value: "19".into(),
            subtitle: None,
            subtitle_positive: false,
            trend: Some(Trend {
                up: false,
                value: "0%".into(),
                label: "vs yesterday".into(),
            }),
        },
    ]
}
