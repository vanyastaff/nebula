//! Provides functionalities for detailed memory profiling.
//!
//! This module defines `MemoryProfiler`, which can collect and aggregate
//! memory allocation and deallocation events, attributing them to their
//! call sites to help identify memory hotspots and leaks.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(feature = "profiling")]
// Backtrace requires profiling feature
use backtrace::Backtrace;
use parking_lot::RwLock; // Prefer parking_lot for better performance

use super::config::{TrackingConfig, TrackingLevel};
use super::memory_stats::MemoryMetrics; // For fallback `AllocationSite` without backtrace

/// Represents a unique allocation site in the code.
/// This would typically include file, line number, and function name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AllocationSite {
    pub file: String,
    pub line: u32,
    pub function: String,
    #[cfg(feature = "profiling")] // Stack trace only with std and profiling
    pub stack_trace: Option<Vec<String>>,
}

impl AllocationSite {
    /// Captures the current allocation site and optionally its stack trace.
    /// This is a simplified example; a real implementation might use
    /// `backtrace` or custom macros for more precise and efficient capture.
    #[cfg(feature = "profiling")] // Requires std and profiling feature
    pub fn capture_current(config: &TrackingConfig) -> Self {
        let mut file = "unknown_file".to_string();
        let mut line = 0;
        let mut function = "unknown_function".to_string();
        let mut stack_trace = None;

        if config.collect_stack_traces {
            let bt = Backtrace::new();
            let frames_to_skip = 2; // Skip this function and the direct caller (e.g., record_allocation_event)

            let filtered_frames: Vec<String> = bt
                .frames()
                .iter()
                .skip(frames_to_skip)
                .take(config.max_stack_depth)
                .filter_map(|frame| {
                    frame.symbols().first().map(|symbol| {
                        let symbol_name = symbol
                            .name()
                            .and_then(|n| n.as_str())
                            .unwrap_or("<unknown>");
                        let file_line = match (symbol.filename(), symbol.lineno()) {
                            (Some(f), Some(l)) => format!(" ({}:{})", f.display(), l),
                            (Some(f), None) => format!(" ({})", f.display()),
                            _ => String::new(),
                        };
                        format!("{}{}", symbol_name, file_line)
                    })
                })
                .collect();

            if let Some(frame) = bt.frames().get(frames_to_skip)
                && let Some(symbol) = frame.symbols().first()
            {
                file = symbol
                    .filename()
                    .and_then(|p| p.to_str())
                    .unwrap_or("unknown_file")
                    .to_string();
                line = symbol.lineno().unwrap_or(0);
                function = symbol
                    .name()
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown_function")
                    .to_string();
            }
            stack_trace = Some(filtered_frames);
        }

        AllocationSite {
            file,
            line,
            function,
            stack_trace,
        }
    }

    /// A simplified constructor for testing or when `Backtrace` is not desired.
    /// This version handles the `stack_trace` field for compilation without
    /// `profiling` feature.
    #[cfg(not(feature = "profiling"))] // Used when std or profiling is NOT enabled
    pub fn new_manual(file: &str, line: u32, function: &str) -> Self {
        AllocationSite {
            file: file.to_string(),
            line,
            function: function.to_string(),
        }
    }
    /// A simplified constructor for testing or when `Backtrace` is not desired.
    /// This version handles the `stack_trace` field for compilation with
    /// `profiling` feature.
    #[cfg(feature = "profiling")] // Used when std AND profiling are enabled
    pub fn new_manual(file: &str, line: u32, function: &str) -> Self {
        AllocationSite {
            file: file.to_string(),
            line,
            function: function.to_string(),
            stack_trace: None, /* Manually created sites won't have a stack trace unless
                                * specifically set */
        }
    }
}

/// Aggregated statistics for a specific allocation site.
#[derive(Debug, Clone, Default)]
pub struct ProfileEntry {
    pub total_allocations: u64,
    pub total_allocated_bytes: usize,
    pub current_allocated_bytes: usize, // Live allocations from this site
    pub total_deallocations: u64,
    pub total_deallocated_bytes: usize,
    pub peak_allocated_bytes: usize, // Peak live allocated from this site

    pub min_allocation_size: usize,
    pub max_allocation_size: usize,

    // Latency tracking (if `detailed_tracking` is true)
    pub total_allocation_latency_nanos: u128,
    pub allocation_count_for_latency: u64, // Counter specifically for latency average
    pub min_allocation_latency_nanos: u128,
    pub max_allocation_latency_nanos: u128,
}

