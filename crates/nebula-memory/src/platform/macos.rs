//! macOS-specific memory optimizations and utilities.
//!
//! This module provides optimizations and utilities specific to macOS
//! platforms, including memory pressure monitoring and system-specific memory
//! management.

use std::io;

#[cfg(all(target_os = "macos", feature = "macos-optimizations"))]
use libc::{c_int, c_void, host_statistics64, mach_task_self, size_t, vm_statistics64};

use crate::platform::MemoryPressureLevel;

/// Get available memory on macOS
pub fn get_available_memory_macos() -> usize {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut stats: vm_statistics64 = std::mem::zeroed();
        let mut count =
            std::mem::size_of::<vm_statistics64>() as u32 / std::mem::size_of::<i32>() as u32;
        let host_port = mach_task_self();

        let kern_result = host_statistics64(
            host_port,
            6, // HOST_VM_INFO64
            (&mut stats) as *mut vm_statistics64 as *mut i32,
            &mut count,
        );

        if kern_result == 0 {
            // KERN_SUCCESS
            let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
            let free_count = stats.free_count as usize;
            return free_count * page_size;
        }
    }

    // Fallback to basic implementation if the above fails
    unsafe {
        let pages = libc::sysconf(libc::_SC_AVPHYS_PAGES) as usize;
        let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
        pages * page_size
    }
}

/// Get detailed memory information for macOS
pub fn get_detailed_memory_info() -> (usize, bool, Option<usize>, Option<usize>) {
    let mut free = 0;
    let mut lockable = None;
    let numa_nodes = Some(1); // macOS generally doesn't use NUMA

    #[cfg(target_os = "macos")]
    unsafe {
        let mut stats: vm_statistics64 = std::mem::zeroed();
        let mut count =
            std::mem::size_of::<vm_statistics64>() as u32 / std::mem::size_of::<i32>() as u32;
        let host_port = mach_task_self();

        let kern_result = host_statistics64(
            host_port,
            6, // HOST_VM_INFO64
            (&mut stats) as *mut vm_statistics64 as *mut i32,
            &mut count,
        );

        if kern_result == 0 {
            // KERN_SUCCESS
            let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
            free = stats.free_count as usize * page_size;
        }

        // Try to get max locked memory from getrlimit
        let mut rlimit = std::mem::zeroed::<libc::rlimit>();
        if libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut rlimit) == 0 {
            if rlimit.rlim_cur != libc::RLIM_INFINITY as libc::rlim_t {
                lockable = Some(rlimit.rlim_cur as usize);
            }
        }
    }

    // macOS generally overcommits memory like most Unix systems
    let overcommit_enabled = true;

    (free, overcommit_enabled, lockable, numa_nodes)
}

/// Detect macOS capabilities for memory management
pub fn detect_macos_capabilities() -> crate::platform::PlatformCapabilities {
    crate::platform::PlatformCapabilities {
        huge_pages_supported: false, // macOS doesn't support huge pages like Linux
        transparent_huge_pages_supported: false,
        numa_supported: false,               // macOS generally doesn't use NUMA
        memory_pressure_notifications: true, // macOS has memory pressure notifications
        mlock_supported: true,
        can_overcommit: true, // macOS generally overcommits memory
    }
}

/// Lock memory to prevent swapping on macOS
#[cfg(all(target_os = "macos", feature = "macos-optimizations"))]
pub fn lock_memory(ptr: *mut c_void, size: usize) -> io::Result<()> {
    let result = unsafe { libc::mlock(ptr, size) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Unlock memory on macOS
#[cfg(all(target_os = "macos", feature = "macos-optimizations"))]
pub fn unlock_memory(ptr: *mut c_void, size: usize) -> io::Result<()> {
    let result = unsafe { libc::munlock(ptr, size) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Memory advice for macOS (similar to madvise)
#[cfg(all(target_os = "macos", feature = "macos-optimizations"))]
pub fn memory_advice(ptr: *mut c_void, size: usize, advice: MemoryAdvice) -> io::Result<()> {
    let advice_value = match advice {
        MemoryAdvice::Normal => libc::MADV_NORMAL,
        MemoryAdvice::Random => libc::MADV_RANDOM,
        MemoryAdvice::Sequential => libc::MADV_SEQUENTIAL,
        MemoryAdvice::WillNeed => libc::MADV_WILLNEED,
        MemoryAdvice::DontNeed => libc::MADV_DONTNEED,
    };

    let result = unsafe { libc::madvise(ptr, size, advice_value) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Memory advice enum
#[derive(Debug, Clone, Copy)]
pub enum MemoryAdvice {
    Normal,
    Random,
    Sequential,
    WillNeed,
    DontNeed,
}

/// Monitor memory pressure on macOS
#[cfg(feature = "monitoring")]
pub struct MacOsMemoryMonitor {
    running: std::sync::atomic::AtomicBool,
    callback: std::sync::Mutex<Option<Box<dyn FnMut(crate::platform::MemoryEvent) + Send>>>,
}

#[cfg(feature = "monitoring")]
impl MacOsMemoryMonitor {
    pub fn new() -> Self {
        Self {
            running: std::sync::atomic::AtomicBool::new(false),
            callback: std::sync::Mutex::new(None),
        }
    }

    fn monitor_memory_pressure(
        running: &std::sync::atomic::AtomicBool,
        callback: &std::sync::Mutex<Option<Box<dyn FnMut(crate::platform::MemoryEvent) + Send>>>,
    ) {
        // This is a simplified implementation
        // In a real implementation, we would use macOS-specific APIs to monitor memory
        // pressure such as memory_pressure_monitor from libdispatch

        while running.load(std::sync::atomic::Ordering::Relaxed) {
            // Simple polling approach - in a real implementation, we'd use native APIs
            let pressure_level = estimate_memory_pressure();

            let mut guard = callback.lock().unwrap();
            if let Some(callback) = &mut *guard {
                callback(crate::platform::MemoryEvent::PressureChange(pressure_level));
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}

#[cfg(feature = "monitoring")]
impl crate::platform::MemoryMonitor for MacOsMemoryMonitor {
    fn start_monitoring(&self) -> io::Result<()> {
        if self.running.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return Ok(()); // Already running
        }

        let running = self.running.clone();
        let callback = self.callback.clone();

        std::thread::spawn(move || {
            Self::monitor_memory_pressure(&running, &callback);
        });

        Ok(())
    }

    fn stop_monitoring(&self) -> io::Result<()> {
        self.running.store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn register_callback<F>(&self, callback: F) -> io::Result<()>
    where F: FnMut(crate::platform::MemoryEvent) + Send + 'static {
        let mut guard = self.callback.lock().unwrap();
        *guard = Some(Box::new(callback));
        Ok(())
    }
}

/// Estimate memory pressure on macOS by looking at available memory
fn estimate_memory_pressure() -> MemoryPressureLevel {
    let available = get_available_memory_macos();
    let total = crate::platform::get_total_memory();

    if total == 0 {
        return MemoryPressureLevel::Unknown;
    }

    let available_percent = (available as f64 / total as f64) * 100.0;

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
