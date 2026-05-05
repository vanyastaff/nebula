#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![allow(unsafe_code)] // CPU affinity on Linux requires unsafe
//! # nebula-system
//!
//! Cross-platform host probes for the Nebula ecosystem.
//!
//! ## Purpose
//!
//! The engine needs to make scheduling decisions and surface operator-visible
//! health signals based on host resource pressure. This crate provides a
//! unified interface for host probes — CPU, memory, disk, network, process —
//! with pressure classifiers that return explicit evidence rather than hiding
//! unsupported or stale probe data behind default values.
//! Does not emit metrics; recording from system data is the caller's job
//! (typically via `nebula-metrics`). See `crates/system/README.md` for
//! the full role description and known platform limitations.
//!
//! ## Role
//!
//! **Host Probes** — cross-cutting infrastructure; `#[allow(unsafe_code)]`
//! is intentional (CPU affinity on Linux requires unsafe).
//!
//! ## Public API
//!
//! - `init() -> SystemResult<()>` — one-time initialization at process startup.
//! - `SystemInfo::get() -> Arc<SystemInfo>` — cached CPU, memory, OS, and hardware snapshot.
//! - `memory::current()`, `memory::pressure() -> MemoryPressure` — memory stats and pressure.
//! - `MemoryPressure` — `Low` / `Medium` / `High` / `Critical` / `Unavailable`.
//! - `cpu::usage()` — CPU stats plus sampling freshness (feature: `sysinfo`).
//! - `SystemError`, `SystemResult<T>` — typed error and result alias.
//! - Optional modules (feature-gated): `process`, `network`, `disk`, `load`.
//!
//! ## Features
//!
//! - `sysinfo` (default): System information gathering via sysinfo crate
//! - `process`: Process information and monitoring
//! - `network`: Network interface statistics
//! - `disk`: Disk and filesystem information
//! - `serde`: Serialization support for data types; intentionally separate from `full`
//!
//! ## Platform Support Matrix
//!
//! | Module    | Linux | macOS | Windows | Notes                                          |
//! |-----------|-------|-------|---------|-------------------------------------------------|
//! | `memory`  | ✓     | ✓     | ✓       | Via `sysinfo`                                   |
//! | `cpu`     | ✓     | ✓     | ✓       | SSE/AVX feature detection x86 only              |
//! | `disk`    | ✓     | ✓     | ✓       | I/O counters Linux-only (`io_stats()`)          |
//! | `network` | ✓     | ✓     | ✓       | Unsupported metadata uses `Availability<T>`     |
//! | `process` | ✓     | ✓     | ✓       | Partial metadata uses `Availability<T>`         |
//!
//! ## Example
//!
//! ```no_run
//! use nebula_system::SystemInfo;
//!
//! fn main() -> nebula_system::SystemResult<()> {
//!     nebula_system::init()?;
//!
//!     let info = SystemInfo::get();
//!     println!("CPU: {} cores", info.cpu.cores);
//!     println!(
//!         "Memory: {} GB",
//!         info.memory.effective.total / (1024 * 1024 * 1024)
//!     );
//!
//!     let pressure = nebula_system::memory::pressure();
//!     if pressure.is_concerning() {
//!         println!("Warning: Memory pressure is high!");
//!     }
//!
//!     Ok(())
//! }
//! ```
pub mod availability;
pub mod error;
pub mod info;
pub mod prelude;
pub mod result;
pub mod utils;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod memory;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod cpu;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod load;

#[cfg(feature = "process")]
#[cfg_attr(docsrs, doc(cfg(feature = "process")))]
pub mod process;

#[cfg(feature = "network")]
#[cfg_attr(docsrs, doc(cfg(feature = "network")))]
pub mod network;

#[cfg(feature = "disk")]
#[cfg_attr(docsrs, doc(cfg(feature = "disk")))]
pub mod disk;

// Re-exports
pub use availability::{Availability, AvailabilityStatus};
pub use error::{SystemError, SystemResult};
pub use info::SystemInfo;
#[cfg(feature = "sysinfo")]
pub use memory::{MemoryInfo, MemoryPressure};
pub use result::SystemResultExt;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the system information subsystem
///
/// This should be called once at program startup to initialize
/// caches and prepare the system information gathering.
pub fn init() -> SystemResult<()> {
    info::init()
}

/// Get a formatted summary of system information
#[must_use]
pub fn summary() -> String {
    info::summary()
}
