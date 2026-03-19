//! Common types and constants for memory management

/// Memory alignment requirements
pub mod alignment {
    /// Minimum alignment for allocations (typically cache line size / 2)
    pub const MIN_ALIGN: usize = 8;

    /// Cache line size for optimal performance
    pub const CACHE_LINE: usize = 64;

    /// Page size (platform dependent, this is common default)
    pub const PAGE_SIZE: usize = 4096;

    /// Large page size (2MB on most platforms)
    pub const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;
}

/// Memory size constants
pub mod size {
    /// 1 Kilobyte
    pub const KB: usize = 1024;

    /// 1 Megabyte
    pub const MB: usize = 1024 * KB;

    /// 1 Gigabyte
    pub const GB: usize = 1024 * MB;

    /// Typical small allocation
    pub const SMALL: usize = 64 * KB;

    /// Typical medium allocation
    pub const MEDIUM: usize = MB;

    /// Typical large allocation
    pub const LARGE: usize = 16 * MB;
}

/// Memory operation hints
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryHint {
    /// Normal memory access pattern
    Normal,
    /// Random access pattern
    Random,
    /// Sequential access pattern
    Sequential,
    /// Memory will be needed soon
    WillNeed,
    /// Memory won't be needed soon
    DontNeed,
}

/// Memory protection flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryProtection {
    /// No access
    None,
    /// Read-only
    Read,
    /// Write-only
    Write,
    /// Read and write
    ReadWrite,
    /// Execute
    Execute,
    /// Read and execute
    ReadExecute,
}

/// Memory allocation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationStrategy {
    /// First fit - use first available block
    FirstFit,
    /// Best fit - use smallest sufficient block
    BestFit,
    /// Worst fit - use largest available block
    WorstFit,
    /// Buddy system allocation
    Buddy,
    /// Segregated free lists
    Segregated,
}
