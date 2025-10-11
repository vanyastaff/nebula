# Syscall Safety Documentation - nebula-system

This document explains the safety contracts and OS assumptions for unsafe syscalls in the nebula-system crate.

## Overview

**Total unsafe blocks in nebula-system: 5**

All unsafe code in this crate interfaces with OS-level syscalls and FFI. Unlike nebula-memory (which minimizes unsafe for performance), unsafe in nebula-system is **unavoidable** - it's the interface between safe Rust and the operating system.

## Safety Philosophy

**Principle:** Syscalls are inherently unsafe because they depend on OS contracts that Rust cannot verify.

Our approach:
1. ✅ **Document OS contracts explicitly** - every SAFETY comment references syscall documentation
2. ✅ **Validate inputs before syscalls** - ensure preconditions are met
3. ✅ **Use safe wrappers where possible** - prefer `region` crate over raw `mmap`/`VirtualAlloc`
4. ✅ **Provide safe public APIs** - users shouldn't need unsafe for common operations
5. ✅ **Platform-specific implementations** - handle Unix/Windows differences correctly

## Unsafe Operations by Category

### 1. Process Priority Management (1 unsafe block)

**File:** `process.rs:326-333`

**Syscall:** `setpriority(PRIO_PROCESS, pid, nice_value)`

**Why unsafe:**
- FFI call to POSIX libc
- Modifies kernel scheduling state
- Requires valid PID and nice value range

**OS Contract:**
```c
// POSIX specification:
int setpriority(int which, id_t who, int prio);
// which: PRIO_PROCESS (0), PRIO_PGRP (1), PRIO_USER (2)
// who: process ID (0 = calling process)
// prio: nice value [-20, 19] (lower = higher priority)
// returns: 0 on success, -1 on failure (sets errno)
```

**Safety guarantees:**
```rust
// SAFETY: `setpriority` is a POSIX syscall that modifies process scheduling priority.
// - PRIO_PROCESS targets a specific process by PID
// - `pid` is validated to be a valid u32
// - `nice_value` is within valid range (-20 to 19)
// The syscall returns 0 on success or -1 on failure (sets errno).
unsafe {
    if setpriority(PRIO_PROCESS, pid as u32, nice_value) != 0 {
        return Err(/* handle error */);
    }
}
```

**Input validation:**
- ✅ PID validated (must be valid u32)
- ✅ nice_value clamped to [-20, 19] via Priority enum
- ✅ Return value checked (0 = success, -1 = error)
- ✅ errno captured on failure

**Platform support:** Unix only (Linux, macOS, BSD)

---

### 2. CPU Affinity Management (1 unsafe block)

**File:** `cpu.rs:381-395`

**Syscalls:** `CPU_ZERO`, `CPU_SET`, `sched_setaffinity`

**Why unsafe:**
- FFI calls to libc CPU affinity macros/functions
- Manipulates `cpu_set_t` bitfield directly
- Modifies kernel thread scheduling

**OS Contract:**
```c
// Linux specification:
void CPU_ZERO(cpu_set_t *set);           // Initialize empty set
void CPU_SET(int cpu, cpu_set_t *set);   // Add CPU to set
int sched_setaffinity(pid_t pid, size_t cpusetsize, const cpu_set_t *mask);
// pid: 0 = calling thread
// returns: 0 on success, -1 on failure (sets errno)
```

**Safety guarantees:**
```rust
// SAFETY: Using libc CPU affinity macros and syscalls:
// - `cpu_set_t` is a C struct with no Drop or pointers, safe to zero-initialize
// - `CPU_ZERO` macro safely initializes the cpu_set_t
// - `CPU_SET` macro safely sets individual CPU bits (cpus validated by caller)
// - `sched_setaffinity(0, ...)` targets current thread (PID=0)
// - Size and pointer to `set` are valid for the duration of the syscall
// Returns 0 on success, -1 on failure (sets errno).
unsafe {
    let mut set: cpu_set_t = mem::zeroed();  // Safe: Plain C struct
    CPU_ZERO(&mut set);                       // Safe: Macro initializes
    for &cpu in cpus {
        CPU_SET(cpu, &mut set);               // Safe: Macro sets bit
    }
    if sched_setaffinity(0, mem::size_of::<cpu_set_t>(), &set) != 0 {
        return Err(/* handle error */);
    }
}
```

