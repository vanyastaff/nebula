//! Utility functions and helpers for nebula-memory
//!
//! This module provides high-performance utilities for memory management
//! with safe abstractions over platform-specific operations.

use core::sync::atomic::{compiler_fence, fence, AtomicU64, AtomicUsize, Ordering};
use core::{hint, mem, ptr};

#[cfg(feature = "std")]
use std::sync::LazyLock;
#[cfg(feature = "std")]
use std::time::{Duration, Instant};

// ============================================================================
// Safe Prefetch Abstraction (как в предыдущем ответе)
// ============================================================================

pub struct PrefetchManager {
    _private: (),
}

impl PrefetchManager {
    pub const fn new() -> Self {
        Self { _private: () }
    }

    #[inline(always)]
    pub fn prefetch_read<T>(&self, data: &T) {
        let ptr = data as *const T;
        unsafe { self.prefetch_read_raw(ptr) }
    }

    #[inline(always)]
    pub fn prefetch_write<T>(&self, data: &mut T) {
        let ptr = data as *mut T;
        unsafe { self.prefetch_write_raw(ptr) }
    }

    #[inline]
    pub fn prefetch_slice_read<T>(&self, slice: &[T]) {
        if slice.is_empty() {
            return;
        }

        let ptr = slice.as_ptr();
        let len = slice.len() * mem::size_of::<T>();
        unsafe { self.prefetch_range_read_raw(ptr as *const u8, len) }
    }

    #[inline]
    pub fn prefetch_slice_write<T>(&self, slice: &mut [T]) {
        if slice.is_empty() {
            return;
        }

        let ptr = slice.as_mut_ptr();
        let len = slice.len() * mem::size_of::<T>();
        unsafe { self.prefetch_range_write_raw(ptr as *mut u8, len) }
    }

