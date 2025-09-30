//! Utility functions and helpers for nebula-memory
//!
//! This module provides common utilities used throughout the crate:
//! - Memory alignment helpers
//! - Size formatting utilities
//! - Platform-specific helpers
//! - Performance measurement tools

use core::sync::atomic::{compiler_fence, fence, Ordering};
use core::{mem, ptr};
#[cfg(feature = "std")]
use std::time::{Duration, Instant};

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
#[inline]
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

/// Formats bytes into human-readable string
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
pub fn format_bytes(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    const THRESHOLD: f64 = 1024.0;

    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= THRESHOLD && unit_index < UNITS.len() - 1 {
        size /= THRESHOLD;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

/// Formats duration into human-readable string
///
/// # Examples
/// ```
/// use std::time::Duration;
///
/// use nebula_memory::utils::format_duration;
///
/// assert_eq!(format_duration(Duration::from_nanos(500)), "500ns");
/// assert_eq!(format_duration(Duration::from_micros(1500)), "1.50ms");
/// assert_eq!(format_duration(Duration::from_secs(65)), "1m 5s");
/// ```
#[cfg(feature = "std")]
pub fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();

    if nanos < 1_000 {
        format!("{}ns", nanos)
    } else if nanos < 1_000_000 {
        format!("{:.2}µs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{:.2}s", duration.as_secs_f64())
        } else if secs < 3600 {
            let mins = secs / 60;
            let secs = secs % 60;
            format!("{}m {}s", mins, secs)
        } else {
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            let secs = secs % 60;
            format!("{}h {}m {}s", hours, mins, secs)
        }
    }
}

/// Format percentage
pub fn format_percentage(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

/// Format rate (per second)
pub fn format_rate(rate: f64) -> String {
    if rate < 1_000.0 {
        format!("{:.1}/s", rate)
    } else if rate < 1_000_000.0 {
        format!("{:.1}K/s", rate / 1_000.0)
    } else {
        format!("{:.1}M/s", rate / 1_000_000.0)
    }
}

/// Get cache line size for current platform
#[inline]
pub fn cache_line_size() -> usize {
    cfg_if::cfg_if! {
        if #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))] {
            64
        } else {
            // Conservative default
            64
        }
    }
}

/// Memory barrier for synchronization
#[inline(always)]
pub fn memory_barrier() {
    fence(Ordering::SeqCst);
}

/// Securely zero memory
#[inline]
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
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn prefetch_read<T>(ptr: *const T) {
    use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
    unsafe {
        _mm_prefetch::<_MM_HINT_T0>(ptr.cast());
    }
}

#[inline]
#[cfg(not(target_arch = "x86_64"))]
pub fn prefetch_read<T>(_ptr: *const T) {}

/// Platform-specific prefetch for write
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn prefetch_write<T>(ptr: *mut T) {
    use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T1};
    unsafe {
        _mm_prefetch::<_MM_HINT_T1>(ptr.cast());
    }
}

#[inline]
#[cfg(not(target_arch = "x86_64"))]
pub fn prefetch_write<T>(_ptr: *mut T) {}

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
        Self { start: Instant::now(), name }
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

/// Platform information
#[derive(Debug, Clone, Copy)]
pub struct PlatformInfo {
    pub page_size: usize,
    pub cache_line_size: usize,
    pub pointer_width: usize,
}

impl PlatformInfo {
    #[inline]
    pub fn current() -> Self {
        Self {
            page_size: page_size(),
            cache_line_size: cache_line_size(),
            pointer_width: mem::size_of::<usize>() * 8,
        }
    }
}

/// Get system page size
#[inline]
pub fn page_size() -> usize {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            4096 // Common page size for Unix systems
        } else if #[cfg(windows)] {
            4096 // Common page size for Windows
        } else {
            4096 // Conservative default
        }
    }
}

/// Check if a number is power of two
#[inline(always)]
pub const fn is_power_of_two(n: usize) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

/// Atomically update maximum value
#[inline]
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
        Self { current: 1, max: 64 }
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
    where F: FnOnce() -> R {
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

    #[cfg(feature = "std")]
    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_format_duration() {
        use std::time::Duration;

        assert_eq!(format_duration(Duration::from_nanos(500)), "500ns");
        assert_eq!(format_duration(Duration::from_micros(500)), "500.00µs");
        assert_eq!(format_duration(Duration::from_millis(500)), "500.00ms");
        assert_eq!(format_duration(Duration::from_secs(30)), "30.00s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m 1s");
    }

    #[test]
    fn test_cache_line_utils() {
        let cache_size = cache_line_size();
        assert!(cache_size > 0);
        assert!(cache_size.is_power_of_two());
    }

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

    #[test]
    fn test_platform_info() {
        let info = PlatformInfo::current();
        assert!(info.page_size >= 4096);
        assert!(info.cache_line_size >= 64);
        assert!(info.pointer_width == 32 || info.pointer_width == 64);
    }
}