impl ProfileEntry {
    pub fn new() -> Self {
        Self {
            total_allocations: 0,
            total_allocated_bytes: 0,
            current_allocated_bytes: 0,
            total_deallocations: 0,
            total_deallocated_bytes: 0,
            peak_allocated_bytes: 0,
            min_allocation_size: usize::MAX,
            max_allocation_size: 0,
            total_allocation_latency_nanos: 0,
            allocation_count_for_latency: 0,
            min_allocation_latency_nanos: u128::MAX,
            max_allocation_latency_nanos: 0,
        }
    }

    /// Records an allocation event for this site.
    pub fn record_allocation(&mut self, size: usize, latency: Option<Duration>) {
        self.total_allocations += 1;
        self.total_allocated_bytes += size;
        self.current_allocated_bytes += size;
        self.peak_allocated_bytes = self.peak_allocated_bytes.max(self.current_allocated_bytes);

        self.min_allocation_size = self.min_allocation_size.min(size);
        self.max_allocation_size = self.max_allocation_size.max(size);

        if let Some(lat) = latency {
            let nanos = lat.as_nanos();
            self.total_allocation_latency_nanos += nanos;
            self.allocation_count_for_latency += 1;
            self.min_allocation_latency_nanos = self.min_allocation_latency_nanos.min(nanos);
            self.max_allocation_latency_nanos = self.max_allocation_latency_nanos.max(nanos);
        }
    }

    /// Records a deallocation event for this site.
    pub fn record_deallocation(&mut self, size: usize) {
        self.total_deallocations += 1;
        self.total_deallocated_bytes += size;
        self.current_allocated_bytes = self.current_allocated_bytes.saturating_sub(size);
    }

    /// Calculates the average allocation size for this site.
    pub fn avg_allocation_size(&self) -> f64 {
        if self.total_allocations == 0 {
            0.0
        } else {
            self.total_allocated_bytes as f64 / self.total_allocations as f64
        }
    }

    /// Calculates the average allocation latency for this site in nanoseconds.
    pub fn avg_allocation_latency_nanos(&self) -> f64 {
        if self.allocation_count_for_latency == 0 {
            0.0
        } else {
            self.total_allocation_latency_nanos as f64 / self.allocation_count_for_latency as f64
        }
    }
}

/// A snapshot of the current profiling data.
#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    /// Map of allocation sites to their aggregated profile entries.
    pub entries: HashMap<AllocationSite, ProfileEntry>,
    /// Timestamp of when this snapshot was taken.
    pub timestamp: std::time::Instant,
    /// Overall MemoryMetrics at the time of snapshot
    pub overall_metrics: MemoryMetrics,
}

/// The main memory profiler.
pub struct MemoryProfiler {
    tracking_config: TrackingConfig,
    // Using `Arc<RwLock<...>>` to allow multiple parts of the memory system
    // to update the profiler concurrently while also allowing reads.
    profiling_data: Arc<RwLock<HashMap<AllocationSite, ProfileEntry>>>,
    total_profiler_sampled: u64, /* Keep track of how many allocations were considered for
                                  * profiling */
    start_time: Instant, // Tracks when profiling began or was last reset
}

impl MemoryProfiler {
    /// Creates a new `MemoryProfiler` instance.
    pub fn new(config: TrackingConfig) -> Self {
        Self {
            tracking_config: config,
            profiling_data: Arc::new(RwLock::new(HashMap::new())),
            total_profiler_sampled: 0,
            start_time: Instant::now(),
        }
    }

    /// Creates a new `MemoryProfiler` instance with custom tracking config.
    pub fn with_tracking_config(tracking_config: TrackingConfig) -> Self {
        Self {
            tracking_config,
            profiling_data: Arc::new(RwLock::new(HashMap::new())),
            total_profiler_sampled: 0,
            start_time: Instant::now(),
        }
    }