    #[inline(always)]
    unsafe fn prefetch_read_raw<T>(&self, ptr: *const T) {
        #[cfg(target_arch = "x86_64")]
        {
            use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            unsafe { _mm_prefetch::<_MM_HINT_T0>(ptr as *const i8) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { core::arch::aarch64::__prefetch(ptr.cast(), 0, 3) }
        }

        #[cfg(target_arch = "arm")]
        {
            unsafe { core::arch::arm::__pld(ptr.cast()) }
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")))]
        {
            let _ = ptr;
        }
    }

    #[inline(always)]
    unsafe fn prefetch_write_raw<T>(&self, ptr: *mut T) {
        #[cfg(target_arch = "x86_64")]
        {
            use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            unsafe { _mm_prefetch::<_MM_HINT_T0>(ptr as *const i8) }
        }

        #[cfg(target_arch = "aarch64")]
        {
            unsafe { core::arch::aarch64::__prefetch(ptr.cast(), 1, 3) }
        }

        #[cfg(target_arch = "arm")]
        {
            unsafe { core::arch::arm::__pldw(ptr.cast()) }
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")))]
        {
            let _ = ptr;
        }
    }

    unsafe fn prefetch_range_read_raw(&self, start: *const u8, len: usize) {
        let cache_line = cache_line_size();
        let mut addr = start as usize;
        let end = addr + len;

        let max_prefetch = cache_line * 16;
        let prefetch_end = end.min(addr + max_prefetch);

        while addr < prefetch_end {
            unsafe { self.prefetch_read_raw(addr as *const u8) }
            addr += cache_line;
        }
    }

    unsafe fn prefetch_range_write_raw(&self, start: *mut u8, len: usize) {
        let cache_line = cache_line_size();
        let mut addr = start as usize;
        let end = addr + len;

        let max_prefetch = cache_line * 16;
        let prefetch_end = end.min(addr + max_prefetch);

        while addr < prefetch_end {
            unsafe { self.prefetch_write_raw(addr as *mut u8) }
            addr += cache_line;
        }
    }
}

#[cfg(feature = "std")]
pub static PREFETCH: LazyLock<PrefetchManager> = LazyLock::new(|| PrefetchManager::new());

// ============================================================================
// Safe Memory Operations
// ============================================================================

pub struct MemoryOps {
    _private: (),
}

impl MemoryOps {
    pub const fn new() -> Self {
        Self { _private: () }
    }

    pub fn secure_zero_slice(&self, slice: &mut [u8]) {
        if slice.is_empty() {
            return;
        }

        let ptr = slice.as_mut_ptr();
        let len = slice.len();
        unsafe { self.secure_zero_raw(ptr, len) }
    }

    pub fn secure_fill_slice(&self, slice: &mut [u8], pattern: u8) {
        if slice.is_empty() {
            return;
        }

        let ptr = slice.as_mut_ptr();
        let len = slice.len();
        unsafe { self.secure_fill_raw(ptr, len, pattern) }
    }

    pub fn copy_slices(&self, src: &[u8], dst: &mut [u8]) -> Result<(), CopyError> {
        if src.len() != dst.len() {
            return Err(CopyError::LengthMismatch);
        }

        if src.is_empty() {
            return Ok(());
        }

        unsafe { self.fast_copy_raw(src.as_ptr(), dst.as_mut_ptr(), src.len()) }

        Ok(())
    }

    pub fn move_slices(&self, src: &[u8], dst: &mut [u8]) -> Result<(), CopyError> {
        if src.len() != dst.len() {
            return Err(CopyError::LengthMismatch);
        }

        if src.is_empty() {
            return Ok(());
        }

        unsafe { self.fast_move_raw(src.as_ptr(), dst.as_mut_ptr(), src.len()) }

        Ok(())
    }

    unsafe fn secure_zero_raw(&self, ptr: *mut u8, len: usize) {
        debug_assert!(!ptr.is_null());

        unsafe {
            for i in 0..len {
                ptr::write_volatile(ptr.add(i), 0);
            }
        }
        compiler_fence(Ordering::SeqCst);
    }

    unsafe fn secure_fill_raw(&self, ptr: *mut u8, len: usize, pattern: u8) {
        debug_assert!(!ptr.is_null());

        unsafe {
            for i in 0..len {
                ptr::write_volatile(ptr.add(i), pattern);
            }
        }
        compiler_fence(Ordering::SeqCst);
    }

    unsafe fn fast_copy_raw(&self, src: *const u8, dst: *mut u8, len: usize) {
        debug_assert!(!src.is_null() && !dst.is_null());

        #[cfg(feature = "std")]
        if len > cache_line_size() * 4 {
            unsafe {
                PREFETCH.prefetch_range_read_raw(src, len.min(2048));
            }
        }

        unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }

    unsafe fn fast_move_raw(&self, src: *const u8, dst: *mut u8, len: usize) {
        debug_assert!(!src.is_null() && !dst.is_null());

        unsafe {
            ptr::copy(src, dst, len);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CopyError {
    LengthMismatch,
}

#[cfg(feature = "std")]
pub static MEMORY: LazyLock<MemoryOps> = LazyLock::new(|| MemoryOps::new());

// ============================================================================
// Memory Barriers - Safe Abstraction
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub enum BarrierType {
    Full,
    Acquire,
    Release,
    Compiler,
}

pub struct BarrierOps;

impl BarrierOps {
    #[inline(always)]
    pub fn barrier(&self, barrier_type: BarrierType) {
        match barrier_type {
            BarrierType::Full => fence(Ordering::SeqCst),
            BarrierType::Acquire => fence(Ordering::Acquire),
            BarrierType::Release => fence(Ordering::Release),
            BarrierType::Compiler => compiler_fence(Ordering::SeqCst),
        }
    }

    #[inline(always)]
    pub fn full_barrier(&self) {
        fence(Ordering::SeqCst);
    }
}

pub const BARRIER: BarrierOps = BarrierOps;

// Convenience functions
#[inline(always)]
pub fn memory_barrier() {
    BARRIER.full_barrier();
}

#[inline(always)]
pub fn memory_barrier_ex(barrier: BarrierType) {
    BARRIER.barrier(barrier);
}

// ============================================================================
// Atomic Operations - Safe Wrappers
// ============================================================================

pub struct AtomicOps;

impl AtomicOps {
    #[inline]
    pub fn max(&self, atomic: &AtomicUsize, value: usize) -> usize {
        atomic.fetch_max(value, Ordering::AcqRel)
    }

    #[inline]
    pub fn min(&self, atomic: &AtomicUsize, value: usize) -> usize {
        atomic.fetch_min(value, Ordering::AcqRel)
    }
}

pub const ATOMIC: AtomicOps = AtomicOps;

// Convenience functions
#[inline]
pub fn atomic_max(atomic: &AtomicUsize, value: usize) -> usize {
    ATOMIC.max(atomic, value)
}

#[inline]
pub fn atomic_min(atomic: &AtomicUsize, value: usize) -> usize {
    ATOMIC.min(atomic, value)
}

// Atomic statistics tracker remains the same as it's already safe
#[derive(Debug)]
pub struct AtomicStats {
    pub count: AtomicU64,
    pub sum: AtomicU64,
    pub max: AtomicUsize,
    pub min: AtomicUsize,
}

impl AtomicStats {
    pub const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
            max: AtomicUsize::new(0),
            min: AtomicUsize::new(usize::MAX),
        }
    }

    #[inline]
    pub fn record(&self, value: usize) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value as u64, Ordering::Relaxed);
        atomic_max(&self.max, value);
        atomic_min(&self.min, value);
    }

    pub fn average(&self) -> f64 {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            self.sum.load(Ordering::Relaxed) as f64 / count as f64
        }
    }

    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.sum.store(0, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
        self.min.store(usize::MAX, Ordering::Relaxed);
    }
}

// ============================================================================
// Spin Loop & Backoff - Already Safe
// ============================================================================

#[inline(always)]
pub fn spin_loop() {
    hint::spin_loop();
}

pub struct Backoff {
    step: u32,
    max_spin: u32,
}

impl Backoff {
    #[inline]
    pub const fn new() -> Self {
        Self { step: 0, max_spin: 6 }
    }

