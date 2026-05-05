//! Disk and filesystem information
//!
//! # Known Limitations
//!
//! - **`DiskStats` I/O counters** are not part of `DiskInfo` because sysinfo does not expose
//!   portable disk I/O counters. Use `io_stats(device)` when callers explicitly need them.
//! - **`io_stats(device)`** currently reads `/sys/block/<device>/stat` on Linux, where `device`
//!   must be a sysfs block-device basename such as `sda` or `nvme0n1`, and returns
//!   `Availability<DiskStats>`. Unsupported platforms, invalid device names, unreadable devices,
//!   and parse failures are explicit availability states, not zero counters.
//! - **`detect_disk_type`** maps only `HDD` and `SSD`; `Network`, `Removable`, and `RamDisk`
//!   variants of `sysinfo::DiskKind` all map to `DiskType::Unknown`.
//! - **Workaround for I/O counters on Linux**: Read `/sys/block/*/stat` directly or use
//!   `io_stats(device)` which already implements this path.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::availability::Availability;

#[cfg(any(target_os = "linux", test))]
fn validate_linux_block_device_name(device: &str) -> Result<&str, &'static str> {
    if device.is_empty() {
        return Err("device name is empty");
    }
    if matches!(device, "." | "..") {
        return Err("device name cannot be a traversal component");
    }
    if device.contains('/') || device.contains('\\') || device.contains('\0') {
        return Err("device name must be a sysfs block-device basename");
    }
    Ok(device)
}

/// Disk information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiskInfo {
    /// Mount point (e.g., "/", "C:\")
    pub mount_point: String,
    /// Device name (e.g., "/dev/sda1", "\\Device\\HarddiskVolume1")
    pub device: String,
    /// Filesystem type (e.g., "ext4", "NTFS")
    pub filesystem: String,
    /// Total space in bytes
    pub total_space: u64,
    /// Available space in bytes
    pub available_space: u64,
    /// Used space in bytes
    pub used_space: u64,
    /// Usage percentage
    pub usage_percent: f32,
    /// Whether the disk is removable
    pub is_removable: bool,
    /// Disk type
    pub disk_type: DiskType,
}

/// Disk type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DiskType {
    /// Hard disk drive
    HDD,
    /// Solid state drive
    SSD,
    /// Network drive
    Network,
    /// Removable drive (USB, etc.)
    Removable,
    /// RAM disk
    RamDisk,
    /// Unknown type
    Unknown,
}

/// I/O statistics for a disk
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiskStats {
    /// Read operations count
    pub read_count: u64,
    /// Write operations count
    pub write_count: u64,
    /// Bytes read
    pub read_bytes: u64,
    /// Bytes written
    pub write_bytes: u64,
    /// Time spent reading (ms)
    pub read_time: u64,
    /// Time spent writing (ms)
    pub write_time: u64,
    /// Current I/O operations in progress
    pub io_in_progress: u32,
}

/// File system information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FileSystemInfo {
    /// Filesystem type
    pub fs_type: String,
    /// Whether the filesystem is read-only
    pub is_readonly: bool,
    /// Whether the filesystem supports compression
    pub supports_compression: bool,
    /// Whether the filesystem is case-sensitive
    pub is_case_sensitive: bool,
    /// Maximum filename length
    pub max_filename_length: Option<usize>,
    /// Block size
    pub block_size: Option<u64>,
}

/// List all disks
pub fn list() -> Vec<DiskInfo> {
    #[cfg(feature = "disk")]
    {
        use sysinfo::Disks;

        // In sysinfo 0.37, disks API is exposed via Disks helper
        let disks = Disks::new_with_refreshed_list();

        disks
            .list()
            .iter()
            .map(|disk| {
                let total = disk.total_space();
                let available = disk.available_space();
                let used = total.saturating_sub(available);
                let usage_percent = if total > 0 {
                    (used as f64 / total as f64 * 100.0) as f32
                } else {
                    0.0
                };

                let disk_type = detect_disk_type(disk.kind());

                DiskInfo {
                    mount_point: disk.mount_point().to_string_lossy().to_string(),
                    device: disk.name().to_string_lossy().to_string(),
                    filesystem: disk.file_system().to_string_lossy().into_owned(),
                    total_space: total,
                    available_space: available,
                    used_space: used,
                    usage_percent,
                    is_removable: disk.is_removable(),
                    disk_type,
                }
            })
            .collect()
    }

    #[cfg(not(feature = "disk"))]
    {
        Vec::new()
    }
}

fn detect_disk_type(disk_type: sysinfo::DiskKind) -> DiskType {
    match disk_type {
        sysinfo::DiskKind::HDD => DiskType::HDD,
        sysinfo::DiskKind::SSD => DiskType::SSD,
        _ => DiskType::Unknown,
    }
}

