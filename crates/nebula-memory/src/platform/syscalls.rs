//! Direct system call optimizations for memory management
//!
//! This module provides direct system call wrappers for memory-related
//! operations, avoiding the overhead of standard library functions where
//! needed.

use std::io;

#[cfg(unix)]
use libc;
#[cfg(windows)]
use winapi;

/// Memory protection flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryProtection {
    /// No access
    None,
    /// Read-only access
    ReadOnly,
    /// Read and write access
    ReadWrite,
    /// Read and execute access
    ReadExecute,
    /// Read, write, and execute access
    ReadWriteExecute,
}

/// Memory mapping flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryMapFlags {
    /// Private mapping
    Private,
    /// Shared mapping
    Shared,
    /// Fixed address mapping
    Fixed,
    /// Anonymous mapping
    Anonymous,
    /// Huge pages mapping (if available)
    HugePages,
}

impl MemoryProtection {
    /// Convert to platform-specific flags
    #[cfg(unix)]
    fn to_unix_flags(self) -> i32 {
        use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};

        match self {
            Self::None => PROT_NONE,
            Self::ReadOnly => PROT_READ,
            Self::ReadWrite => PROT_READ | PROT_WRITE,
            Self::ReadExecute => PROT_READ | PROT_EXEC,
            Self::ReadWriteExecute => PROT_READ | PROT_WRITE | PROT_EXEC,
        }
    }

    #[cfg(windows)]
    fn to_windows_flags(self) -> u32 {
        use winapi::um::winnt::{
            PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
        };

        match self {
            Self::None => PAGE_NOACCESS,
            Self::ReadOnly => PAGE_READONLY,
            Self::ReadWrite => PAGE_READWRITE,
            Self::ReadExecute => PAGE_EXECUTE_READ,
            Self::ReadWriteExecute => PAGE_EXECUTE_READWRITE,
        }
    }
}

/// Memory mapping with direct syscalls
///
/// This function uses direct system calls to map memory, which can be more
/// efficient than using standard library functions in some cases.
pub fn memory_map(
    addr: Option<*mut u8>,
    size: usize,
    protection: MemoryProtection,
    flags: &[MemoryMapFlags],
) -> io::Result<*mut u8> {
    #[cfg(unix)]
    {
        use std::ptr;

        use libc::{
            mmap, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_HUGETLB, MAP_PRIVATE, MAP_SHARED,
        };

        let prot = protection.to_unix_flags();

        let mut map_flags = 0;
        for flag in flags {
            map_flags |= match flag {
                MemoryMapFlags::Private => MAP_PRIVATE,
                MemoryMapFlags::Shared => MAP_SHARED,
                MemoryMapFlags::Fixed => MAP_FIXED,
                MemoryMapFlags::Anonymous => MAP_ANONYMOUS,
                MemoryMapFlags::HugePages => {
                    #[cfg(target_os = "linux")]
                    {
                        MAP_HUGETLB
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        0 // Not supported on non-Linux Unix systems
                    }
                },
            };
        }

        let ptr = unsafe {
            mmap(
                addr.unwrap_or(ptr::null_mut()) as *mut libc::c_void,
                size,
                prot,
                map_flags,
                -1, // fd
                0,  // offset
            )
        };

        if ptr == MAP_FAILED {
            Err(io::Error::last_os_error())
        } else {
            Ok(ptr as *mut u8)
        }
    }

    #[cfg(windows)]
    {
        use std::ptr;

        use winapi::um::memoryapi::VirtualAlloc;
        use winapi::um::winnt::{MEM_COMMIT, MEM_LARGE_PAGES, MEM_RESERVE};

        let page_protection = protection.to_windows_flags();

        let mut alloc_type = MEM_COMMIT | MEM_RESERVE;
        for flag in flags {
            if let MemoryMapFlags::HugePages = flag {
                alloc_type |= MEM_LARGE_PAGES;
            }
        }

        let addr_ptr = addr.unwrap_or(ptr::null_mut());

        let ptr = unsafe {
            VirtualAlloc(addr_ptr as *mut std::ffi::c_void, size, alloc_type, page_protection)
        };

        if ptr.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(ptr as *mut u8)
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Fallback for unsupported platforms
        let layout = std::alloc::Layout::from_size_align(size, 64)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            Err(io::Error::new(io::ErrorKind::OutOfMemory, "Memory allocation failed"))
        } else {
            Ok(ptr)
        }
    }
}

