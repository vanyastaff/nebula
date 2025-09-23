//! Linux-specific memory optimizations and utilities.
//!
//! This module provides optimizations and utilities specific to Linux
//! platforms, including syscall optimizations, memory pressure detection, and
//! huge page support.

use std::fs;
use std::path::Path;

use libc;
#[cfg(feature = "numa-aware")]
use libc::c_int;
#[cfg(feature = "linux-optimizations")]
use libc::c_void;
#[cfg(feature = "numa-aware")]
use numa;

use crate::platform::MemoryPressureLevel;

/// Get available memory on Linux by reading from /proc/meminfo
pub fn get_available_memory_linux() -> usize {
    if let Ok(contents) = fs::read_to_string("/proc/meminfo") {
        for line in contents.lines() {
            if line.starts_with("MemAvailable:") {
                if let Some(value) = line.split_whitespace().nth(1) {
                    if let Ok(kb) = value.parse::<usize>() {
                        return kb * 1024; // Convert KB to bytes
                    }
                }
            }
        }
    }

    // Fallback to using sysinfo if available memory cannot be read
    unsafe {
        let pages = libc::sysconf(libc::_SC_AVPHYS_PAGES) as usize;
        let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
        pages * page_size
    }
}

/// Get detailed memory information for Linux
pub fn get_detailed_memory_info() -> (usize, bool, Option<usize>, Option<usize>) {
    let mut free = 0;
    let mut overcommit_enabled = false;
    let mut lockable = None;
    let mut numa_nodes = None;

    // Read free memory from /proc/meminfo
    if let Ok(contents) = fs::read_to_string("/proc/meminfo") {
        for line in contents.lines() {
            if line.starts_with("MemFree:") {
                if let Some(value) = line.split_whitespace().nth(1) {
                    if let Ok(kb) = value.parse::<usize>() {
                        free = kb * 1024; // Convert KB to bytes
                    }
                }
            }
        }
    }

    // Check overcommit setting
    if let Ok(content) = fs::read_to_string("/proc/sys/vm/overcommit_memory") {
        if let Ok(value) = content.trim().parse::<i32>() {
            // 0 = heuristic, 1 = always overcommit, 2 = never overcommit
            overcommit_enabled = value == 0 || value == 1;
        }
    }

    // Try to get max locked memory
    if let Ok(output) = std::process::Command::new("ulimit").arg("-l").output() {
        if let Ok(value) = String::from_utf8_lossy(&output.stdout).trim().parse::<usize>() {
            if value != 0 {
                lockable = Some(value * 1024); // Convert KB to bytes
            }
        }
    }

    // Try to detect NUMA nodes
    if Path::new("/sys/devices/system/node").exists() {
        if let Ok(entries) = fs::read_dir("/sys/devices/system/node") {
            let count = entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    let path = entry.path();
                    path.is_dir()
                        && path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("node"))
                            .unwrap_or(false)
                })
                .count();

            if count > 0 {
                numa_nodes = Some(count);
            }
        }
    }

    (free, overcommit_enabled, lockable, numa_nodes)
}

/// Detect Linux capabilities for memory management
pub fn detect_linux_capabilities() -> crate::platform::PlatformCapabilities {
    let mut capabilities = crate::platform::PlatformCapabilities {
        huge_pages_supported: false,
        transparent_huge_pages_supported: false,
        numa_supported: false,
        memory_pressure_notifications: false,
        mlock_supported: true, // Linux generally supports mlock
        can_overcommit: false,
    };

    // Check for huge pages support
    if Path::new("/proc/sys/vm/nr_hugepages").exists() {
        capabilities.huge_pages_supported = true;
    }

    // Check for transparent huge pages
    if Path::new("/sys/kernel/mm/transparent_hugepage/enabled").exists() {
        capabilities.transparent_huge_pages_supported = true;
    }

    // Check for NUMA support
    if Path::new("/sys/devices/system/node").exists() {
        capabilities.numa_supported = true;
    }

    // Check for memory pressure notifications
    if Path::new("/proc/pressure/memory").exists() {
        capabilities.memory_pressure_notifications = true;
    }

    // Check for overcommit
    if let Ok(content) = fs::read_to_string("/proc/sys/vm/overcommit_memory") {
        if let Ok(value) = content.trim().parse::<i32>() {
            // 0 = heuristic, 1 = always overcommit, 2 = never overcommit
            capabilities.can_overcommit = value == 0 || value == 1;
        }
    }

    capabilities
}

/// Allocate huge pages on Linux
#[cfg(feature = "linux-optimizations")]
pub fn allocate_huge_pages(size: usize) -> io::Result<*mut c_void> {
    use std::ptr;

    use libc::{MAP_ANONYMOUS, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};

    let ptr = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB,
            -1,
            0,
        )
    };

    if ptr == libc::MAP_FAILED {
        Err(io::Error::last_os_error())
    } else {
        Ok(ptr)
    }
}

