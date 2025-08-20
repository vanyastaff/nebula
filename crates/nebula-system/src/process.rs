//! Process information and management

use crate::error::{Result, SystemError};
use std::collections::HashMap;

#[cfg(feature = "serde")]
use serde::{Serialize, Deserialize};

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
    /// Command line arguments
    pub cmd: Vec<String>,
    /// Working directory
    pub cwd: Option<String>,
    /// Environment variables
    pub environ: HashMap<String, String>,
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
    /// User ID (Unix)
    pub uid: Option<u32>,
    /// Group ID (Unix)
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

/// Get information about current process
pub fn current() -> Result<ProcessInfo> {
    #[cfg(feature = "process")]
    {
        let pid = std::process::id();
        get_process(pid)
    }

    #[cfg(not(feature = "process"))]
    {
        Err(SystemError::NotSupported(
            "Process feature not enabled".to_string()
        ))
    }
}

/// Get information about a specific process
pub fn get_process(pid: u32) -> Result<ProcessInfo> {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;
        use sysinfo::{Pid, ProcessesToUpdate};

        let mut sys = SYSINFO_SYSTEM.write();
        // sysinfo 0.37 removed refresh_process; use refresh_processes with a specific pid
        let _ = sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), false);

        if let Some(process) = sys.process(Pid::from_u32(pid)) {
            let status = match process.status() {
                sysinfo::ProcessStatus::Run => ProcessStatus::Running,
                sysinfo::ProcessStatus::Sleep => ProcessStatus::Sleeping,
                sysinfo::ProcessStatus::Stop => ProcessStatus::Stopped,
                sysinfo::ProcessStatus::Zombie => ProcessStatus::Zombie,
                sysinfo::ProcessStatus::Dead => ProcessStatus::Dead,
                _ => ProcessStatus::Unknown,
            };

            Ok(ProcessInfo {
                pid,
                parent_pid: process.parent().map(|p| p.as_u32()),
                name: process.name().to_string_lossy().to_string(),
                exe_path: process.exe().map(|p| p.to_string_lossy().to_string()),
                cmd: Vec::new(),
                cwd: process.cwd().map(|p| p.to_string_lossy().to_string()),
                environ: HashMap::new(),
                status,
                memory: process.memory() as usize,
                virtual_memory: process.virtual_memory() as usize,
                cpu_usage: process.cpu_usage(),
                thread_count: 1, // sysinfo doesn't provide thread count directly
                uid: None,
                gid: None,
            })
        } else {
            Err(SystemError::NotFound(format!("Process {} not found", pid)))
        }
    }

    #[cfg(not(feature = "process"))]
    {
        Err(SystemError::NotSupported(
            "Process feature not enabled".to_string()
        ))
    }
}

/// List all processes
pub fn list() -> Vec<ProcessInfo> {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;

        let mut sys = SYSINFO_SYSTEM.write();
        // sysinfo 0.37: refresh_processes requires params
        let _ = sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        sys.processes()
            .iter()
            .map(|(pid, process)| {
                let status = match process.status() {
                    sysinfo::ProcessStatus::Run => ProcessStatus::Running,
                    sysinfo::ProcessStatus::Sleep => ProcessStatus::Sleeping,
                    sysinfo::ProcessStatus::Stop => ProcessStatus::Stopped,
                    sysinfo::ProcessStatus::Zombie => ProcessStatus::Zombie,
                    sysinfo::ProcessStatus::Dead => ProcessStatus::Dead,
                    _ => ProcessStatus::Unknown,
                };

                ProcessInfo {
                    pid: pid.as_u32(),
                    parent_pid: process.parent().map(|p| p.as_u32()),
                    name: process.name().to_string_lossy().to_string(),
                    exe_path: process.exe().map(|p| p.to_string_lossy().to_string()),
                    cmd: Vec::new(),
                    cwd: process.cwd().map(|p| p.to_string_lossy().to_string()),
                    environ: HashMap::new(), // Skip for performance
                    status,
                    memory: process.memory() as usize,
                    virtual_memory: process.virtual_memory() as usize,
                    cpu_usage: process.cpu_usage(),
                    thread_count: 1,
                    uid: None,
                    gid: None,
                }
            })
            .collect()
    }

    #[cfg(not(feature = "process"))]
    {
        Vec::new()
    }
}

