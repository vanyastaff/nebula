//! Low-level system calls for memory allocators
//!
//! This module provides direct, unsafe system call wrappers for memory operations
//! used by custom allocators. For high-level system information, use `nebula-system`.
//!
//! # Architecture
//!
//! - **direct.rs** - Direct syscall wrappers (mmap, VirtualAlloc, etc.)
//! - **info.rs** - Allocator-specific memory information
//! - **numa.rs** - NUMA-aware allocation (when available)
//!
//! # Safety
//!
//! All functions in this module are `unsafe` and require careful usage.
//! Incorrect use can lead to memory corruption, segfaults, or undefined behavior.

// Re-export allocator-specific memory info
mod info;
pub use info::*;

// Direct syscalls module (was syscalls.rs)
mod direct;
pub use direct::*;

// NUMA support (feature-gated, temporarily disabled)
// #[cfg(feature = "numa-aware")]
// mod numa;
// #[cfg(feature = "numa-aware")]
// pub use numa::*;

/// Platform capabilities for memory allocators
#[derive(Debug, Clone, Copy)]
pub struct AllocatorCapabilities {
    /// Huge pages are supported
    pub huge_pages_supported: bool,
    /// NUMA is available
    pub numa_supported: bool,
    /// Memory can be locked (mlock)
    pub mlock_supported: bool,
}

impl AllocatorCapabilities {
    /// Detect available allocator capabilities
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            use std::fs;
            let huge_pages = fs::metadata("/sys/kernel/mm/hugepages").is_ok();
            let numa = fs::metadata("/sys/devices/system/node").is_ok();

            Self {
                huge_pages_supported: huge_pages,
                numa_supported: numa,
                mlock_supported: true, // Usually available on Linux
            }
        }

        #[cfg(target_os = "macos")]
        {
            Self {
                huge_pages_supported: false, // macOS doesn't expose huge pages API
                numa_supported: false,
                mlock_supported: true,
            }
        }

        #[cfg(windows)]
        {
            Self {
                huge_pages_supported: true, // Windows supports large pages
                numa_supported: false,      // Would need runtime detection
                mlock_supported: true,      // VirtualLock
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            Self {
                huge_pages_supported: false,
                numa_supported: false,
                mlock_supported: false,
            }
        }
    }
}

/// Initialize allocator-specific subsystems (if needed)
pub fn initialize() -> std::io::Result<()> {
    // Currently no initialization needed
    // In future: NUMA library init, huge pages setup, etc.
    Ok(())
}
