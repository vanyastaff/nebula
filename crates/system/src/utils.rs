//! Utility functions for system information formatting and conversion
//!
//! This module provides common utilities used throughout the Nebula ecosystem:
//! - Human-readable formatting (bytes, duration, rate, percentage)
//! - Platform information (page size, cache line size)
//! - Performance measurement tools

/// Format bytes as human-readable string
///
/// Converts byte counts to human-readable format with appropriate units.
///
/// # Examples
///
/// ```
/// use nebula_system::utils::format_bytes;
///
/// assert_eq!(format_bytes(1024), "1.00 KB");
/// assert_eq!(format_bytes(1536), "1.50 KB");
/// assert_eq!(format_bytes(1048576), "1.00 MB");
/// ```
#[inline]
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

/// Format bytes as human-readable string (usize variant)
///
/// Converts byte counts to human-readable format with appropriate units.
#[inline]
#[must_use]
pub fn format_bytes_usize(bytes: usize) -> String {
    format_bytes(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
    }

    #[test]
    fn test_format_bytes_usize() {
        assert_eq!(format_bytes_usize(1024), "1.00 KB");
    }
}

/// Format duration into human-readable string
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use nebula_system::utils::format_duration;
///
/// assert_eq!(format_duration(Duration::from_nanos(500)), "500ns");
/// assert_eq!(format_duration(Duration::from_micros(1500)), "1.50ms");
/// assert_eq!(format_duration(Duration::from_secs(65)), "1m 5s");
/// ```
#[must_use]
pub fn format_duration(duration: std::time::Duration) -> String {
    let nanos = duration.as_nanos();

    if nanos < 1_000 {
        format!("{nanos}ns")
    } else if nanos < 1_000_000 {
        format!("{:.2}µs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{:.2}s", duration.as_secs_f64())
        } else if secs < 3600 {
            let mins = secs / 60;
            let secs = secs % 60;
            format!("{mins}m {secs}s")
        } else {
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            let secs = secs % 60;
            format!("{hours}h {mins}m {secs}s")
        }
    }
}

/// Format percentage
#[must_use]
pub fn format_percentage(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

/// Format rate (per second)
#[must_use]
pub fn format_rate(rate: f64) -> String {
    if rate < 1_000.0 {
        format!("{rate:.1}/s")
    } else if rate < 1_000_000.0 {
        format!("{:.1}K/s", rate / 1_000.0)
    } else {
        format!("{:.1}M/s", rate / 1_000_000.0)
    }
}

/// Get cache line size for current platform
#[inline]
#[must_use]
pub fn cache_line_size() -> usize {
    // x86_64 and aarch64 typically use 64-byte cache lines
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        64
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        // Conservative default for other architectures
        64
    }
}

/// Check if a number is power of two
#[inline]
#[must_use]
pub const fn is_power_of_two(n: usize) -> bool {
    n != 0 && n.is_power_of_two()
}

/// Platform information
#[derive(Debug, Clone, Copy)]
pub struct PlatformInfo {
    /// System page size in bytes
    pub page_size: usize,
    /// CPU cache line size in bytes
    pub cache_line_size: usize,
    /// Pointer width in bits (32 or 64)
    pub pointer_width: usize,
}

impl PlatformInfo {
    /// Get current platform information
    #[inline]
    #[must_use]
    pub fn current() -> Self {
        Self {
            page_size: crate::info::SystemInfo::get().memory.page_size,
            cache_line_size: cache_line_size(),
            pointer_width: std::mem::size_of::<usize>() * 8,
        }
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_nanos(500)), "500ns");
        assert_eq!(format_duration(Duration::from_micros(500)), "500.00µs");
        assert_eq!(format_duration(Duration::from_millis(500)), "500.00ms");
        assert_eq!(format_duration(Duration::from_secs(30)), "30.00s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m 1s");
    }

    #[test]
    fn test_format_percentage() {
        assert_eq!(format_percentage(0.0), "0.0%");
        assert_eq!(format_percentage(0.5), "50.0%");
        assert_eq!(format_percentage(1.0), "100.0%");
    }

    #[test]
    fn test_format_rate() {
        assert_eq!(format_rate(500.0), "500.0/s");
        assert_eq!(format_rate(1500.0), "1.5K/s");
        assert_eq!(format_rate(1_500_000.0), "1.5M/s");
    }

    #[test]
    fn test_cache_line_size() {
        let size = cache_line_size();
        assert!(size > 0);
        assert!(is_power_of_two(size));
    }

    #[test]
    fn test_is_power_of_two() {
        assert!(is_power_of_two(1));
        assert!(is_power_of_two(2));
        assert!(is_power_of_two(64));
        assert!(is_power_of_two(1024));
        assert!(!is_power_of_two(0));
        assert!(!is_power_of_two(3));
        assert!(!is_power_of_two(100));
    }

    #[test]
    fn test_platform_info() {
        let info = PlatformInfo::current();
        assert!(info.page_size >= 4096);
        assert!(info.cache_line_size >= 64);
        assert!(info.pointer_width == 32 || info.pointer_width == 64);
    }
}
