//! CPU information and utilities

use crate::core::{SystemError, SystemResult};
use crate::info::SystemInfo;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// CPU usage information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CpuUsage {
    /// Per-core usage percentage (0-100)
    pub per_core: Vec<f32>,
    /// Average usage across all cores
    pub average: f32,
    /// Peak usage among all cores
    pub peak: f32,
    /// Number of cores above threshold (default 80%)
    pub cores_under_pressure: usize,
}

/// CPU features detection
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CpuFeatures {
    /// SSE support
    pub sse: bool,
    /// SSE2 support
    pub sse2: bool,
    /// SSE3 support
    pub sse3: bool,
    /// SSSE3 support
    pub ssse3: bool,
    /// SSE4.1 support
    pub sse41: bool,
    /// SSE4.2 support
    pub sse42: bool,
    /// AVX support
    pub avx: bool,
    /// AVX2 support
    pub avx2: bool,
    /// AVX512 support
    pub avx512: bool,
    /// AES-NI support
    pub aes: bool,
    /// POPCNT support
    pub popcnt: bool,
    /// FMA support
    pub fma: bool,
}

/// CPU cache information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CacheInfo {
    /// L1 data cache size per core (bytes)
    pub l1_data: Option<usize>,
    /// L1 instruction cache size per core (bytes)
    pub l1_instruction: Option<usize>,
    /// L2 cache size per core (bytes)
    pub l2: Option<usize>,
    /// L3 cache size (shared, bytes)
    pub l3: Option<usize>,
    /// Cache line size (bytes)
    pub line_size: usize,
}

/// CPU topology information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CpuTopology {
    /// Number of physical CPU packages/sockets
    pub packages: usize,
    /// Number of physical cores per package
    pub cores_per_package: usize,
    /// Number of threads per core (hyperthreading)
    pub threads_per_core: usize,
    /// NUMA nodes
    pub numa_nodes: Vec<NumaNode>,
}

/// NUMA node information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NumaNode {
    /// Node ID
    pub id: usize,
    /// CPUs in this node
    pub cpus: Vec<usize>,
    /// Memory in this node (bytes)
    pub memory: usize,
}

/// CPU pressure levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CpuPressure {
    /// Less than 50% usage
    Low,
    /// 50-70% usage
    Medium,
    /// 70-85% usage
    High,
    /// More than 85% usage
    Critical,
}

impl CpuPressure {
    /// Check if CPU pressure is concerning
    #[must_use]
    pub fn is_concerning(&self) -> bool {
        *self >= CpuPressure::High
    }

    /// Create from usage percentage
    #[must_use]
    pub fn from_usage(usage: f32) -> Self {
        if usage > 85.0 {
            CpuPressure::Critical
        } else if usage > 70.0 {
            CpuPressure::High
        } else if usage > 50.0 {
            CpuPressure::Medium
        } else {
            CpuPressure::Low
        }
    }
}

/// Get current CPU usage
pub fn usage() -> CpuUsage {
    #[cfg(feature = "sysinfo")]
    {
        use crate::info::SYSINFO_SYSTEM;

        let mut sys = SYSINFO_SYSTEM.write();
        sys.refresh_cpu_usage();

        let per_core: Vec<f32> = sys.cpus().iter().map(sysinfo::Cpu::cpu_usage).collect();

        let average = if per_core.is_empty() {
            0.0
        } else {
            per_core.iter().sum::<f32>() / per_core.len() as f32
        };

        let peak = per_core.iter().copied().fold(0.0, f32::max);

        let cores_under_pressure = per_core.iter().filter(|&&usage| usage > 80.0).count();

        CpuUsage {
            per_core,
            average,
            peak,
            cores_under_pressure,
        }
    }

    #[cfg(not(feature = "sysinfo"))]
    {
        CpuUsage {
            per_core: vec![],
            average: 0.0,
            peak: 0.0,
            cores_under_pressure: 0,
        }
    }
}

/// Get CPU pressure level
#[must_use]
pub fn pressure() -> CpuPressure {
    let usage = usage();
    CpuPressure::from_usage(usage.average)
}

/// Get CPU features
#[must_use]
pub fn features() -> CpuFeatures {
    #[cfg(all(any(target_arch = "x86", target_arch = "x86_64"), feature = "sysinfo"))]
    {
        CpuFeatures {
            sse: is_x86_feature_detected!("sse"),
            sse2: is_x86_feature_detected!("sse2"),
            sse3: is_x86_feature_detected!("sse3"),
            ssse3: is_x86_feature_detected!("ssse3"),
            sse41: is_x86_feature_detected!("sse4.1"),
            sse42: is_x86_feature_detected!("sse4.2"),
            avx: is_x86_feature_detected!("avx"),
            avx2: is_x86_feature_detected!("avx2"),
            avx512: cfg!(target_feature = "avx512f"),
            aes: is_x86_feature_detected!("aes"),
            popcnt: is_x86_feature_detected!("popcnt"),
            fma: is_x86_feature_detected!("fma"),
        }
    }

    #[cfg(not(all(any(target_arch = "x86", target_arch = "x86_64"), feature = "sysinfo")))]
    {
        CpuFeatures::default()
    }
}

