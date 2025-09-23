//! Platform-specific memory optimizations and utilities
//!
//! This module provides platform-specific implementations for memory management
//! and optimizations on different operating systems.

#[cfg(unix)]
use libc;
#[cfg(windows)]
use winapi;

mod memory_info;
pub use memory_info::*;

// Conditionally include platform-specific implementations
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

// NUMA support (feature-gated)
#[cfg(feature = "numa-aware")]
mod numa;
#[cfg(feature = "numa-aware")]
pub use numa::*;

// Direct syscall optimizations
mod syscalls;
pub use syscalls::*;

/// Initialize platform-specific optimizations and features
pub fn initialize() -> std::io::Result<()> {
    // Detect platform capabilities
    let _capabilities = detect_capabilities();

    // Initialize NUMA support if available and enabled
    #[cfg(all(feature = "numa-aware", unix))]
    {
        if _capabilities.numa_supported {
            // Initialize NUMA library
            #[cfg(target_os = "linux")]
            {
                if numa::numa_available() {
                    // NUMA is available, perform any initialization if needed
                }
            }
        }
    }

    // Initialize memory pressure monitoring if enabled
    #[cfg(feature = "monitoring")]
    {
        if _capabilities.memory_pressure_notifications {
            // Could start a background thread for monitoring, but we'll do this
            // on demand
        }
    }

    Ok(())
}

/// Returns the page size for the current platform
pub fn get_page_size() -> usize {
    #[cfg(unix)]
    {
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
    }
    #[cfg(windows)]
    {
        use winapi::um::sysinfoapi::{GetSystemInfo, SYSTEM_INFO};
        unsafe {
            let mut system_info: SYSTEM_INFO = std::mem::zeroed();
            GetSystemInfo(&mut system_info);
            system_info.dwPageSize as usize
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        // Default fallback for other platforms
        4096
    }
}

/// Returns the total physical memory available on the system
pub fn get_total_memory() -> usize {
    #[cfg(unix)]
    {
        unsafe {
            let pages = libc::sysconf(libc::_SC_PHYS_PAGES) as usize;
            let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
            pages * page_size
        }
    }
    #[cfg(windows)]
    {
        use winapi::um::sysinfoapi::GetPhysicallyInstalledSystemMemory;
        let mut memory_kb: u64 = 0;
        unsafe {
            if GetPhysicallyInstalledSystemMemory(&mut memory_kb) != 0 {
                return (memory_kb * 1024) as usize;
            }
        }
        // Fallback if function fails
        0
    }
    #[cfg(not(any(unix, windows)))]
    {
        // Default fallback for other platforms
        0
    }
}

/// Returns the available memory on the system
pub fn get_available_memory() -> usize {
    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::get_available_memory_linux()
    }
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::get_available_memory_macos()
    }
    #[cfg(windows)]
    {
        crate::platform::windows::get_available_memory_windows()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        // Default fallback
        0
    }
}

/// Platform capability detection
#[derive(Debug, Clone, Copy)]
pub struct PlatformCapabilities {
    pub huge_pages_supported: bool,
    pub transparent_huge_pages_supported: bool,
    pub numa_supported: bool,
    pub memory_pressure_notifications: bool,
    pub mlock_supported: bool,
    pub can_overcommit: bool,
}

/// Detect platform capabilities
pub fn detect_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "linux")]
    {
        linux::detect_linux_capabilities()
    }
    #[cfg(target_os = "macos")]
    {
        macos::detect_macos_capabilities()
    }
    #[cfg(windows)]
    {
        windows::detect_windows_capabilities()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        // Default for unsupported platforms
        PlatformCapabilities {
            huge_pages_supported: false,
            transparent_huge_pages_supported: false,
            numa_supported: false,
            memory_pressure_notifications: false,
            mlock_supported: false,
            can_overcommit: false,
        }
    }
}
