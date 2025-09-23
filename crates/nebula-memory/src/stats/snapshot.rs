//! Memory state snapshots and comparisons
//!
//! This module provides functionality for capturing, storing, and comparing
//! memory state snapshots for debugging and analysis purposes.

#[cfg(not(feature = "std"))]
use alloc::{collections::BTreeMap as HashMap, format, string::String, vec::Vec};
#[cfg(feature = "std")]
use std::{collections::HashMap, time::Instant};

use super::memory_stats::MemoryMetrics;
use crate::error::{MemoryError, MemoryResult};
use crate::utils;

/// Snapshot format for serialization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotFormat {
    /// Human-readable text
    Text,
    /// JSON format
    Json,
    /// CSV for data analysis
    Csv,
    /// Binary format (placeholder)
    Binary,
}

/// Complete memory state snapshot
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    /// Unique snapshot identifier
    pub id: u64,
    /// When the snapshot was taken
    #[cfg(feature = "std")]
    pub timestamp: Instant,
    /// Core memory metrics
    pub metrics: MemoryMetrics,
    /// Component-specific snapshots
    pub components: Vec<ComponentSnapshot>,
    /// System memory information
    pub system_info: Option<SystemMemoryInfo>,
    /// Custom metadata
    pub metadata: HashMap<String, String>,
    /// Snapshot tags for categorization
    pub tags: Vec<String>,
}

/// Snapshot of a specific component (pool, arena, cache, etc.)
#[derive(Debug, Clone)]
pub struct ComponentSnapshot {
    /// Component name (e.g., "string_pool", "main_arena")
    pub name: String,
    /// Component type (e.g., "Pool", "Arena", "Cache")
    pub component_type: ComponentType,
    /// Component-specific metrics
    pub metrics: MemoryMetrics,
    /// Additional component details
    pub details: HashMap<String, ComponentDetail>,
}

/// Type of memory component
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentType {
    Pool,
    Arena,
    Cache,
    Allocator,
    Custom,
}

/// Component detail value
#[derive(Debug, Clone)]
pub enum ComponentDetail {
    /// Integer count
    Count(u64),
    /// Size in bytes
    Size(u64),
    /// Percentage value (0.0-1.0)
    Percentage(f64),
    /// Text description
    Text(String),
    /// Boolean flag
    Flag(bool),
}

/// System memory information
#[derive(Debug, Clone)]
pub struct SystemMemoryInfo {
    /// Total system memory in bytes
    pub total_memory: u64,
    /// Available system memory in bytes
    pub available_memory: u64,
    /// Current process memory usage in bytes
    pub process_memory: u64,
    /// Swap memory used in bytes
    pub swap_used: u64,
    /// Total swap memory in bytes
    pub swap_total: u64,
    /// Memory pressure level (0.0-1.0)
    pub memory_pressure: f64,
}

impl MemorySnapshot {
    /// Create a new snapshot
    pub fn new(id: u64, metrics: MemoryMetrics) -> Self {
        Self {
            id,
            #[cfg(feature = "std")]
            timestamp: Instant::now(),
            metrics,
            components: Vec::new(),
            system_info: None,
            metadata: HashMap::new(),
            tags: Vec::new(),
        }
    }

    /// Create a snapshot with timestamp
    #[cfg(feature = "std")]
    pub fn new_with_timestamp(id: u64, metrics: MemoryMetrics, timestamp: Instant) -> Self {
        Self {
            id,
            timestamp,
            metrics,
            components: Vec::new(),
            system_info: None,
            metadata: HashMap::new(),
            tags: Vec::new(),
        }
    }

    /// Add a component snapshot
    pub fn add_component(&mut self, component: ComponentSnapshot) {
        self.components.push(component);
    }

    /// Add multiple components
    pub fn add_components(&mut self, components: Vec<ComponentSnapshot>) {
        self.components.extend(components);
    }