/// Unmap memory
pub fn memory_unmap(addr: *mut u8, size: usize) -> io::Result<()> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::munmap(addr as *mut libc::c_void, size) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    {
        use winapi::um::memoryapi::VirtualFree;
        use winapi::um::winnt::MEM_RELEASE;

        let result = unsafe { VirtualFree(addr as *mut std::ffi::c_void, 0, MEM_RELEASE) };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Fallback for unsupported platforms
        let layout = std::alloc::Layout::from_size_align(size, 64)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        unsafe { std::alloc::dealloc(addr, layout) };
        Ok(())
    }
}

/// Change memory protection
pub fn memory_protect(addr: *mut u8, size: usize, protection: MemoryProtection) -> io::Result<()> {
    #[cfg(unix)]
    {
        let prot = protection.to_unix_flags();
        let result = unsafe { libc::mprotect(addr as *mut libc::c_void, size, prot) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    {
        use std::ptr;

        use winapi::um::memoryapi::VirtualProtect;

        let prot = protection.to_windows_flags();
        let mut old_protect = 0;

        let result =
            unsafe { VirtualProtect(addr as *mut std::ffi::c_void, size, prot, &mut old_protect) };

        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Not supported on other platforms
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory protection not supported on this platform",
        ))
    }
}

/// Advise memory access pattern
pub fn memory_advise(addr: *mut u8, size: usize, advice: MemoryAdvice) -> io::Result<()> {
    #[cfg(unix)]
    {
        use libc::madvise;

        let advice_val = match advice {
            MemoryAdvice::Normal => libc::MADV_NORMAL,
            MemoryAdvice::Random => libc::MADV_RANDOM,
            MemoryAdvice::Sequential => libc::MADV_SEQUENTIAL,
            MemoryAdvice::WillNeed => libc::MADV_WILLNEED,
            MemoryAdvice::DontNeed => libc::MADV_DONTNEED,
            #[cfg(target_os = "linux")]
            MemoryAdvice::HugePage => libc::MADV_HUGEPAGE,
            #[cfg(not(target_os = "linux"))]
            MemoryAdvice::HugePage => libc::MADV_NORMAL, // Fallback
            MemoryAdvice::Free => {
                #[cfg(target_os = "linux")]
                {
                    libc::MADV_FREE
                }
                #[cfg(not(target_os = "linux"))]
                {
                    libc::MADV_DONTNEED // Fallback for non-Linux
                }
            },
        };

        let result = unsafe { madvise(addr as *mut libc::c_void, size, advice_val) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    {
        // Windows doesn't have direct equivalent to madvise, but we can use
        // VirtualAlloc/VirtualFree for similar functionality in some cases
        use winapi::um::memoryapi::{VirtualAlloc, VirtualFree};
        use winapi::um::winnt::{MEM_DECOMMIT, MEM_RESET};

        let result = match advice {
            MemoryAdvice::DontNeed | MemoryAdvice::Free => {
                // We can decommit the memory
                unsafe { VirtualFree(addr as *mut std::ffi::c_void, size, MEM_DECOMMIT) }
            },
            _ => {
                // For other advice types, we don't have a direct equivalent
                // Return success for now
                1
            },
        };

        if result == 0 && advice != MemoryAdvice::Normal {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Not supported on other platforms
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory advice not supported on this platform",
        ))
    }
}

/// Memory access pattern advice
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAdvice {
    /// Normal access pattern
    Normal,
    /// Random access pattern
    Random,
    /// Sequential access pattern
    Sequential,
    /// Will need the memory soon
    WillNeed,
    /// Don't need the memory soon
    DontNeed,
    /// Use huge pages if available
    HugePage,
    /// Memory can be freed
    Free,
}

/// Synchronize memory with physical storage
pub fn memory_sync(addr: *mut u8, size: usize, sync_type: MemorySyncType) -> io::Result<()> {
    #[cfg(unix)]
    {
        use libc::{msync, MS_ASYNC, MS_INVALIDATE, MS_SYNC};

        let flags = match sync_type {
            MemorySyncType::Sync => MS_SYNC,
            MemorySyncType::Async => MS_ASYNC,
            MemorySyncType::Invalidate => MS_INVALIDATE,
        };

        let result = unsafe { msync(addr as *mut libc::c_void, size, flags) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    {
        use winapi::um::memoryapi::FlushViewOfFile;

        let result = unsafe { FlushViewOfFile(addr as *const std::ffi::c_void, size) };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Not supported on other platforms
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory synchronization not supported on this platform",
        ))
    }
}

/// Memory synchronization type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySyncType {
    /// Synchronous flush (blocks until complete)
    Sync,
    /// Asynchronous flush (returns immediately)
    Async,
    /// Invalidate cached data
    Invalidate,
}

