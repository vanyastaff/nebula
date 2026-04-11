//! System load aggregation for adaptive worker scaling.
//!
//! Combines CPU and memory pressure into a single [`SystemLoad`] snapshot
//! that runtime/engine components can poll to decide whether to accept
//! more work or shed load.
//!
//! # Example
//!
//! ```no_run
//! use nebula_system::load::system_load;
//!
//! let load = system_load();
//! if load.can_accept_work() {
//!     println!("headroom: {:.0}%", load.headroom() * 100.0);
//!     // spawn another worker
//! } else {
//!     println!("system under pressure — shedding load");
//! }
//! ```

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{cpu::CpuPressure, info::SYSINFO_SYSTEM, memory::MemoryPressure};

/// Aggregated system load snapshot.
///
/// Designed for adaptive worker scaling: poll periodically,
/// use [`can_accept_work`](Self::can_accept_work) to decide whether to spawn
/// more workers, or [`headroom`](Self::headroom) for proportional scaling.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SystemLoad {
    /// CPU pressure level
    pub cpu: CpuPressure,
    /// Memory pressure level
    pub memory: MemoryPressure,
    /// CPU usage percentage (0–100)
    pub cpu_usage_percent: f32,
    /// Memory usage percentage (0–100)
    pub memory_usage_percent: f64,
}

impl SystemLoad {
    /// Quick check: is the system healthy enough to accept more work?
    ///
    /// Returns `false` when CPU **or** memory pressure is High or Critical.
    #[must_use]
    pub fn can_accept_work(&self) -> bool {
        !self.cpu.is_concerning() && !self.memory.is_concerning()
    }

    /// How much headroom is available, as a fraction in `[0.0, 1.0]`.
    ///
    /// `1.0` = fully idle, `0.0` = at capacity. Takes the minimum of
    /// CPU and memory headroom so a bottleneck in either dimension
    /// is reflected.
    #[must_use]
    pub fn headroom(&self) -> f64 {
        let cpu_headroom = (100.0 - self.cpu_usage_percent as f64) / 100.0;
        let mem_headroom = (100.0 - self.memory_usage_percent) / 100.0;
        cpu_headroom.min(mem_headroom).clamp(0.0, 1.0)
    }
}

/// Get current system load (CPU + memory combined).
///
/// Acquires the sysinfo write lock once and refreshes both CPU and memory
/// in a single critical section. Avoid calling more often than every 100ms
/// in production.
#[must_use]
pub fn system_load() -> SystemLoad {
    let mut sys = SYSINFO_SYSTEM.write();
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    // CPU average — single pass over cpus slice
    let cpus = sys.cpus();
    let cpu_avg = if cpus.is_empty() {
        0.0f32
    } else {
        cpus.iter().map(sysinfo::Cpu::cpu_usage).sum::<f32>() / cpus.len() as f32
    };

    // Memory stats
    let total = sys.total_memory() as usize;
    let available = sys.available_memory() as usize;
    let used = total.saturating_sub(available);

    // Drop the lock before computing derived values
    drop(sys);

    let memory_usage_percent = if total > 0 {
        used.checked_mul(10000)
            .and_then(|v| v.checked_div(total))
            .map_or_else(
                || (used as f64 / total as f64) * 100.0,
                |v| v as f64 / 100.0,
            )
    } else {
        0.0
    };

    let memory_pressure = if memory_usage_percent > 85.0 {
        MemoryPressure::Critical
    } else if memory_usage_percent > 70.0 {
        MemoryPressure::High
    } else if memory_usage_percent > 50.0 {
        MemoryPressure::Medium
    } else {
        MemoryPressure::Low
    };

    SystemLoad {
        cpu: CpuPressure::from_usage(cpu_avg),
        memory: memory_pressure,
        cpu_usage_percent: cpu_avg,
        memory_usage_percent,
    }
}