    /// Add metadata entry
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }

    /// Add multiple metadata entries
    pub fn add_metadata_entries(&mut self, entries: HashMap<String, String>) {
        self.metadata.extend(entries);
    }

    /// Add tag
    pub fn add_tag(&mut self, tag: String) {
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    /// Add multiple tags
    pub fn add_tags(&mut self, tags: Vec<String>) {
        for tag in tags {
            self.add_tag(tag);
        }
    }

    /// Set system information
    pub fn set_system_info(&mut self, info: SystemMemoryInfo) {
        self.system_info = Some(info);
    }

    /// Get component by name
    pub fn get_component(&self, name: &str) -> Option<&ComponentSnapshot> {
        self.components.iter().find(|c| c.name == name)
    }

    /// Get components by type
    pub fn get_components_by_type(&self, component_type: ComponentType) -> Vec<&ComponentSnapshot> {
        self.components.iter().filter(|c| c.component_type == component_type).collect()
    }

    /// Calculate total component memory
    pub fn total_component_memory(&self) -> usize {
        self.components.iter().map(|c| c.metrics.current_allocated).sum()
    }

    /// Format the snapshot in the specified format
    pub fn format(&self, format: SnapshotFormat) -> MemoryResult<String> {
        match format {
            SnapshotFormat::Text => Ok(self.format_text()),
            SnapshotFormat::Json => self.format_json(),
            SnapshotFormat::Csv => Ok(self.format_csv()),
            SnapshotFormat::Binary => Err(MemoryError::NotSupported {
                feature: "binary format",
                context: Some("Binary serialization not implemented".to_string()),
            }),
        }
    }

    /// Format as human-readable text
    fn format_text(&self) -> String {
        let mut output = format!(
            "Memory Snapshot #{}\n\
             ===================\n",
            self.id
        );

        #[cfg(feature = "std")]
        {
            output.push_str(&format!("Timestamp: {:?}\n", self.timestamp));
        }

        if !self.tags.is_empty() {
            output.push_str(&format!("Tags: {}\n", self.tags.join(", ")));
        }

        output.push_str("\nOverall Metrics:\n");
        output.push_str(&format!(
            "  Current Allocated: {}\n",
            utils::format_bytes(self.metrics.current_allocated)
        ));
        output.push_str(&format!(
            "  Peak Allocated: {}\n",
            utils::format_bytes(self.metrics.peak_allocated)
        ));
        output.push_str(&format!("  Total Allocations: {}\n", self.metrics.allocations));
        output.push_str(&format!(
            "  Fragmentation: {}\n",
            utils::format_percentage(self.metrics.fragmentation_ratio())
        ));

        #[cfg(feature = "std")]
        if self.metrics.elapsed_secs > 0.0 {
            output.push_str(&format!(
                "  Allocation Rate: {:.2} allocs/sec\n",
                self.metrics.allocation_rate()
            ));
        }

        // System information
        if let Some(sys) = &self.system_info {
            output.push_str("\nSystem Memory:\n");
            output.push_str(&format!(
                "  Total: {}\n",
                utils::format_bytes(sys.total_memory as usize)
            ));
            output.push_str(&format!(
                "  Available: {}\n",
                utils::format_bytes(sys.available_memory as usize)
            ));
            output.push_str(&format!(
                "  Process: {} ({:.1}%)\n",
                utils::format_bytes(sys.process_memory as usize),
                (sys.process_memory as f64 / sys.total_memory as f64) * 100.0
            ));

            if sys.swap_total > 0 {
                output.push_str(&format!(
                    "  Swap: {} / {}\n",
                    utils::format_bytes(sys.swap_used as usize),
                    utils::format_bytes(sys.swap_total as usize)
                ));
            }

            output.push_str(&format!(
                "  Memory Pressure: {}\n",
                utils::format_percentage(sys.memory_pressure)
            ));
        }

        // Components
        if !self.components.is_empty() {
            output.push_str("\nComponents:\n");

            // Group by type for better organization
            for component_type in [
                ComponentType::Pool,
                ComponentType::Arena,
                ComponentType::Cache,
                ComponentType::Allocator,
                ComponentType::Custom,
            ] {
                let components = self.get_components_by_type(component_type);
                if !components.is_empty() {
                    output.push_str(&format!("\n  {:?}s:\n", component_type));

                    for comp in components {
                        output.push_str(&format!("    {}:\n", comp.name));
                        output.push_str(&format!(
                            "      Current: {}\n",
                            utils::format_bytes(comp.metrics.current_allocated)
                        ));
                        output.push_str(&format!(
                            "      Peak: {}\n",
                            utils::format_bytes(comp.metrics.peak_allocated)
                        ));

                        // Add component-specific details
                        for (key, detail) in &comp.details {
                            output.push_str(&format!("      {}: {}\n", key, detail.format()));
                        }
                    }
                }
            }
        }

        // Metadata
        if !self.metadata.is_empty() {
            output.push_str("\nMetadata:\n");
            for (key, value) in &self.metadata {
                output.push_str(&format!("  {}: {}\n", key, value));
            }
        }

        output
    }

    /// Format as JSON
    fn format_json(&self) -> MemoryResult<String> {
        // Simple JSON formatting without external dependencies
        let mut json = String::new();
        json.push_str("{\n");
        json.push_str(&format!("  \"id\": {},\n", self.id));

        #[cfg(feature = "std")]
        {
            json.push_str(&format!("  \"timestamp\": \"{:?}\",\n", self.timestamp));
        }

        if !self.tags.is_empty() {
            json.push_str("  \"tags\": [");
            for (i, tag) in self.tags.iter().enumerate() {
                if i > 0 {
                    json.push_str(", ");
                }
                json.push_str(&format!("\"{}\"", tag));
            }
            json.push_str("],\n");
        }

        // Overall metrics
        json.push_str("  \"metrics\": {\n");
        json.push_str(&format!("    \"current_allocated\": {},\n", self.metrics.current_allocated));
        json.push_str(&format!("    \"peak_allocated\": {},\n", self.metrics.peak_allocated));
        json.push_str(&format!("    \"allocations\": {},\n", self.metrics.allocations));
        json.push_str(&format!("    \"deallocations\": {},\n", self.metrics.deallocations));
        json.push_str(&format!(
            "    \"fragmentation_ratio\": {:.4},\n",
            self.metrics.fragmentation_ratio()
        ));
        json.push_str(&format!("    \"hit_rate\": {:.4}\n", self.metrics.hit_rate));
        json.push_str("  },\n");

        // System info
        if let Some(sys) = &self.system_info {
            json.push_str("  \"system_info\": {\n");
            json.push_str(&format!("    \"total_memory\": {},\n", sys.total_memory));
            json.push_str(&format!("    \"available_memory\": {},\n", sys.available_memory));
            json.push_str(&format!("    \"process_memory\": {},\n", sys.process_memory));
            json.push_str(&format!("    \"swap_used\": {},\n", sys.swap_used));
            json.push_str(&format!("    \"swap_total\": {},\n", sys.swap_total));
            json.push_str(&format!("    \"memory_pressure\": {:.4}\n", sys.memory_pressure));
            json.push_str("  },\n");
        } else {
            json.push_str("  \"system_info\": null,\n");
        }

        // Components
        json.push_str("  \"components\": [\n");
        for (i, comp) in self.components.iter().enumerate() {
            if i > 0 {
                json.push_str(",\n");
            }
            json.push_str("    {\n");
            json.push_str(&format!("      \"name\": \"{}\",\n", comp.name));
            json.push_str(&format!("      \"type\": \"{:?}\",\n", comp.component_type));
            json.push_str(&format!(
                "      \"current_allocated\": {},\n",
                comp.metrics.current_allocated
            ));
            json.push_str(&format!("      \"peak_allocated\": {}\n", comp.metrics.peak_allocated));
            json.push_str("    }");
        }
        json.push_str("\n  ],\n");

        // Metadata
        json.push_str("  \"metadata\": {\n");
        let mut first = true;
        for (key, value) in &self.metadata {
            if !first {
                json.push_str(",\n");
            }
            json.push_str(&format!("    \"{}\": \"{}\"", key, value));
            first = false;
        }
        json.push_str("\n  }\n");
        json.push_str("}");

        Ok(json)
    }

    /// Format as CSV
    fn format_csv(&self) -> String {
        let mut csv = String::from("component,type,current_usage,peak_usage,allocations,deallocations,fragmentation_ratio\n");

        // Overall stats
        csv.push_str(&format!(
            "overall,System,{},{},{},{},{:.4}\n",
            self.metrics.current_allocated,
            self.metrics.peak_allocated,
            self.metrics.allocations,
            self.metrics.deallocations,
            self.metrics.fragmentation_ratio()
        ));

        // Component stats
        for comp in &self.components {
            csv.push_str(&format!(
                "{},{:?},{},{},{},{},{:.4}\n",
                comp.name,
                comp.component_type,
                comp.metrics.current_allocated,
                comp.metrics.peak_allocated,
                comp.metrics.allocations,
                comp.metrics.deallocations,
                comp.metrics.fragmentation_ratio()
            ));
        }

        csv
    }

    /// Compare with another snapshot
    pub fn diff(&self, other: &MemorySnapshot) -> SnapshotDiff {
        let metrics_diff = self.metrics.diff(&other.metrics);
        let component_diffs = self.compare_components(other);

        SnapshotDiff {
            from_id: self.id,
            to_id: other.id,
            #[cfg(feature = "std")]
            time_delta: other.timestamp.duration_since(self.timestamp),
            metrics_diff,
            component_diffs,
            system_diff: self.compare_system_info(other),
        }
    }

    /// Compare component snapshots
    fn compare_components(&self, other: &MemorySnapshot) -> Vec<ComponentDiff> {
        let mut diffs = Vec::new();

        // Compare existing components
        for comp in &self.components {
            if let Some(other_comp) = other.get_component(&comp.name) {
                diffs.push(ComponentDiff {
                    name: comp.name.clone(),
                    component_type: comp.component_type,
                    status: ComponentStatus::Modified,
                    metrics_diff: comp.metrics.diff(&other_comp.metrics),
                });
            } else {
                diffs.push(ComponentDiff {
                    name: comp.name.clone(),
                    component_type: comp.component_type,
                    status: ComponentStatus::Removed,
                    metrics_diff: comp.metrics.diff(&MemoryMetrics::default()),
                });
            }
        }

        // Find new components
        for other_comp in &other.components {
            if !self.components.iter().any(|c| c.name == other_comp.name) {
                diffs.push(ComponentDiff {
                    name: other_comp.name.clone(),
                    component_type: other_comp.component_type,
                    status: ComponentStatus::Added,
                    metrics_diff: MemoryMetrics::default().diff(&other_comp.metrics),
                });
            }
        }

        diffs
    }

    /// Compare system information
    fn compare_system_info(&self, other: &MemorySnapshot) -> Option<SystemMemoryDiff> {
        match (&self.system_info, &other.system_info) {
            (Some(self_sys), Some(other_sys)) => Some(SystemMemoryDiff {
                total_memory_delta: other_sys.total_memory as i64 - self_sys.total_memory as i64,
                available_memory_delta: other_sys.available_memory as i64
                    - self_sys.available_memory as i64,
                process_memory_delta: other_sys.process_memory as i64
                    - self_sys.process_memory as i64,
                memory_pressure_delta: other_sys.memory_pressure - self_sys.memory_pressure,
            }),
            _ => None,
        }
    }
}

