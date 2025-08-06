//! Simple timer for measuring execution time

use std::time::Instant;

/// Simple timer for measuring and logging execution time
pub struct Timer {
    name: String,
    start: Instant,
}

impl Timer {
    /// Create a new timer and start measuring
    pub fn new(name: &str) -> Self {
        tracing::debug!("⏱️ Starting timer: {}", name);
        Self {
            name: name.to_string(),
            start: Instant::now(),
        }
    }

    /// Start a new timer with a span
    pub fn start(name: &str) -> Self {
        let timer = Self::new(name);
        tracing::span!(tracing::Level::DEBUG, "timer", name = name);
        timer
    }

    /// Get elapsed time without finishing the timer
    pub fn elapsed(&self) -> core::time::Duration {
        self.start.elapsed()
    }

    /// Get elapsed time in milliseconds
    pub fn elapsed_ms(&self) -> u128 {
        self.elapsed().as_millis()
    }

    /// Log a checkpoint with elapsed time
    pub fn checkpoint(&self, message: &str) {
        tracing::debug!("🏁 {} ({}): {}ms", self.name, message, self.elapsed_ms());
    }

    /// Finish the timer and log the total duration
    pub fn finish(self) {
        let duration = self.elapsed();
        let ms = duration.as_millis();

        let emoji = match ms {
            0..=10 => "⚡",      // Very fast
            11..=100 => "🏃",    // Fast
            101..=1000 => "🚶",  // Medium
            _ => "🐌",           // Slow
        };

        // Use appropriate logging level based on duration
        match ms {
            0..=10 => tracing::debug!(
                duration_ms = ms as u64,
                "{} Timer '{}' finished in {}ms", emoji, self.name, ms
            ),
            11..=100 => tracing::debug!(
                duration_ms = ms as u64,
                "{} Timer '{}' finished in {}ms", emoji, self.name, ms
            ),
            101..=1000 => tracing::info!(
                duration_ms = ms as u64,
                "{} Timer '{}' finished in {}ms", emoji, self.name, ms
            ),
            _ => tracing::warn!(
                duration_ms = ms as u64,
                "{} Timer '{}' finished in {}ms", emoji, self.name, ms
            ),
        }
    }

    /// Get the timer name
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        // Auto-finish if not manually finished
        let duration = self.elapsed();
        let ms = duration.as_millis();
        tracing::debug!("Timer '{}' dropped after {}ms", self.name, ms);
    }
}

/// Time a block of code
#[macro_export]
macro_rules! timed {
    ($name:expr, $block:block) => {
        {
            let _timer = $crate::Timer::new($name);
            let result = $block;
            _timer.finish();
            result
        }
    };
}