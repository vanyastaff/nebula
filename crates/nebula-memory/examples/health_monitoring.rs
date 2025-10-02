//! Pool health monitoring example - demonstrates leak detection and health checks

use nebula_memory::pool::{
    HealthConfig, ObjectPool, PoolConfig, PoolHealth, PoolHealthMonitor, Poolable,
};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
struct Connection {
    id: u64,
    url: String,
    active: bool,
}

impl Connection {
    fn new(id: u64) -> Self {
        Self {
            id,
            url: format!("https://api.example.com/v1"),
            active: true,
        }
    }

    fn close(&mut self) {
        self.active = false;
    }
}

impl Poolable for Connection {
    fn reset(&mut self) {
        self.active = true;
    }

    fn memory_usage(&self) -> usize {
        std::mem::size_of::<Self>() + self.url.capacity()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Health Monitoring Example ===\n");

    // Example 1: Basic health monitoring
    println!("1. Basic Health Monitoring:");
    {
        let config = HealthConfig {
            max_failure_rate: 0.2, // 20% failures tolerated
            max_leak_rate: 0.1,    // 10% leaks tolerated
            min_utilization: 0.1,
            max_utilization: 0.9,
            rate_window: Duration::from_secs(60),
        };

        let monitor = PoolHealthMonitor::new(config, 10);

        // Simulate some operations
        for _ in 0..10 {
            monitor.record_checkout();
            monitor.record_return();
        }

        monitor.update_available(8);
        monitor.update_capacity(10);

        let health = monitor.check_health();
        println!("   Pool health: {:?}", health);
        println!(
            "   Status: {}",
            if matches!(health, PoolHealth::Healthy) {
                "✓ Healthy"
            } else {
                "⚠ Degraded or Critical"
            }
        );
    }

    println!();

    // Example 2: Leak detection
    println!("2. Leak Detection:");
    {
        let monitor = PoolHealthMonitor::new(HealthConfig::default(), 100);

        // Simulate checkouts and returns
        for _ in 0..20 {
            monitor.record_checkout();
        }

        for _ in 0..15 {
            monitor.record_return();
        }

        // 5 objects not returned - potential leak!
        let leak_report = monitor.detect_leaks();

        println!("   Total checkouts: {}", leak_report.total_checkouts);
        println!("   Total returns: {}", leak_report.total_returns);
        println!("   Potential leaks: {}", leak_report.potential_leaks);
        println!("   Leak rate: {:.1}%", leak_report.leak_rate * 100.0);

        if leak_report.has_leaks() {
            println!(
                "   ⚠ Warning: {} objects may have leaked!",
                leak_report.total_leaks()
            );
        }
    }

    println!();

    // Example 3: High failure rate detection
    println!("3. High Failure Rate Detection:");
    {
        let config = HealthConfig {
            max_failure_rate: 0.1, // Only 10% failures allowed
            ..Default::default()
        };

        let monitor = PoolHealthMonitor::new(config, 50);

        // Simulate operations with high failure rate
        for i in 0..100 {
            monitor.record_checkout();

            if i % 5 == 0 {
                // Every 5th checkout fails
                monitor.record_failure();
            } else {
                monitor.record_return();
            }
        }

        let health = monitor.check_health();
        let metrics = monitor.metrics();

        println!("   Total checkouts: {}", metrics.total_checkouts);
        println!("   Total failures: {}", metrics.total_failures);
        println!("   Failure rate: {:.1}%", metrics.failure_rate() * 100.0);
        println!("   Health status: {:?}", health);

        if matches!(health, PoolHealth::Critical) {
            println!("   ⚠ CRITICAL: Failure rate exceeds threshold!");
        }
    }

    println!();

    // Example 4: Pool utilization monitoring
    println!("4. Pool Utilization Monitoring:");
    {
        let monitor = PoolHealthMonitor::new(HealthConfig::default(), 100);

        // Low utilization
        monitor.update_available(95);
        monitor.update_capacity(100);

        let metrics = monitor.metrics();
        println!("   Capacity: {}", metrics.pool_capacity);
        println!("   Available: {}", metrics.available_objects);
        println!("   Utilization: {:.1}%", metrics.utilization() * 100.0);

        if metrics.utilization() < 0.2 {
            println!("   💡 Tip: Pool is underutilized, consider reducing capacity");
        }

        // High utilization
        monitor.update_available(5);
        let metrics2 = monitor.metrics();
        println!("\n   After heavy usage:");
        println!("   Available: {}", metrics2.available_objects);
        println!("   Utilization: {:.1}%", metrics2.utilization() * 100.0);

        if metrics2.utilization() > 0.9 {
            println!("   ⚠ Warning: Pool is nearly exhausted, consider increasing capacity");
        }
    }

    println!();

    // Example 5: Real-world connection pool monitoring
    println!("5. Real-World Connection Pool:");
    {
        let config = PoolConfig {
            initial_capacity: 10,
            max_capacity: Some(20),
            validate_on_return: true,
            ..Default::default()
        };

        let pool = ObjectPool::new(10, || Connection::new(1));
        let monitor = PoolHealthMonitor::new(HealthConfig::default(), 10);

        println!("   Simulating connection pool usage...");

        // Simulate normal operations
        for i in 0..50 {
            monitor.record_checkout();

            // Occasional connection failures
            if i % 15 == 0 {
                monitor.record_failure();
                println!("   Connection failed (#{i})");
            } else {
                monitor.record_return();
            }

            // Simulate some leaked connections
            if i == 25 {
                monitor.record_leak();
                println!("   Connection leaked (forgot to return)");
            }
        }

        monitor.update_available(7);

        let health = monitor.check_health();
        let metrics = monitor.metrics();
        let leaks = monitor.detect_leaks();

        println!("\n   === Pool Health Report ===");
        println!("   Health Status: {:?}", health);
        println!("   Total Checkouts: {}", metrics.total_checkouts);
        println!("   Total Returns: {}", metrics.total_returns);
        println!("   Total Failures: {}", metrics.total_failures);
        println!("   Failure Rate: {:.1}%", metrics.failure_rate() * 100.0);
        println!("   Utilization: {:.1}%", metrics.utilization() * 100.0);
        println!("   Potential Leaks: {}", leaks.potential_leaks);
        println!("   Known Leaks: {}", leaks.known_leaks);

        if !metrics.is_healthy() {
            println!("\n   ⚠ Action Required: Pool health is degraded!");
        } else {
            println!("\n   ✓ Pool is healthy");
        }
    }

    println!("\n=== Health monitoring example completed successfully! ===");
    Ok(())
}
