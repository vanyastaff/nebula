//! Disk and filesystem information

use crate::core::{NebulaError, SystemError, SystemResult};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

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
        use sysinfo::{DiskKind, Disks};

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
                    filesystem: format!("{:?}", disk.file_system()).replace("\"", ""),
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

/// Get disk I/O statistics
pub fn io_stats(device: &str) -> Option<DiskStats> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        // Try to read from /sys/block/{device}/stat
        let device_name = device.split('/').last().unwrap_or(device);
        let stat_path = format!("/sys/block/{}/stat", device_name);

        if let Ok(content) = fs::read_to_string(&stat_path) {
            let parts: Vec<&str> = content.split_whitespace().collect();

            if parts.len() >= 11 {
                return Some(DiskStats {
                    read_count: parts[0].parse().unwrap_or(0),
                    write_count: parts[4].parse().unwrap_or(0),
                    read_bytes: parts[2].parse::<u64>().unwrap_or(0) * 512, // sectors to bytes
                    write_bytes: parts[6].parse::<u64>().unwrap_or(0) * 512,
                    read_time: parts[3].parse().unwrap_or(0),
                    write_time: parts[7].parse().unwrap_or(0),
                    io_in_progress: parts[8].parse().unwrap_or(0),
                });
            }
        }
    }

    None
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
pub fn pressure(mount_point: Option<&str>) -> DiskPressure {
    if let Some(mp) = mount_point {
        get_disk(mp)
            .map(|d| DiskPressure::from_usage(d.usage_percent))
            .unwrap_or(DiskPressure::Low)
    } else {
        // Check overall disk usage
        DiskPressure::from_usage(total_usage().usage_percent)
    }
}

/// Find disk by device name
pub fn find_by_device(device: &str) -> Option<DiskInfo> {
    list().into_iter().find(|disk| disk.device == device)
}

/// Get recommended I/O block size for a disk
pub fn optimal_block_size(mount_point: Option<&str>) -> usize {
    if let Some(mp) = mount_point {
        if let Some(disk) = get_disk(mp) {
            match disk.disk_type {
                DiskType::SSD => 4096,       // 4KB for SSDs
                DiskType::HDD => 65536,      // 64KB for HDDs
                DiskType::Network => 131072, // 128KB for network
                _ => 8192,                   // 8KB default
            }
        } else {
            8192
        }
    } else {
        8192
    }
}

/// Format bytes as human-readable string
///
/// Re-exported from utils for convenience.
pub use crate::utils::format_bytes;

/// Check if a path has enough free space
pub fn has_enough_space(path: &str, required_bytes: u64) -> bool {
    // Find the disk containing this path
    let mut best_match = None;
    let mut best_match_len = 0;

    for disk in list() {
        if path.starts_with(&disk.mount_point) {
            let mount_len = disk.mount_point.len();
            if mount_len > best_match_len {
                best_match = Some(disk);
                best_match_len = mount_len;
            }
        }
    }

    best_match
        .map(|disk| disk.available_space >= required_bytes)
        .unwrap_or(false)
}

/// Get filesystem information for a path
pub fn filesystem_info(path: &str) -> Option<FileSystemInfo> {
    #[cfg(unix)]
    {
        use libc::statvfs;
        use std::ffi::CString;

        let c_path = CString::new(path).ok()?;
        // SAFETY: `statvfs` is a C struct with no Drop implementation or pointers.
        // Zeroing it creates a valid (though uninitialized) instance that statvfs() will fill.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };

        // SAFETY: `c_path.as_ptr()` is a valid null-terminated C string from CString.
        // `stat` is a valid mutable reference to an allocated statvfs struct.
        // The statvfs() syscall will either fill it (return 0) or fail (return -1).
        unsafe {
            if statvfs(c_path.as_ptr(), &mut stat) == 0 {
                return Some(FileSystemInfo {
                    fs_type: detect_fs_type(path),
                    is_readonly: stat.f_flag & libc::ST_RDONLY != 0,
                    supports_compression: false, // Would need filesystem-specific checks
                    is_case_sensitive: cfg!(unix),
                    max_filename_length: Some(stat.f_namemax as usize),
                    block_size: Some(stat.f_bsize),
                });
            }
        }
    }

    None
}

fn detect_fs_type(path: &str) -> String {
    // Find disk for this path
    for disk in list() {
        if path.starts_with(&disk.mount_point) {
            return disk.filesystem;
        }
    }

    "unknown".to_string()
}