    /// Records an allocation event with associated call site information.
    ///
    /// This method should be called by the memory allocator/pool whenever
    /// an allocation occurs, providing the size of the allocation and
    /// optionally the duration it took.
    ///
    /// # Arguments
    /// * `size` - The size of the allocated memory in bytes.
    /// * `latency` - Optional duration representing the time taken for the
    ///   allocation.
    pub fn record_allocation_event(&mut self, size: usize, latency: Option<Duration>) {
        if !self.is_profiling_enabled() {
            return;
        }

        #[cfg(feature = "profiling")]
        {
            // Apply profiler-specific sampling rate and size threshold
            if size < self.tracking_config.profiler_size_threshold {
                return;
            }

            // Simple pseudo-random sampling
            let should_sample = {
                let sampled_guard = self.total_profiler_sampled as f64;
                let r = (sampled_guard * 1.618033988749895) % 1.0;
                r < self.tracking_config.profiler_sampling_rate
            };

            if self.tracking_config.profiler_sampling_rate >= 1.0
                || (self.tracking_config.profiler_sampling_rate > 0.0 && should_sample)
            {
                #[cfg(feature = "profiling")]
                let site = self.capture_allocation_site();

                #[cfg(not(feature = "profiling"))]
                let site = AllocationSite::new_manual("unknown", 0, "unknown_function");

                let mut data = self.profiling_data.write();
                data.entry(site)
                    .or_default()
                    .record_allocation(size, latency);

                self.total_profiler_sampled += 1;
            }
        }

        #[cfg(not(feature = "profiling"))]
        {
            // Просто заглушка для случая, когда профилирование отключено
            let _ = size;
            let _ = latency;
        }
    }

    /// Captures allocation site with stack trace based on config
    #[cfg(feature = "profiling")]
    fn capture_allocation_site(&self) -> AllocationSite {
        let mut file = "unknown_file".to_string();
        let mut line = 0;
        let mut function = "unknown_function".to_string();
        let mut stack_trace = None;

        if self.tracking_config.collect_stack_traces {
            let bt = Backtrace::new();
            let frames_to_skip = 3; // Skip this function, record_allocation_event and caller

            let filtered_frames: Vec<String> = bt
                .frames()
                .iter()
                .skip(frames_to_skip)
                .take(self.tracking_config.max_stack_depth)
                .filter_map(|frame| {
                    if let Some(symbol) = frame.symbols().first() {
                        let symbol_name = symbol
                            .name()
                            .and_then(|n| n.as_str())
                            .unwrap_or("<unknown>");

                        let file_path = symbol
                            .filename()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| String::from("<unknown>"));

                        let line_num = symbol.lineno().unwrap_or(0);

                        Some(format!("{} ({}:{})", symbol_name, file_path, line_num))
                    } else {
                        None
                    }
                })
                .collect();

            // Get file and line info from the first frame
            if let Some(frame) = bt.frames().get(frames_to_skip)
                && let Some(symbol) = frame.symbols().first()
            {
                if let Some(filename) = symbol.filename() {
                    file = filename.display().to_string();
                }
                line = symbol.lineno().unwrap_or(0);
                if let Some(name) = symbol.name().and_then(|n| n.as_str()) {
                    function = name.to_string();
                }
            }

            stack_trace = Some(filtered_frames);
        }