/// Get CPU cache information
#[must_use]
pub fn cache_info() -> CacheInfo {
    let info = SystemInfo::get();

    // Try to detect from /sys on Linux
    #[cfg(target_os = "linux")]
    {
        let l1d = read_cache_size("/sys/devices/system/cpu/cpu0/cache/index0/size");
        let l1i = read_cache_size("/sys/devices/system/cpu/cpu0/cache/index1/size");
        let l2 = read_cache_size("/sys/devices/system/cpu/cpu0/cache/index2/size");
        let l3 = read_cache_size("/sys/devices/system/cpu/cpu0/cache/index3/size");

        return CacheInfo {
            l1_data: l1d,
            l1_instruction: l1i,
            l2,
            l3,
            line_size: info.hardware.cache_line_size,
        };
    }

    // Default values for other platforms
    #[cfg(not(target_os = "linux"))]
    {
        CacheInfo {
            l1_data: Some(32 * 1024),        // 32 KB (typical)
            l1_instruction: Some(32 * 1024), // 32 KB (typical)
            l2: Some(256 * 1024),            // 256 KB (typical)
            l3: Some(8 * 1024 * 1024),       // 8 MB (typical)
            line_size: info.hardware.cache_line_size,
        }
    }
}

#[cfg(target_os = "linux")]
fn read_cache_size(path: &str) -> Option<usize> {
    use std::fs;

    fs::read_to_string(path).ok().and_then(|s| {
        let s = s.trim();
        if s.ends_with('K') {
            s[..s.len() - 1].parse::<usize>().ok().map(|v| v * 1024)
        } else if s.ends_with('M') {
            s[..s.len() - 1]
                .parse::<usize>()
                .ok()
                .map(|v| v * 1024 * 1024)
        } else {
            s.parse().ok()
        }
    })
}

/// Get CPU topology
#[must_use]
pub fn topology() -> CpuTopology {
    let info = SystemInfo::get();

    // Simplified topology detection
    let threads = info.cpu.threads;
    let cores = info.cpu.cores;
    let threads_per_core = if cores > 0 { threads / cores } else { 1 };

    // Detect NUMA nodes
    let numa_nodes = detect_numa_nodes();

    CpuTopology {
        packages: 1, // Simplified - would need platform-specific code
        cores_per_package: cores,
        threads_per_core,
        numa_nodes,
    }
}

fn detect_numa_nodes() -> Vec<NumaNode> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        let mut nodes = Vec::new();
        let node_path = "/sys/devices/system/node/";

        if let Ok(entries) = fs::read_dir(node_path) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with("node") {
                    if let Ok(id) = name_str[4..].parse::<usize>() {
                        // Read CPUs for this node
                        let cpu_list_path = format!("{}/node{}/cpulist", node_path, id);
                        let cpus = fs::read_to_string(cpu_list_path)
                            .ok()
                            .and_then(|s| parse_cpu_list(&s))
                            .unwrap_or_default();

                        nodes.push(NumaNode {
                            id,
                            cpus,
                            memory: 0, // Would need to read from meminfo
                        });
                    }
                }
            }
        }

        if !nodes.is_empty() {
            return nodes;
        }
    }

    // Default single node
    let info = SystemInfo::get();
    vec![NumaNode {
        id: 0,
        cpus: (0..info.cpu.threads).collect(),
        memory: info.memory.total,
    }]
}

#[cfg(target_os = "linux")]
fn parse_cpu_list(s: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();

    for part in s.trim().split(',') {
        if let Some(dash_pos) = part.find('-') {
            // Range like "0-3"
            let start = part[..dash_pos].parse::<usize>().ok()?;
            let end = part[dash_pos + 1..].parse::<usize>().ok()?;
            cpus.extend(start..=end);
        } else {
            // Single CPU
            if let Ok(cpu) = part.parse::<usize>() {
                cpus.push(cpu);
            }
        }
    }

    Some(cpus)
}

/// Get optimal number of threads for parallel work
#[must_use]
pub fn optimal_thread_count() -> usize {
    let info = SystemInfo::get();
    let topology = topology();

    // If we have NUMA nodes, consider them
    if topology.numa_nodes.len() > 1 {
        // Use one thread per physical core to minimize NUMA effects
        info.cpu.cores
    } else {
        // Use all logical processors
        info.cpu.threads
    }
}

/// CPU affinity management
#[cfg(feature = "sysinfo")]
pub mod affinity {
    use super::{SystemError, SystemResult};

    /// Set CPU affinity for current thread
    #[cfg(target_os = "linux")]
    pub fn set_current_thread(cpus: &[usize]) -> SystemResult<()> {
        use libc::{CPU_SET, CPU_ZERO, cpu_set_t, sched_setaffinity};
        use std::mem;

        // SAFETY: Using libc CPU affinity macros and syscalls:
        // - `cpu_set_t` is a C struct with no Drop or pointers, safe to zero-initialize
        // - `CPU_ZERO` macro safely initializes the cpu_set_t
        // - `CPU_SET` macro safely sets individual CPU bits (cpus validated by caller)
        // - `sched_setaffinity(0, ...)` targets current thread (PID=0)
        // - Size and pointer to `set` are valid for the duration of the syscall
        // Returns 0 on success, -1 on failure (sets errno).
        unsafe {
            let mut set: cpu_set_t = mem::zeroed();
            CPU_ZERO(&mut set);

            for &cpu in cpus {
                CPU_SET(cpu, &mut set);
            }

            if sched_setaffinity(0, mem::size_of::<cpu_set_t>(), &set) != 0 {
                return Err(crate::core::error::SystemError::PlatformError(format!(
                    "Failed to set CPU affinity: {}",
                    std::io::Error::last_os_error()
                )));
            }
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    /// Set CPU affinity for current thread (not supported on this platform)
    pub fn set_current_thread(_cpus: &[usize]) -> SystemResult<()> {
        Err(SystemError::feature_not_supported(
            "CPU affinity not supported on this platform",
        ))
    }
}
