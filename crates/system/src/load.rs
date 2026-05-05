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
//!     if let Some(headroom) = load.headroom().value() {
//!         println!("headroom: {:.0}%", headroom * 100.0);
//!     }
//!     // spawn another worker
//! } else {
//!     println!("system under pressure - shedding load");
//! }
//! ```

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{
    availability::{Availability, AvailabilityStatus},
    cpu::{self, CpuPressure},
    info::MemoryCapacitySource,
    memory::{self, MemoryPressure},
};

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
    pub cpu_usage_percent: Availability<f32>,
    /// Memory usage percentage (0–100)
    pub memory_usage_percent: Availability<f64>,
    /// CPU sampling status backing `cpu_usage_percent`.
    pub cpu_sample_status: AvailabilityStatus,
    /// Effective memory capacity source backing `memory_usage_percent`.
    pub memory_capacity_source: MemoryCapacitySource,
}

impl SystemLoad {
    /// Quick check: is the system healthy enough to accept more work?
    ///
    /// Returns `false` when CPU **or** memory pressure is High or Critical.
    #[must_use]
    pub fn can_accept_work(&self) -> bool {
        self.cpu_usage_percent.is_available()
            && self.memory_usage_percent.is_available()
            && !self.cpu.is_concerning()
            && !self.memory.is_concerning()
    }

    /// How much headroom is available, as a fraction in `[0.0, 1.0]`.
    ///
    /// `1.0` = fully idle, `0.0` = at capacity. Takes the minimum of
    /// CPU and memory headroom so a bottleneck in either dimension
    /// is reflected.
    #[must_use]
    pub fn headroom(&self) -> Availability<f64> {
        match (
            self.cpu_usage_percent.value().copied(),
            self.memory_usage_percent.value().copied(),
        ) {
            (Some(cpu_usage), Some(memory_usage)) => {
                let cpu_headroom = (100.0 - cpu_usage as f64) / 100.0;
                let mem_headroom = (100.0 - memory_usage) / 100.0;
                Availability::available(cpu_headroom.min(mem_headroom).clamp(0.0, 1.0))
            },
            _ => Availability::unavailable(
                "headroom requires available CPU and memory usage samples",
            ),
        }
    }
}

/// Get current system load (CPU + memory combined).
///
/// CPU usage is sampler-backed and memory usage is based on the effective
/// memory capacity reported by `memory::pressure_report()`. Avoid calling this
/// directly in tight scheduling loops; prefer a caller-owned refresh policy.
#[must_use]
pub fn system_load() -> SystemLoad {
    let cpu_usage = cpu::usage();
    let memory_report = memory::pressure_report();

    let cpu_usage_percent = match cpu_usage.sample_status {
        AvailabilityStatus::Available => Availability::available(cpu_usage.average),
        AvailabilityStatus::NotSampled => {
            Availability::not_sampled("first CPU sample has no previous backend refresh")
        },
        AvailabilityStatus::Stale => Availability::stale(
            Some(cpu_usage.average),
            "CPU sample refreshed before backend minimum interval",
        ),
        AvailabilityStatus::Unsupported => {
            Availability::unsupported("CPU usage sampling is unsupported")
        },
        AvailabilityStatus::Unavailable => Availability::unavailable("CPU usage is unavailable"),
        AvailabilityStatus::PermissionDenied => {
            Availability::permission_denied("CPU usage probe was denied")
        },
        AvailabilityStatus::NotImplemented => {
            Availability::not_implemented("CPU usage sampling is not implemented")
        },
    };

    let cpu_sample_status = cpu_usage.sample_status;

    SystemLoad {
        cpu: CpuPressure::from_usage(cpu_usage.average),
        memory: memory_report.level,
        cpu_usage_percent,
        memory_usage_percent: memory_report.usage_percent,
        cpu_sample_status,
        memory_capacity_source: memory_report.capacity_source,
    }
}