        AllocationSite {
            file,
            line,
            function,
            stack_trace,
        }
    }

    /// Records a deallocation event for the specified allocation site.
    ///
    /// # Arguments
    /// * `size` - The size of the memory being deallocated.
    /// * `site` - The allocation site that originally allocated the memory.
    pub fn record_deallocation_event(&mut self, size: usize, site: AllocationSite) {
        if !self.is_profiling_enabled() {
            return;
        }

        let mut data = self.profiling_data.write();
        if let Some(entry) = data.get_mut(&site) {
            entry.record_deallocation(size);
        } else {
            // If site is not found, create a new entry with just the deallocation.
            // This can happen if the allocation was before profiling started,
            // or if sampling missed the allocation event.
            let mut new_entry = ProfileEntry::new();
            new_entry.record_deallocation(size);
            data.insert(site, new_entry);
        }
    }

    /// Retrieves a snapshot of the current profiling data.
    ///
    /// This can be used to generate reports or analyze memory usage patterns.
    pub fn get_profile_snapshot(&self, overall_metrics: MemoryMetrics) -> ProfileSnapshot {
        ProfileSnapshot {
            entries: self.profiling_data.read().clone(), // Clone to get a consistent snapshot
            timestamp: std::time::Instant::now(),
            overall_metrics,
        }
    }

    /// Resets all collected profiling data.
    pub fn reset(&mut self) {
        self.profiling_data.write().clear();
        self.total_profiler_sampled = 0;
        self.start_time = Instant::now();
    }

    /// Checks if profiling is enabled based on the configuration.
    /// Note: This considers `detailed_tracking` AND `profiler_sampling_rate >
    /// 0`.
    pub fn is_profiling_enabled(&self) -> bool {
        #[cfg(feature = "profiling")]
        {
            self.tracking_config.detailed_tracking
                && self.tracking_config.profiler_sampling_rate > 0.0
                && (self.tracking_config.level == TrackingLevel::Debug
                    || self.tracking_config.level == TrackingLevel::Detailed)
        }
        #[cfg(not(feature = "profiling"))]
        {
            false // Profiling is never enabled if the feature is not active
        }
    }

    /// Generates a structured profiling report.
    pub fn generate_report(&self, overall_metrics: MemoryMetrics) -> ProfileReport {
        let snapshot = self.get_profile_snapshot(overall_metrics);
        let hot_spots = Self::find_hot_spots_from_snapshot(&snapshot);
        let allocation_histogram = Self::build_size_histogram_from_snapshot(&snapshot);

        ProfileReport {
            duration: Some(self.start_time.elapsed()),
            total_sampled: self.total_profiler_sampled,
            profiler_sampling_rate: self.tracking_config.profiler_sampling_rate,
            hot_spots,
            allocation_histogram,
            largest_allocations: Self::find_largest_allocations_from_snapshot(&snapshot, 10),
        }
    }

    /// Helper to find allocation hot spots from a snapshot.
    fn find_hot_spots_from_snapshot(snapshot: &ProfileSnapshot) -> Vec<HotSpot> {
        let mut hot_spots: Vec<HotSpot> = snapshot
            .entries
            .iter()
            .map(|(site, entry)| HotSpot {
                location: format!("{}:{}:{}", site.file, site.line, site.function),
                count: entry.total_allocations,
                total_size: entry.total_allocated_bytes as u64,
                average_size: entry.avg_allocation_size(),
                #[cfg(feature = "profiling")]
                stack_trace: site.stack_trace.clone(),
            })
            .collect();

        // Sort by total size (descending)
        hot_spots.sort_by(|a, b| b.total_size.cmp(&a.total_size));
        hot_spots.truncate(20); // Top 20 hot spots

        hot_spots
    }

    /// Helper to build a size histogram from a snapshot.
    fn build_size_histogram_from_snapshot(snapshot: &ProfileSnapshot) -> Vec<SizeBucket> {
        let mut buckets = vec![
            SizeBucket {
                min_size: 0,
                max_size: 64,
                count: 0,
                total_size: 0,
            },
            SizeBucket {
                min_size: 64,
                max_size: 256,
                count: 0,
                total_size: 0,
            },
            SizeBucket {
                min_size: 256,
                max_size: 1024,
                count: 0,
                total_size: 0,
            },
            SizeBucket {
                min_size: 1024,
                max_size: 4096,
                count: 0,
                total_size: 0,
            },
            SizeBucket {
                min_size: 4096,
                max_size: 16384,
                count: 0,
                total_size: 0,
            },
            SizeBucket {
                min_size: 16384,
                max_size: 65536,
                count: 0,
                total_size: 0,
            },
            SizeBucket {
                min_size: 65536,
                max_size: usize::MAX,
                count: 0,
                total_size: 0,
            },
        ];

        for entry in snapshot.entries.values() {
            // For histogram, we consider each allocation event, not just the site total
            // This is a simplification; ideally, you'd iterate individual recorded
            // allocations if `detailed_tracking` implies storing individual
            // allocation events. Given current `ProfileEntry` aggregates, we
            // distribute `total_allocations` and `total_allocated_bytes` into
            // buckets based on average size. A more precise histogram might
            // require storing raw allocation sizes if `detailed_tracking` is true.
            // For now, we'll use `avg_allocation_size` as a proxy if `ProfileEntry` is the
            // source.

            // The original intent of `build_size_histogram` was likely to iterate
            // `self.allocations` (a Vec of `AllocationSite` from the old code).
            // With `ProfileEntry` which aggregates by site, we need to adapt.
            // The `ProfileEntry` gives `total_allocated_bytes` and `total_allocations`.
            // We'll approximate by placing all of a site's `total_allocated_bytes` into the
            // bucket corresponding to its `avg_allocation_size`. This is an
            // approximation.
            let avg_size = entry.avg_allocation_size() as usize; // Use integer avg for bucket
            for bucket in &mut buckets {
                if avg_size >= bucket.min_size && avg_size < bucket.max_size {
                    bucket.count += entry.total_allocations;
                    bucket.total_size += entry.total_allocated_bytes as u64; // Use total allocated bytes
                    break;
                }
            }
        }
        buckets.retain(|b| b.count > 0);
        buckets
    }

    /// Helper to find largest individual allocations from a snapshot.
    /// This method is an approximation given `ProfileEntry` aggregates by site.
    /// To get truly "largest individual allocations", the profiler would need
    /// to store individual allocation events when `detailed_tracking` is
    /// enabled. For now, we'll return the sites with the largest
    /// `max_allocation_size` observed.
    fn find_largest_allocations_from_snapshot(
        snapshot: &ProfileSnapshot,
        count: usize,
    ) -> Vec<AllocationInfo> {
        let mut allocations: Vec<AllocationInfo> = snapshot
            .entries
            .iter()
            .filter(|(_site, entry)| entry.max_allocation_size > 0)
            .map(|(site, entry)| AllocationInfo {
                size: entry.max_allocation_size, // Use max_allocation_size for this report
                count: 1,                        /* Represents one 'largest' observed allocation,
                                                  * not total count at site */
                location: format!("{}:{}:{}", site.file, site.line, site.function),
                #[cfg(feature = "profiling")]
                stack_trace: site.stack_trace.clone(),
            })
            .collect();

        allocations.sort_by(|a, b| b.size.cmp(&a.size));
        allocations.truncate(count);
        allocations
    }
}