impl ComponentSnapshot {
    /// Create new component snapshot
    pub fn new(name: String, component_type: ComponentType, metrics: MemoryMetrics) -> Self {
        Self { name, component_type, metrics, details: HashMap::new() }
    }

    /// Add detail
    pub fn add_detail(&mut self, key: String, value: ComponentDetail) {
        self.details.insert(key, value);
    }

    /// Add multiple details
    pub fn add_details(&mut self, details: HashMap<String, ComponentDetail>) {
        self.details.extend(details);
    }
}

impl ComponentDetail {
    /// Format detail for display
    pub fn format(&self) -> String {
        match self {
            Self::Count(n) => n.to_string(),
            Self::Size(s) => utils::format_bytes(*s as usize),
            Self::Percentage(p) => utils::format_percentage(*p),
            Self::Text(t) => t.clone(),
            Self::Flag(b) => {
                if *b {
                    "Yes".to_string()
                } else {
                    "No".to_string()
                }
            },
        }
    }
}

/// Difference between two snapshots
#[derive(Debug, Clone)]
pub struct SnapshotDiff {
    pub from_id: u64,
    pub to_id: u64,
    #[cfg(feature = "std")]
    pub time_delta: std::time::Duration,
    pub metrics_diff: MetricsDiff,
    pub component_diffs: Vec<ComponentDiff>,
    pub system_diff: Option<SystemMemoryDiff>,
}

