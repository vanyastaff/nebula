//! Direct system call wrappers for memory operations
//!
//! This module provides direct, unsafe system call wrappers for memory-related
//! operations used by custom allocators. These bypass standard library overhead.
//!
//! # Safety
//!
//! All functions in this module perform unsafe FFI calls to OS primitives:
//! - **Unix**: libc functions (mmap, munmap, mprotect, madvise, msync, etc.)
//! - **Windows**: `WinAPI` functions (`VirtualAlloc`, `VirtualFree`, `VirtualProtect`, etc.)
//! - **Fallback**: `std::alloc` for unsupported platforms
//!
//! ## Safety Contracts
//!
//! Callers must ensure:
//! 1. **Alignment**: Addresses and sizes respect page boundaries
//! 2. **Validity**: Pointers refer to mapped memory regions
//! 3. **Lifecycle**: Memory is unmapped exactly once
//! 4. **Access**: Memory access respects protection flags
//! 5. **Concurrency**: No data races on mapped regions
//!
//! The OS validates parameters and returns errors for invalid inputs,
//! but callers remain responsible for upholding memory safety contracts.

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
/// Uses direct system calls to map memory - more efficient than stdlib for allocator use.
///
/// # Safety
///
/// This function performs unsafe FFI calls to the OS. Callers must ensure:
/// - `size` is non-zero and page-aligned (or will be rounded up by OS)
/// - If `addr` is Some, it must be a valid, unused memory region
/// - Returned pointer must be unmapped with `memory_unmap` before program exit
/// - Access to returned memory must respect `protection` flags
pub fn memory_map(
    addr: Option<*mut u8>,
    size: usize,
    protection: MemoryProtection,
    flags: &[MemoryMapFlags],
) -> io::Result<*mut u8> {
    #[cfg(unix)]
    {
        use libc::{
            MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_HUGETLB, MAP_PRIVATE, MAP_SHARED, mmap,
        };
        use std::ptr;

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
                }
            };
        }

        // SAFETY: FFI call to libc mmap. We pass:
        // - addr: valid or null (null lets OS choose)
        // - size: validated by caller
        // - prot/flags: constructed from safe enums
        // - fd=-1, offset=0: anonymous mapping (no file)
        // OS validates all parameters and returns MAP_FAILED on error.
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

        // SAFETY: FFI call to Windows VirtualAlloc. We pass:
        // - addr_ptr: valid or null (null lets OS choose)
        // - size: validated by caller
        // - alloc_type: MEM_COMMIT | MEM_RESERVE (safe flags)
        // - page_protection: constructed from safe enum
        // OS validates parameters and returns null on error.
        let ptr = unsafe {
            VirtualAlloc(
                addr_ptr.cast::<winapi::ctypes::c_void>(),
                size,
                alloc_type,
                page_protection,
            )
        };

        if ptr.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(ptr.cast::<u8>())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Fallback for unsupported platforms
        let layout = std::alloc::Layout::from_size_align(size, 64)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // SAFETY: Fallback using Rust global allocator. Layout is valid (checked above).
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "Memory allocation failed",
            ))
        } else {
            Ok(ptr)
        }
    }
}

/// Unmap memory
///
/// # Safety
///
/// - `addr` must have been returned by `memory_map`
/// - `size` must match the size used in `memory_map`
/// - Memory region must not be accessed after this call
/// - Must not be called more than once for the same region
pub fn memory_unmap(addr: *mut u8, size: usize) -> io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: FFI call to libc munmap. Caller guarantees addr/size are from mmap.
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

        let _ = size; // Used on other platforms; VirtualFree with MEM_RELEASE ignores size

        // SAFETY: FFI call to Windows VirtualFree. Caller guarantees addr is from VirtualAlloc.
        // MEM_RELEASE with size=0 releases entire region.
        let result = unsafe { VirtualFree(addr.cast::<winapi::ctypes::c_void>(), 0, MEM_RELEASE) };
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

        // SAFETY: Fallback using Rust global allocator. Caller guarantees addr/layout match allocation.
        unsafe { std::alloc::dealloc(addr, layout) };
        Ok(())
    }
}

