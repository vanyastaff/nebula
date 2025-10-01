//! Prelude module for convenient imports
//!
//! This module re-exports the most commonly used types and functions
//! from the nebula-system crate for easy importing.
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
pub use crate::info::{SystemInfo, CpuInfo, OsInfo, HardwareInfo};

// Memory types (if feature enabled)
#[cfg(feature = "memory")]
pub use crate::memory::{MemoryInfo, MemoryPressure};

// CPU types (if feature enabled)
#[cfg(feature = "sysinfo")]
pub use crate::cpu::{CpuUsage, CpuPressure, CpuFeatures, CacheInfo, CpuTopology};

// Process types (if feature enabled)
#[cfg(feature = "process")]
pub use crate::process::{ProcessInfo, ProcessTree, ProcessStatus, ProcessStats};

// Network types (if feature enabled)
#[cfg(feature = "network")]
pub use crate::network::{NetworkInterface, NetworkUsage, NetworkConfig};

// Disk types (if feature enabled)
#[cfg(feature = "disk")]
pub use crate::disk::{DiskInfo, DiskUsage};

// Re-export common utility functions
pub use crate::{init, summary};

// Utility functions
pub use crate::utils::{
    format_bytes, format_bytes_usize, format_duration, format_percentage, format_rate,
    cache_line_size, is_power_of_two, PlatformInfo,
};
