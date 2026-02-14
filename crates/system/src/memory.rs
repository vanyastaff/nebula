//! Memory management utilities

// External dependencies
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// Internal crates
use crate::core::{SystemError, SystemResult};
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
    #[must_use]
    pub fn is_concerning(&self) -> bool {
        *self >= MemoryPressure::High
    }

    /// Check if memory pressure is critical
    #[must_use]
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
#[must_use]
pub fn current() -> MemoryInfo {
    let sys_memory = SystemInfo::current_memory();
    let used = sys_memory.total.saturating_sub(sys_memory.available);

    // Calculate usage percent with checked arithmetic to avoid precision loss
    // For very large memory values, direct f64 conversion can lose precision.
    // We use checked_mul to compute (used * 10000) / total, then divide by 100
    // to get percentage with 2 decimal precision.
    let usage_percent = if sys_memory.total > 0 {
        used.checked_mul(10000)
            .and_then(|v| v.checked_div(sys_memory.total))
            .map_or_else(
                || {
                    // Fallback to direct f64 if overflow (extremely rare)
                    (used as f64 / sys_memory.total as f64) * 100.0
                },
                |v| v as f64 / 100.0,
            )
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
#[must_use]
pub fn pressure() -> MemoryPressure {
    current().pressure
}

/// Format bytes for human-readable display
///
/// Re-exported from utils for convenience.
pub use crate::utils::format_bytes_usize as format_bytes;

// Memory management functions (only with memory feature)
#[cfg(feature = "memory")]
/// Low-level memory management helpers backed by the `region` crate.
pub mod management {
    use super::{MemoryProtection, SystemError, SystemResult};

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
    ///
    /// # Safety
    ///
    /// This function allocates uninitialized memory which is inherently unsafe.
    ///
    /// ## Preconditions
    ///
    /// - `size` must be greater than 0 and within platform allocation limits
    /// - `protection` must be a valid protection flag for the target platform
    ///
    /// ## Postconditions
    ///
    /// - Returns a non-null, properly aligned pointer to allocated memory
    /// - The memory is **uninitialized** - caller MUST initialize before reading
    /// - The memory region has the requested protection flags applied
    ///
    /// ## Caller Responsibilities
    ///
    /// - Must initialize the memory before any reads
    /// - Must ensure proper deallocation (RAII patterns recommended)
    /// - Must not access the memory after deallocation
    /// - Must respect the memory protection flags
    ///
    /// ## Panics
    ///
    /// May panic if `size` exceeds platform-specific allocation limits.
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::SystemError`] if:
    /// - The operating system cannot allocate the requested memory (out of memory)
    /// - The protection flags are invalid for the platform
    /// - The allocation size is invalid
    pub unsafe fn allocate(size: usize, protection: MemoryProtection) -> SystemResult<*mut u8> {
        // SAFETY: `region::alloc` allocates a valid memory region with the requested
        // protection flags. We cast the const pointer to mut because the allocated
        // region is writable (depending on protection flags). We use `std::mem::forget`
        // to prevent the RAII guard from deallocating - the caller now owns the pointer.
        region::alloc(size, protection)
            .map(|alloc| {
                let ptr = alloc.as_ptr::<u8>().cast_mut();
                std::mem::forget(alloc);
                ptr
            })
            .map_err(|e| SystemError::memory_operation_error(format!("allocate: {e}")))
    }

    /// Free allocated memory
    ///
    /// # Safety
    ///
    /// This function is currently **not implemented** for region-based allocations.
    /// Manual deallocation is not supported - use RAII patterns instead.
    ///
    /// ## Note
    ///
    /// The `region` crate uses RAII guards for memory management. Manual `free`
    /// operations are not supported and will always return an error. Consider
    /// using the allocation guard pattern or wrapping allocations in RAII types.
    ///
    /// # Errors
    ///
    /// Always returns [`NebulaError::SystemError`] with "not supported" message.
    pub unsafe fn free(_ptr: *mut u8, _size: usize) -> SystemResult<()> {
        Err(SystemError::feature_not_supported(
            "Manual free is not supported for region allocations; use RAII handle instead",
        ))
    }

    /// Change memory protection
    ///
    /// # Safety
    ///
    /// This function modifies memory protection flags, which can lead to undefined
    /// behavior if used incorrectly.
    ///
    /// ## Preconditions
    ///
    /// - `ptr` must point to the start of a valid, previously allocated memory region
    /// - `ptr` must be properly aligned for the memory region
    /// - `size` must exactly match the size of the allocated region
    /// - The memory region must not be in use during the protection change
    /// - `protection` must be valid for the target platform
    ///
    /// ## Safety Invariants
    ///
    /// - Changing protection to more restrictive (e.g., NONE) while code holds
    ///   references will cause immediate faults
    /// - Making executable memory must follow platform-specific security policies
    /// - Protection changes are not atomic - race conditions possible
    ///
    /// ## Caller Responsibilities
    ///
    /// - Ensure no references to the memory exist during protection changes
    /// - Verify the new protection flags allow intended operations
    /// - Handle potential platform-specific restrictions (W^X policies)
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::SystemError`] if:
    /// - The pointer is invalid or not at a region boundary
    /// - The size doesn't match the allocation
    /// - The protection flags are invalid or violate platform security policies
    /// - The OS denies the protection change
    pub unsafe fn protect(
        ptr: *mut u8,
        size: usize,
        protection: MemoryProtection,
    ) -> SystemResult<()> {
        // SAFETY: Caller must ensure ptr is valid, size is correct, and no aliasing
        // references exist. We delegate to region::protect which performs the OS-level
        // system call to change memory protection flags.
        unsafe {
            region::protect(ptr, size, protection)
                .map_err(|e| SystemError::memory_operation_error(format!("protect: {e}")))
        }
    }

    /// Lock memory pages (prevent swapping)
    ///
    /// # Safety
    ///
    /// Locks memory pages into RAM, preventing them from being swapped to disk.
    /// This is unsafe because it affects system-wide memory management.
    ///
    /// ## Preconditions
    ///
    /// - `ptr` must point to valid, accessible memory
    /// - `ptr` must be page-aligned (typically 4KB on most systems)
    /// - `size` must be a multiple of the system page size
    /// - The process must have sufficient privileges (may require `CAP_IPC_LOCK` on Linux)
    /// - Total locked memory must not exceed system limits (`RLIMIT_MEMLOCK`)
    ///
    /// ## Use Cases
    ///
    /// - Storing cryptographic keys (prevent swap exposure)
    /// - Real-time systems (prevent page faults)
    /// - High-security data (prevent disk writes)
    ///
    /// ## Platform Notes
    ///
    /// - Linux: Requires `CAP_IPC_LOCK` or root for large amounts
    /// - Windows: Requires `SeLockMemoryPrivilege` for large amounts
    /// - macOS: Subject to system-wide limits
    ///
    /// ## Caller Responsibilities
    ///
    /// - Must eventually unlock the memory
    /// - Should not lock excessive amounts (impacts system performance)
    /// - Must handle privilege/limit errors gracefully
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::SystemError`] if:
    /// - The pointer or size is not page-aligned
    /// - The process lacks necessary privileges
    /// - System locked memory limit exceeded
    /// - The memory region is invalid or inaccessible
    pub unsafe fn lock(ptr: *mut u8, size: usize) -> SystemResult<()> {
        // SAFETY: Caller must ensure ptr is valid, page-aligned, and size is correct.
        // The returned guard is immediately dropped - this permanently locks the memory
        // until unlock() is called. This is intentional but means caller MUST call unlock.
        region::lock(ptr, size)
            .map(|_guard| ())
            .map_err(|e| SystemError::memory_operation_error(format!("lock: {e}")))
    }

    /// Unlock memory pages
    ///
    /// # Safety
    ///
    /// Unlocks previously locked memory pages, allowing them to be swapped to disk.
    ///
    /// ## Preconditions
    ///
    /// - `ptr` must point to memory that was previously locked with [`lock()`]
    /// - `ptr` must be page-aligned (same alignment used in lock call)
    /// - `size` must match exactly the size used in the corresponding lock call
    /// - The memory must still be valid and allocated
    ///
    /// ## Behavior
    ///
    /// - Allows the OS to swap the pages to disk if needed
    /// - Should be called for every successful [`lock()`] call
    /// - Multiple unlock calls on the same memory may be an error
    ///
    /// ## Caller Responsibilities
    ///
    /// - Match each `lock()` with exactly one `unlock()`
    /// - Use the same ptr and size as the corresponding lock
    /// - Ensure the memory is still valid (not freed)
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::SystemError`] if:
    /// - The memory was not previously locked
    /// - The pointer or size is incorrect
    /// - The memory region is invalid
    /// - System call fails for platform-specific reasons
    pub unsafe fn unlock(ptr: *mut u8, size: usize) -> SystemResult<()> {
        // SAFETY: Caller must ensure ptr was previously locked, is still valid,
        // and ptr/size match the original lock() call. We delegate to region::unlock
        // which performs the OS-level system call.
        region::unlock(ptr, size)
            .map_err(|e| SystemError::memory_operation_error(format!("unlock: {e}")))
    }

    /// Query memory region information
    ///
    /// # Safety
    ///
    /// Queries information about a memory region. This is unsafe because it accesses
    /// low-level memory management information.
    ///
    /// ## Preconditions
    ///
    /// - `ptr` must point to valid, accessible memory
    /// - The memory at `ptr` must be mapped (not freed/unmapped)
    /// - The pointer should ideally point to the start of a region, but any
    ///   address within a region is acceptable
    ///
    /// ## Return Value
    ///
    /// Returns a [`MemoryRegion`] containing:
    /// - `base`: The queried pointer (not the actual region base)
    /// - `size`: The length of the memory region containing `ptr`
    /// - `protection`: Current protection flags for the region
    /// - `shared`: Whether the region is shared with other processes
    ///
    /// ## Platform Notes
    ///
    /// - Linux: Uses `/proc/self/maps` or `mincore()`
    /// - Windows: Uses `VirtualQuery()`
    /// - macOS: Uses `vm_region()` Mach calls
    ///
    /// ## Caller Responsibilities
    ///
    /// - Must ensure the pointer is valid and mapped
    /// - Should not rely on `base` being the true region base (use the queried ptr)
    /// - Handle potential platform-specific query failures
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::SystemError`] if:
    /// - The pointer is invalid or unmapped
    /// - The memory region cannot be queried (platform restrictions)
    /// - The OS denies access to region information
    pub unsafe fn query(ptr: *const u8) -> SystemResult<MemoryRegion> {
        // SAFETY: Caller must ensure ptr is valid and points to mapped memory.
        // We delegate to region::query which uses platform-specific APIs to
        // retrieve memory region information from the OS.
        let region = region::query(ptr)
            .map_err(|e| SystemError::memory_operation_error(format!("query: {e}")))?;

        Ok(MemoryRegion {
            // Base address is approximated by the queried pointer since region base may be inaccessible here
            base: ptr as usize,
            size: region.len(),
            protection: region.protection(),
            shared: region.is_shared(),
        })
    }
}
