//! System information gathering

use std::{sync::Arc, time::SystemTime};

#[cfg(feature = "sysinfo")]
use parking_lot::RwLock;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{availability::Availability, error::SystemResult};

/// Complete system information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SystemInfo {
    /// Metadata describing how this snapshot was captured.
    pub metadata: SnapshotMetadata,
    /// Operating system information
    pub os: OsInfo,
    /// CPU information
    pub cpu: CpuInfo,
    /// Memory information
    pub memory: MemoryInfo,
    /// Hardware information
    pub hardware: HardwareInfo,
}

/// Freshness semantics for a system snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SnapshotFreshness {
    /// Snapshot is cached for the process lifetime.
    Cached,
    /// Snapshot was freshly observed when returned.
    Fresh,
    /// Snapshot is known to be stale.
    Stale,
}

/// Metadata attached to a system snapshot.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SnapshotMetadata {
    /// Wall-clock time when the snapshot was observed.
    pub observed_at: SystemTime,
    /// Freshness contract for this snapshot.
    pub freshness: SnapshotFreshness,
    /// Backend/source used to create the snapshot.
    pub source: String,
}

/// Operating system information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OsInfo {
    /// OS name (e.g., "Windows", "Linux", "macOS")
    pub name: String,
    /// OS version
    pub version: String,
    /// Kernel version
    pub kernel_version: String,
    /// System hostname
    pub hostname: String,
    /// CPU architecture
    pub arch: String,
    /// OS family
    pub family: OsFamily,
}

/// OS family classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum OsFamily {
    /// Microsoft Windows
    Windows,
    /// Linux distributions
    Linux,
    /// Apple macOS
    MacOS,
    /// BSD variants
    BSD,
    /// Other Unix-like systems
    Unix,
    /// Unknown OS
    Unknown,
}

/// CPU information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CpuInfo {
    /// CPU brand/model string
    pub brand: String,
    /// Number of physical cores
    pub cores: usize,
    /// Number of logical processors (threads)
    pub threads: usize,
    /// CPU frequency in MHz
    pub frequency_mhz: u64,
    /// CPU vendor
    pub vendor: String,
}

/// Memory information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryInfo {
    /// Metadata describing how this memory snapshot was captured.
    pub metadata: SnapshotMetadata,
    /// Total physical memory in bytes
    pub total: usize,
    /// Available physical memory in bytes
    pub available: usize,
    /// Memory page size in bytes
    pub page_size: usize,
    /// Total swap space in bytes
    pub swap_total: usize,
    /// Available swap space in bytes
    pub swap_available: usize,
    /// Effective memory capacity for scheduling decisions.
    pub effective: EffectiveMemoryInfo,
    /// Linux cgroup memory limit if detected.
    pub cgroup: Availability<CgroupMemoryInfo>,
}

/// Source used for effective memory capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MemoryCapacitySource {
    /// Effective capacity is the host memory reported by the OS.
    Host,
    /// Effective capacity comes from Linux cgroup limits.
    Cgroup,
}

/// Effective memory capacity for scheduling-facing probes.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EffectiveMemoryInfo {
    /// Effective total memory in bytes.
    pub total: usize,
    /// Effective available memory in bytes.
    pub available: usize,
    /// Source used to derive effective capacity.
    pub source: MemoryCapacitySource,
}

/// Linux cgroup memory data.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CgroupMemoryInfo {
    /// Total memory limit in bytes for the cgroup.
    pub total: usize,
    /// Free memory in bytes for the cgroup.
    pub free: usize,
    /// Free swap in bytes for the cgroup.
    pub free_swap: usize,
    /// Resident set size in bytes for the cgroup.
    pub rss: usize,
}

/// Hardware information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HardwareInfo {
    /// CPU cache line size in bytes
    pub cache_line_size: usize,
    /// Memory allocation granularity
    pub allocation_granularity: usize,
    /// Number of NUMA nodes
    pub numa_nodes: usize,
    /// Huge page size if supported
    pub huge_page_size: Option<usize>,
}

