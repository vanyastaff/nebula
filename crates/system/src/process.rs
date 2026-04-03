//! Process information
//!
//! Provides process listing, lookup, and statistics. Designed for
//! monitoring sandbox workers and system-level process awareness.
//!
//! # Known Limitations
//!
//! - **`thread_count`**: always `1` — sysinfo does not expose thread count portably.
//! - **`uid` / `gid`**: always `None` — not populated even on Unix.

use crate::core::{SystemError, SystemResult};
use std::time::{Duration, Instant};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Process information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProcessInfo {
    /// Process ID
    pub pid: u32,
    /// Parent process ID
    pub parent_pid: Option<u32>,
    /// Process name
    pub name: String,
    /// Executable path
    pub exe_path: Option<String>,
    /// Working directory
    pub cwd: Option<String>,
    /// Process status
    pub status: ProcessStatus,
    /// Memory usage in bytes
    pub memory: usize,
    /// Virtual memory size in bytes
    pub virtual_memory: usize,
    /// CPU usage percentage
    pub cpu_usage: f32,
    /// Number of threads
    pub thread_count: usize,
    /// User ID (Unix only, always None currently)
    pub uid: Option<u32>,
    /// Group ID (Unix only, always None currently)
    pub gid: Option<u32>,
}

/// Process status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ProcessStatus {
    /// Running
    Running,
    /// Sleeping
    Sleeping,
    /// Waiting
    Waiting,
    /// Stopped
    Stopped,
    /// Zombie
    Zombie,
    /// Dead
    Dead,
    /// Unknown
    Unknown,
}

/// Process statistics
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProcessStats {
    /// Total number of processes
    pub total: usize,
    /// Number of running processes
    pub running: usize,
    /// Number of sleeping processes
    pub sleeping: usize,
    /// Total memory used by all processes
    pub total_memory: usize,
    /// Total CPU usage by all processes
    pub total_cpu: f32,
}

/// Process tree node
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProcessTree {
    /// Process information
    pub process: ProcessInfo,
    /// Child processes
    pub children: Vec<ProcessTree>,
}

#[cfg(feature = "process")]
fn map_status(status: sysinfo::ProcessStatus) -> ProcessStatus {
    match status {
        sysinfo::ProcessStatus::Run => ProcessStatus::Running,
        sysinfo::ProcessStatus::Sleep => ProcessStatus::Sleeping,
        sysinfo::ProcessStatus::Stop => ProcessStatus::Stopped,
        sysinfo::ProcessStatus::Zombie => ProcessStatus::Zombie,
        sysinfo::ProcessStatus::Dead => ProcessStatus::Dead,
        _ => ProcessStatus::Unknown,
    }
}

#[cfg(feature = "process")]
fn process_from_sysinfo(pid: u32, process: &sysinfo::Process) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: process.parent().map(|p| p.as_u32()),
        name: process.name().to_string_lossy().to_string(),
        exe_path: process.exe().map(|p| p.to_string_lossy().to_string()),
        cwd: process.cwd().map(|p| p.to_string_lossy().to_string()),
        status: map_status(process.status()),
        memory: process.memory() as usize,
        virtual_memory: process.virtual_memory() as usize,
        cpu_usage: process.cpu_usage(),
        thread_count: 1,
        uid: None,
        gid: None,
    }
}

/// Get information about current process
pub fn current() -> SystemResult<ProcessInfo> {
    #[cfg(feature = "process")]
    {
        get_process(std::process::id())
    }

    #[cfg(not(feature = "process"))]
    {
        Err(SystemError::feature_not_supported(
            "Process feature not enabled",
        ))
    }
}

/// Get information about a specific process
pub fn get_process(pid: u32) -> SystemResult<ProcessInfo> {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;
        use sysinfo::{Pid, ProcessesToUpdate};

        let mut sys = SYSINFO_SYSTEM.write();
        let _ = sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), false);

        sys.process(Pid::from_u32(pid))
            .map(|p| process_from_sysinfo(pid, p))
            .ok_or_else(|| SystemError::resource_not_found(format!("Process {pid} not found")))
    }

    #[cfg(not(feature = "process"))]
    {
        Err(SystemError::feature_not_supported(
            "Process feature not enabled",
        ))
    }
}

/// List all processes
pub fn list() -> Vec<ProcessInfo> {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;

        let mut sys = SYSINFO_SYSTEM.write();
        let _ = sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        sys.processes()
            .iter()
            .map(|(pid, process)| process_from_sysinfo(pid.as_u32(), process))
            .collect()
    }

    #[cfg(not(feature = "process"))]
    {
        Vec::new()
    }
}

