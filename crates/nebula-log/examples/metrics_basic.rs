//! Basic metrics example showing counter, gauge, and histogram usage
//!
//! This example demonstrates:
//! - Setting up a Prometheus metrics exporter
//! - Using counter, gauge, and histogram
//! - Adding labels to metrics
//! - Accessing metrics via HTTP endpoint
//!
//! Run with: cargo run --example metrics_basic --features observability
//! Then visit: http://localhost:9000/metrics

use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula-Log Metrics Basic Example ===\n");

    // Setup Prometheus exporter on port 9000
    println!("Setting up Prometheus exporter on http://localhost:9000/metrics");
    let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
    let handle = builder
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    println!("Metrics endpoint ready!\n");

    // Example 1: Counter - monotonically increasing values
    println!("1. Counter Example:");
    println!("   Counting requests...");

    for i in 1..=5 {
        metrics::counter!("nebula.example.requests_total", 1);
        println!("   Request #{} counted", i);
        std::thread::sleep(Duration::from_millis(100));
    }

    // Example 2: Counter with labels
    println!("\n2. Counter with Labels:");
    metrics::counter!("nebula.example.http_requests_total", 1,
        "method" => "GET",
        "status" => "200"
    );
    metrics::counter!("nebula.example.http_requests_total", 1,
        "method" => "POST",
        "status" => "201"
    );
    metrics::counter!("nebula.example.http_requests_total", 1,
        "method" => "GET",
        "status" => "404"
    );
    println!("   HTTP requests recorded with method and status labels");

    // Example 3: Gauge - point-in-time values
    println!("\n3. Gauge Example:");
    println!("   Simulating memory usage...");

    for memory_mb in [100.0, 150.0, 200.0, 180.0, 160.0] {
        metrics::gauge!("nebula.example.memory_bytes", memory_mb * 1024.0 * 1024.0);
        println!("   Memory: {:.0} MB", memory_mb);
        std::thread::sleep(Duration::from_millis(100));
    }

    // Example 4: Histogram - distribution of values
    println!("\n4. Histogram Example:");
    println!("   Recording request durations...");

    let durations = [0.12, 0.25, 0.15, 0.42, 0.18, 0.35, 0.22, 0.50, 0.14, 0.28];
    for (i, &duration) in durations.iter().enumerate() {
        metrics::histogram!("nebula.example.request_duration_seconds", duration);
        println!("   Request #{}: {:.2}s", i + 1, duration);
        std::thread::sleep(Duration::from_millis(50));
    }

    // Example 5: Using describe_* for documentation
    println!("\n5. Metric Descriptions:");
    metrics::describe_counter!(
        "nebula.example.requests_total",
        "Total number of requests processed"
    );
    metrics::describe_gauge!(
        "nebula.example.memory_bytes",
        "Current memory usage in bytes"
    );
    metrics::describe_histogram!(
        "nebula.example.request_duration_seconds",
        metrics::Unit::Seconds,
        "Request processing duration"
    );
    println!("   Metric descriptions added (visible in Prometheus)");

    // Example 6: Custom business metrics
    println!("\n6. Business Metrics:");

    // Simulating order processing
    for i in 1..=3 {
        metrics::counter!("nebula.example.orders_total", 1, "status" => "completed");
        metrics::gauge!("nebula.example.order_value_usd", 125.50 * i as f64);
        metrics::histogram!("nebula.example.order_processing_seconds", 0.15 * i as f64);
        println!("   Order #{} processed: ${:.2}", i, 125.50 * i as f64);
        std::thread::sleep(Duration::from_millis(100));
    }

    // Export current metrics
    println!("\n7. Exporting Metrics:");
    let metrics_text = handle.render();
    println!("   Exported {} lines of Prometheus metrics", metrics_text.lines().count());

    println!("\n=== Example Complete! ===");
    println!("\nMetrics are available at: http://localhost:9000/metrics");
    println!("Press Ctrl+C to exit...\n");

    // Keep the exporter alive
    println!("Sample metrics output:");
    println!("{}", "-".repeat(80));
    for (i, line) in metrics_text.lines().take(20).enumerate() {
        println!("{}", line);
        if i >= 19 {
            println!("... ({} more lines)", metrics_text.lines().count() - 20);
            break;
        }
    }
    println!("{}", "-".repeat(80));

    // In a real application, this would keep running
    println!("\nTo see all metrics, run:");
    println!("  curl http://localhost:9000/metrics");

    Ok(())
}