    #[inline]
    pub const fn with_max_spin(max_spin: u32) -> Self {
        Self { step: 0, max_spin }
    }

    #[inline]
    pub fn reset(&mut self) {
        self.step = 0;
    }

    #[inline]
    pub fn spin(&mut self) {
        for _ in 0..(1 << self.step.min(self.max_spin)) {
            hint::spin_loop();
        }

        if self.step <= self.max_spin {
            self.step += 1;
        }
    }

    #[inline]
    pub fn spin_or_yield(&mut self) {
        if self.step <= self.max_spin {
            self.spin();
        } else {
            #[cfg(feature = "std")]
            std::thread::yield_now();
            #[cfg(not(feature = "std"))]
            self.spin();
        }
    }

    #[inline]
    pub fn is_completed(&self) -> bool {
        self.step > self.max_spin
    }
}

// ============================================================================
// Memory Alignment - Safe Operations
// ============================================================================

pub struct AlignmentOps;

impl AlignmentOps {
    #[inline(always)]
    pub const fn align_up(&self, value: usize, alignment: usize) -> usize {
        debug_assert!(alignment.is_power_of_two());

        if value == usize::MAX {
            return usize::MAX;
        }

        let added = value.saturating_add(alignment - 1);
        added & !(alignment - 1)
    }

    /// Unsafe unchecked version for hot paths
    ///
    /// # Safety
    /// Caller must ensure that `value + alignment - 1` doesn't overflow
    #[inline(always)]
    pub const unsafe fn align_up_unchecked(&self, value: usize, alignment: usize) -> usize {
        debug_assert!(alignment.is_power_of_two());
        (value + alignment - 1) & !(alignment - 1)
    }

    #[inline(always)]
    pub const fn try_align_up(&self, value: usize, alignment: usize) -> Option<usize> {
        debug_assert!(alignment.is_power_of_two());

        match value.checked_add(alignment - 1) {
            Some(added) => Some(added & !(alignment - 1)),
            None => None,
        }
    }

    #[inline(always)]
    pub const fn align_down(&self, value: usize, alignment: usize) -> usize {
        debug_assert!(alignment.is_power_of_two());
        value & !(alignment - 1)
    }

