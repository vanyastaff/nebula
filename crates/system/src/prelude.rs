//! Prelude module for convenient imports
//!
//! # Example
//!
//! ```rust
//! use nebula_system::prelude::*;
//!
//! fn main() -> SystemResult<()> {
//!     let info = SystemInfo::get();
//!     println!("CPU: {} cores", info.cpu.cores);
//!     Ok(())
//! }
//! ```

// Core types
pub use crate::core::{SystemError, SystemResult, SystemResultExt};
pub use crate::info::{CpuInfo, HardwareInfo, OsInfo, SystemInfo};

// Memory types
#[cfg(feature = "sysinfo")]
pub use crate::memory::{MemoryInfo, MemoryPressure};

// CPU types
#[cfg(feature = "sysinfo")]
pub use crate::cpu::{CacheInfo, CpuFeatures, CpuPressure, CpuTopology, CpuUsage};

// Load types
#[cfg(feature = "sysinfo")]
pub use crate::load::SystemLoad;

// Process types
#[cfg(feature = "process")]
pub use crate::process::{
    ProcessInfo, ProcessMonitor, ProcessSample, ProcessStats, ProcessStatus, ProcessTree,
};

// Network types
#[cfg(feature = "network")]
pub use crate::network::{NetworkInterface, NetworkUsage};

// Disk types
#[cfg(feature = "disk")]
pub use crate::disk::{DiskInfo, DiskUsage};

// Top-level functions
pub use crate::{init, summary};

// Utility functions
pub use crate::utils::{
    PlatformInfo, cache_line_size, format_bytes, format_bytes_usize, format_duration,
    format_percentage, format_rate, is_power_of_two,
};
