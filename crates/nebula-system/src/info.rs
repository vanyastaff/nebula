//! System information gathering

use crate::error::Result;
use parking_lot::RwLock;
use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Complete system information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SystemInfo {
    /// Operating system information
    pub os: OsInfo,
    /// CPU information
    pub cpu: CpuInfo,
    /// Memory information
    pub memory: MemoryInfo,
    /// Hardware information
    pub hardware: HardwareInfo,
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
    pub fn get() -> Arc<SystemInfo> {
        SYSTEM_INFO.clone()
    }

    /// Refresh and get current information
    pub fn refresh() -> Arc<SystemInfo> {
        let mut cache = SYSTEM_INFO_CACHE.write();
        *cache = Arc::new(detect_system_info());
        cache.clone()
    }

    /// Get current memory information (always fresh)
    pub fn current_memory() -> MemoryInfo {
        #[cfg(feature = "sysinfo")]
        {
            let mut sys = SYSINFO_SYSTEM.write();
            sys.refresh_memory();

            MemoryInfo {
                total: sys.total_memory() as usize,
                available: sys.available_memory() as usize,
                page_size: page_size(),
                swap_total: sys.total_swap() as usize,
                swap_available: sys.free_swap() as usize,
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
static SYSTEM_INFO: std::sync::LazyLock<Arc<SystemInfo>> = std::sync::LazyLock::new(|| Arc::new(detect_system_info()));

static SYSTEM_INFO_CACHE: std::sync::LazyLock<RwLock<Arc<SystemInfo>>> =
    std::sync::LazyLock::new(|| RwLock::new(SYSTEM_INFO.clone()));

#[cfg(feature = "sysinfo")]
pub(crate) static SYSINFO_SYSTEM: std::sync::LazyLock<RwLock<sysinfo::System>> = std::sync::LazyLock::new(|| {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    RwLock::new(sys)
});

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
                    .first().map_or_else(|| "Unknown".to_string(), |c| c.brand().to_string()),
                cores: System::physical_core_count().unwrap_or(cpus.len()),
                threads: cpus.len(),
                frequency_mhz: cpus.first().map_or(0, sysinfo::Cpu::frequency),
                vendor: cpus
                    .first().map_or_else(|| "Unknown".to_string(), |c| c.vendor_id().to_string()),
            }
        };

        let memory = MemoryInfo {
            total: sys.total_memory() as usize,
            available: sys.available_memory() as usize,
            page_size: page_size(),
            swap_total: sys.total_swap() as usize,
            swap_available: sys.free_swap() as usize,
        };

        let hardware = HardwareInfo {
            cache_line_size: detect_cache_line_size(),
            allocation_granularity: detect_allocation_granularity(),
            numa_nodes: detect_numa_nodes(),
            huge_page_size: detect_huge_page_size(),
        };

        SystemInfo {
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
                total: 0,
                available: 0,
                page_size: page_size(),
                swap_total: 0,
                swap_available: 0,
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
    #[cfg(feature = "memory")]
    return region::page::size();

    #[cfg(not(feature = "memory"))]
    return 4096; // Default
}

fn detect_cache_line_size() -> usize {
    // Most modern processors use 64-byte cache lines
    match std::env::consts::ARCH {
        "x86_64" | "x86" => 64,
        "aarch64" | "arm" => 64,
        _ => 64,
    }
}

fn detect_allocation_granularity() -> usize {
    #[cfg(windows)]
    return 65536; // Windows uses 64KB

    #[cfg(not(windows))]
    return page_size();
}

fn detect_numa_nodes() -> usize {
    // Simplified - would need platform-specific code for accurate detection
    1
}

fn detect_huge_page_size() -> Option<usize> {
    #[cfg(target_os = "linux")]
    return Some(2 * 1024 * 1024); // 2MB on Linux

    #[cfg(target_os = "windows")]
    return Some(2 * 1024 * 1024); // 2MB on Windows

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    return None;
}

/// Initialize the system information subsystem
pub fn init() -> Result<()> {
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