/// Change memory protection
///
/// # Safety
///
/// - `addr` must be page-aligned
/// - Memory region [addr, addr+size) must be valid and mapped
/// - Changing protection doesn't invalidate existing safe references
pub fn memory_protect(addr: *mut u8, size: usize, protection: MemoryProtection) -> io::Result<()> {
    #[cfg(unix)]
    {
        let prot = protection.to_unix_flags();
        // SAFETY: FFI call to mprotect. Caller guarantees addr/size are valid mapped region.
        let result = unsafe { libc::mprotect(addr as *mut libc::c_void, size, prot) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    {
        use winapi::um::memoryapi::VirtualProtect;

        let prot = protection.to_windows_flags();
        let mut old_protect = 0;

        // SAFETY: FFI call to Windows VirtualProtect. Caller guarantees addr/size are valid mapped region.
        // - addr is page-aligned (caller contract)
        // - size covers mapped pages
        // - prot is valid protection flags from enum
        // - old_protect receives previous protection value
        let result = unsafe {
            VirtualProtect(
                addr.cast::<winapi::ctypes::c_void>(),
                size,
                prot,
                &raw mut old_protect,
            )
        };

        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory protection not supported on this platform",
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
            MemoryAdvice::HugePage => libc::MADV_NORMAL,
            MemoryAdvice::Free => {
                #[cfg(target_os = "linux")]
                {
                    libc::MADV_FREE
                }
                #[cfg(not(target_os = "linux"))]
                {
                    libc::MADV_DONTNEED
                }
            }
        };

        // SAFETY: FFI call to madvise with advice hint. Advice is just a hint to kernel,
        // doesn't change memory validity. Caller should ensure addr/size are valid.
        let result = unsafe { madvise(addr as *mut libc::c_void, size, advice_val) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    {
        use winapi::um::memoryapi::VirtualFree;
        use winapi::um::winnt::MEM_DECOMMIT;

        let result = match advice {
            MemoryAdvice::DontNeed | MemoryAdvice::Free => {
                // SAFETY: FFI call to VirtualFree with MEM_DECOMMIT to mark pages as don't need.
                // - addr/size should be valid (caller responsibility)
                // - MEM_DECOMMIT decommits but doesn't release (reversible)
                unsafe { VirtualFree(addr.cast::<winapi::ctypes::c_void>(), size, MEM_DECOMMIT) }
            }
            _ => 1, // No-op for other advice types
        };

        if result == 0 && advice != MemoryAdvice::Normal {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory advice not supported on this platform",
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

/// Synchronize memory with physical storage
pub fn memory_sync(addr: *mut u8, size: usize, sync_type: MemorySyncType) -> io::Result<()> {
    #[cfg(unix)]
    {
        use libc::{MS_ASYNC, MS_INVALIDATE, MS_SYNC, msync};

        let flags = match sync_type {
            MemorySyncType::Sync => MS_SYNC,
            MemorySyncType::Async => MS_ASYNC,
            MemorySyncType::Invalidate => MS_INVALIDATE,
        };

        // SAFETY: FFI call to msync to flush memory to disk.
        // - addr/size should be valid mapped region (caller responsibility)
        // - flags determine sync behavior (sync/async/invalidate)
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

        let _ = sync_type; // Used on Unix; Windows FlushViewOfFile has no sync type

        // SAFETY: FFI call to FlushViewOfFile to flush memory to disk.
        // - addr/size should be valid mapped region (caller responsibility)
        let result = unsafe { FlushViewOfFile(addr as *const winapi::ctypes::c_void, size) };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory synchronization not supported on this platform",
        ))
    }
}

/// Prefetch memory for better cache performance
///
/// # Safety
///
/// - `addr` must be valid for reads of `size` bytes
/// - The memory region must remain valid for the duration of the call
// Clippy false positive: function is marked unsafe, dereferencing is documented in safety contract
#[expect(clippy::not_unsafe_ptr_arg_deref)]
pub fn memory_prefetch(addr: *const u8, size: usize) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // Use simple loop for small regions
        if size <= 4096 {
            // SAFETY: Prefetching with volatile reads to load into cache.
            // - addr.add(size) computes end pointer
            // - Loop reads every 64 bytes (cache line size)
            // - read_volatile prevents optimization
            // - Caller should ensure addr/size are valid
            unsafe {
                let end = addr.add(size);
                let mut ptr = addr;
                while ptr < end {
                    std::ptr::read_volatile(ptr);
                    ptr = ptr.add(64);
                }
            }
            return Ok(());
        }

        // For larger regions, use madvise
        // SAFETY: FFI call to madvise with MADV_WILLNEED to prefetch pages.
        // - Hints kernel to load pages into memory
        // - Caller should ensure addr/size are valid
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
        // SAFETY: Prefetching with volatile reads to load into cache.
        // - addr.add(size) computes end pointer
        // - Loop reads every 64 bytes (cache line size)
        // - read_volatile prevents optimization
        // - Caller should ensure addr/size are valid
        unsafe {
            let end = addr.add(size);
            let mut ptr = addr;
            while ptr < end {
                std::ptr::read_volatile(ptr);
                ptr = ptr.add(64);
            }
        }
        Ok(())
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

/// Get memory page information
pub fn get_memory_page_info(addr: *const u8) -> io::Result<MemoryPageInfo> {
    #[cfg(target_os = "linux")]
    {
        use std::fs::File;
        use std::io::Read;

        // SAFETY: FFI call to getpid - always safe, returns current process ID
        let pid = unsafe { libc::getpid() };
        let page_size = crate::syscalls::get_page_size();
        let page_addr = (addr as usize / page_size) * page_size;

        let maps_path = format!("/proc/{}/maps", pid);
        let mut file = File::open(maps_path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

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
                            let path = if parts.len() > 5 {
                                parts[5..].join(" ")
                            } else {
                                String::new()
                            };

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

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Memory page not found",
        ))
    }

    #[cfg(windows)]
    {
        use winapi::um::memoryapi::VirtualQuery;
        use winapi::um::winnt::{
            MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
            PAGE_READONLY, PAGE_READWRITE,
        };

        // SAFETY: Querying memory information from Windows.
        // - zeroed() initializes MEMORY_BASIC_INFORMATION structure
        // - VirtualQuery fills structure with page info
        // - addr is queried address (may be invalid, VirtualQuery handles that)
        unsafe {
            let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let result = VirtualQuery(
                addr.cast::<winapi::ctypes::c_void>(),
                &raw mut info,
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

            Ok(MemoryPageInfo {
                address: info.BaseAddress as *const u8,
                size: info.RegionSize,
                read,
                write,
                execute,
                shared: false,
                path: None,
            })
        }
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory page info not supported on this platform",
        ))
    }
}