/// Profiling report generated by `MemoryProfiler`.
#[derive(Debug, Clone)]
pub struct ProfileReport {
    pub duration: Option<Duration>,
    pub total_sampled: u64, /* Total allocations processed by the profiler logic (after size
                             * threshold, before sampling) */
    pub profiler_sampling_rate: f64, // The actual sampling rate used by the profiler
    pub hot_spots: Vec<HotSpot>,
    pub allocation_histogram: Vec<SizeBucket>,
    pub largest_allocations: Vec<AllocationInfo>,
}

/// Represents an allocation "hot spot" - a code location
/// responsible for a significant amount of memory allocation.
#[derive(Debug, Clone)]
pub struct HotSpot {
    pub location: String, // e.g., "file.rs:line:function_name"
    pub count: u64,       // Total allocations from this site
    pub total_size: u64,  // Total bytes allocated from this site
    pub average_size: f64,
    #[cfg(feature = "profiling")]
    pub stack_trace: Option<Vec<String>>,
}

/// Represents a bucket in the memory allocation size histogram.
#[derive(Debug, Clone)]
pub struct SizeBucket {
    pub min_size: usize,
    pub max_size: usize,
    pub count: u64,      // Number of allocations falling into this bucket
    pub total_size: u64, // Total bytes allocated in this bucket
}

/// Information about a specific allocation (e.g., one of the largest).
#[derive(Debug, Clone)]
pub struct AllocationInfo {
    pub size: usize, // Size of this specific allocation
    pub count: u64,  /* This could be 1 for individual, or sum if aggregated by specific
                      * criteria */
    pub location: String, // Location where this allocation was made
    #[cfg(feature = "profiling")]
    pub stack_trace: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::stats::config::{SamplingConfig, TrackingConfig, TrackingLevel};
    use crate::stats::memory_stats::MemoryMetrics;

    // Вспомогательная функция для создания конфигурации для тестов
    #[cfg(feature = "profiling")]
    fn create_test_config(
        detailed_tracking: bool,
        level: TrackingLevel,
        collect_stack_traces: bool,
        profiler_sampling_rate: f64,
        profiler_size_threshold: usize,
        max_stack_depth: usize,
    ) -> TrackingConfig {
        TrackingConfig {
            detailed_tracking,
            level,
            max_history: 0,                       // Не важно для тестов профайлера
            sampling_interval: Duration::ZERO,    // Не важно для тестов профайлера
            tracked_metrics: Vec::new(),          // Не важно для тестов профайлера
            sampling: SamplingConfig::disabled(), // Не важно для тестов профайлера
            collect_stack_traces,
            profiler_sampling_rate,
            profiler_size_threshold,
            max_stack_depth,
        }
    }