**Input validation:**
- ✅ cpus slice validated by caller (must be valid CPU IDs)
- ✅ cpu_set_t zero-initialized (safe for C struct with no pointers)
- ✅ Size calculation uses mem::size_of (always correct)
- ✅ Return value checked

**Platform support:** Linux only (macOS uses different API)

---

### 3. Filesystem Information (2 unsafe blocks)

**File:** `disk.rs:336, 341-350`

**Syscalls:** `std::mem::zeroed()`, `statvfs(path, &mut stat)`

**Why unsafe:**
- FFI call to POSIX libc
- Zero-initialization of C struct
- Pointer passing to syscall

**OS Contract:**
```c
// POSIX specification:
struct statvfs {
    unsigned long f_bsize;    // Filesystem block size
    unsigned long f_frsize;   // Fragment size
    fsblkcnt_t    f_blocks;   // Size of fs in f_frsize units
    fsblkcnt_t    f_bfree;    // Free blocks
    fsblkcnt_t    f_bavail;   // Free blocks for unprivileged users
    fsfilcnt_t    f_files;    // Number of inodes
    fsfilcnt_t    f_ffree;    // Free inodes
    fsfilcnt_t    f_favail;   // Free inodes for unprivileged users
    unsigned long f_fsid;     // Filesystem ID
    unsigned long f_flag;     // Mount flags (ST_RDONLY, ST_NOSUID, etc.)
    unsigned long f_namemax;  // Maximum filename length
};

int statvfs(const char *path, struct statvfs *buf);
// path: null-terminated filesystem path
// buf: pointer to statvfs struct to fill
// returns: 0 on success, -1 on failure (sets errno)
```

**Safety guarantees:**

**Block 1: Zero-initialization**
```rust
// SAFETY: `statvfs` is a C struct with no Drop implementation or pointers.
// Zeroing it creates a valid (though uninitialized) instance that statvfs() will fill.
let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
```

**Block 2: Syscall**
```rust
// SAFETY: `c_path.as_ptr()` is a valid null-terminated C string from CString.
// `stat` is a valid mutable reference to an allocated statvfs struct.
// The statvfs() syscall will either fill it (return 0) or fail (return -1).
unsafe {
    if statvfs(c_path.as_ptr(), &mut stat) == 0 {
        // Read fields from stat
    }
}
```

**Input validation:**
- ✅ CString::new validates no interior nulls
- ✅ c_path.as_ptr() always valid (CString owns buffer)
- ✅ stat is stack-allocated, always valid
- ✅ Return value checked

**Platform support:** Unix only (Linux, macOS, BSD)

---

### 4. Memory Protection (1 unsafe block)

**File:** `memory.rs:243-246`

**Syscall:** `region::protect(ptr, size, protection)` (wraps `mprotect`/`VirtualProtect`)

**Why unsafe:**
- Modifies memory protection flags
- Can cause segfaults if used incorrectly
- Affects all threads accessing the memory region

**OS Contracts:**

**Unix (`mprotect`):**
```c
// POSIX specification:
int mprotect(void *addr, size_t len, int prot);
// addr: must be page-aligned
// len: number of bytes (rounded up to page size)
// prot: PROT_NONE, PROT_READ, PROT_WRITE, PROT_EXEC (bitwise OR)
// returns: 0 on success, -1 on failure (sets errno)
```

