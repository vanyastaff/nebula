//! Windows-specific memory optimizations and utilities.
//!
//! This module provides optimizations and utilities specific to Windows
//! platforms, including memory pressure detection and system-specific memory
//! management.

use std::io;

#[cfg(windows)]
use winapi::um::{
    memoryapi::{VirtualLock, VirtualUnlock},
    sysinfoapi::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
};

use crate::platform::MemoryPressureLevel;

/// Get available memory on Windows
pub fn get_available_memory_windows() -> usize {
    #[cfg(windows)]
    unsafe {
        let mut mem_status: MEMORYSTATUSEX = std::mem::zeroed();
        mem_status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;

        if GlobalMemoryStatusEx(&mut mem_status) != 0 {
            return mem_status.ullAvailPhys as usize;
        }
    }

    // Fallback for non-windows or failure
    0
}

/// Get detailed memory information for Windows
pub fn get_detailed_memory_info() -> (usize, bool, Option<usize>, Option<usize>) {
    let mut free = 0;
    let overcommit_enabled = true; // Windows generally overcommits memory
    let mut lockable = None;
    let numa_nodes = None; // Will be determined below if possible

    #[cfg(windows)]
    unsafe {
        let mut mem_status: MEMORYSTATUSEX = std::mem::zeroed();
        mem_status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;

        if GlobalMemoryStatusEx(&mut mem_status) != 0 {
            free = mem_status.ullAvailPhys as usize;

            // Windows doesn't have a direct equivalent to RLIMIT_MEMLOCK
            // We can use working set size as an approximation
            lockable = Some(mem_status.ullTotalPhys as usize);
        }

        // Try to determine NUMA node count
        let mut nodes = 0;
        if let Some(get_numa_node_count) = get_numa_node_count_fn() {
            nodes = get_numa_node_count();
            if nodes > 0 {
                return (free, overcommit_enabled, lockable, Some(nodes));
            }
        }
    }

    (free, overcommit_enabled, lockable, numa_nodes)
}

/// Detect Windows capabilities for memory management
pub fn detect_windows_capabilities() -> crate::platform::PlatformCapabilities {
    let mut numa_supported = false;

    #[cfg(windows)]
    {
        if let Some(get_numa_node_count) = get_numa_node_count_fn() {
            numa_supported = get_numa_node_count() > 1;
        }
    }

    crate::platform::PlatformCapabilities {
        huge_pages_supported: true,              // Windows supports large pages
        transparent_huge_pages_supported: false, // Windows doesn't have THP like Linux
        numa_supported,
        memory_pressure_notifications: true, // Windows has memory resource notifications
        mlock_supported: true,               // Windows supports VirtualLock
        can_overcommit: true,                // Windows generally overcommits memory
    }
}

/// Lock memory to prevent swapping on Windows
#[cfg(windows)]
pub fn lock_memory(ptr: *mut std::ffi::c_void, size: usize) -> io::Result<()> {
    let result = unsafe { VirtualLock(ptr, size) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Unlock memory on Windows
#[cfg(windows)]
pub fn unlock_memory(ptr: *mut std::ffi::c_void, size: usize) -> io::Result<()> {
    let result = unsafe { VirtualUnlock(ptr, size) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Get function to retrieve NUMA node count on Windows
#[cfg(windows)]
fn get_numa_node_count_fn() -> Option<fn() -> usize> {
    use std::ffi::CString;

    use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};

    unsafe {
        let kernel32 = GetModuleHandleA(b"kernel32.dll\0".as_ptr() as _);
        if kernel32.is_null() {
            return None;
        }

        let func_name = CString::new("GetNumaHighestNodeNumber").unwrap();
        let func_ptr = GetProcAddress(kernel32, func_name.as_ptr());

        if func_ptr.is_null() {
            return None;
        }

        Some(|| {
            let func: extern "system" fn(*mut u32) -> i32 = std::mem::transmute(func_ptr);

            let mut highest_node = 0u32;
            if func(&mut highest_node) != 0 {
                // Node numbers are 0-based, so add 1 to get count
                return (highest_node + 1) as usize;
            }
            0
        })
    }
}

/// Windows Large Page Allocation
#[cfg(all(windows, feature = "windows-optimizations"))]
pub fn allocate_large_pages(size: usize) -> io::Result<*mut std::ffi::c_void> {
    use winapi::um::memoryapi::VirtualAlloc;
    use winapi::um::winnt::{MEM_COMMIT, MEM_LARGE_PAGES, MEM_RESERVE, PAGE_READWRITE};

    // Note: The process needs the "Lock Pages in Memory" privilege
    // This is typically only available to administrators
    let ptr = unsafe {
        VirtualAlloc(
            std::ptr::null_mut(),
            size,
            MEM_COMMIT | MEM_RESERVE | MEM_LARGE_PAGES,
            PAGE_READWRITE,
        )
    };

    if ptr.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(ptr)
    }
}

/// Free Large Pages on Windows
#[cfg(all(windows, feature = "windows-optimizations"))]
pub unsafe fn free_large_pages(ptr: *mut std::ffi::c_void) -> io::Result<()> {
    use winapi::um::memoryapi::VirtualFree;
    use winapi::um::winnt::MEM_RELEASE;

    let result = VirtualFree(ptr, 0, MEM_RELEASE);
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Monitor memory pressure on Windows
#[cfg(feature = "monitoring")]
pub struct WindowsMemoryMonitor {
    running: std::sync::atomic::AtomicBool,
    callback: std::sync::Mutex<Option<Box<dyn FnMut(crate::platform::MemoryEvent) + Send>>>,
}

#[cfg(feature = "monitoring")]
impl WindowsMemoryMonitor {
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
        // In a real implementation, we would use Windows-specific APIs
        // like CreateMemoryResourceNotification and WaitForSingleObject

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
impl crate::platform::MemoryMonitor for WindowsMemoryMonitor {
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

/// Estimate memory pressure on Windows by looking at available memory
fn estimate_memory_pressure() -> MemoryPressureLevel {
    #[cfg(windows)]
    unsafe {
        let mut mem_status: MEMORYSTATUSEX = std::mem::zeroed();
        mem_status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;

        if GlobalMemoryStatusEx(&mut mem_status) != 0 {
            // mem_status.dwMemoryLoad is a percentage of memory in use
            let memory_load = mem_status.dwMemoryLoad as f64;

            if memory_load > 95.0 {
                return MemoryPressureLevel::Critical;
            } else if memory_load > 85.0 {
                return MemoryPressureLevel::High;
            } else if memory_load > 70.0 {
                return MemoryPressureLevel::Medium;
            } else {
                return MemoryPressureLevel::Low;
            }
        }
    }

    MemoryPressureLevel::Unknown
}