impl SystemInfo {
    /// Get cached system information
    ///
    /// Returns a cheap reference-counted pointer to the cached system info.
    /// `Arc::clone` is cheap (just increments a counter).
    #[inline]
    #[must_use]
    pub fn get() -> Arc<SystemInfo> {
        Arc::clone(&SYSTEM_INFO)
    }

    /// Get current memory information.
    ///
    /// With the `sysinfo` backend enabled, this refreshes memory and returns
    /// `SnapshotFreshness::Fresh` metadata. Without it, this returns the cached
    /// fallback snapshot and preserves cached freshness metadata.
    pub fn current_memory() -> MemoryInfo {
        #[cfg(feature = "sysinfo")]
        {
            let mut sys = SYSINFO_SYSTEM.write();
            sys.refresh_memory();

            MemoryInfo {
                metadata: SnapshotMetadata {
                    observed_at: SystemTime::now(),
                    freshness: SnapshotFreshness::Fresh,
                    source: "sysinfo".to_string(),
                },
                total: sys.total_memory() as usize,
                available: sys.available_memory() as usize,
                page_size: page_size(),
                swap_total: sys.total_swap() as usize,
                swap_available: sys.free_swap() as usize,
                effective: effective_memory_info(&sys),
                cgroup: cgroup_memory_info(&sys),
            }
        }

        #[cfg(not(feature = "sysinfo"))]
        {
            let info = Self::get();
            info.memory.clone()
        }
    }
}

// Global cached instances
static SYSTEM_INFO: std::sync::LazyLock<Arc<SystemInfo>> =
    std::sync::LazyLock::new(|| Arc::new(detect_system_info()));

#[cfg(feature = "sysinfo")]
pub(crate) static SYSINFO_SYSTEM: std::sync::LazyLock<RwLock<sysinfo::System>> =
    std::sync::LazyLock::new(|| {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();
        RwLock::new(sys)
    });

