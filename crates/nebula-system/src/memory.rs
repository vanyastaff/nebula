//! Memory management utilities

// External dependencies
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// Internal crates
use crate::core::{NebulaError, SystemError, SystemResult};
use crate::info::SystemInfo;

// Re-export from region for convenience
#[cfg(feature = "memory")]
pub use region::Protection as MemoryProtection;

/// Memory pressure levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MemoryPressure {
    /// Less than 50% memory used
    Low,
    /// 50-70% memory used
    Medium,
    /// 70-85% memory used
    High,
    /// More than 85% memory used
    Critical,
}

impl MemoryPressure {
    /// Check if memory pressure is concerning (High or Critical)
    pub fn is_concerning(&self) -> bool {
        *self >= MemoryPressure::High
    }

    /// Check if memory pressure is critical
    pub fn is_critical(&self) -> bool {
        *self == MemoryPressure::Critical
    }
}

/// Memory information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryInfo {
    /// Total physical memory in bytes
    pub total: usize,
    /// Available physical memory in bytes
    pub available: usize,
    /// Used physical memory in bytes
    pub used: usize,
    /// Memory usage percentage
    pub usage_percent: f64,
    /// Current memory pressure
    pub pressure: MemoryPressure,
}

/// Get current memory information
pub fn current() -> MemoryInfo {
    let sys_memory = SystemInfo::current_memory();
    let used = sys_memory.total.saturating_sub(sys_memory.available);
    let usage_percent = if sys_memory.total > 0 {
        (used as f64 / sys_memory.total as f64) * 100.0
    } else {
        0.0
    };

    let pressure = if usage_percent > 85.0 {
        MemoryPressure::Critical
    } else if usage_percent > 70.0 {
        MemoryPressure::High
    } else if usage_percent > 50.0 {
        MemoryPressure::Medium
    } else {
        MemoryPressure::Low
    };

    MemoryInfo {
        total: sys_memory.total,
        available: sys_memory.available,
        used,
        usage_percent,
        pressure,
    }
}

/// Get current memory pressure
pub fn pressure() -> MemoryPressure {
    current().pressure
}

/// Format bytes for human-readable display
pub fn format_bytes(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

// Memory management functions (only with memory feature)
#[cfg(feature = "memory")]
/// Low-level memory management helpers backed by the `region` crate.
pub mod management {
    use super::*;

    /// Memory region information
    #[derive(Debug, Clone)]
    pub struct MemoryRegion {
        /// Base address of the region
        pub base: usize,
        /// Size of the region in bytes
        pub size: usize,
        /// Protection flags
        pub protection: MemoryProtection,
        /// Whether the region is shared
        pub shared: bool,
    }

    /// Allocate memory with specific protection
    pub unsafe fn allocate(size: usize, protection: MemoryProtection) -> SystemResult<*mut u8> {
        region::alloc(size, protection)
            .map(|alloc| {
                // region::Allocation::as_ptr returns a const pointer; we expose a mut pointer for API symmetry.
                // This is safe as the allocated region is writable depending on protection flags.
                let ptr = alloc.as_ptr::<u8>().cast_mut();
                std::mem::forget(alloc);
                ptr
            })
            .map_err(|e| NebulaError::system_memory_error("allocate", e.to_string()))
    }

    /// Free allocated memory
    pub unsafe fn free(_ptr: *mut u8, _size: usize) -> SystemResult<()> {
        Err(NebulaError::system_not_supported(
            "Manual free is not supported for region allocations; use RAII handle instead"
        ))
    }

    /// Change memory protection
    pub unsafe fn protect(ptr: *mut u8, size: usize, protection: MemoryProtection) -> SystemResult<()> {
        unsafe {
            region::protect(ptr, size, protection)
                .map_err(|e| NebulaError::system_memory_error("protect", e.to_string()))
        }
    }

    /// Lock memory pages (prevent swapping)
    pub unsafe fn lock(ptr: *mut u8, size: usize) -> SystemResult<()> {
        region::lock(ptr, size)
            .map(|_guard| ())
            .map_err(|e| NebulaError::system_memory_error("lock", e.to_string()))
    }

    /// Unlock memory pages
    pub unsafe fn unlock(ptr: *mut u8, size: usize) -> SystemResult<()> {
        region::unlock(ptr, size).map_err(|e| NebulaError::system_memory_error("unlock", e.to_string()))
    }

    /// Query memory region information
    pub unsafe fn query(ptr: *const u8) -> SystemResult<MemoryRegion> {
        let region =
            region::query(ptr).map_err(|e| NebulaError::system_memory_error("query", e.to_string()))?;

        Ok(MemoryRegion {
            // Base address is approximated by the queried pointer since region base may be inaccessible here
            base: ptr as usize,
            size: region.len(),
            protection: region.protection(),
            shared: region.is_shared(),
        })
    }
}
