//! Statistics export example - demonstrates exporting stats in various formats

#[cfg(feature = "stats")]
use nebula_memory::stats::{GlobalStats, MemoryStats, StatsExporter, ExportFormat};

#[cfg(feature = "stats")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Stats Export Example ===\n");

    // Create some sample statistics
    let mut stats = MemoryStats::default();

    // Simulate some allocations
    for i in 0..100 {
        stats.record_allocation(1024 * (i % 10 + 1), true);
    }

    // Simulate some deallocations
    for i in 0..50 {
        stats.record_deallocation(1024 * (i % 10 + 1));
    }

    // Simulate some failures
    for _ in 0..5 {
        stats.record_allocation(1024 * 1024, false);
    }

    // Create global stats from memory stats
    let global_stats = GlobalStats::from_memory_stats(&stats);

    println!("1. JSON Export:");
    println!("{}", "=".repeat(60));
    let json = global_stats.export(ExportFormat::Json);
    println!("{}", json);

    println!("\n2. Prometheus Export:");
    println!("{}", "=".repeat(60));
    let prometheus = global_stats.export(ExportFormat::Prometheus);
    println!("{}", prometheus);

    println!("\n3. Plain Text Export:");
    println!("{}", "=".repeat(60));
    let text = global_stats.export(ExportFormat::PlainText);
    println!("{}", text);

    println!("\n4. CSV Export:");
    println!("{}", "=".repeat(60));
    let csv = global_stats.export(ExportFormat::Csv);
    println!("{}", csv);

    // Alternative: use convenience methods
    println!("\n5. Using Convenience Methods:");
    println!("{}", "=".repeat(60));

    println!("JSON (via to_json()):");
    println!("{}", global_stats.to_json());

    println!("\nPrometheus (via to_prometheus()):");
    let prom_lines: Vec<&str> = global_stats.to_prometheus()
        .lines()
        .take(5)
        .collect();
    for line in prom_lines {
        println!("{}", line);
    }
    println!("...");

    // Export MemoryMetrics directly
    println!("\n6. Exporting MemoryMetrics:");
    println!("{}", "=".repeat(60));
    let metrics = stats.metrics();
    println!("JSON:");
    println!("{}", metrics.to_json());

    // Demonstrate health status reporting
    println!("\n7. Health Status:");
    println!("{}", "=".repeat(60));
    println!("Total Allocated: {} bytes", global_stats.total_allocated);
    println!("Peak Allocated: {} bytes", global_stats.peak_allocated);
    println!("Utilization: {:.1}%", global_stats.utilization_percent());

    if global_stats.is_critical() {
        println!("Status: ⚠ CRITICAL - Memory usage above 90%");
    } else if global_stats.is_high() {
        println!("Status: ⚠ HIGH - Memory usage above 75%");
    } else {
        println!("Status: ✓ NORMAL");
    }

    println!("\n8. Integration Example - Save to File:");
    println!("{}", "=".repeat(60));

    // Example: Save JSON to file
    use std::fs;
    let json_output = global_stats.to_json();
    fs::write("target/memory_stats.json", &json_output)?;
    println!("✓ Saved JSON to: target/memory_stats.json");

    // Example: Save Prometheus metrics to file
    let prom_output = global_stats.to_prometheus();
    fs::write("target/memory_metrics.prom", &prom_output)?;
    println!("✓ Saved Prometheus metrics to: target/memory_metrics.prom");

    // Example: Save CSV to file
    let csv_output = global_stats.to_csv();
    fs::write("target/memory_stats.csv", &csv_output)?;
    println!("✓ Saved CSV to: target/memory_stats.csv");

    println!("\n=== Stats export example completed successfully! ===");
    println!("Files created in target/ directory:");
    println!("  - memory_stats.json");
    println!("  - memory_metrics.prom");
    println!("  - memory_stats.csv");

    Ok(())
}

#[cfg(not(feature = "stats"))]
fn main() {
    println!("This example requires the 'stats' feature to be enabled.");
    println!("Run with: cargo run --example stats_export --features stats");
}