**Windows (`VirtualProtect`):**
```c
// Windows API:
BOOL VirtualProtect(
    LPVOID lpAddress,        // base address (must be page-aligned)
    SIZE_T dwSize,           // size in bytes
    DWORD  flNewProtect,     // new protection flags
    PDWORD lpflOldProtect    // receives old protection flags
);
// returns: non-zero on success, 0 on failure (GetLastError)
```

**Safety guarantees:**
```rust
/// # Safety
///
/// ## Preconditions
/// - `ptr` must point to memory allocated by this module or the OS
/// - `ptr` must be page-aligned (typically 4KB)
/// - `size` must be greater than 0
/// - No references (safe or unsafe) can exist to the memory during the call
/// - The memory region must not have been freed
///
/// ## Postconditions
/// - Memory protection is changed atomically for the entire region
/// - All threads immediately observe the new protection
/// - Existing references become invalid if protection is more restrictive
///
/// # Errors
/// - Pointer not page-aligned
/// - Size doesn't match allocation
/// - Protection flags violate platform security policies
pub unsafe fn protect(
    ptr: *mut u8,
    size: usize,
    protection: MemoryProtection,
) -> SystemResult<()> {
    // SAFETY: Caller must ensure ptr is valid, size is correct, and no aliasing
    // references exist. We delegate to region::protect which performs the OS-level
    // system call to change memory protection flags.
    unsafe {
        region::protect(ptr, size, protection)
            .map_err(|e| NebulaError::system_memory_error("protect", e.to_string()))
    }
}
```

**Input validation:**
- ✅ Caller must validate (documented in Safety section)
- ✅ region crate performs platform-specific validation
- ✅ Error propagation via Result

**Platform support:** Cross-platform (Unix and Windows)

---

## Why Unsafe Cannot Be Eliminated

### 1. FFI Boundary

**Problem:** Rust cannot verify C function contracts

```rust
// This MUST be unsafe - it's FFI
unsafe fn setpriority(which: c_int, who: id_t, prio: c_int) -> c_int;
```

**No safe alternative exists** - this is the OS interface.

### 2. OS Contract Validation

**Problem:** Rust cannot verify syscall preconditions

```rust
// Rust doesn't know that:
// - ptr must be page-aligned
// - size must be multiple of page size
// - protection flags must be valid for platform
unsafe { mprotect(ptr, size, prot) }
```

**Why not safe:**
- Page alignment is runtime property
- Valid protection flags vary by OS
- Rust type system cannot express these constraints

### 3. C Struct Initialization

**Problem:** C structs may have invalid zero state in Rust

```rust
// This is unsafe because Rust can't verify statvfs layout matches C
let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
```

**Why not MaybeUninit:**
- MaybeUninit<statvfs> is still unsafe to read
- Syscall fills it, not Rust code
- No type-safe way to express "C will initialize this"

### 4. System-Wide Effects

**Problem:** Memory protection affects all threads

```rust
// Changing protection can:
// - Make existing references invalid
// - Cause segfaults in other threads
// - Violate Rust's aliasing rules
unsafe { region::protect(ptr, size, PROT_NONE) }
```

**Why not safe:**
- Rust's borrow checker is per-thread
- Can't track system-wide memory state
- Would need whole-program verification

## Safety Validation Strategy

### 1. Input Validation

**Before every syscall:**
```rust
// Validate inputs
let c_path = CString::new(path).ok()?;  // No interior nulls
let nice_value = priority.to_nice();     // Clamped to [-20, 19]

// Then call
unsafe { syscall(validated_input) }
```

### 2. Error Handling

**After every syscall:**
```rust
unsafe {
    if syscall(...) != 0 {
        // Capture errno immediately
        let err = std::io::Error::last_os_error();
        return Err(SystemError::from(err));
    }
}
```

### 3. Documentation

**Every unsafe block has:**
- ✅ SAFETY comment referencing OS documentation
- ✅ Preconditions and postconditions
- ✅ Platform-specific notes
- ✅ Error conditions

### 4. Platform Abstraction