/// Difference between memory metrics
#[derive(Debug, Clone)]
pub struct MetricsDiff {
    pub current_allocated_delta: i64,
    pub peak_allocated_delta: i64,
    pub allocations_delta: i64,
    pub deallocations_delta: i64,
    pub fragmentation_delta: f64,
}

/// Component difference
#[derive(Debug, Clone)]
pub struct ComponentDiff {
    pub name: String,
    pub component_type: ComponentType,
    pub status: ComponentStatus,
    pub metrics_diff: MetricsDiff,
}

/// Component status in diff
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentStatus {
    Added,
    Removed,
    Modified,
}

/// System memory difference
#[derive(Debug, Clone)]
pub struct SystemMemoryDiff {
    pub total_memory_delta: i64,
    pub available_memory_delta: i64,
    pub process_memory_delta: i64,
    pub memory_pressure_delta: f64,
}

// Extension trait for MemoryMetrics to calculate diffs
trait MetricsDiffExt {
    fn diff(&self, other: &Self) -> MetricsDiff;
}

impl MetricsDiffExt for MemoryMetrics {
    fn diff(&self, other: &MemoryMetrics) -> MetricsDiff {
        MetricsDiff {
            current_allocated_delta: other.current_allocated as i64 - self.current_allocated as i64,
            peak_allocated_delta: other.peak_allocated as i64 - self.peak_allocated as i64,
            allocations_delta: other.allocations as i64 - self.allocations as i64,
            deallocations_delta: other.deallocations as i64 - self.deallocations as i64,
            fragmentation_delta: other.fragmentation_ratio() - self.fragmentation_ratio(),
        }
    }
}