/// Get specific disk by mount point
pub fn get_disk(mount_point: &str) -> Option<DiskInfo> {
    list()
        .into_iter()
        .find(|disk| disk.mount_point == mount_point)
}

/// Get total disk usage across all disks
pub fn total_usage() -> DiskUsage {
    let disks = list();

    let total: u64 = disks.iter().map(|d| d.total_space).sum();
    let available: u64 = disks.iter().map(|d| d.available_space).sum();
    let used = total.saturating_sub(available);

    DiskUsage {
        total_space: total,
        used_space: used,
        available_space: available,
        usage_percent: if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        },
    }
}

/// Disk usage summary
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiskUsage {
    /// Total space across all disks
    pub total_space: u64,
    /// Used space across all disks
    pub used_space: u64,
    /// Available space across all disks
    pub available_space: u64,
    /// Overall usage percentage
    pub usage_percent: f32,
}

/// Check if running on SSD
pub fn is_ssd(mount_point: Option<&str>) -> bool {
    if let Some(mp) = mount_point {
        get_disk(mp)
            .map(|d| d.disk_type == DiskType::SSD)
            .unwrap_or(false)
    } else {
        // Check root/system disk
        #[cfg(unix)]
        let system_mount = "/";
        #[cfg(windows)]
        let system_mount = "C:\\";

        get_disk(system_mount)
            .map(|d| d.disk_type == DiskType::SSD)
            .unwrap_or(false)
    }
}

/// Get disk I/O statistics (Linux only — reads `/sys/block/<device>/stat`).
///
/// `device` must be a sysfs block-device basename such as `sda` or `nvme0n1`.
/// Paths, separators, empty strings, and traversal components are rejected.
#[allow(unused_variables)] // target-dependent: consumed only inside #[cfg(target_os = "linux")]; #[expect] would be unfulfilled on Linux
pub fn io_stats(device: &str) -> Availability<DiskStats> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        let device_name = match validate_linux_block_device_name(device) {
            Ok(device_name) => device_name,
            Err(reason) => {
                return Availability::unavailable(format!(
                    "invalid Linux block device name for /sys/block lookup: {reason}"
                ));
            },
        };
        let stat_path = format!("/sys/block/{}/stat", device_name);

        if let Ok(content) = fs::read_to_string(&stat_path) {
            let parts: Vec<&str> = content.split_whitespace().collect();

            if parts.len() >= 11 {
                let parse = || -> Option<DiskStats> {
                    Some(DiskStats {
                        read_count: parts[0].parse().ok()?,
                        write_count: parts[4].parse().ok()?,
                        read_bytes: parts[2].parse::<u64>().ok()?.saturating_mul(512),
                        write_bytes: parts[6].parse::<u64>().ok()?.saturating_mul(512),
                        read_time: parts[3].parse().ok()?,
                        write_time: parts[7].parse().ok()?,
                        io_in_progress: parts[8].parse().ok()?,
                    })
                };
                return parse().map_or_else(
                    || Availability::unavailable(format!("failed to parse {stat_path}")),
                    Availability::available,
                );
            }
            return Availability::unavailable(format!(
                "disk stats file {stat_path} had too few fields"
            ));
        }

        return Availability::unavailable(format!("failed to read {stat_path}"));
    }

    #[cfg(not(target_os = "linux"))]
    {
        Availability::unsupported("disk I/O stats are currently implemented only on Linux")
    }
}

/// Monitor disk pressure
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DiskPressure {
    /// Less than 50% used
    Low,
    /// 50-75% used
    Medium,
    /// 75-90% used
    High,
    /// More than 90% used
    Critical,
}

impl DiskPressure {
    /// Create from usage percentage
    pub fn from_usage(usage_percent: f32) -> Self {
        if usage_percent > 90.0 {
            DiskPressure::Critical
        } else if usage_percent > 75.0 {
            DiskPressure::High
        } else if usage_percent > 50.0 {
            DiskPressure::Medium
        } else {
            DiskPressure::Low
        }
    }

    /// Check if disk pressure is concerning
    pub fn is_concerning(&self) -> bool {
        *self >= DiskPressure::High
    }
}

/// Get disk pressure for a specific mount point
pub fn pressure(mount_point: Option<&str>) -> Availability<DiskPressure> {
    if let Some(mp) = mount_point {
        get_disk(mp).map_or_else(
            || Availability::unavailable(format!("mount point {mp} was not found")),
            |d| Availability::available(DiskPressure::from_usage(d.usage_percent)),
        )
    } else {
        // Check overall disk usage
        let usage = total_usage();
        if usage.total_space == 0 {
            Availability::unavailable("no disks were available for aggregate pressure")
        } else {
            Availability::available(DiskPressure::from_usage(usage.usage_percent))
        }
    }
}