    #[inline(always)]
    pub const fn is_aligned(&self, value: usize, alignment: usize) -> bool {
        debug_assert!(alignment.is_power_of_two());
        value & (alignment - 1) == 0
    }

    #[inline(always)]
    pub fn is_aligned_ref<T>(&self, reference: &T, alignment: usize) -> bool {
        let ptr = reference as *const T;
        self.is_aligned(ptr as usize, alignment)
    }

    #[inline(always)]
    pub fn align_ref_up<T>(&self, reference: &T, alignment: usize) -> usize {
        let addr = reference as *const T as usize;
        self.align_up(addr, alignment)
    }

    #[inline(always)]
    pub fn align_ref_down<T>(&self, reference: &T, alignment: usize) -> usize {
        let addr = reference as *const T as usize;
        self.align_down(addr, alignment)
    }

    #[inline(always)]
    pub const fn alignment_for_type<T>(&self) -> usize {
        mem::align_of::<T>()
    }

    #[inline(always)]
    pub const fn padding_needed(&self, value: usize, alignment: usize) -> usize {
        let aligned = self.align_up(value, alignment);
        aligned.saturating_sub(value)
    }
}

pub const ALIGN: AlignmentOps = AlignmentOps;

// Convenience functions
#[inline(always)]
pub const fn align_up(value: usize, alignment: usize) -> usize {
    ALIGN.align_up(value, alignment)
}

/// Unsafe version for hot paths
///
/// # Safety
/// Caller must ensure that `value + alignment - 1` doesn't overflow
#[inline(always)]
pub const unsafe fn align_up_unchecked(value: usize, alignment: usize) -> usize {
    // SAFETY: Caller guarantees no overflow
    unsafe { ALIGN.align_up_unchecked(value, alignment) }
}

#[inline(always)]
pub const fn try_align_up(value: usize, alignment: usize) -> Option<usize> {
    ALIGN.try_align_up(value, alignment)
}

#[inline(always)]
pub const fn align_down(value: usize, alignment: usize) -> usize {
    ALIGN.align_down(value, alignment)
}

#[inline(always)]
pub const fn is_aligned(value: usize, alignment: usize) -> bool {
    ALIGN.is_aligned(value, alignment)
}

#[inline(always)]
pub fn is_aligned_ptr<T>(ptr: *const T, alignment: usize) -> bool {
    ALIGN.is_aligned(ptr as usize, alignment)
}

// Safe pointer alignment functions using references
#[inline(always)]
pub fn align_ptr_up<T>(ptr: *const T, alignment: usize) -> *const T {
    let addr = ptr as usize;
    let aligned = align_up(addr, alignment);
    aligned as *const T
}

#[inline(always)]
pub fn align_ptr_down<T>(ptr: *const T, alignment: usize) -> *const T {
    let addr = ptr as usize;
    let aligned = align_down(addr, alignment);
    aligned as *const T
}

#[inline(always)]
pub fn align_ptr_up_mut<T>(ptr: *mut T, alignment: usize) -> *mut T {
    align_ptr_up(ptr as *const T, alignment) as *mut T
}

#[inline(always)]
pub const fn alignment_for_type<T>() -> usize {
    mem::align_of::<T>()
}

#[inline(always)]
pub const fn padding_needed(value: usize, alignment: usize) -> usize {
    ALIGN.padding_needed(value, alignment)
}

// ============================================================================
// Power of Two Operations - Already Safe
// ============================================================================

#[inline]
pub const fn next_power_of_two(value: usize) -> usize {
    match value.checked_next_power_of_two() {
        Some(v) => v,
        None => usize::MAX / 2 + 1,
    }
}

#[inline(always)]
pub const fn is_power_of_two(value: usize) -> bool {
    value != 0 && (value & (value - 1)) == 0
}

#[inline(always)]
pub const fn log2_power_of_two(value: usize) -> u32 {
    debug_assert!(is_power_of_two(value));
    value.trailing_zeros()
}

// ============================================================================
// Platform Information - Safe Cached Access
// ============================================================================

#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub page_size: usize,
    pub cache_line_size: usize,
    pub pointer_width: usize,
    pub huge_page_size: Option<usize>,
    pub numa_nodes: usize,
    pub cpu_count: usize,
    pub total_memory: Option<usize>,
}