#[cfg(feature = "sysinfo")]
fn to_usize_saturating(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

#[cfg(feature = "sysinfo")]
fn cgroup_memory_info(sys: &sysinfo::System) -> Availability<CgroupMemoryInfo> {
    #[cfg(target_os = "linux")]
    {
        sys.cgroup_limits().map_or_else(
            || Availability::unavailable("no Linux cgroup memory limit detected"),
            |limits| {
                Availability::available(CgroupMemoryInfo {
                    total: to_usize_saturating(limits.total_memory),
                    free: to_usize_saturating(limits.free_memory),
                    free_swap: to_usize_saturating(limits.free_swap),
                    rss: to_usize_saturating(limits.rss),
                })
            },
        )
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = sys;
        Availability::unsupported("cgroup memory limits are Linux-only")
    }
}

#[cfg(feature = "sysinfo")]
fn effective_memory_info(sys: &sysinfo::System) -> EffectiveMemoryInfo {
    let host_total = sys.total_memory();
    let host_available = sys.available_memory();

    #[cfg(target_os = "linux")]
    {
        effective_memory_from_values(
            host_total,
            host_available,
            sys.cgroup_limits()
                .map(|limits| (limits.total_memory, limits.free_memory)),
        )
    }

    #[cfg(not(target_os = "linux"))]
    {
        effective_memory_from_values(host_total, host_available, None)
    }
}

#[cfg(feature = "sysinfo")]
fn effective_memory_from_values(
    host_total: u64,
    host_available: u64,
    cgroup: Option<(u64, u64)>,
) -> EffectiveMemoryInfo {
    if let Some((cgroup_total, cgroup_available)) = cgroup {
        // sysinfo reports unbounded cgroups as very large pseudo-limits.
        // Treat values larger than physical host memory as unlimited and
        // fall back to the host capacity rather than exposing usize::MAX.
        if cgroup_total > 0 && cgroup_total <= host_total {
            return EffectiveMemoryInfo {
                total: to_usize_saturating(cgroup_total),
                available: to_usize_saturating(cgroup_available),
                source: MemoryCapacitySource::Cgroup,
            };
        }
    }

    EffectiveMemoryInfo {
        total: to_usize_saturating(host_total),
        available: to_usize_saturating(host_available),
        source: MemoryCapacitySource::Host,
    }
}

fn detect_system_info() -> SystemInfo {
    #[cfg(feature = "sysinfo")]
    {
        use sysinfo::System;

        let sys = SYSINFO_SYSTEM.read();

        let os = OsInfo {
            name: System::name().unwrap_or_else(|| "Unknown".to_string()),
            version: System::os_version().unwrap_or_else(|| "Unknown".to_string()),
            kernel_version: System::kernel_version().unwrap_or_else(|| "Unknown".to_string()),
            hostname: System::host_name().unwrap_or_else(|| "Unknown".to_string()),
            arch: std::env::consts::ARCH.to_string(),
            family: detect_os_family(),
        };

        let cpu = {
            let cpus = sys.cpus();
            CpuInfo {
                brand: cpus
                    .first()
                    .map_or_else(|| "Unknown".to_string(), |c| c.brand().to_string()),
                cores: System::physical_core_count().unwrap_or(cpus.len()),
                threads: cpus.len(),
                frequency_mhz: cpus.first().map_or(0, sysinfo::Cpu::frequency),
                vendor: cpus
                    .first()
                    .map_or_else(|| "Unknown".to_string(), |c| c.vendor_id().to_string()),
            }
        };

        let memory = MemoryInfo {
            metadata: SnapshotMetadata {
                observed_at: SystemTime::now(),
                freshness: SnapshotFreshness::Cached,
                source: "sysinfo".to_string(),
            },
            total: sys.total_memory() as usize,
            available: sys.available_memory() as usize,
            page_size: page_size(),
            swap_total: sys.total_swap() as usize,
            swap_available: sys.free_swap() as usize,
            effective: effective_memory_info(&sys),
            cgroup: cgroup_memory_info(&sys),
        };

        let hardware = HardwareInfo {
            cache_line_size: detect_cache_line_size(),
            allocation_granularity: detect_allocation_granularity(),
            numa_nodes: detect_numa_nodes(),
            huge_page_size: detect_huge_page_size(),
        };

        SystemInfo {
            metadata: SnapshotMetadata {
                observed_at: SystemTime::now(),
                freshness: SnapshotFreshness::Cached,
                source: "sysinfo".to_string(),
            },
            os,
            cpu,
            memory,
            hardware,
        }
    }

    #[cfg(not(feature = "sysinfo"))]
    {
        // Fallback implementation without sysinfo
        SystemInfo {
            metadata: SnapshotMetadata {
                observed_at: SystemTime::now(),
                freshness: SnapshotFreshness::Cached,
                source: "fallback:no-sysinfo".to_string(),
            },
            os: OsInfo {
                name: std::env::consts::OS.to_string(),
                version: "Unknown".to_string(),
                kernel_version: "Unknown".to_string(),
                hostname: "Unknown".to_string(),
                arch: std::env::consts::ARCH.to_string(),
                family: detect_os_family(),
            },
            cpu: CpuInfo {
                brand: "Unknown".to_string(),
                cores: std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1),
                threads: std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1),
                frequency_mhz: 0,
                vendor: "Unknown".to_string(),
            },
            memory: MemoryInfo {
                metadata: SnapshotMetadata {
                    observed_at: SystemTime::now(),
                    freshness: SnapshotFreshness::Cached,
                    source: "fallback:no-sysinfo".to_string(),
                },
                total: 0,
                available: 0,
                page_size: page_size(),
                swap_total: 0,
                swap_available: 0,
                effective: EffectiveMemoryInfo {
                    total: 0,
                    available: 0,
                    source: MemoryCapacitySource::Host,
                },
                cgroup: Availability::unsupported("sysinfo feature is disabled"),
            },
            hardware: HardwareInfo {
                cache_line_size: 64,
                allocation_granularity: page_size(),
                numa_nodes: 1,
                huge_page_size: None,
            },
        }
    }
}

