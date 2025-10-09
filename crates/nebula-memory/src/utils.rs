//! Utility functions and helpers for nebula-memory
//!
//! This module provides common utilities used throughout the crate:
//! - Memory alignment helpers
//! - Size formatting utilities
//! - Platform-specific helpers
//! - Performance measurement tools
//! - Checked arithmetic operations

use core::ptr;
use core::sync::atomic::{Ordering, compiler_fence, fence};
#[cfg(feature = "std")]
use std::time::{Duration, Instant};

use crate::allocator::{AllocError, AllocResult};

/// Aligns a value up to the nearest multiple of alignment
///
/// # Examples
/// ```
/// use nebula_memory::utils::align_up;
///
/// assert_eq!(align_up(7, 8), 8);
/// assert_eq!(align_up(8, 8), 8);
/// assert_eq!(align_up(9, 8), 16);
/// ```
#[inline(always)]
pub const fn align_up(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}

/// Aligns a value down to the nearest multiple of alignment
///
/// # Examples
/// ```
/// use nebula_memory::utils::align_down;
///
/// assert_eq!(align_down(7, 8), 0);
/// assert_eq!(align_down(8, 8), 8);
/// assert_eq!(align_down(9, 8), 8);
/// ```
#[inline(always)]
pub const fn align_down(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    value & !(alignment - 1)
}

/// Checks if a value is aligned to the given alignment
///
/// # Examples
/// ```
/// use nebula_memory::utils::is_aligned;
///
/// assert!(is_aligned(16, 8));
/// assert!(is_aligned(32, 16));
/// assert!(!is_aligned(17, 8));
/// ```
#[inline(always)]
pub const fn is_aligned(value: usize, alignment: usize) -> bool {
    debug_assert!(alignment.is_power_of_two());
    value & (alignment - 1) == 0
}

/// Calculates padding needed to align a value
///
/// # Examples
/// ```
/// use nebula_memory::utils::padding_needed;
///
/// assert_eq!(padding_needed(7, 8), 1);
/// assert_eq!(padding_needed(8, 8), 0);
/// assert_eq!(padding_needed(9, 8), 7);
/// ```
#[inline(always)]
pub const fn padding_needed(value: usize, alignment: usize) -> usize {
    align_up(value, alignment) - value
}

/// Rounds up to the next power of two
///
/// # Examples
/// ```
/// use nebula_memory::utils::next_power_of_two;
///
/// assert_eq!(next_power_of_two(7), 8);
/// assert_eq!(next_power_of_two(8), 8);
/// assert_eq!(next_power_of_two(9), 16);
/// ```
#[inline(always)]
pub const fn next_power_of_two(mut value: usize) -> usize {
    if value == 0 {
        return 1;
    }
    value -= 1;
    value |= value >> 1;
    value |= value >> 2;
    value |= value >> 4;
    value |= value >> 8;
    value |= value >> 16;
    #[cfg(target_pointer_width = "64")]
    {
        value |= value >> 32;
    }
    value + 1
}

/// Check if a pointer is properly aligned
#[inline(always)]
pub fn is_aligned_ptr<T>(ptr: *const T, alignment: usize) -> bool {
    is_aligned(ptr as usize, alignment)
}

/// Format bytes into human-readable string
///
/// Re-exported from nebula-system for convenience and consistency across the ecosystem.
///
/// # Examples
/// ```
/// use nebula_memory::utils::format_bytes;
///
/// assert_eq!(format_bytes(1024), "1.00 KB");
/// assert_eq!(format_bytes(1536), "1.50 KB");
/// assert_eq!(format_bytes(1048576), "1.00 MB");
/// ```
#[cfg(feature = "std")]
pub use nebula_system::utils::format_bytes_usize as format_bytes;

/// Format duration into human-readable string
///
/// Re-exported from nebula-system for consistency.
#[cfg(feature = "std")]
pub use nebula_system::utils::format_duration;

/// Format percentage
///
/// Re-exported from nebula-system for consistency.
pub use nebula_system::utils::format_percentage;

/// Format rate (per second)
///
/// Re-exported from nebula-system for consistency.
pub use nebula_system::utils::format_rate;

/// Get cache line size for current platform
///
/// Re-exported from nebula-system for consistency.
pub use nebula_system::utils::cache_line_size;

/// Memory barrier for synchronization
#[inline(always)]
pub fn memory_barrier() {
    fence(Ordering::SeqCst);
}