impl PlatformInfo {
    #[cfg(feature = "std")]
    pub fn current() -> &'static Self {
        static PLATFORM_INFO: LazyLock<PlatformInfo> = LazyLock::new(|| PlatformInfo::detect());
        &PLATFORM_INFO
    }

    #[cfg(not(feature = "std"))]
    pub const fn current() -> Self {
        Self {
            page_size: 4096,
            cache_line_size: 64,
            pointer_width: mem::size_of::<usize>() * 8,
            huge_page_size: None,
            numa_nodes: 1,
            cpu_count: 1,
            total_memory: None,
        }
    }

    #[cfg(feature = "std")]
    fn detect() -> Self {
        Self {
            page_size: page_size(),
            cache_line_size: cache_line_size(),
            pointer_width: mem::size_of::<usize>() * 8,
            huge_page_size: detect_huge_page_size(),
            numa_nodes: detect_numa_nodes(),
            cpu_count: num_cpus(),
            total_memory: total_memory(),
        }
    }
}

// Platform detection functions
#[cfg(all(feature = "std", unix))]
pub fn page_size() -> usize {
    // SAFETY: sysconf is thread-safe
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

#[cfg(all(feature = "std", windows))]
pub fn page_size() -> usize {
    4096
}

#[cfg(not(feature = "std"))]
pub const fn page_size() -> usize {
    4096
}

#[cfg(feature = "std")]
pub fn cache_line_size() -> usize {
    static CACHE_LINE_SIZE: LazyLock<usize> = LazyLock::new(|| detect_cache_line_size());
    *CACHE_LINE_SIZE
}

#[cfg(not(feature = "std"))]
pub const fn cache_line_size() -> usize {
    64
}

#[cfg(feature = "std")]
fn detect_cache_line_size() -> usize {
    #[cfg(all(target_os = "linux"))]
    {
        if let Ok(size) =
            std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cache/index0/coherency_line_size")
        {
            if let Ok(size) = size.trim().parse::<usize>() {
                return size;
            }
        }
    }

    #[cfg(all(target_os = "macos"))]
    {
        // SAFETY: sysctlbyname is safe with proper parameters
        unsafe {
            let mut size: usize = 0;
            let mut len = mem::size_of::<usize>();
            let res = libc::sysctlbyname(
                "hw.cachelinesize\0".as_ptr() as *const _,
                &mut size as *mut _ as *mut _,
                &mut len,
                ptr::null_mut(),
                0,
            );
            if res == 0 && size > 0 {
                return size;
            }
        }
    }

    64
}

#[cfg(feature = "std")]
fn num_cpus() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

#[cfg(all(feature = "std", target_os = "linux"))]
fn total_memory() -> Option<usize> {
    // SAFETY: sysconf is thread-safe
    unsafe {
        let pages = libc::sysconf(libc::_SC_PHYS_PAGES);
        let page_size = libc::sysconf(libc::_SC_PAGESIZE);

        if pages > 0 && page_size > 0 {
            Some((pages * page_size) as usize)
        } else {
            None
        }
    }
}

#[cfg(not(all(feature = "std", target_os = "linux")))]
fn total_memory() -> Option<usize> {
    None
}

#[cfg(all(feature = "std", target_os = "linux"))]
fn detect_huge_page_size() -> Option<usize> {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("Hugepagesize:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(size) = parts[1].parse::<usize>() {
                        return Some(size * 1024);
                    }
                }
            }
        }
    }
    None
}

#[cfg(all(feature = "std", windows))]
fn detect_huge_page_size() -> Option<usize> {
    Some(2 * 1024 * 1024)
}

#[cfg(not(all(feature = "std", any(target_os = "linux", windows))))]
fn detect_huge_page_size() -> Option<usize> {
    None
}

