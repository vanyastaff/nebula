//! Monitored allocator with system pressure awareness
//!
//! This allocator integrates with nebula-system monitoring to automatically
//! adjust allocation behavior based on system memory pressure.
//!
//! # Safety
//!
//! This module wraps another allocator with monitoring:
//! - Allocator trait impl: Forwards to inner allocator with pressure checks
//! - GlobalAlloc impl: Compatibility layer for global allocator usage
//!
//! ## Safety Contracts
//!
//! - allocate/deallocate/grow/shrink: Forwarded to inner allocator (preserves contracts)
//! - Pressure checks may deny allocations but don't affect memory safety
//! - Statistics tracking is atomic and thread-safe
//! - GlobalAlloc impl converts between Allocator and GlobalAlloc interfaces

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;

#[cfg(feature = "std")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "logging")]
use nebula_log::{debug, error, warn};

use crate::allocator::{
    AllocError, AllocErrorCode, AllocResult, Allocator, AllocatorStats, AtomicAllocatorStats,
    StatisticsProvider,
};
use crate::error::{MemoryError, MemoryResult};

#[cfg(feature = "std")]
use crate::monitoring::{IntegratedStats, MemoryMonitor, MonitoringConfig, PressureAction};

/// Allocator wrapper that monitors system memory pressure and adjusts behavior
#[derive(Debug)]
pub struct MonitoredAllocator<A> {
    /// Underlying allocator
    inner: A,
    /// Statistics tracking
    stats: AtomicAllocatorStats,
    #[cfg(feature = "std")]
    /// Memory pressure monitor
    monitor: Arc<Mutex<MemoryMonitor>>,
    /// Configuration
    config: MonitoredConfig,
}

/// Configuration for monitored allocator
#[derive(Debug, Clone)]
pub struct MonitoredConfig {
    /// Maximum allocation size during high pressure
    pub max_high_pressure_alloc: usize,
    /// Maximum allocation size during critical pressure
    pub max_critical_pressure_alloc: usize,
    /// Enable detailed logging
    pub detailed_logging: bool,
    /// Fail allocations during critical pressure
    pub fail_on_critical: bool,
}

impl Default for MonitoredConfig {
    fn default() -> Self {
        Self {
            max_high_pressure_alloc: 64 * 1024,    // 64KB
            max_critical_pressure_alloc: 4 * 1024, // 4KB
            detailed_logging: true,
            fail_on_critical: false,
        }
    }
}

impl<A> MonitoredAllocator<A>
where
    A: Allocator,
{
    /// Create a new monitored allocator with default configuration
    #[cfg(feature = "std")]
    pub fn new(allocator: A) -> Self {
        Self::with_config(allocator, MonitoredConfig::default())
    }

    /// Create a new monitored allocator with custom configuration
    #[cfg(feature = "std")]
    pub fn with_config(allocator: A, config: MonitoredConfig) -> Self {
        Self {
            inner: allocator,
            stats: AtomicAllocatorStats::new(),
            monitor: Arc::new(Mutex::new(MemoryMonitor::new())),
            config,
        }
    }

    /// Create a monitored allocator with custom monitoring configuration
    #[cfg(feature = "std")]
    pub fn with_monitoring_config(
        allocator: A,
        config: MonitoredConfig,
        monitoring_config: MonitoringConfig,
    ) -> Self {
        Self {
            inner: allocator,
            stats: AtomicAllocatorStats::new(),
            monitor: Arc::new(Mutex::new(MemoryMonitor::with_config(monitoring_config))),
            config,
        }
    }

    /// Get current integrated statistics combining allocator and system metrics
    #[cfg(feature = "std")]
    pub fn integrated_stats(&self) -> MemoryResult<IntegratedStats> {
        let allocator_stats = self.stats.snapshot();
        let monitor = self.monitor.lock().map_err(|e| {
            MemoryError::initialization_failed(format!("Monitor lock failed: {}", e))
        })?;
        let monitoring_stats = monitor.get_stats();

        Ok(IntegratedStats::new(allocator_stats, monitoring_stats))
    }

    /// Check if allocation should be allowed based on size and system pressure
    #[cfg(feature = "std")]
    fn should_allow_allocation(&self, layout: Layout) -> MemoryResult<bool> {
        let mut monitor = self.monitor.lock().map_err(|e| {
            MemoryError::initialization_failed(format!("Monitor lock failed: {}", e))
        })?;

        // Check if large allocation should be allowed
        if !monitor.should_allow_large_allocation(layout.size())? {
            #[cfg(feature = "logging")]
            if self.config.detailed_logging {
                warn!(
                    "Allocation denied by monitor: size={}, align={}",
                    layout.size(),
                    layout.align()
                );
            }
            return Ok(false);
        }

        // Check pressure-specific limits
        let (memory_info, action) = monitor.check_pressure()?;

        let allowed = match action {
            PressureAction::None | PressureAction::Warn => true,
            PressureAction::ReduceAllocations | PressureAction::ForceCleanup => {
                layout.size() <= self.config.max_high_pressure_alloc
            }
            PressureAction::DenyLargeAllocations => {
                layout.size() <= self.config.max_critical_pressure_alloc
            }
            PressureAction::Emergency => {
                if self.config.fail_on_critical {
                    false
                } else {
                    layout.size() <= self.config.max_critical_pressure_alloc
                }
            }
        };

        #[cfg(feature = "logging")]
        if !allowed && self.config.detailed_logging {
            error!(
                "Allocation denied due to pressure action {:?}: size={}, pressure={:.1}%",
                action,
                layout.size(),
                memory_info.usage_percent
            );
        }

        Ok(allowed)
    }

    /// Get the underlying allocator
    pub fn inner(&self) -> &A {
        &self.inner
    }

    /// Get mutable reference to the underlying allocator
    pub fn inner_mut(&mut self) -> &mut A {
        &mut self.inner
    }
}