/// Get process statistics
///
/// Refreshes process data before reading to avoid stale snapshots.
pub fn stats() -> ProcessStats {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;

        let mut sys = SYSINFO_SYSTEM.write();
        let _ = sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut total = 0;
        let mut running = 0;
        let mut sleeping = 0;
        let mut total_memory = 0;
        let mut total_cpu = 0.0;

        for process in sys.processes().values() {
            total += 1;
            match process.status() {
                sysinfo::ProcessStatus::Run => running += 1,
                sysinfo::ProcessStatus::Sleep => sleeping += 1,
                _ => {}
            }
            total_memory += process.memory() as usize;
            total_cpu += process.cpu_usage();
        }

        ProcessStats {
            total,
            running,
            sleeping,
            total_memory,
            total_cpu,
        }
    }

    #[cfg(not(feature = "process"))]
    {
        ProcessStats {
            total: 0,
            running: 0,
            sleeping: 0,
            total_memory: 0,
            total_cpu: 0.0,
        }
    }
}

/// Find processes by name (substring match)
pub fn find_by_name(name: &str) -> Vec<ProcessInfo> {
    list()
        .into_iter()
        .filter(|p| p.name.contains(name))
        .collect()
}

/// Get child processes of a parent
pub fn children(parent_pid: u32) -> Vec<ProcessInfo> {
    list()
        .into_iter()
        .filter(|p| p.parent_pid == Some(parent_pid))
        .collect()
}

/// Build process tree
pub fn tree() -> Vec<ProcessTree> {
    use std::collections::HashMap;

    let processes = list();
    let mut roots = Vec::new();
    let mut children_map: HashMap<u32, Vec<ProcessInfo>> = HashMap::new();

    for process in processes {
        if let Some(parent_pid) = process.parent_pid {
            children_map.entry(parent_pid).or_default().push(process);
        } else {
            roots.push(process);
        }
    }

    fn build_tree(
        process: &ProcessInfo,
        children_map: &HashMap<u32, Vec<ProcessInfo>>,
    ) -> ProcessTree {
        let children = children_map
            .get(&process.pid)
            .map(|children| {
                children
                    .iter()
                    .map(|child| build_tree(child, children_map))
                    .collect()
            })
            .unwrap_or_default();

        ProcessTree {
            process: process.clone(),
            children,
        }
    }

    roots
        .iter()
        .map(|root| build_tree(root, &children_map))
        .collect()
}

// ── Per-process monitoring ───────────────────────────────────────────────

/// A snapshot of process resource usage from [`ProcessMonitor::sample`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProcessSample {
    /// Process ID
    pub pid: u32,
    /// CPU usage percentage (0–100+ for multi-core)
    pub cpu_usage: f32,
    /// Resident memory in bytes
    pub memory: usize,
    /// Virtual memory in bytes
    pub virtual_memory: usize,
    /// Current process status
    pub status: ProcessStatus,
}

/// Tracks resource usage of a specific OS process over time.
///
/// Designed for sandbox monitoring: create when spawning a worker,
/// poll periodically via [`sample`](Self::sample), drop when the worker exits.
///
/// # Example
///
/// ```no_run
/// use nebula_system::process::ProcessMonitor;
///
/// let mut monitor = ProcessMonitor::new(std::process::id()).unwrap();
/// if let Some(sample) = monitor.sample() {
///     println!("memory: {} bytes, cpu: {:.1}%", sample.memory, sample.cpu_usage);
/// }
/// println!("peak memory: {} bytes", monitor.peak_memory());
/// println!("tracked for: {:?}", monitor.elapsed());
/// ```
#[derive(Debug)]
pub struct ProcessMonitor {
    pid: u32,
    peak_memory: usize,
    created_at: Instant,
}

impl ProcessMonitor {
    /// Create a monitor for the given PID.
    ///
    /// Verifies that the process exists at creation time.
    ///
    /// # Errors
    ///
    /// Returns [`SystemError::ResourceNotFound`] if the PID does not exist.
    /// Returns [`SystemError::FeatureNotSupported`] if the `process` feature is disabled.
    pub fn new(pid: u32) -> SystemResult<Self> {
        // Verify the process exists
        let info = get_process(pid)?;

        Ok(Self {
            pid,
            peak_memory: info.memory,
            created_at: Instant::now(),
        })
    }

    /// Sample current process metrics.
    ///
    /// Returns `None` if the process has exited since the last sample.
    /// Updates the internal peak memory high-water mark.
    #[must_use]
    pub fn sample(&mut self) -> Option<ProcessSample> {
        let info = get_process(self.pid).ok()?;

        if info.memory > self.peak_memory {
            self.peak_memory = info.memory;
        }

        Some(ProcessSample {
            pid: self.pid,
            cpu_usage: info.cpu_usage,
            memory: info.memory,
            virtual_memory: info.virtual_memory,
            status: info.status,
        })
    }

    /// Peak resident memory observed across all samples (bytes).
    #[must_use]
    pub fn peak_memory(&self) -> usize {
        self.peak_memory
    }

    /// How long this monitor has been tracking the process.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// The monitored PID.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }
}
