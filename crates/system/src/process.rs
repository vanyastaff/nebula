//! Process information
//!
//! Provides process listing, lookup, and statistics. Designed for
//! monitoring sandbox workers and system-level process awareness.
//!
//! # Known Limitations
//!
//! - **`thread_count`** is exposed as `Availability<usize>`. Linux task metadata is reported when
//!   sysinfo exposes it; other platforms return an explicit unsupported/unavailable status.
//! - **`uid` / `gid`** are exposed as `Availability<u32>` because platform and permission support
//!   differs. Unknown identifiers must not be interpreted as UID/GID zero.
//! - **`cpu_usage`** requires previous backend refresh state. First and stale samples are explicit
//!   availability states, not measured `0.0` values.

use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

use parking_lot::RwLock;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "process")]
use crate::availability::sample_status_for_interval;
use crate::{
    availability::{Availability, AvailabilityStatus},
    error::{SystemError, SystemResult},
};

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
    /// CPU usage percentage.
    ///
    /// Process CPU usage requires previous refresh state in the backend, so
    /// first/stale samples are represented explicitly instead of returning
    /// `0.0` as if it were a measured value.
    pub cpu_usage: Availability<f32>,
    /// Number of threads
    pub thread_count: Availability<usize>,
    /// User ID where the platform/backend exposes it
    pub uid: Availability<u32>,
    /// Group ID where the platform/backend exposes it
    pub gid: Availability<u32>,
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
    pub total_cpu: Availability<f32>,
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
static PROCESS_CPU_SAMPLE_STATE: LazyLock<RwLock<Option<Instant>>> =
    LazyLock::new(|| RwLock::new(None));

#[cfg(feature = "process")]
fn next_process_cpu_sample_status() -> AvailabilityStatus {
    let now = Instant::now();
    let mut last_sample = PROCESS_CPU_SAMPLE_STATE.write();
    sample_status_for_interval(now, &mut last_sample, sysinfo::MINIMUM_CPU_UPDATE_INTERVAL)
}

#[cfg(feature = "process")]
fn sampled_cpu_usage(value: f32, status: AvailabilityStatus) -> Availability<f32> {
    match status {
        AvailabilityStatus::Available => Availability::available(value),
        AvailabilityStatus::NotSampled => {
            Availability::not_sampled("first process CPU sample has no previous backend refresh")
        },
        AvailabilityStatus::Stale => Availability::stale(
            Some(value),
            "process CPU sample refreshed before backend minimum interval",
        ),
        _ => Availability::unavailable("process CPU sample is unavailable"),
    }
}

#[cfg(feature = "process")]
fn uid_availability(uid: Option<&sysinfo::Uid>) -> Availability<u32> {
    match uid.and_then(|id| id.to_string().parse::<u32>().ok()) {
        Some(uid) => Availability::available(uid),
        None if cfg!(windows) => {
            Availability::unsupported("process uid is not available on Windows")
        },
        None => Availability::unavailable("backend did not return process uid"),
    }
}

#[cfg(feature = "process")]
fn gid_availability(gid: Option<sysinfo::Gid>) -> Availability<u32> {
    match gid.and_then(|id| id.to_string().parse::<u32>().ok()) {
        Some(gid) => Availability::available(gid),
        None if cfg!(windows) => {
            Availability::unsupported("process gid is not available on Windows")
        },
        None => Availability::unavailable("backend did not return process gid"),
    }
}

#[cfg(feature = "process")]
fn thread_count_availability(process: &sysinfo::Process) -> Availability<usize> {
    match process.tasks() {
        Some(tasks) => Availability::available(tasks.len().max(1)),
        None => Availability::unsupported(
            "process thread/task count is exposed only on platforms where sysinfo reports tasks",
        ),
    }
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
fn process_from_sysinfo(
    pid: u32,
    process: &sysinfo::Process,
    cpu_status: AvailabilityStatus,
) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: process.parent().map(sysinfo::Pid::as_u32),
        name: process.name().to_string_lossy().to_string(),
        exe_path: process.exe().map(|p| p.to_string_lossy().to_string()),
        cwd: process.cwd().map(|p| p.to_string_lossy().to_string()),
        status: map_status(process.status()),
        memory: process.memory() as usize,
        virtual_memory: process.virtual_memory() as usize,
        cpu_usage: sampled_cpu_usage(process.cpu_usage(), cpu_status),
        thread_count: thread_count_availability(process),
        uid: uid_availability(process.user_id()),
        gid: gid_availability(process.group_id()),
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
        use sysinfo::{Pid, ProcessesToUpdate};

        use crate::info::SYSINFO_SYSTEM;

        let mut sys = SYSINFO_SYSTEM.write();
        let _ = sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), false);
        let cpu_status = next_process_cpu_sample_status();

        sys.process(Pid::from_u32(pid))
            .map(|p| process_from_sysinfo(pid, p, cpu_status))
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
        let cpu_status = next_process_cpu_sample_status();

        sys.processes()
            .iter()
            .map(|(pid, process)| process_from_sysinfo(pid.as_u32(), process, cpu_status))
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
        let cpu_status = next_process_cpu_sample_status();

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
                _ => {},
            }
            total_memory += process.memory() as usize;
            total_cpu += process.cpu_usage();
        }

        ProcessStats {
            total,
            running,
            sleeping,
            total_memory,
            total_cpu: sampled_cpu_usage(total_cpu, cpu_status),
        }
    }

    #[cfg(not(feature = "process"))]
    {
        ProcessStats {
            total: 0,
            running: 0,
            sleeping: 0,
            total_memory: 0,
            total_cpu: Availability::unsupported("Process feature not enabled"),
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
    pub cpu_usage: Availability<f32>,
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
///     println!(
///         "memory: {} bytes, cpu: {:?}",
///         sample.memory, sample.cpu_usage
///     );
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