impl SnapshotDiff {
    /// Format diff as human-readable text
    pub fn format(&self) -> String {
        let mut output = format!(
            "Snapshot Diff: #{} → #{}\n\
             ========================\n",
            self.from_id, self.to_id
        );

        #[cfg(feature = "std")]
        {
            output.push_str(&format!("Time Delta: {}\n", utils::format_duration(self.time_delta)));
        }

        // Overall metrics changes
        output.push_str("\nOverall Changes:\n");
        output.push_str(&format!(
            "  Current Usage: {}\n",
            format_bytes_delta(self.metrics_diff.current_allocated_delta)
        ));
        output.push_str(&format!(
            "  Peak Usage: {}\n",
            format_bytes_delta(self.metrics_diff.peak_allocated_delta)
        ));
        output.push_str(&format!("  Allocations: {:+}\n", self.metrics_diff.allocations_delta));
        output.push_str(&format!(
            "  Fragmentation: {:+.2}%\n",
            self.metrics_diff.fragmentation_delta * 100.0
        ));

        // System changes
        if let Some(sys_diff) = &self.system_diff {
            output.push_str("\nSystem Changes:\n");
            output.push_str(&format!(
                "  Available Memory: {}\n",
                format_bytes_delta(sys_diff.available_memory_delta)
            ));
            output.push_str(&format!(
                "  Process Memory: {}\n",
                format_bytes_delta(sys_diff.process_memory_delta)
            ));
            output.push_str(&format!(
                "  Memory Pressure: {:+.2}%\n",
                sys_diff.memory_pressure_delta * 100.0
            ));
        }

        // Component changes
        if !self.component_diffs.is_empty() {
            output.push_str("\nComponent Changes:\n");

            for diff in &self.component_diffs {
                let status_str = match diff.status {
                    ComponentStatus::Added => "+ Added",
                    ComponentStatus::Removed => "- Removed",
                    ComponentStatus::Modified => "* Modified",
                };

                output.push_str(&format!(
                    "  {} {}: current {}, peak {}\n",
                    status_str,
                    diff.name,
                    format_bytes_delta(diff.metrics_diff.current_allocated_delta),
                    format_bytes_delta(diff.metrics_diff.peak_allocated_delta)
                ));
            }
        }

        output
    }