#[cfg(all(feature = "std", target_os = "linux"))]
fn detect_numa_nodes() -> usize {
    if let Ok(entries) = std::fs::read_dir("/sys/devices/system/node/") {
        let count = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().map(|s| s.starts_with("node")).unwrap_or(false))
            .count();

        if count > 0 {
            return count;
        }
    }
    1
}

#[cfg(not(all(feature = "std", target_os = "linux")))]
fn detect_numa_nodes() -> usize {
    1
}

// ============================================================================
// Formatting Utilities
// ============================================================================

#[cfg(feature = "std")]
pub fn format_bytes(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    const THRESHOLD: f64 = 1000.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

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

#[cfg(feature = "std")]
pub fn format_bytes_binary(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"];
    const THRESHOLD: f64 = 1024.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

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

#[cfg(feature = "std")]
pub fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();

    if nanos == 0 {
        return "0ns".to_string();
    } else if nanos < 1_000 {
        format!("{}ns", nanos)
    } else if nanos < 1_000_000 {
        format!("{:.2}μs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{:.2}s", duration.as_secs_f64())
        } else if secs < 3600 {
            let mins = secs / 60;
            let secs = secs % 60;
            if secs > 0 {
                format!("{}m {}s", mins, secs)
            } else {
                format!("{}m", mins)
            }
        } else if secs < 86400 {
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            if mins > 0 {
                format!("{}h {}m", hours, mins)
            } else {
                format!("{}h", hours)
            }
        } else {
            let days = secs / 86400;
            let hours = (secs % 86400) / 3600;
            if hours > 0 {
                format!("{}d {}h", days, hours)
            } else {
                format!("{}d", days)
            }
        }
    }
}

// ============================================================================
// Performance Measurement
// ============================================================================

#[cfg(feature = "std")]
pub mod perf {
    use super::*;

    #[derive(Debug)]
    pub struct Timer {
        start: Instant,
        name: &'static str,
        auto_print: bool,
        operations: Option<u64>,
    }

    impl Timer {
        #[inline]
        pub fn new(name: &'static str) -> Self {
            Self { start: Instant::now(), name, auto_print: true, operations: None }
        }

        #[inline]
        pub fn silent(name: &'static str) -> Self {
            Self { start: Instant::now(), name, auto_print: false, operations: None }
        }

        #[inline]
        pub fn with_operations(name: &'static str, operations: u64) -> Self {
            Self { start: Instant::now(), name, auto_print: true, operations: Some(operations) }
        }

        #[inline]
        pub fn elapsed(&self) -> Duration {
            self.start.elapsed()
        }

        pub fn print(&self) {
            let elapsed = self.elapsed();

            match self.operations {
                Some(ops) => {
                    let throughput = calculate_throughput(ops, elapsed);
                    println!(
                        "{}: {} ({} ops, {})",
                        self.name,
                        super::format_duration(elapsed),
                        ops,
                        format_throughput(throughput)
                    );
                },
                None => {
                    println!("{}: {}", self.name, super::format_duration(elapsed));
                },
            }
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            if self.auto_print {
                self.print();
            }
        }
    }