/// Securely zero memory
#[inline(always)]
pub fn secure_zero(ptr: *mut u8, len: usize) {
    if len == 0 {
        return;
    }

    unsafe {
        ptr::write_bytes(ptr, 0, len);
    }
    compiler_fence(Ordering::SeqCst);
}

/// Platform-specific prefetch for read
#[inline(always)]
#[cfg(target_arch = "x86_64")]
pub fn prefetch_read<T>(ptr: *const T) {
    use core::arch::x86_64::{_MM_HINT_T0, _mm_prefetch};
    unsafe {
        _mm_prefetch::<_MM_HINT_T0>(ptr.cast());
    }
}

#[inline(always)]
#[cfg(not(target_arch = "x86_64"))]
pub fn prefetch_read<T>(_ptr: *const T) {}

/// Platform-specific prefetch for write
#[inline(always)]
#[cfg(target_arch = "x86_64")]
pub fn prefetch_write<T>(ptr: *mut T) {
    use core::arch::x86_64::{_MM_HINT_T1, _mm_prefetch};
    unsafe {
        _mm_prefetch::<_MM_HINT_T1>(ptr.cast());
    }
}

#[inline(always)]
#[cfg(not(target_arch = "x86_64"))]
pub fn prefetch_write<T>(_ptr: *mut T) {}

/// SIMD-optimized memory copy for aligned data (AVX2)
///
/// # Safety
/// - `dst` and `src` must be valid for reads/writes of `len` bytes
/// - `dst` and `src` should be 32-byte aligned for optimal performance
/// - Regions must not overlap
#[inline]
#[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
pub unsafe fn copy_aligned_simd(dst: *mut u8, src: *const u8, len: usize) {
    use core::arch::x86_64::*;

    if len == 0 {
        return;
    }

    // Process 32-byte chunks with AVX2
    let chunks = len / 32;
    let remainder = len % 32;

    for i in 0..chunks {
        let offset = i * 32;
        // Load 32 bytes from source
        let data = _mm256_loadu_si256(src.add(offset) as *const __m256i);
        // Store 32 bytes to destination
        _mm256_storeu_si256(dst.add(offset) as *mut __m256i, data);
    }

    // Handle remainder with scalar copy
    if remainder > 0 {
        ptr::copy_nonoverlapping(src.add(chunks * 32), dst.add(chunks * 32), remainder);
    }
}

/// SIMD-optimized memory copy (fallback for non-AVX2)
#[inline]
#[cfg(all(feature = "simd", target_arch = "x86_64", not(target_feature = "avx2")))]
pub unsafe fn copy_aligned_simd(dst: *mut u8, src: *const u8, len: usize) {
    ptr::copy_nonoverlapping(src, dst, len);
}

/// SIMD-optimized memory copy (fallback for non-x86_64)
#[inline]
#[cfg(not(all(feature = "simd", target_arch = "x86_64")))]
pub unsafe fn copy_aligned_simd(dst: *mut u8, src: *const u8, len: usize) {
    ptr::copy_nonoverlapping(src, dst, len);
}

/// SIMD-optimized memory fill with pattern (AVX2)
///
/// # Safety
/// - `dst` must be valid for writes of `len` bytes
/// - Works best with 32-byte aligned destination
#[inline]
#[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
pub unsafe fn fill_simd(dst: *mut u8, pattern: u8, len: usize) {
    use core::arch::x86_64::*;

    if len == 0 {
        return;
    }

    // Create pattern vector (broadcast byte to all lanes)
    let pattern_vec = _mm256_set1_epi8(pattern as i8);

    // Process 32-byte chunks
    let chunks = len / 32;
    let remainder = len % 32;

    for i in 0..chunks {
        let offset = i * 32;
        _mm256_storeu_si256(dst.add(offset) as *mut __m256i, pattern_vec);
    }

    // Handle remainder
    if remainder > 0 {
        ptr::write_bytes(dst.add(chunks * 32), pattern, remainder);
    }
}

/// SIMD-optimized memory fill (fallback)
#[inline]
#[cfg(not(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2")))]
pub unsafe fn fill_simd(dst: *mut u8, pattern: u8, len: usize) {
    ptr::write_bytes(dst, pattern, len);
}