/// Find disk by device name
pub fn find_by_device(device: &str) -> Option<DiskInfo> {
    list().into_iter().find(|disk| disk.device == device)
}

/// Get recommended I/O block size for a disk
pub fn optimal_block_size(mount_point: Option<&str>) -> usize {
    if let Some(mp) = mount_point
        && let Some(disk) = get_disk(mp)
    {
        match disk.disk_type {
            DiskType::SSD => 4096,        // 4KB for SSDs
            DiskType::HDD => 65536,       // 64KB for HDDs
            DiskType::Network => 131_072, // 128KB for network
            _ => 8192,                    // 8KB default
        }
    } else {
        8192
    }
}

/// Format bytes as human-readable string
///
/// Re-exported from utils for convenience.
pub use crate::utils::format_bytes;

/// Check if a path has enough free space.
///
/// Returns unavailable when the containing disk cannot be resolved, rather
/// than collapsing probe failure into `false`.
pub fn has_enough_space(path: &str, required_bytes: u64) -> Availability<bool> {
    disk_for_path(path).map(|disk| disk.available_space >= required_bytes)
}

/// Find the disk containing a path.
pub fn disk_for_path(path: impl AsRef<std::path::Path>) -> Availability<DiskInfo> {
    use std::path::PathBuf;

    // Find the disk containing this path
    let mut best_match = None;
    let mut best_match_len = 0;
    let path = path.as_ref();
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    for disk in list() {
        let mount = PathBuf::from(&disk.mount_point);
        let mount = std::fs::canonicalize(&mount).unwrap_or(mount);
        if path.starts_with(&mount) {
            let mount_len = mount.components().count();
            if mount_len > best_match_len {
                best_match = Some(disk);
                best_match_len = mount_len;
            }
        }
    }

    best_match.map_or_else(
        || Availability::unavailable(format!("no mounted disk matched path {}", path.display())),
        Availability::available,
    )
}

/// Get disk pressure for the disk containing a path.
pub fn pressure_for_path(path: impl AsRef<std::path::Path>) -> Availability<DiskPressure> {
    disk_for_path(path).map(|disk| DiskPressure::from_usage(disk.usage_percent))
}

/// Get filesystem information for a path (Unix only — uses `statvfs`)
#[allow(unused_variables)] // target-dependent: consumed only inside #[cfg(unix)]; #[expect] would be unfulfilled on Unix
pub fn filesystem_info(path: &str) -> Availability<FileSystemInfo> {
    #[cfg(unix)]
    {
        use std::ffi::CString;

        use libc::statvfs;

        let c_path = match CString::new(path) {
            Ok(path) => path,
            Err(_) => {
                return Availability::unavailable(
                    "path contained an interior NUL byte and cannot be passed to statvfs",
                );
            },
        };
        // SAFETY: `statvfs` is a C struct with no Drop implementation or pointers.
        // Zeroing it creates a valid (though uninitialized) instance that statvfs() will fill.
        let mut stat: statvfs = unsafe { std::mem::zeroed() };

        // SAFETY: `c_path.as_ptr()` is a valid null-terminated C string from CString.
        // `stat` is a valid mutable reference to an allocated statvfs struct.
        // The statvfs() syscall will either fill it (return 0) or fail (return -1).
        unsafe {
            if statvfs(c_path.as_ptr(), &mut stat) == 0 {
                return Availability::available(FileSystemInfo {
                    fs_type: detect_fs_type(path),
                    is_readonly: stat.f_flag & libc::ST_RDONLY != 0,
                    supports_compression: false, // Would need filesystem-specific checks
                    is_case_sensitive: cfg!(unix),
                    max_filename_length: Some(stat.f_namemax as usize),
                    block_size: Some(stat.f_bsize),
                });
            }
        }

        return Availability::unavailable(format!(
            "statvfs failed for {path}: {}",
            std::io::Error::last_os_error()
        ));
    }

    #[cfg(not(unix))]
    {
        Availability::unsupported("filesystem_info is currently implemented only with Unix statvfs")
    }
}

#[cfg(unix)]
fn detect_fs_type(path: &str) -> String {
    // Find disk for this path
    for disk in list() {
        if path.starts_with(&disk.mount_point) {
            return disk.filesystem;
        }
    }

    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::validate_linux_block_device_name;

    #[test]
    fn linux_block_device_name_must_be_a_basename() {
        for invalid in [
            "",
            ".",
            "..",
            "/dev/sda",
            "sda/../stat",
            r"sda\..\stat",
            "sda\0",
        ] {
            assert!(
                validate_linux_block_device_name(invalid).is_err(),
                "{invalid:?} must not be accepted as a sysfs block device basename"
            );
        }

        assert_eq!(validate_linux_block_device_name("sda"), Ok("sda"));
        assert_eq!(validate_linux_block_device_name("nvme0n1"), Ok("nvme0n1"));
    }
}