fn detect_os_family() -> OsFamily {
    match std::env::consts::OS {
        "windows" => OsFamily::Windows,
        "linux" => OsFamily::Linux,
        "macos" => OsFamily::MacOS,
        "freebsd" | "openbsd" | "netbsd" => OsFamily::BSD,
        "android" | "ios" => OsFamily::Unix,
        _ => OsFamily::Unknown,
    }
}

fn page_size() -> usize {
    // Default page size for most architectures
    4096
}

#[cfg(feature = "sysinfo")]
fn detect_cache_line_size() -> usize {
    // Most modern processors use 64-byte cache lines
    64
}

#[cfg(feature = "sysinfo")]
fn detect_allocation_granularity() -> usize {
    #[cfg(windows)]
    return 65536; // Windows uses 64KB

    #[cfg(not(windows))]
    return page_size();
}

#[cfg(feature = "sysinfo")]
fn detect_numa_nodes() -> usize {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        let node_path = "/sys/devices/system/node/";
        if let Ok(entries) = fs::read_dir(node_path) {
            let count = entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_prefix("node")
                        .is_some_and(|n| n.parse::<usize>().is_ok())
                })
                .count();
            if count > 0 {
                return count;
            }
        }
    }

    1
}

#[cfg(feature = "sysinfo")]
#[allow(clippy::unnecessary_wraps)] // target-dependent: each cfg branch sees only one return flavour; #[expect] would be unfulfilled on targets where only Some(_) or only None is visible
fn detect_huge_page_size() -> Option<usize> {
    #[cfg(target_os = "linux")]
    return Some(2 * 1024 * 1024); // 2MB on Linux

    #[cfg(target_os = "windows")]
    return Some(2 * 1024 * 1024); // 2MB on Windows

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    return None;
}

/// Initialize the system information subsystem
pub fn init() -> SystemResult<()> {
    // Force lazy static initialization
    let _ = SystemInfo::get();

    #[cfg(feature = "sysinfo")]
    {
        // Refresh system information
        let mut sys = SYSINFO_SYSTEM.write();
        sys.refresh_all();
    }

    Ok(())
}

/// Get a formatted summary of system information
#[must_use]
pub fn summary() -> String {
    let info = SystemInfo::get();

    format!(
        "System: {} {} ({})\n\
         CPU: {} ({} cores / {} threads @ {} MHz)\n\
         Memory: {:.2} GB total, {:.2} GB available\n\
         Architecture: {}\n\
         Page Size: {} bytes",
        info.os.name,
        info.os.version,
        info.os.kernel_version,
        info.cpu.brand,
        info.cpu.cores,
        info.cpu.threads,
        info.cpu.frequency_mhz,
        info.memory.total as f64 / (1024.0 * 1024.0 * 1024.0),
        info.memory.available as f64 / (1024.0 * 1024.0 * 1024.0),
        info.os.arch,
        info.memory.page_size
    )
}

#[cfg(all(test, feature = "sysinfo"))]
mod tests {
    use super::{MemoryCapacitySource, effective_memory_from_values};

    #[test]
    fn unbounded_cgroup_limit_falls_back_to_host_memory() {
        let info = effective_memory_from_values(64 * 1024, 32 * 1024, Some((u64::MAX, u64::MAX)));

        assert_eq!(info.source, MemoryCapacitySource::Host);
        assert_eq!(info.total, 64 * 1024);
        assert_eq!(info.available, 32 * 1024);
    }

    #[test]
    fn bounded_cgroup_limit_is_effective_memory() {
        let info = effective_memory_from_values(64 * 1024, 32 * 1024, Some((2048, 1024)));

        assert_eq!(info.source, MemoryCapacitySource::Cgroup);
        assert_eq!(info.total, 2048);
        assert_eq!(info.available, 1024);
    }
}