/// SIMD-optimized memory compare (AVX2)
///
/// Returns true if memory regions are equal
///
/// # Safety
/// - Both pointers must be valid for reads of `len` bytes
#[inline]
#[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
pub unsafe fn compare_simd(a: *const u8, b: *const u8, len: usize) -> bool {
    use core::arch::x86_64::*;

    if len == 0 {
        return true;
    }

    // Process 32-byte chunks
    let chunks = len / 32;
    let remainder = len % 32;

    for i in 0..chunks {
        let offset = i * 32;
        let va = _mm256_loadu_si256(a.add(offset) as *const __m256i);
        let vb = _mm256_loadu_si256(b.add(offset) as *const __m256i);

        // Compare and get mask
        let cmp = _mm256_cmpeq_epi8(va, vb);
        let mask = _mm256_movemask_epi8(cmp);

        // If not all bits are set, regions differ
        if mask != -1 {
            return false;
        }
    }

    // Compare remainder
    if remainder > 0 {
        let offset = chunks * 32;
        for i in 0..remainder {
            if *a.add(offset + i) != *b.add(offset + i) {
                return false;
            }
        }
    }

    true
}

/// SIMD-optimized memory compare (fallback)
#[inline]
#[cfg(not(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2")))]
pub unsafe fn compare_simd(a: *const u8, b: *const u8, len: usize) -> bool {
    if len == 0 {
        return true;
    }

    for i in 0..len {
        if *a.add(i) != *b.add(i) {
            return false;
        }
    }
    true
}

/// Timer for performance measurements
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct Timer {
    start: Instant,
    name: &'static str,
}

#[cfg(feature = "std")]
impl Timer {
    #[inline]
    pub fn new(name: &'static str) -> Self {
        Self {
            start: Instant::now(),
            name,
        }
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn print(&self) {
        println!("{}: {}", self.name, format_duration(self.elapsed()));
    }
}

#[cfg(feature = "std")]
impl Drop for Timer {
    fn drop(&mut self) {
        self.print();
    }
}

// ============================================================================
// Checked Arithmetic for Overflow Safety
// ============================================================================

/// Extension trait for checked arithmetic operations that return AllocResult
///
/// This trait provides a consistent way to handle overflow/underflow in
/// memory-related calculations throughout the crate.
///
/// # Examples
///
/// ```
/// use nebula_memory::utils::CheckedArithmetic;
///
/// let a: usize = 10;
/// let b: usize = 20;
///
/// assert_eq!(a.try_add(b).unwrap(), 30);
/// assert!(usize::MAX.try_add(1).is_err());
/// ```
pub trait CheckedArithmetic: Sized {
    /// Checked addition. Returns AllocError on overflow.
    fn try_add(self, rhs: Self) -> AllocResult<Self>;

    /// Checked subtraction. Returns AllocError on underflow.
    fn try_sub(self, rhs: Self) -> AllocResult<Self>;

    /// Checked multiplication. Returns AllocError on overflow.
    fn try_mul(self, rhs: Self) -> AllocResult<Self>;

    /// Checked division. Returns AllocError on division by zero.
    fn try_div(self, rhs: Self) -> AllocResult<Self>;
}

impl CheckedArithmetic for usize {
    #[inline]
    fn try_add(self, rhs: Self) -> AllocResult<Self> {
        self.checked_add(rhs)
            .ok_or_else(|| AllocError::invalid_layout(&format!("overflow: {} + {}", self, rhs)))
    }

    #[inline]
    fn try_sub(self, rhs: Self) -> AllocResult<Self> {
        self.checked_sub(rhs)
            .ok_or_else(|| AllocError::invalid_layout(&format!("underflow: {} - {}", self, rhs)))
    }

    #[inline]
    fn try_mul(self, rhs: Self) -> AllocResult<Self> {
        self.checked_mul(rhs)
            .ok_or_else(|| AllocError::invalid_layout(&format!("overflow: {} * {}", self, rhs)))
    }

    #[inline]
    fn try_div(self, rhs: Self) -> AllocResult<Self> {
        self.checked_div(rhs)
            .ok_or_else(|| AllocError::invalid_input("division by zero"))
    }
}

impl CheckedArithmetic for isize {
    #[inline]
    fn try_add(self, rhs: Self) -> AllocResult<Self> {
        self.checked_add(rhs)
            .ok_or_else(|| AllocError::invalid_layout(&format!("overflow: {} + {}", self, rhs)))
    }

    #[inline]
    fn try_sub(self, rhs: Self) -> AllocResult<Self> {
        self.checked_sub(rhs)
            .ok_or_else(|| AllocError::invalid_layout(&format!("underflow: {} - {}", self, rhs)))
    }

    #[inline]
    fn try_mul(self, rhs: Self) -> AllocResult<Self> {
        self.checked_mul(rhs)
            .ok_or_else(|| AllocError::invalid_layout(&format!("overflow: {} * {}", self, rhs)))
    }

