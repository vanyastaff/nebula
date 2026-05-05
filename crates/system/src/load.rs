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
//! match load.can_accept_work().value().copied() {
//!     Some(true) => {
//!         if let Some(headroom) = load.headroom().value() {
//!             println!("headroom: {:.0}%", headroom * 100.0);
//!         }
//!         // spawn another worker
//!     },
//!     Some(false) => println!("system under pressure - shedding load"),
//!     None => println!(
//!         "load signal unavailable: {:?}",
//!         load.can_accept_work().status()
//!     ),
//! }
//! ```

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{
    availability::{
        Availability, AvailabilityStatus, AvailabilityStatusMessages, availability_from_status,
    },
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
    /// Returns `Availability::available(false)` when CPU **or** memory pressure
    /// is High or Critical. Returns `NotSampled`, `Stale`, `Unsupported`, or
    /// `Unavailable` instead of collapsing missing probe evidence into `false`.
    #[must_use]
    pub fn can_accept_work(&self) -> Availability<bool> {
        let decision = !self.cpu.is_concerning() && !self.memory.is_concerning();

        if !decision {
            return Availability::available(false);
        }

        let cpu_status = self.cpu_usage_percent.status();
        let memory_status = self.memory_usage_percent.status();

        if cpu_status == AvailabilityStatus::Available
            && memory_status == AvailabilityStatus::Available
        {
            return Availability::available(true);
        }

        if cpu_status == AvailabilityStatus::Stale || memory_status == AvailabilityStatus::Stale {
            return Availability::stale(
                Some(true),
                "work admission decision is based on stale CPU or memory usage",
            );
        }

        if cpu_status == AvailabilityStatus::NotSampled
            || memory_status == AvailabilityStatus::NotSampled
        {
            return Availability::not_sampled(
                "work admission requires CPU and memory samples to warm up",
            );
        }

        if cpu_status == AvailabilityStatus::PermissionDenied
            || memory_status == AvailabilityStatus::PermissionDenied
        {
            return Availability::permission_denied(
                "work admission requires CPU and memory probes",
            );
        }

        if cpu_status == AvailabilityStatus::Unsupported
            || memory_status == AvailabilityStatus::Unsupported
        {
            return Availability::unsupported(
                "work admission requires supported CPU and memory probes",
            );
        }

        if cpu_status == AvailabilityStatus::NotImplemented
            || memory_status == AvailabilityStatus::NotImplemented
        {
            return Availability::not_implemented(
                "work admission requires implemented CPU and memory probes",
            );
        }

        Availability::unavailable("work admission requires available CPU and memory probes")
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

    let cpu_usage_percent = availability_from_status(
        cpu_usage.sample_status,
        cpu_usage.average,
        Some(cpu_usage.average),
        AvailabilityStatusMessages {
            not_sampled: "first CPU sample has no previous backend refresh",
            stale: "CPU sample refreshed before backend minimum interval",
            unsupported: "CPU usage sampling is unsupported",
            unavailable: "CPU usage is unavailable",
            permission_denied: "CPU usage probe was denied",
            not_implemented: "CPU usage sampling is not implemented",
        },
    );

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