    #[cfg(not(feature = "profiling"))]
    fn create_test_config(detailed_tracking: bool, level: TrackingLevel) -> TrackingConfig {
        TrackingConfig {
            detailed_tracking,
            level,
            max_history: 0,                       // Не важно для тестов профайлера
            sampling_interval: Duration::ZERO,    // Не важно для тестов профайлера
            tracked_metrics: Vec::new(),          // Не важно для тестов профайлера
            sampling: SamplingConfig::disabled(), // Не важно для тестов профайлера
        }
    }

    #[test]
    fn test_profiler_new() {
        #[cfg(feature = "profiling")]
        let config = create_test_config(true, TrackingLevel::Detailed, true, 1.0, 1, 32);

        #[cfg(not(feature = "profiling"))]
        let config = create_test_config(true, TrackingLevel::Detailed);

        let profiler = MemoryProfiler::new(config);
        #[cfg(feature = "profiling")]
        assert!(profiler.is_profiling_enabled());
        assert!(profiler.profiling_data.read().is_empty());
    }

    #[test]
    fn test_profiler_record_allocation() {
        #[cfg(feature = "profiling")]
        let config = create_test_config(true, TrackingLevel::Detailed, true, 1.0, 1, 32);

        #[cfg(not(feature = "profiling"))]
        let config = create_test_config(true, TrackingLevel::Detailed);

        let mut profiler = MemoryProfiler::new(config);

        // Симулируем вызовы из разных мест для разных сайтов
        #[inline(never)] // Предотвращаем инлайнинг для получения различных мест вызова
        fn allocate_from_x(profiler: &mut MemoryProfiler, size: usize, latency: Option<Duration>) {
            profiler.record_allocation_event(size, latency);
        }

        #[inline(never)]
        fn allocate_from_y(profiler: &mut MemoryProfiler, size: usize, latency: Option<Duration>) {
            profiler.record_allocation_event(size, latency);
        }

        allocate_from_x(&mut profiler, 100, Some(Duration::from_nanos(10)));
        allocate_from_x(&mut profiler, 200, Some(Duration::from_nanos(20)));
        allocate_from_y(&mut profiler, 50, None); // Без задержки

        #[cfg(feature = "profiling")] // Только если профилирование включено
        {
            let data = profiler.profiling_data.read();
            assert_eq!(data.len(), 2); // Должно быть два разных места аллокации

            // Ищем записи по определенным критериям
            let mut entries: Vec<&ProfileEntry> = data.values().collect();
            entries.sort_by_key(|e| e.total_allocated_bytes);

            let entry_smaller_alloc = entries
                .iter()
                .find(|&e| e.total_allocated_bytes == 50)
                .unwrap();
            assert_eq!(entry_smaller_alloc.total_allocations, 1);
            assert_eq!(entry_smaller_alloc.current_allocated_bytes, 50);

            let entry_larger_alloc = entries
                .iter()
                .find(|&e| e.total_allocated_bytes == 300)
                .unwrap();
            assert_eq!(entry_larger_alloc.total_allocations, 2);
            assert_eq!(entry_larger_alloc.current_allocated_bytes, 300);
            assert_eq!(entry_larger_alloc.peak_allocated_bytes, 300);
            assert_eq!(entry_larger_alloc.allocation_count_for_latency, 2);
            assert_eq!(entry_larger_alloc.min_allocation_latency_nanos, 10);
            assert_eq!(entry_larger_alloc.max_allocation_latency_nanos, 20);
            assert_eq!(entry_larger_alloc.total_allocation_latency_nanos, 30);
            assert!((entry_larger_alloc.avg_allocation_latency_nanos() - 15.0).abs() < 0.01);

            assert_eq!(profiler.total_profiler_sampled, 3);
        }
        #[cfg(not(feature = "profiling"))]
        {
            assert!(profiler.profiling_data.read().is_empty());
            assert_eq!(profiler.total_profiler_sampled, 0);
        }
    }