    #[inline]
    fn try_div(self, rhs: Self) -> AllocResult<Self> {
        self.checked_div(rhs)
            .ok_or_else(|| AllocError::invalid_input("division by zero"))
    }
}

// ============================================================================
// Platform Information
// ============================================================================

/// Platform information
///
/// Re-exported from nebula-system for consistency across the ecosystem.
pub use nebula_system::utils::PlatformInfo;

/// Get system page size
///
/// Uses actual syscall-based detection via syscalls module instead of hardcoded values
#[inline]
pub fn page_size() -> usize {
    crate::syscalls::get_page_size()
}

/// Check if a number is power of two
///
/// Re-exported from nebula-system for consistency.
pub use nebula_system::utils::is_power_of_two;

/// Atomically update maximum value
#[inline(always)]
pub fn atomic_max(current: &core::sync::atomic::AtomicUsize, value: usize) {
    let mut max = current.load(core::sync::atomic::Ordering::Relaxed);
    loop {
        if value <= max {
            break;
        }
        match current.compare_exchange_weak(
            max,
            value,
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(x) => max = x,
        }
    }
}

/// Memory barrier types for advanced synchronization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierType {
    /// Load-load barrier
    LoadLoad,
    /// Store-store barrier
    StoreStore,
    /// Load-store barrier
    LoadStore,
    /// Store-load barrier
    StoreLoad,
    /// Release barrier
    Release,
    /// Acquire barrier
    Acquire,
    /// Full barrier
    Full,
}

/// Extended memory barrier with specific ordering
#[inline(always)]
pub fn memory_barrier_ex(barrier_type: BarrierType) {
    match barrier_type {
        BarrierType::LoadLoad => fence(Ordering::Acquire),
        BarrierType::StoreStore => fence(Ordering::Release),
        BarrierType::LoadStore => fence(Ordering::AcqRel),
        BarrierType::StoreLoad => fence(Ordering::SeqCst),
        BarrierType::Release => fence(Ordering::Release),
        BarrierType::Acquire => fence(Ordering::Acquire),
        BarrierType::Full => fence(Ordering::SeqCst),
    }
}

/// Memory operations utilities
pub struct MemoryOps;

impl MemoryOps {
    /// Create new MemoryOps instance
    #[inline]
    pub fn new() -> Self {
        Self
    }
    /// Copy memory with prefetching
    #[inline]
    pub unsafe fn copy_with_prefetch(dst: *mut u8, src: *const u8, len: usize) {
        if len == 0 {
            return;
        }

        // Prefetch source data
        prefetch_read(src);
        if len > cache_line_size() {
            unsafe {
                prefetch_read(src.add(cache_line_size()));
            }
        }

        // Use platform-optimized copy
        unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }

    /// Zero memory with optimization
    #[inline]
    pub unsafe fn zero_optimized(ptr: *mut u8, len: usize) {
        if len == 0 {
            return;
        }
        unsafe {
            ptr::write_bytes(ptr, 0, len);
        }
        compiler_fence(Ordering::SeqCst);
    }

    /// Secure fill slice with pattern
    #[inline]
    pub unsafe fn secure_fill_slice(slice: &mut [u8], pattern: u8) {
        unsafe {
            ptr::write_bytes(slice.as_mut_ptr(), pattern, slice.len());
        }
        compiler_fence(Ordering::SeqCst);
    }
}

/// Backoff utility for spin loops
#[derive(Debug, Clone)]
pub struct Backoff {
    current: u32,
    max: u32,
}

impl Backoff {
    /// Create new backoff with default parameters
    #[inline]
    pub fn new() -> Self {
        Self {
            current: 1,
            max: 64,
        }
    }

    /// Create backoff with custom maximum
    #[inline]
    pub fn with_max(max: u32) -> Self {
        Self { current: 1, max }
    }

    /// Create backoff with custom maximum spin (alias for with_max)
    #[inline]
    pub fn with_max_spin(max: u32) -> Self {
        Self::with_max(max)
    }

    /// Perform backoff
    #[inline]
    pub fn spin(&mut self) {
        for _ in 0..self.current {
            core::hint::spin_loop();
        }
        if self.current < self.max {
            self.current *= 2;
        }
    }

    /// Reset backoff
    #[inline]
    pub fn reset(&mut self) {
        self.current = 1;
    }