/// Free huge pages memory
#[cfg(feature = "linux-optimizations")]
pub unsafe fn free_huge_pages(ptr: *mut c_void, size: usize) -> io::Result<()> {
    let result = libc::munmap(ptr, size);
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Lock memory to prevent swapping
#[cfg(feature = "linux-optimizations")]
pub fn lock_memory(ptr: *mut c_void, size: usize) -> io::Result<()> {
    let result = unsafe { libc::mlock(ptr, size) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Unlock memory
#[cfg(feature = "linux-optimizations")]
pub fn unlock_memory(ptr: *mut c_void, size: usize) -> io::Result<()> {
    let result = unsafe { libc::munlock(ptr, size) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Monitor memory pressure on Linux
#[cfg(feature = "monitoring")]
pub struct LinuxMemoryMonitor {
    running: Arc<AtomicBool>,
    callback: Arc<Mutex<Option<Box<dyn FnMut(crate::platform::MemoryEvent) + Send>>>>,
}

#[cfg(feature = "monitoring")]
impl LinuxMemoryMonitor {
    pub fn new() -> Self {
        Self { running: Arc::new(AtomicBool::new(false)), callback: Arc::new(Mutex::new(None)) }
    }
}

#[cfg(feature = "monitoring")]
impl crate::platform::MemoryMonitor for LinuxMemoryMonitor {
    fn start_monitoring(&self) -> io::Result<()> {
        if self.running.swap(true, Ordering::Relaxed) {
            return Ok(()); // Already running
        }

        let running = self.running.clone();
        let callback = self.callback.clone();

        std::thread::spawn(move || {
            let path = "/proc/pressure/memory";
            if let Ok(mut file) = std::fs::File::open(path) {
                let mut buffer = [0u8; 1024];

                while running.load(Ordering::Relaxed) {
                    match file.read(&mut buffer) {
                        Ok(bytes_read) if bytes_read > 0 => {
                            if let Ok(content) = std::str::from_utf8(&buffer[..bytes_read]) {
                                let pressure_level = parse_memory_pressure(content);

                                let mut guard = callback.lock().unwrap();
                                if let Some(callback) = &mut *guard {
                                    callback(crate::platform::MemoryEvent::PressureChange(
                                        pressure_level,
                                    ));
                                }
                            }
                        },
                        _ => { /* Ignore errors */ },
                    }

                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        });

        Ok(())
    }

    fn stop_monitoring(&self) -> io::Result<()> {
        self.running.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn register_callback(
        &self,
        callback: Box<dyn FnMut(crate::platform::MemoryEvent) + Send + 'static>,
    ) -> io::Result<()> {
        let mut guard = self.callback.lock().unwrap();
        *guard = Some(callback);
        Ok(())
    }
}

/// Parse memory pressure from Linux pressure file
fn parse_memory_pressure(content: &str) -> MemoryPressureLevel {
    // Example content: "some avg10=0.00 avg60=0.00 avg300=0.00 total=0"
    for line in content.lines() {
        if line.starts_with("some") {
            if let Some(avg60_part) = line.split_whitespace().find(|s| s.starts_with("avg60=")) {
                if let Some(avg60_str) = avg60_part.strip_prefix("avg60=") {
                    if let Ok(avg60) = avg60_str.parse::<f64>() {
                        if avg60 > 80.0 {
                            return MemoryPressureLevel::Critical;
                        } else if avg60 > 50.0 {
                            return MemoryPressureLevel::High;
                        } else if avg60 > 20.0 {
                            return MemoryPressureLevel::Medium;
                        } else {
                            return MemoryPressureLevel::Low;
                        }
                    }
                }
            }
        }
    }

    MemoryPressureLevel::Unknown
}

/// Get NUMA node count
#[cfg(feature = "numa-aware")]
pub fn get_numa_node_count() -> io::Result<usize> {
    if !numa::numa_available() {
        return Err(io::Error::new(io::ErrorKind::Other, "NUMA not available"));
    }

    Ok(numa::numa_num_configured_nodes())
}

/// Bind memory allocation to a specific NUMA node
#[cfg(feature = "numa-aware")]
pub fn bind_to_numa_node(node: usize) -> io::Result<()> {
    if !numa::numa_available() {
        return Err(io::Error::new(io::ErrorKind::Other, "NUMA not available"));
    }

    let mut bitmask = numa::numa_allocate_nodemask();
    numa::numa_bitmask_clearall(bitmask);
    numa::numa_bitmask_setbit(bitmask, node);
    numa::numa_bind(bitmask);
    numa::numa_free_nodemask(bitmask);

    Ok(())
}

/// Get current NUMA node for the calling thread
#[cfg(feature = "numa-aware")]
pub fn get_current_numa_node() -> io::Result<usize> {
    if !numa::numa_available() {
        return Err(io::Error::new(io::ErrorKind::Other, "NUMA not available"));
    }

    Ok(numa::numa_preferred())
}