/// Prefetch memory
pub fn memory_prefetch(addr: *const u8, size: usize) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // Use a simple loop for prefetching
        if size <= 4096 {
            unsafe {
                let end = addr.add(size);
                let mut ptr = addr;
                while ptr < end {
                    // Use volatile read as a simple prefetch method
                    std::ptr::read_volatile(ptr);
                    ptr = ptr.add(64);
                }
            }
            return Ok(());
        }

        // For larger regions, use madvise
        unsafe {
            let result = libc::madvise(addr as *mut libc::c_void, size, libc::MADV_WILLNEED);

            if result == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Simple prefetch using volatile reads
        unsafe {
            let end = addr.add(size);
            let mut ptr = addr;
            while ptr < end {
                // Do simple volatile reads
                std::ptr::read_volatile(ptr);
                ptr = ptr.add(64);
            }
        }
        Ok(())
    }
}

/// Get memory page information
pub fn get_memory_page_info(addr: *const u8) -> io::Result<MemoryPageInfo> {
    #[cfg(target_os = "linux")]
    {
        use std::fs::File;
        use std::io::Read;

        let pid = unsafe { libc::getpid() };
        let page_size = crate::platform::get_page_size();
        let page_addr = (addr as usize / page_size) * page_size;

        let maps_path = format!("/proc/{}/maps", pid);
        let mut file = File::open(maps_path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        // Parse maps file to find the page info
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let addr_range: Vec<&str> = parts[0].split('-').collect();
                if addr_range.len() == 2 {
                    if let (Ok(start), Ok(end)) = (
                        usize::from_str_radix(addr_range[0], 16),
                        usize::from_str_radix(addr_range[1], 16),
                    ) {
                        if start <= page_addr && page_addr < end {
                            let perms = parts[1];
                            let path =
                                if parts.len() > 5 { parts[5..].join(" ") } else { String::new() };

                            return Ok(MemoryPageInfo {
                                address: page_addr as *const u8,
                                size: end - start,
                                read: perms.contains('r'),
                                write: perms.contains('w'),
                                execute: perms.contains('x'),
                                shared: perms.contains('s'),
                                path: Some(path),
                            });
                        }
                    }
                }
            }
        }

        // Page not found in maps
        Err(io::Error::new(io::ErrorKind::NotFound, "Memory page not found"))
    }

    #[cfg(windows)]
    {
        use winapi::um::memoryapi::VirtualQuery;
        use winapi::um::winnt::{
            MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE, PAGE_EXECUTE_READ,
            PAGE_EXECUTE_READWRITE, PAGE_READONLY, PAGE_READWRITE,
        };

        unsafe {
            let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let result = VirtualQuery(
                addr as *const std::ffi::c_void,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            );

            if result == 0 {
                return Err(io::Error::last_os_error());
            }

            let read = info.Protect
                & (PAGE_READONLY | PAGE_READWRITE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE)
                != 0;
            let write = info.Protect & (PAGE_READWRITE | PAGE_EXECUTE_READWRITE) != 0;
            let execute =
                info.Protect & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE) != 0;
            let committed = info.State & MEM_COMMIT != 0;

            Ok(MemoryPageInfo {
                address: info.BaseAddress as *const u8,
                size: info.RegionSize,
                read,
                write,
                execute,
                shared: false, // Windows API doesn't easily expose this
                path: None,    // Windows API doesn't easily expose this
            })
        }
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        // Not supported on other platforms
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory page info not supported on this platform",
        ))
    }
}

/// Memory page information
#[derive(Debug, Clone)]
pub struct MemoryPageInfo {
    /// Base address of the page
    pub address: *const u8,
    /// Size of the memory region
    pub size: usize,
    /// Whether the page is readable
    pub read: bool,
    /// Whether the page is writable
    pub write: bool,
    /// Whether the page is executable
    pub execute: bool,
    /// Whether the page is shared
    pub shared: bool,
    /// Path to mapped file (if any)
    pub path: Option<String>,
}