#[cfg(not(feature = "std"))]
impl<A> MonitoredAllocator<A>
where
    A: Allocator,
{
    /// Create a new monitored allocator (no-std version - no monitoring)
    pub fn new(allocator: A) -> Self {
        Self::with_config(allocator, MonitoredConfig::default())
    }

    /// Create a new monitored allocator with custom configuration (no-std version)
    pub fn with_config(allocator: A, config: MonitoredConfig) -> Self {
        Self {
            inner: allocator,
            stats: AtomicAllocatorStats::new(),
            config,
        }
    }

    /// Check if allocation should be allowed (no-std version - always true)
    fn should_allow_allocation(&self, _layout: Layout) -> MemoryResult<bool> {
        Ok(true)
    }

    /// Get the underlying allocator
    pub fn inner(&self) -> &A {
        &self.inner
    }

    /// Get mutable reference to the underlying allocator
    pub fn inner_mut(&mut self) -> &mut A {
        &mut self.inner
    }
}

// SAFETY: MonitoredAllocator forwards all operations to inner Allocator A.
// - All safety contracts preserved through delegation
// - Pressure checking happens before allocation (doesn't affect safety)
// - Statistics tracking is thread-safe (atomic operations)
// - Monitoring errors are caught and logged, allocation proceeds
unsafe impl<A> Allocator for MonitoredAllocator<A>
where
    A: Allocator,
{
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // Check if allocation should be allowed
        match self.should_allow_allocation(layout) {
            Ok(true) => {
                // Proceed with allocation
                // SAFETY: Forwarding to inner allocator.
                // - layout is valid (caller contract)
                // - inner.allocate upholds same safety contract as self.allocate
                match self.inner.allocate(layout) {
                    Ok(ptr) => {
                        self.stats.record_allocation(layout.size());

                        #[cfg(feature = "logging")]
                        if self.config.detailed_logging {
                            debug!(
                                "Allocation successful: size={}, align={}",
                                layout.size(),
                                layout.align()
                            );
                        }

                        Ok(ptr)
                    }
                    Err(err) => {
                        self.stats.record_allocation_failure();

                        #[cfg(feature = "logging")]
                        if self.config.detailed_logging {
                            warn!(
                                "Allocation failed: size={}, align={}, error={:?}",
                                layout.size(),
                                layout.align(),
                                err
                            );
                        }

                        Err(err)
                    }
                }
            }
            Ok(false) => {
                // Allocation denied by monitor
                self.stats.record_allocation_failure();
                Err(AllocError::with_layout(0, layout))
            }
            Err(monitor_error) => {
                // Monitor error - log and allow allocation
                #[cfg(feature = "logging")]
                if self.config.detailed_logging {
                    warn!("Monitor error, allowing allocation: {}", monitor_error);
                }

                // SAFETY: Forwarding to inner allocator (monitor error fallback).
                // - layout is valid (caller contract)
                // - inner.allocate upholds safety contract
                match self.inner.allocate(layout) {
                    Ok(ptr) => {
                        self.stats.record_allocation(layout.size());
                        Ok(ptr)
                    }
                    Err(err) => {
                        self.stats.record_allocation_failure();
                        Err(err)
                    }
                }
            }
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Forwarding to inner allocator.
        // - ptr/layout match allocation (caller contract)
        // - inner.deallocate upholds safety contract
        self.inner.deallocate(ptr, layout);
        self.stats.record_deallocation(layout.size());

        #[cfg(feature = "logging")]
        if self.config.detailed_logging {
            debug!(
                "Deallocation: size={}, align={}",
                layout.size(),
                layout.align()
            );
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Check if the new size should be allowed
        match self.should_allow_allocation(new_layout) {
            Ok(true) => {
                // SAFETY: Forwarding to inner allocator.
                // - ptr/old_layout/new_layout valid (caller contract)
                // - inner.grow upholds safety contract
                match self.inner.grow(ptr, old_layout, new_layout) {
                    Ok(new_ptr) => {
                        self.stats
                            .record_reallocation(old_layout.size(), new_layout.size());
                        Ok(new_ptr)
                    }
                    Err(err) => {
                        self.stats.record_allocation_failure();
                        Err(err)
                    }
                }
            }
            Ok(false) => {
                self.stats.record_allocation_failure();
                Err(AllocError::with_layout(0, new_layout))
            }
            Err(_) => {
                // Monitor error - allow growth
                // SAFETY: Forwarding to inner allocator (monitor error fallback).
                // - ptr/old_layout/new_layout valid (caller contract)
                // - inner.grow upholds safety contract
                match self.inner.grow(ptr, old_layout, new_layout) {
                    Ok(new_ptr) => {
                        self.stats
                            .record_reallocation(old_layout.size(), new_layout.size());
                        Ok(new_ptr)
                    }
                    Err(err) => {
                        self.stats.record_allocation_failure();
                        Err(err)
                    }
                }
            }
        }
    }

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Forwarding to inner allocator.
        // - ptr/old_layout/new_layout valid (caller contract)
        // - Shrinking always allowed (reduces memory usage)
        // - inner.shrink upholds safety contract
        match self.inner.shrink(ptr, old_layout, new_layout) {
            Ok(new_ptr) => {
                self.stats
                    .record_reallocation(old_layout.size(), new_layout.size());
                Ok(new_ptr)
            }
            Err(err) => {
                self.stats.record_allocation_failure();
                Err(err)
            }
        }
    }
}

