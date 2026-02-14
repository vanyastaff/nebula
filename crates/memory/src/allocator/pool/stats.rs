//! Pool allocator statistics

/// Statistics for pool allocator
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// Total allocations performed
    pub total_allocs: u32,
    /// Total deallocations performed
    pub total_deallocs: u32,
    /// Peak memory usage in bytes
    pub peak_usage: usize,
    /// Current memory usage in bytes
    pub current_usage: usize,
    /// Size of each block
    pub block_size: usize,
    /// Total number of blocks
    pub block_count: usize,
    /// Currently free blocks
    pub free_blocks: usize,
}
