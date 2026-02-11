//! Statistics export formats (JSON, Prometheus, etc.)

use super::collector::GlobalStats;
use super::memory_stats::MemoryMetrics;

/// Export format for statistics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// JSON format
    Json,
    /// Prometheus metrics format
    Prometheus,
    /// Plain text format
    PlainText,
    /// CSV format
    Csv,
}

/// Export statistics to various formats
pub trait StatsExporter {
    /// Export to string in the specified format
    fn export(&self, format: ExportFormat) -> String;

    /// Export to JSON format
    fn to_json(&self) -> String {
        self.export(ExportFormat::Json)
    }

    /// Export to Prometheus format
    fn to_prometheus(&self) -> String {
        self.export(ExportFormat::Prometheus)
    }

    /// Export to plain text format
    fn to_text(&self) -> String {
        self.export(ExportFormat::PlainText)
    }
}

impl StatsExporter for GlobalStats {
    fn export(&self, format: ExportFormat) -> String {
        match format {
            ExportFormat::Json => self.to_json_impl(),
            ExportFormat::Prometheus => self.to_prometheus_impl(),
            ExportFormat::PlainText => self.to_text_impl(),
            ExportFormat::Csv => self.to_csv_impl(),
        }
    }
}

impl GlobalStats {
    fn to_json_impl(&self) -> String {
        format!(
            r#"{{
  "total_allocated": {},
  "peak_allocated": {},
  "total_allocations": {},
  "total_deallocations": {},
  "active_allocations": {},
  "fragmentation_ratio": {:.4},
  "allocation_failures": {},
  "avg_allocation_size": {:.2},
  "median_allocation_size": {},
  "allocation_rate": {:.2},
  "deallocation_rate": {:.2},
  "utilization_percent": {:.2},
  "is_critical": {},
  "is_high": {}
}}"#,
            self.total_allocated,
            self.peak_allocated,
            self.total_allocations,
            self.total_deallocations,
            self.active_allocations,
            self.fragmentation_ratio,
            self.allocation_failures,
            self.avg_allocation_size,
            self.median_allocation_size,
            self.allocation_rate,
            self.deallocation_rate,
            self.utilization_percent(),
            self.is_critical(),
            self.is_high()
        )
    }

    fn to_prometheus_impl(&self) -> String {
        format!(
            r"# HELP memory_total_allocated_bytes Total allocated memory in bytes
# TYPE memory_total_allocated_bytes gauge
memory_total_allocated_bytes {}

# HELP memory_peak_allocated_bytes Peak allocated memory in bytes
# TYPE memory_peak_allocated_bytes gauge
memory_peak_allocated_bytes {}

# HELP memory_total_allocations Total number of allocations
# TYPE memory_total_allocations counter
memory_total_allocations {}

# HELP memory_total_deallocations Total number of deallocations
# TYPE memory_total_deallocations counter
memory_total_deallocations {}

# HELP memory_active_allocations Active allocations
# TYPE memory_active_allocations gauge
memory_active_allocations {}

# HELP memory_fragmentation_ratio Memory fragmentation ratio
# TYPE memory_fragmentation_ratio gauge
memory_fragmentation_ratio {:.4}

# HELP memory_allocation_failures Total allocation failures
# TYPE memory_allocation_failures counter
memory_allocation_failures {}

# HELP memory_avg_allocation_size Average allocation size in bytes
# TYPE memory_avg_allocation_size gauge
memory_avg_allocation_size {:.2}

# HELP memory_utilization_percent Memory utilization percentage
# TYPE memory_utilization_percent gauge
memory_utilization_percent {:.2}
",
            self.total_allocated,
            self.peak_allocated,
            self.total_allocations,
            self.total_deallocations,
            self.active_allocations,
            self.fragmentation_ratio,
            self.allocation_failures,
            self.avg_allocation_size,
            self.utilization_percent()
        )
    }

    fn to_text_impl(&self) -> String {
        format!(
            r"Global Memory Statistics
========================
Total Allocated:      {} bytes
Peak Allocated:       {} bytes
Total Allocations:    {}
Total Deallocations:  {}
Active Allocations:   {}
Fragmentation Ratio:  {:.4}
Allocation Failures:  {}
Avg Allocation Size:  {:.2} bytes
Median Alloc Size:    {} bytes
Allocation Rate:      {:.2} allocs/sec
Deallocation Rate:    {:.2} deallocs/sec
Utilization:          {:.2}%
Status:               {}
",
            self.total_allocated,
            self.peak_allocated,
            self.total_allocations,
            self.total_deallocations,
            self.active_allocations,
            self.fragmentation_ratio,
            self.allocation_failures,
            self.avg_allocation_size,
            self.median_allocation_size,
            self.allocation_rate,
            self.deallocation_rate,
            self.utilization_percent(),
            if self.is_critical() {
                "CRITICAL"
            } else if self.is_high() {
                "HIGH"
            } else {
                "NORMAL"
            }
        )
    }

    fn to_csv_impl(&self) -> String {
        format!(
            "total_allocated,peak_allocated,total_allocations,total_deallocations,active_allocations,fragmentation_ratio,allocation_failures,avg_allocation_size,utilization_percent\n{},{},{},{},{},{:.4},{},{:.2},{:.2}",
            self.total_allocated,
            self.peak_allocated,
            self.total_allocations,
            self.total_deallocations,
            self.active_allocations,
            self.fragmentation_ratio,
            self.allocation_failures,
            self.avg_allocation_size,
            self.utilization_percent()
        )
    }
}