impl<A> StatisticsProvider for MonitoredAllocator<A>
where
    A: Allocator,
{
    fn statistics(&self) -> AllocatorStats {
        self.stats.snapshot()
    }

    fn reset_statistics(&self) {
        self.stats.reset();

        #[cfg(feature = "std")]
        if let Ok(mut monitor) = self.monitor.lock() {
            monitor.reset_stats();
        }
    }
}

// Implement GlobalAlloc for convenience when using as a global allocator
// SAFETY: GlobalAlloc impl wraps Allocator trait.
// - alloc/dealloc/realloc converted from Allocator methods
// - Null returned on allocation failure (GlobalAlloc contract)
// - All safety contracts forwarded from Allocator
unsafe impl<A> GlobalAlloc for MonitoredAllocator<A>
where
    A: Allocator + Sync,
{
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Calling self.allocate (Allocator trait method).
        // - layout valid (caller contract)
        // - Converting NonNull<[u8]> to *mut u8
        match self.allocate(layout) {
            Ok(ptr) => ptr.as_ptr() as *mut u8,
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Calling self.deallocate if ptr is non-null.
        // - ptr/layout valid (caller contract)
        // - NonNull::new safely handles null check
        if let Some(ptr) = NonNull::new(ptr) {
            self.deallocate(ptr, layout);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Reallocating via grow/shrink.
        // - ptr/layout valid (caller contract)
        // - NonNull::new safely handles null check
        // - new_layout validated before use
        if let Some(ptr) = NonNull::new(ptr) {
            if let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) {
                if new_size > layout.size() {
                    // Growing
                    match self.grow(ptr, layout, new_layout) {
                        Ok(new_ptr) => new_ptr.as_ptr() as *mut u8,
                        Err(_) => core::ptr::null_mut(),
                    }
                } else {
                    // Shrinking
                    match self.shrink(ptr, layout, new_layout) {
                        Ok(new_ptr) => new_ptr.as_ptr() as *mut u8,
                        Err(_) => core::ptr::null_mut(),
                    }
                }
            } else {
                core::ptr::null_mut()
            }
        } else {
            core::ptr::null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::system::SystemAllocator;

    #[test]
    fn test_monitored_allocator_creation() {
        let system_alloc = SystemAllocator::new();
        let monitored = MonitoredAllocator::new(system_alloc);

        let stats = monitored.statistics();
        assert_eq!(stats.allocation_count, 0);
        assert_eq!(stats.allocated_bytes, 0);
    }

    #[test]
    fn test_allocation_tracking() {
        let system_alloc = SystemAllocator::new();
        let monitored = MonitoredAllocator::new(system_alloc);

        let layout = Layout::from_size_align(1024, 8).unwrap();

        unsafe {
            if let Ok(ptr) = monitored.allocate(layout) {
                let stats = monitored.statistics();
                assert_eq!(stats.allocation_count, 1);
                assert_eq!(stats.allocated_bytes, 1024);

                monitored.deallocate(ptr.cast(), layout);

                let stats = monitored.statistics();
                assert_eq!(stats.deallocation_count, 1);
                assert_eq!(stats.allocated_bytes, 0);
            }
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_integrated_stats() {
        let system_alloc = SystemAllocator::new();
        let monitored = MonitoredAllocator::new(system_alloc);

        // Allocate some memory to generate stats
        let layout = Layout::from_size_align(1024, 8).unwrap();
        unsafe {
            if let Ok(ptr) = monitored.allocate(layout) {
                let integrated = monitored.integrated_stats().unwrap();

                assert_eq!(integrated.allocator.allocation_count, 1);
                assert!(integrated.combined.health_score > 0.0);

                monitored.deallocate(ptr.cast(), layout);
            }
        }
    }
}
