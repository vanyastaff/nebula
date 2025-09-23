//! Memory information collection and analysis
//!
//! This module provides utilities to gather system memory information
//! in a platform-independent way.

use std::fmt;

/// Memory information structure containing system memory details
#[derive(Debug, Clone)]
pub struct MemoryInfo {
    /// Total physical memory on the system (in bytes)
    pub total: usize,
    /// Available memory on the system (in bytes)
    pub available: usize,
    /// Free memory on the system (in bytes)
    pub free: usize,
    /// Page size of the system (in bytes)
    pub page_size: usize,
    /// Whether memory overcommit is enabled
    pub overcommit_enabled: bool,
    /// Memory that can be locked (in bytes, if applicable)
    pub lockable: Option<usize>,
    /// Number of NUMA nodes (if applicable)
    pub numa_nodes: Option<usize>,
}

impl MemoryInfo {
    /// Get current system memory information
    pub fn get() -> Self {
        let total = crate::platform::get_total_memory();
        let available = crate::platform::get_available_memory();
        let page_size = crate::platform::get_page_size();

        // Platform-specific implementations will fill these in more accurately
        #[cfg(target_os = "linux")]
        let (free, overcommit_enabled, lockable, numa_nodes) =
            crate::platform::linux::get_detailed_memory_info();

        #[cfg(target_os = "macos")]
        let (free, overcommit_enabled, lockable, numa_nodes) =
            crate::platform::macos::get_detailed_memory_info();

        #[cfg(target_os = "windows")]
        let (free, overcommit_enabled, lockable, numa_nodes) =
            crate::platform::windows::get_detailed_memory_info();

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let (free, overcommit_enabled, lockable, numa_nodes) = (0, false, None, None);

        Self { total, available, free, page_size, overcommit_enabled, lockable, numa_nodes }
    }

    /// Calculate memory pressure level based on available memory
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
        const KB: usize = 1024;
        const MB: usize = KB * 1024;
        const GB: usize = MB * 1024;

        if size >= GB {
            format!("{:.2} GB", size as f64 / GB as f64)
        } else if size >= MB {
            format!("{:.2} MB", size as f64 / MB as f64)
        } else if size >= KB {
            format!("{:.2} KB", size as f64 / KB as f64)
        } else {
            format!("{} bytes", size)
        }
    }
}

impl fmt::Display for MemoryInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Memory Information:")?;
        writeln!(f, "  Total: {}", Self::format_size(self.total))?;
        writeln!(f, "  Available: {}", Self::format_size(self.available))?;
        writeln!(f, "  Free: {}", Self::format_size(self.free))?;
        writeln!(f, "  Page Size: {}", Self::format_size(self.page_size))?;
        writeln!(
            f,
            "  Overcommit: {}",
            if self.overcommit_enabled { "Enabled" } else { "Disabled" }
        )?;

        if let Some(lockable) = self.lockable {
            writeln!(f, "  Lockable: {}", Self::format_size(lockable))?;
        }

        if let Some(numa_nodes) = self.numa_nodes {
            writeln!(f, "  NUMA Nodes: {}", numa_nodes)?;
        }

        writeln!(f, "  Pressure Level: {:?}", self.pressure_level())?;

        Ok(())
    }
}

/// Memory pressure levels for the system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Memory events that can be monitored
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

/// Create a platform-specific memory monitor
#[cfg(feature = "monitoring")]
pub fn create_memory_monitor() -> Box<dyn MemoryMonitor + 'static> {
    #[cfg(target_os = "linux")]
    {
        Box::new(crate::platform::linux::LinuxMemoryMonitor::new())
    }

    #[cfg(target_os = "macos")]
    {
        Box::new(crate::platform::macos::MacOsMemoryMonitor::new())
    }

    #[cfg(target_os = "windows")]
    {
        Box::new(crate::platform::windows::WindowsMemoryMonitor::new())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Box::new(DefaultMemoryMonitor::new())
    }
}

/// Default memory monitor that does nothing
#[cfg(feature = "monitoring")]
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub struct DefaultMemoryMonitor;

#[cfg(feature = "monitoring")]
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl DefaultMemoryMonitor {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "monitoring")]
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl MemoryMonitor for DefaultMemoryMonitor {
    fn start_monitoring(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn stop_monitoring(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn register_callback(
        &self,
        _callback: Box<dyn FnMut(MemoryEvent) + Send + 'static>,
    ) -> std::io::Result<()> {
        Ok(())
    }
}