    /// Spin or yield depending on iteration count
    #[inline]
    pub fn spin_or_yield(&mut self) {
        if self.current < 8 {
            self.spin();
        } else {
            #[cfg(feature = "std")]
            std::thread::yield_now();
            #[cfg(not(feature = "std"))]
            core::hint::spin_loop();
        }
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

/// Prefetch manager for optimized memory access
pub struct PrefetchManager {
    distance: usize,
    enabled: bool,
}

impl PrefetchManager {
    /// Create new prefetch manager
    #[inline]
    pub fn new() -> Self {
        Self {
            distance: cache_line_size() * 2,
            enabled: true,
        }
    }

    /// Set prefetch distance
    #[inline]
    pub fn set_distance(&mut self, distance: usize) {
        self.distance = distance;
    }

    /// Enable/disable prefetching
    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Prefetch for read at current + distance
    #[inline]
    pub fn prefetch_read_ahead<T>(&self, current: *const T) {
        if self.enabled {
            prefetch_read(unsafe { current.cast::<u8>().add(self.distance) });
        }
    }

    /// Prefetch for write at current + distance
    #[inline]
    pub fn prefetch_write_ahead<T>(&self, current: *mut T) {
        if self.enabled {
            prefetch_write(unsafe { current.cast::<u8>().add(self.distance) });
        }
    }

    /// Prefetch slice for read
    #[inline]
    pub fn prefetch_slice_read<T>(&self, slice: &[T]) {
        if self.enabled && !slice.is_empty() {
            prefetch_read(slice.as_ptr());
        }
    }
}

impl Default for PrefetchManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Performance measurement utilities
#[cfg(feature = "std")]
pub mod perf {
    use super::*;

    /// Measures the execution time of a closure
    ///
    /// # Examples
    /// ```
    /// use nebula_memory::utils::perf::measure_time;
    ///
    /// let (result, duration) = measure_time(|| {
    ///     // Some computation
    ///     42
    /// });
    /// assert_eq!(result, 42);
    /// ```
    pub fn measure_time<F, R>(f: F) -> (R, Duration)
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let result = f();
        let duration = start.elapsed();
        (result, duration)
    }

    /// Measures throughput in operations per second
    pub fn calculate_throughput(operations: u64, duration: Duration) -> f64 {
        if duration.as_secs_f64() == 0.0 {
            0.0
        } else {
            operations as f64 / duration.as_secs_f64()
        }
    }

    /// Formats throughput as human-readable string
    pub fn format_throughput(ops_per_sec: f64) -> String {
        if ops_per_sec < 1_000.0 {
            format!("{:.2} ops/s", ops_per_sec)
        } else if ops_per_sec < 1_000_000.0 {
            format!("{:.2}K ops/s", ops_per_sec / 1_000.0)
        } else if ops_per_sec < 1_000_000_000.0 {
            format!("{:.2}M ops/s", ops_per_sec / 1_000_000.0)
        } else {
            format!("{:.2}B ops/s", ops_per_sec / 1_000_000_000.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alignment_functions() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);

        assert_eq!(align_down(0, 8), 0);
        assert_eq!(align_down(1, 8), 0);
        assert_eq!(align_down(7, 8), 0);
        assert_eq!(align_down(8, 8), 8);
        assert_eq!(align_down(15, 8), 8);

        assert!(is_aligned(0, 8));
        assert!(is_aligned(8, 8));
        assert!(is_aligned(16, 8));
        assert!(!is_aligned(7, 8));
        assert!(!is_aligned(9, 8));

        assert_eq!(padding_needed(0, 8), 0);
        assert_eq!(padding_needed(1, 8), 7);
        assert_eq!(padding_needed(8, 8), 0);
    }

    #[test]
    fn test_power_of_two() {
        assert_eq!(next_power_of_two(0), 1);
        assert_eq!(next_power_of_two(1), 1);
        assert_eq!(next_power_of_two(3), 4);
        assert_eq!(next_power_of_two(63), 64);
    }

    // Tests for format_bytes and format_duration are now in nebula-system

    // Test for cache_line_size is now in nebula-system

    #[cfg(feature = "std")]
    #[test]
    fn test_perf_utils() {
        let (result, duration) = perf::measure_time(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            42
        });

        assert_eq!(result, 42);
        assert!(duration.as_millis() >= 10);

        let throughput = perf::calculate_throughput(1000, Duration::from_secs(1));
        assert_eq!(throughput, 1000.0);

        assert_eq!(perf::format_throughput(500.0), "500.00 ops/s");
        assert_eq!(perf::format_throughput(1500.0), "1.50K ops/s");
        assert_eq!(perf::format_throughput(1_500_000.0), "1.50M ops/s");
    }

    // Test for PlatformInfo is now in nebula-system
}