    /// Check if there are significant changes
    pub fn has_significant_changes(&self) -> bool {
        let threshold = 1024; // 1KB threshold for significance

        self.metrics_diff.current_allocated_delta.abs() > threshold
            || self.metrics_diff.allocations_delta.abs() > 100
            || self.metrics_diff.fragmentation_delta.abs() > 0.1
            || !self.component_diffs.is_empty()
    }
}

/// Helper function to format byte deltas
fn format_bytes_delta(delta: i64) -> String {
    let sign = if delta >= 0 { "+" } else { "" };
    format!("{}{}", sign, utils::format_bytes(delta.abs() as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_metrics(current: usize, peak: usize, allocs: u64) -> MemoryMetrics {
        let mut metrics = MemoryMetrics::default();
        metrics.current_allocated = current;
        metrics.peak_allocated = peak;
        metrics.allocations = allocs;
        #[cfg(feature = "std")]
        {
            metrics.timestamp = std::time::Instant::now();
        }
        metrics
    }

    #[test]
    fn test_snapshot_creation() {
        let metrics = create_test_metrics(1000, 1500, 10);
        let mut snapshot = MemorySnapshot::new(1, metrics);

        assert_eq!(snapshot.id, 1);
        assert_eq!(snapshot.metrics.current_allocated, 1000);
        assert!(snapshot.components.is_empty());

        snapshot.add_tag("test".to_string());
        snapshot.add_metadata("version".to_string(), "1.0".to_string());

        assert_eq!(snapshot.tags.len(), 1);
        assert_eq!(snapshot.metadata.len(), 1);
    }

    #[test]
    fn test_component_snapshot() {
        let metrics = create_test_metrics(500, 600, 5);
        let mut comp =
            ComponentSnapshot::new("test_pool".to_string(), ComponentType::Pool, metrics);

        comp.add_detail("capacity".to_string(), ComponentDetail::Count(100));
        comp.add_detail("hit_rate".to_string(), ComponentDetail::Percentage(0.85));

        assert_eq!(comp.name, "test_pool");
        assert_eq!(comp.component_type, ComponentType::Pool);
        assert_eq!(comp.details.len(), 2);
    }

    #[test]
    fn test_snapshot_formatting() {
        let metrics = create_test_metrics(1000, 1500, 10);
        let mut snapshot = MemorySnapshot::new(1, metrics);

        let comp_metrics = create_test_metrics(500, 600, 5);
        let component =
            ComponentSnapshot::new("test_pool".to_string(), ComponentType::Pool, comp_metrics);
        snapshot.add_component(component);

        // Test text format
        let text = snapshot.format(SnapshotFormat::Text).unwrap();
        assert!(text.contains("Memory Snapshot #1"));
        assert!(text.contains("test_pool"));
        assert!(text.contains("Current Allocated: 1000 B"));

        // Test JSON format
        let json = snapshot.format(SnapshotFormat::Json).unwrap();
        assert!(json.contains("\"id\": 1"));
        assert!(json.contains("\"current_allocated\": 1000"));

        // Test CSV format
        let csv = snapshot.format(SnapshotFormat::Csv).unwrap();
        assert!(csv.contains("component,type,current_usage"));
        assert!(csv.contains("overall,System,1000"));
        assert!(csv.contains("test_pool,Pool,500"));
    }

    #[test]
    fn test_snapshot_diff() {
        let metrics1 = create_test_metrics(1000, 1500, 10);
        let mut snapshot1 = MemorySnapshot::new(1, metrics1);

        let comp1 = ComponentSnapshot::new(
            "pool1".to_string(),
            ComponentType::Pool,
            create_test_metrics(500, 600, 5),
        );
        snapshot1.add_component(comp1);

        #[cfg(feature = "std")]
        std::thread::sleep(std::time::Duration::from_millis(10));

        let metrics2 = create_test_metrics(1200, 1600, 15);
        let mut snapshot2 = MemorySnapshot::new(2, metrics2);

        let comp2 = ComponentSnapshot::new(
            "pool1".to_string(),
            ComponentType::Pool,
            create_test_metrics(600, 700, 8),
        );
        let comp3 = ComponentSnapshot::new(
            "pool2".to_string(),
            ComponentType::Pool,
            create_test_metrics(100, 150, 2),
        );
        snapshot2.add_component(comp2);
        snapshot2.add_component(comp3);

        let diff = snapshot1.diff(&snapshot2);

        assert_eq!(diff.from_id, 1);
        assert_eq!(diff.to_id, 2);
        assert_eq!(diff.metrics_diff.current_allocated_delta, 200); // 1200 - 1000
        assert_eq!(diff.metrics_diff.allocations_delta, 5); // 15 - 10

        // Component diffs: pool1 modified, pool2 added
        assert_eq!(diff.component_diffs.len(), 2);

        let pool1_diff = diff.component_diffs.iter().find(|d| d.name == "pool1").unwrap();
        assert_eq!(pool1_diff.status, ComponentStatus::Modified);
        assert_eq!(pool1_diff.metrics_diff.current_allocated_delta, 100); // 600 - 500

        let pool2_diff = diff.component_diffs.iter().find(|d| d.name == "pool2").unwrap();
        assert_eq!(pool2_diff.status, ComponentStatus::Added);
        assert_eq!(pool2_diff.metrics_diff.current_allocated_delta, 100); // 100
                                                                          // - 0
    }

    #[test]
    fn test_system_memory_info() {
        let metrics = create_test_metrics(1000, 1500, 10);
        let mut snapshot = MemorySnapshot::new(1, metrics);

        let sys_info = SystemMemoryInfo {
            total_memory: 16 * 1024 * 1024 * 1024,    // 16GB
            available_memory: 8 * 1024 * 1024 * 1024, // 8GB
            process_memory: 1 * 1024 * 1024 * 1024,   // 1GB
            swap_used: 0,
            swap_total: 2 * 1024 * 1024 * 1024, // 2GB
            memory_pressure: 0.3,
        };

        snapshot.set_system_info(sys_info);

        let text = snapshot.format(SnapshotFormat::Text).unwrap();
        assert!(text.contains("System Memory:"));
        assert!(text.contains("Total: 16.00 GB"));
        assert!(text.contains("Process: 1.00 GB (6.2%)"));
        assert!(text.contains("Memory Pressure: 30.0%"));
    }

    #[test]
    fn test_component_details_formatting() {
        let details = [
            ComponentDetail::Count(42),
            ComponentDetail::Size(1024),
            ComponentDetail::Percentage(0.75),
            ComponentDetail::Text("active".to_string()),
            ComponentDetail::Flag(true),
        ];

        assert_eq!(details[0].format(), "42");
        assert_eq!(details[1].format(), "1.00 KB");
        assert_eq!(details[2].format(), "75.0%");
        assert_eq!(details[3].format(), "active");
        assert_eq!(details[4].format(), "Yes");
    }

    #[test]
    fn test_diff_significance() {
        let metrics1 = create_test_metrics(1000, 1500, 10);
        let snapshot1 = MemorySnapshot::new(1, metrics1);

        // Small change - not significant
        let metrics2 = create_test_metrics(1100, 1500, 12);
        let snapshot2 = MemorySnapshot::new(2, metrics2);
        let diff = snapshot1.diff(&snapshot2);
        assert!(!diff.has_significant_changes());

        // Large change - significant
        let metrics3 = create_test_metrics(5000, 6000, 100);
        let snapshot3 = MemorySnapshot::new(3, metrics3);
        let diff = snapshot1.diff(&snapshot3);
        assert!(diff.has_significant_changes());
    }
}