**Use safe wrappers:**
```rust
// Instead of raw mmap/VirtualAlloc
unsafe { mmap(...) }

// Use region crate (RAII + validation)
region::alloc(size, protection)?  // Returns RAII guard
```

### 5. Safe Public APIs

**Expose safe interfaces:**
```rust
// Unsafe implementation
unsafe fn set_priority_impl(pid: u32, priority: Priority) -> Result<()>

// Safe public API
pub fn set_process_priority(pid: u32, priority: Priority) -> Result<()> {
    validate_inputs(pid, priority)?;
    unsafe { set_priority_impl(pid, priority) }
}
```

## Testing Strategy

### 1. Unit Tests

**Test each syscall wrapper:**
- ✅ Valid inputs succeed
- ✅ Invalid inputs fail gracefully
- ✅ Error codes propagate correctly

### 2. Integration Tests

**Test platform-specific behavior:**
- ✅ Unix-specific code on Linux/macOS
- ✅ Windows-specific code on Windows
- ✅ Feature flags work correctly

### 3. Property-Based Tests

**Test invariants:**
- Memory regions remain valid after protect()
- Process priority changes are observable
- CPU affinity actually constrains threads

### 4. CI Validation

**Multi-platform testing:**
- ✅ Linux (x86_64, ARM64)
- ✅ macOS (x86_64, ARM64)
- ✅ Windows (x86_64)

## Unsafe Budget

| Category | Count | Can Eliminate? | Rationale |
|----------|-------|----------------|-----------|
| FFI syscalls | 5 | ❌ No | Required for OS interface |
| C struct zeroed | 2 | ❌ No | FFI requires valid C structs |
| Memory protection | 1 | ❌ No | Inherently unsafe operation |
| **Total** | **5** | **0%** | **All necessary** |

**Conclusion:** 100% of unsafe in nebula-system is unavoidable. All unsafe operations:
1. Interface with OS via FFI
2. Are properly documented
3. Have validated inputs
4. Use safe wrappers where possible
5. Expose safe public APIs

## Platform-Specific Notes

### Unix (Linux, macOS, BSD)

**Syscalls used:**
- `setpriority` - Process priority (process.rs)
- `sched_setaffinity` - CPU affinity (cpu.rs, Linux only)
- `statvfs` - Filesystem info (disk.rs)
- `mprotect` - Memory protection (via region crate)

**Platform differences:**
- macOS doesn't support `sched_setaffinity` (uses thread_policy_set)
- BSD uses different priority ranges
- Linux has more granular CPU affinity control

### Windows

**APIs used:**
- `SetPriorityClass` / `SetThreadPriority` - Process/thread priority
- `SetThreadAffinityMask` - CPU affinity
- `GetDiskFreeSpaceEx` - Filesystem info
- `VirtualProtect` - Memory protection

**Platform differences:**
- Priority classes instead of nice values
- Affinity mask instead of CPU set
- Different filesystem APIs
- Different memory protection flags

## References

### Official Documentation

**POSIX:**
- setpriority: https://pubs.opengroup.org/onlinepubs/9699919799/functions/setpriority.html
- sched_setaffinity: https://man7.org/linux/man-pages/man2/sched_setaffinity.2.html
- statvfs: https://pubs.opengroup.org/onlinepubs/9699919799/functions/statvfs.html
- mprotect: https://man7.org/linux/man-pages/man2/mprotect.2.html

**Windows:**
- SetPriorityClass: https://docs.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass
- SetThreadAffinityMask: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadaffinitymask
- VirtualProtect: https://docs.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualprotect

### Rust Crates

**region:** Safe cross-platform memory management
- Docs: https://docs.rs/region
- Provides RAII wrappers for mmap/VirtualAlloc
- Validates inputs and handles errors

**libc:** Raw FFI bindings to C standard library
- Docs: https://docs.rs/libc
- Direct access to POSIX functions
- Platform-specific implementations

---

*Last updated: Phase 4 (2025-10-11)*
*Related: Issue #9 - Phase 4: nebula-system Audit*
