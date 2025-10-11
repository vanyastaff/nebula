//! Allocator-specific memory information
//!
//! This module provides memory information relevant to custom allocators.
//! For general system memory info, use `nebula-system::memory` directly.
//!
//! # Safety
//!
//! This module uses platform-specific syscalls to query memory information:
//! - Unix: libc::sysconf for page size
//! - Windows: GetSystemInfo with SYSTEM_INFO structure
//! - All FFI calls validated by OS

use std::fmt;

/// Memory pressure levels for allocator-specific monitoring
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPressureLevel {
    /// Low memory pressure (plenty of memory available)
    Low,
    /// Medium memory pressure
    Medium,
    /// High memory pressure (low available memory)
    High,
    /// Critical memory pressure (very little memory available)
    Critical,
    /// Unknown memory pressure (couldn't determine)
    Unknown,
}

impl MemoryPressureLevel {
    /// Check if memory pressure is concerning
    #[inline]
    pub fn is_concerning(&self) -> bool {
        matches!(self, Self::High | Self::Critical)
    }
}

/// Memory information for allocator management
///
/// Extended version of system memory info with allocator-specific details.
#[derive(Debug, Clone)]
pub struct MemoryInfo {
    /// Total physical memory (bytes)
    pub total: usize,
    /// Available memory (bytes)
    pub available: usize,
    /// Page size (bytes)
    pub page_size: usize,
    /// Memory that can be locked (bytes, if applicable)
    pub lockable: Option<usize>,
    /// Number of NUMA nodes (if applicable)
    pub numa_nodes: Option<usize>,
}

impl MemoryInfo {
    /// Get current allocator-relevant memory information
    pub fn get() -> Self {
        let sys_info = nebula_system::info::SystemInfo::get();

        Self {
            total: sys_info.memory.total,
            available: sys_info.memory.available,
            page_size: sys_info.memory.page_size,
            lockable: None,
            numa_nodes: Some(sys_info.hardware.numa_nodes),
        }
    }

    /// Calculate memory pressure level
    pub fn pressure_level(&self) -> MemoryPressureLevel {
        if self.total == 0 {
            return MemoryPressureLevel::Unknown;
        }

        let available_percent = (self.available as f64 / self.total as f64) * 100.0;

        if available_percent < 5.0 {
            MemoryPressureLevel::Critical
        } else if available_percent < 15.0 {
            MemoryPressureLevel::High
        } else if available_percent < 30.0 {
            MemoryPressureLevel::Medium
        } else {
            MemoryPressureLevel::Low
        }
    }

    /// Format memory size as human-readable string
    pub fn format_size(size: usize) -> String {
        nebula_system::utils::format_bytes_usize(size)
    }
}

impl fmt::Display for MemoryInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Allocator Memory Information:")?;
        writeln!(f, "  Total: {}", Self::format_size(self.total))?;
        writeln!(f, "  Available: {}", Self::format_size(self.available))?;
        writeln!(f, "  Page Size: {}", Self::format_size(self.page_size))?;

        if let Some(lockable) = self.lockable {
            writeln!(f, "  Lockable: {}", Self::format_size(lockable))?;
        }

        if let Some(numa_nodes) = self.numa_nodes {
            writeln!(f, "  NUMA Nodes: {}", numa_nodes)?;
        }

        writeln!(f, "  Pressure: {:?}", self.pressure_level())?;

        Ok(())
    }
}

/// Memory events for monitoring
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEvent {
    /// Memory pressure changed
    PressureChange(MemoryPressureLevel),
    /// Memory was trimmed by the system
    Trim(usize),
    /// Out of memory event occurred
    OutOfMemory,
    /// Memory configuration changed
    ConfigChange,
}

/// Memory monitoring capability
#[cfg(feature = "monitoring")]
pub trait MemoryMonitor: Send + Sync {
    /// Start monitoring memory events
    fn start_monitoring(&self) -> std::io::Result<()>;

    /// Stop monitoring memory events
    fn stop_monitoring(&self) -> std::io::Result<()>;

    /// Register a callback for memory events
    fn register_callback(
        &self,
        callback: Box<dyn FnMut(MemoryEvent) + Send + 'static>,
    ) -> std::io::Result<()>;
}

/// Get page size using platform-specific syscalls
pub fn get_page_size() -> usize {
    #[cfg(unix)]
    {
        // SAFETY: FFI call to libc::sysconf.
        // - _SC_PAGESIZE is a valid sysconf parameter
        // - sysconf returns page size or -1 on error
        // - Cast to usize is safe (page size is always positive)
        // - OS validates the query
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
    }

    #[cfg(windows)]
    {
        use winapi::um::sysinfoapi::{GetSystemInfo, SYSTEM_INFO};

        // SAFETY: FFI call to Windows GetSystemInfo.
        // - SYSTEM_INFO initialized with zeroed() (all zero bytes are valid)
        // - GetSystemInfo fills the structure with valid system information
        // - dwPageSize field contains the page size
        // - OS validates all structure fields
        unsafe {
            let mut info: SYSTEM_INFO = std::mem::zeroed();
            GetSystemInfo(&mut info);
            info.dwPageSize as usize
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        4096 // Default fallback
    }
}
