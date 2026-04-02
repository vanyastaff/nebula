# API

## Public Surface

- **Stable APIs:** `init()`, `SystemInfo`, `memory::current()`, `memory::pressure()`, `SystemError`, `SystemResult`
- **Experimental APIs:** `memory::management::*` (unsafe, low-level)
- **Hidden/internal APIs:** `SYSINFO_SYSTEM`, `SYSTEM_INFO_CACHE`, platform-specific helpers

## Usage Patterns

1. Call `init()` once at startup
2. Use `SystemInfo::get()` for cached info; `refresh()` when fresh data needed
3. Use `memory::pressure()` for backpressure decisions
4. Use feature-gated modules (`#[cfg(feature = "...")]`) for optional functionality

## Minimal Example

```rust
use nebula_system::{SystemInfo, MemoryPressure};

fn main() -> nebula_system::SystemResult<()> {
    nebula_system::init()?;

    let info = SystemInfo::get();
    println!("CPU: {} cores", info.cpu.cores);
    println!("Memory: {:.2} GB", info.memory.total as f64 / 1e9);

    let pressure = nebula_system::memory::pressure();
    if pressure.is_concerning() {
        println!("Warning: Memory pressure is high!");
    }

    Ok(())
}
```

## Advanced Example

```rust
use nebula_system::prelude::*;

fn monitor_system() -> SystemResult<()> {
    init()?;

    let info = SystemInfo::get();
    let cpu_pressure = nebula_system::cpu::pressure();
    let mem_pressure = memory::pressure();

    if cpu_pressure.is_concerning() || mem_pressure.is_concerning() {
        eprintln!(
            "System under pressure: CPU {:?}, Memory {:?}",
            cpu_pressure, mem_pressure
        );
    }

    #[cfg(feature = "disk")]
    {
        let disk_pressure = nebula_system::disk::pressure(None);
        if disk_pressure.is_concerning() {
            eprintln!("Disk pressure: {:?}", disk_pressure);
        }
    }

    Ok(())
}
```

## Error Semantics

- **Retryable errors:** None (system info is typically transient; caller may retry)
- **Fatal errors:** `PermissionDenied`, `FeatureNotSupported` (configuration/privilege)
- **Validation errors:** `SystemParseError`, `ResourceNotFound`

## Compatibility Rules

- **Major version bump:** Public API breaking changes; removal of deprecated items
- **Deprecation policy:** 2 minor versions with `#[deprecated]` before removal