/// Get process statistics
pub fn stats() -> ProcessStats {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;

        let sys = SYSINFO_SYSTEM.read();

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

/// Find processes by name
pub fn find_by_name(name: &str) -> Vec<ProcessInfo> {
    list()
        .into_iter()
        .filter(|p| p.name.contains(name))
        .collect()
}

/// Kill a process
pub fn kill(pid: u32) -> Result<()> {
    #[cfg(feature = "process")]
    {
        use crate::info::SYSINFO_SYSTEM;
        use sysinfo::Pid;

        let mut sys = SYSINFO_SYSTEM.write();
        let pid = Pid::from_u32(pid);

        if let Some(process) = sys.process(pid) {
            if process.kill() {
                Ok(())
            } else {
                Err(SystemError::PlatformError {
                    message: "Failed to kill process".to_string(),
                    code: None,
                })
            }
        } else {
            Err(SystemError::NotFound(format!("Process {} not found", pid.as_u32())))
        }
    }

    #[cfg(not(feature = "process"))]
    {
        Err(SystemError::NotSupported(
            "Process feature not enabled".to_string()
        ))
    }
}

/// Process priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Priority {
    /// Lowest priority
    Idle,
    /// Below normal priority
    BelowNormal,
    /// Normal priority (default)
    Normal,
    /// Above normal priority
    AboveNormal,
    /// High priority
    High,
    /// Realtime priority (requires privileges)
    Realtime,
}

/// Set process priority
pub fn set_priority(pid: u32, priority: Priority) -> Result<()> {
    #[cfg(all(feature = "process", unix))]
    {
        use libc::{setpriority, PRIO_PROCESS};

        let nice_value = match priority {
            Priority::Idle => 19,
            Priority::BelowNormal => 10,
            Priority::Normal => 0,
            Priority::AboveNormal => -5,
            Priority::High => -10,
            Priority::Realtime => -20,
        };

        unsafe {
            if setpriority(PRIO_PROCESS, pid as u32, nice_value) != 0 {
                return Err(SystemError::PlatformError {
                    message: "Failed to set process priority".to_string(),
                    code: Some(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)),
                });
            }
        }

        Ok(())
    }

    #[cfg(not(all(feature = "process", unix)))]
    {
        Err(SystemError::NotSupported(
            "Process priority not supported on this platform".to_string()
        ))
    }
}

/// Get child processes of a parent
pub fn children(parent_pid: u32) -> Vec<ProcessInfo> {
    list()
        .into_iter()
        .filter(|p| p.parent_pid == Some(parent_pid))
        .collect()
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

/// Build process tree
pub fn tree() -> Vec<ProcessTree> {
    let processes = list();
    let mut roots = Vec::new();
    let mut children_map: HashMap<u32, Vec<ProcessInfo>> = HashMap::new();

    // Group processes by parent
    for process in processes {
        if let Some(parent_pid) = process.parent_pid {
            children_map.entry(parent_pid)
                .or_insert_with(Vec::new)
                .push(process);
        } else {
            // Root process (no parent)
            roots.push(process);
        }
    }

    // Build tree recursively
    fn build_tree(
        process: ProcessInfo,
        children_map: &HashMap<u32, Vec<ProcessInfo>>
    ) -> ProcessTree {
        let children = children_map
            .get(&process.pid)
            .map(|children| {
                children.iter()
                    .map(|child| build_tree(child.clone(), children_map))
                    .collect()
            })
            .unwrap_or_default();

        ProcessTree { process, children }
    }

    roots.into_iter()
        .map(|root| build_tree(root, &children_map))
        .collect()
}