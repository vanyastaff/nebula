//! Simple example showing basic usage

use nebula_log::{debug, error, info, trace, warn, Logger, Timer, timed, Format};
use std::{thread, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger for development
    Logger::new().format(Format::Json).init()?;


    info!("🚀 Nebula Log Example Started");

    // Basic logging
    trace!("This is a trace message");
    debug!("Debug information with value: {}", 42);
    info!(user_id = "user123", action = "login", "User logged in successfully");
    warn!("This is a warning message");
    error!("This is an error message");

    // Timer examples
    info!("📊 Timer Examples");

    // Manual timer
    let timer = Timer::new("manual_operation");
    thread::sleep(Duration::from_millis(50));
    timer.checkpoint("halfway");
    thread::sleep(Duration::from_millis(30));
    timer.finish();

    // Macro timer
    let result = timed!("macro_operation", {
        thread::sleep(Duration::from_millis(25));
        calculate_something(10, 20)
    });
    info!("Result: {}", result);

    // Different timing categories
    demonstrate_timing_categories();

    info!("✅ Example completed");
    Ok(())
}

fn calculate_something(a: i32, b: i32) -> i32 {
    debug!("Calculating {} + {}", a, b);
    a + b
}

fn demonstrate_timing_categories() {
    info!("🎯 Timing Categories Demo");

    // Very fast (⚡ - green)
    timed!("very_fast", {
        thread::sleep(Duration::from_millis(5));
    });

    // Fast (🏃 - cyan)
    timed!("fast", {
        thread::sleep(Duration::from_millis(50));
    });

    // Medium (🚶 - yellow)
    timed!("medium", {
        thread::sleep(Duration::from_millis(500));
    });

    // Slow (🐌 - red)
    timed!("slow", {
        thread::sleep(Duration::from_millis(1200));
    });
}