impl StatsExporter for MemoryMetrics {
    fn export(&self, format: ExportFormat) -> String {
        match format {
            ExportFormat::Json => self.to_json_impl(),
            ExportFormat::Prometheus => self.to_prometheus_impl(),
            ExportFormat::PlainText => self.to_text_impl(),
            ExportFormat::Csv => self.to_csv_impl(),
        }
    }
}

impl MemoryMetrics {
    fn to_json_impl(&self) -> String {
        format!(
            r#"{{
  "current_allocated": {},
  "peak_allocated": {},
  "allocations": {},
  "deallocations": {},
  "allocation_failures": {}
}}"#,
            self.current_allocated,
            self.peak_allocated,
            self.allocations,
            self.deallocations,
            self.allocation_failures
        )
    }

    fn to_prometheus_impl(&self) -> String {
        format!(
            r"memory_metrics_current_allocated_bytes {}
memory_metrics_peak_allocated_bytes {}
memory_metrics_allocations {}
memory_metrics_deallocations {}
memory_metrics_allocation_failures {}
",
            self.current_allocated,
            self.peak_allocated,
            self.allocations,
            self.deallocations,
            self.allocation_failures
        )
    }

    fn to_text_impl(&self) -> String {
        format!(
            r#"Memory Metrics
==============
Current Allocated:   {} bytes
Peak Allocated:      {} bytes
Allocations:         {}
Deallocations:       {}
Allocation Failures: {}
"#,
            self.current_allocated,
            self.peak_allocated,
            self.allocations,
            self.deallocations,
            self.allocation_failures
        )
    }

    fn to_csv_impl(&self) -> String {
        format!(
            "current_allocated,peak_allocated,allocations,deallocations,allocation_failures\n{},{},{},{},{}",
            self.current_allocated,
            self.peak_allocated,
            self.allocations,
            self.deallocations,
            self.allocation_failures
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_stats_json_export() {
        let mut stats = GlobalStats::default();
        stats.total_allocated = 1000;
        stats.peak_allocated = 1500;
        stats.total_allocations = 100;

        let json = stats.to_json();
        assert!(json.contains("\"total_allocated\": 1000"));
        assert!(json.contains("\"peak_allocated\": 1500"));
    }

    #[test]
    fn test_global_stats_prometheus_export() {
        let mut stats = GlobalStats::default();
        stats.total_allocated = 2000;
        stats.total_allocations = 50;

        let prom = stats.to_prometheus();
        assert!(prom.contains("memory_total_allocated_bytes 2000"));
        assert!(prom.contains("memory_total_allocations 50"));
    }

    #[test]
    fn test_global_stats_text_export() {
        let stats = GlobalStats::default();
        let text = stats.to_text();
        assert!(text.contains("Global Memory Statistics"));
        assert!(text.contains("Total Allocated:"));
    }

    #[test]
    fn test_memory_metrics_export() {
        let metrics = MemoryMetrics {
            allocations: 25,
            deallocations: 10,
            current_allocated: 500,
            peak_allocated: 1000,
            total_allocated_bytes: 1000,
            total_deallocated_bytes: 500,
            total_allocation_time_nanos: 0,
            operations: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            allocation_failures: 2,
            oom_errors: 0,
            hit_rate: 0.0,
            elapsed_secs: 0.0,
            timestamp: std::time::Instant::now(),
        };

        let json = metrics.to_json();
        assert!(json.contains("\"current_allocated\": 500"));

        let prom = metrics.to_prometheus();
        assert!(prom.contains("memory_metrics_current_allocated_bytes 500"));
    }
}