    #[test]
    fn test_profiler_record_deallocation() {
        #[cfg(feature = "profiling")]
        let config = create_test_config(
            true,
            TrackingLevel::Detailed,
            false, // Без стеков вызовов для упрощения
            1.0,
            1,
            0,
        );

        #[cfg(not(feature = "profiling"))]
        let config = create_test_config(true, TrackingLevel::Detailed);

        let mut profiler = MemoryProfiler::new(config);
        let site = AllocationSite::new_manual("file_c.rs", 30, "func_z");

        #[cfg(feature = "profiling")]
        {
            // Добавляем записи вручную для симуляции начального состояния
            profiler.profiling_data.write().insert(site.clone(), {
                let mut entry = ProfileEntry::new();
                entry.record_allocation(100, None);
                entry.record_allocation(200, None);
                entry
            });
            assert_eq!(
                profiler
                    .profiling_data
                    .read()
                    .get(&site)
                    .unwrap()
                    .current_allocated_bytes,
                300
            );

            profiler.record_deallocation_event(50, site.clone());
            assert_eq!(
                profiler
                    .profiling_data
                    .read()
                    .get(&site)
                    .unwrap()
                    .current_allocated_bytes,
                250
            );

            profiler.record_deallocation_event(300, site.clone()); // Деаллокация больше, чем живые данные
            assert_eq!(
                profiler
                    .profiling_data
                    .read()
                    .get(&site)
                    .unwrap()
                    .current_allocated_bytes,
                0
            ); // Должно насыщаться на 0
            assert_eq!(
                profiler
                    .profiling_data
                    .read()
                    .get(&site)
                    .unwrap()
                    .total_deallocations,
                2
            );
            assert_eq!(
                profiler
                    .profiling_data
                    .read()
                    .get(&site)
                    .unwrap()
                    .total_deallocated_bytes,
                350
            );
        }
        #[cfg(not(feature = "profiling"))]
        {
            // Когда профилирование отключено, данные не записываются
            assert!(profiler.profiling_data.read().is_empty());
        }
    }

    #[test]
    fn test_profiler_disabled() {
        #[cfg(feature = "profiling")]
        let config = create_test_config(
            false,
            TrackingLevel::Minimal,
            false,
            0.0, // Профайлер фактически отключен через rate
            usize::MAX,
            0,
        );

        #[cfg(not(feature = "profiling"))]
        let config = create_test_config(false, TrackingLevel::Minimal);

        let mut profiler = MemoryProfiler::new(config);
        assert!(!profiler.is_profiling_enabled());

        profiler.record_allocation_event(100, None);
        profiler.record_deallocation_event(50, AllocationSite::new_manual("file.rs", 1, "func"));

        assert!(profiler.profiling_data.read().is_empty()); // Данные не должны записываться
        assert_eq!(profiler.total_profiler_sampled, 0);
    }

    #[test]
    fn test_profiler_sampling_and_threshold() {
        #[cfg(feature = "profiling")]
        let config = create_test_config(
            true,
            TrackingLevel::Detailed,
            false,
            0.5, // 50% семплирование
            100, // порог в 100 байт
            0,
        );

        #[cfg(not(feature = "profiling"))]
        let config = create_test_config(true, TrackingLevel::Detailed);

        let mut profiler = MemoryProfiler::new(config);

        #[cfg(feature = "profiling")] // Запускаем этот блок только если профилирование включено
        {
            // Аллокации: 50, 150, 200, 80, 120 (всего 5)
            // Ожидаемые к обработке (>=100): 150, 200, 120 (всего 3)
            // Ожидаемые к семплированию (50% из обработанных): примерно 1 или 2

            profiler.record_allocation_event(50, None); // Ниже порога, не обрабатывается
            profiler.record_allocation_event(150, None); // Обрабатывается, может быть семплирована
            profiler.record_allocation_event(200, None); // Обрабатывается, может быть семплирована
            profiler.record_allocation_event(80, None); // Ниже порога, не обрабатывается
            profiler.record_allocation_event(120, None); // Обрабатывается, может быть семплирована

            // Из-за простого PRNG точное количество не гарантируется, но должно быть
            // примерно 1-2
            let sampled_count = profiler
                .profiling_data
                .read()
                .values()
                .map(|e| e.total_allocations)
                .sum::<u64>();
            // Используем диапазон для проверок из-за природы простого семплирования
            assert!(sampled_count >= 1 && sampled_count <= 3); // Минимум 1, максимум 3 если все семплированы
            assert_eq!(profiler.total_profiler_sampled, sampled_count);
        }
        #[cfg(not(feature = "profiling"))]
        {
            assert!(profiler.profiling_data.read().is_empty());
            assert_eq!(profiler.total_profiler_sampled, 0);
        }
    }