    #[inline]
    pub fn measure_time<F, R>(f: F) -> (R, Duration)
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let result = f();
        let duration = start.elapsed();
        (result, duration)
    }

    #[inline]
    pub fn calculate_throughput(operations: u64, duration: Duration) -> f64 {
        let secs = duration.as_secs_f64();
        if secs == 0.0 {
            f64::INFINITY
        } else {
            operations as f64 / secs
        }
    }

    pub fn format_throughput(ops_per_sec: f64) -> String {
        if ops_per_sec == f64::INFINITY {
            "∞ ops/s".to_string()
        } else if ops_per_sec < 1.0 {
            format!("{:.3} ops/s", ops_per_sec)
        } else if ops_per_sec < 1_000.0 {
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

// ============================================================================
// Debug Utilities
// ============================================================================

#[inline(always)]
pub fn debug_assert_aligned<T>(ptr: *const T) {
    debug_assert!(
        is_aligned_ptr(ptr, mem::align_of::<T>()),
        "Pointer {:p} is not aligned for type {} (requires {} byte alignment)",
        ptr,
        core::any::type_name::<T>(),
        mem::align_of::<T>()
    );
}

// Safe version using references
#[inline(always)]
pub fn debug_assert_aligned_ref<T>(reference: &T) {
    let ptr = reference as *const T;
    debug_assert_aligned(ptr);
}

#[cfg(feature = "std")]
pub fn hexdump(data: &[u8], offset: usize) -> String {
    use std::fmt::Write;

    let mut result = String::with_capacity(data.len() * 4);

    for (i, chunk) in data.chunks(16).enumerate() {
        let _ = write!(&mut result, "{:08x}  ", offset + i * 16);

        for (j, byte) in chunk.iter().enumerate() {
            if j == 8 {
                result.push(' ');
            }
            let _ = write!(&mut result, "{:02x} ", byte);
        }

        if chunk.len() < 16 {
            for j in chunk.len()..16 {
                if j == 8 {
                    result.push(' ');
                }
                result.push_str("   ");
            }
        }

        result.push_str(" |");

        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' {
                result.push(*byte as char);
            } else {
                result.push('.');
            }
        }

        result.push_str("|\n");
    }

    result
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alignment_saturating() {
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(usize::MAX - 5, 8), usize::MAX - 7);
        assert_eq!(align_up(usize::MAX - 7, 8), usize::MAX - 7);
        assert_eq!(align_up(usize::MAX, 8), usize::MAX);
    }

    #[test]
    fn test_try_align_up() {
        assert_eq!(try_align_up(7, 8), Some(8));
        assert_eq!(try_align_up(usize::MAX - 5, 8), None);
        assert_eq!(try_align_up(usize::MAX - 7, 8), Some(usize::MAX - 7));
    }

    #[test]
    fn test_atomic_stats() {
        let stats = AtomicStats::new();

        stats.record(10);
        stats.record(20);
        stats.record(5);

        assert_eq!(stats.count.load(Ordering::Relaxed), 3);
        assert_eq!(stats.sum.load(Ordering::Relaxed), 35);
        assert_eq!(stats.max.load(Ordering::Relaxed), 20);
        assert_eq!(stats.min.load(Ordering::Relaxed), 5);
        assert!((stats.average() - 11.67).abs() < 0.01);
    }

    #[test]
    fn test_backoff() {
        let mut backoff = Backoff::new();

        for _ in 0..5 {
            backoff.spin();
        }
        assert!(!backoff.is_completed());

        let mut backoff = Backoff::with_max_spin(2);
        for _ in 0..10 {
            backoff.spin();
        }
        assert!(backoff.is_completed());
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1000), "1.00 KB");
        assert_eq!(format_bytes_binary(1024), "1.00 KiB");

        assert_eq!(format_duration(Duration::from_nanos(500)), "500ns");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
        assert_eq!(format_duration(Duration::from_secs(90000)), "1d 1h");
    }

    #[test]
    fn test_safe_prefetch() {
        let data = vec![1, 2, 3, 4, 5];
        let prefetch = PrefetchManager::new();

        prefetch.prefetch_read(&data[0]);
        prefetch.prefetch_slice_read(&data);
    }

    #[test]
    fn test_safe_memory_ops() {
        let memory = MemoryOps::new();
        let mut buffer = vec![1u8; 100];

        memory.secure_zero_slice(&mut buffer);
        assert!(buffer.iter().all(|&b| b == 0));

        memory.secure_fill_slice(&mut buffer, 0xFF);
        assert!(buffer.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_safe_alignment_checks() {
        let data = 42u64;
        assert!(ALIGN.is_aligned_ref(&data, mem::align_of::<u64>()));

        let aligned_addr = ALIGN.align_ref_up(&data, 128);
        assert!(ALIGN.is_aligned(aligned_addr, 128));
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_hexdump() {
        let data = b"Hello, World!";
        let dump = hexdump(data, 0);
        assert!(dump.contains("48 65 6c 6c 6f"));
        assert!(dump.contains("|Hello, World!|"));
    }
}