    #[test]
    fn test_profiler_generate_report() {
        #[cfg(feature = "profiling")]
        let config = create_test_config(true, TrackingLevel::Detailed, true, 1.0, 1, 4);

        #[cfg(not(feature = "profiling"))]
        let config = create_test_config(true, TrackingLevel::Detailed);

        let mut profiler = MemoryProfiler::new(config);

        #[cfg(feature = "profiling")]
        {
            // Симулируем аллокации из двух разных мест
            #[inline(never)]
            fn hotspot_func_a(p: &mut MemoryProfiler, size: usize) {
                p.record_allocation_event(size, None);
            }
            #[inline(never)]
            fn hotspot_func_b(p: &mut MemoryProfiler, size: usize) {
                p.record_allocation_event(size, None);
            }

            hotspot_func_a(&mut profiler, 100);
            hotspot_func_a(&mut profiler, 200);
            hotspot_func_b(&mut profiler, 500);
            hotspot_func_a(&mut profiler, 150); // Еще из A
            hotspot_func_b(&mut profiler, 1000); // Еще из B

            let dummy_metrics = MemoryMetrics {
                allocations: 5,
                deallocations: 0,
                current_allocated: 1950,
                peak_allocated: 1950,
                total_allocated_bytes: 1950,
                total_deallocated_bytes: 0,
                total_allocation_time_nanos: 0,
                operations: 0,
                hits: 0,
                misses: 0,
                evictions: 0,
                allocation_failures: 0,
                oom_errors: 0,
                hit_rate: 0.0,
                elapsed_secs: 1.0,
                timestamp: std::time::Instant::now(),
            };

            let report = profiler.generate_report(dummy_metrics.clone());

            assert!(report.duration.is_some());
            assert_eq!(report.total_sampled, 5); // Все 5 аллокаций обработаны
            assert_eq!(report.profiler_sampling_rate, 1.0);

            assert_eq!(report.hot_spots.len(), 2); // Должно найти два горячих места (func_a, func_b)

            // Находим горячее место func_a
            let hotspot_a = report
                .hot_spots
                .iter()
                .find(|h| h.location.contains("hotspot_func_a"))
                .unwrap();
            assert_eq!(hotspot_a.count, 3);
            assert_eq!(hotspot_a.total_size, 100 + 200 + 150); // 450 байт
            assert!((hotspot_a.average_size - (450.0 / 3.0)).abs() < 0.01);
            #[cfg(feature = "profiling")]
            assert!(hotspot_a.stack_trace.is_some());

            // Находим горячее место func_b
            let hotspot_b = report
                .hot_spots
                .iter()
                .find(|h| h.location.contains("hotspot_func_b"))
                .unwrap();
            assert_eq!(hotspot_b.count, 2);
            assert_eq!(hotspot_b.total_size, 500 + 1000); // 1500 байт
            assert!((hotspot_b.average_size - (1500.0 / 2.0)).abs() < 0.01);
            #[cfg(feature = "profiling")]
            assert!(hotspot_b.stack_trace.is_some());

            // Проверяем самые большие аллокации (должны быть отсортированы по размеру по
            // убыванию)
            assert_eq!(report.largest_allocations.len(), 2);
            assert_eq!(report.largest_allocations[0].size, 1000); // Из func_b
            assert_eq!(report.largest_allocations[1].size, 200); // Из func_a (макс размер)

            // Проверяем гистограмму
            assert!(!report.allocation_histogram.is_empty());
            let total_hist_allocs: u64 = report.allocation_histogram.iter().map(|b| b.count).sum();
            let total_hist_bytes: u64 = report
                .allocation_histogram
                .iter()
                .map(|b| b.total_size)
                .sum();

            // Общее количество аллокаций и байт в гистограмме должно совпадать с общим из
            // горячих мест
            assert_eq!(total_hist_allocs, hotspot_a.count + hotspot_b.count);
            assert_eq!(
                total_hist_bytes,
                hotspot_a.total_size + hotspot_b.total_size
            );
        }
        #[cfg(not(feature = "profiling"))]
        {
            // Если профилирование отключено, отчет будет содержать пустые данные
            let dummy_metrics = MemoryMetrics::default();
            let report = profiler.generate_report(dummy_metrics);
            assert!(report.hot_spots.is_empty());
            assert!(report.allocation_histogram.is_empty());
            assert!(report.largest_allocations.is_empty());
            assert_eq!(report.total_sampled, 0);
        }
    }
}
