# nebula-system

Cross-platform system information and utilities for the Nebula ecosystem.

## Overview

`nebula-system` provides a unified interface for gathering system information, monitoring hardware, and managing system resources across different platforms (Linux, macOS, Windows).

## Features

- **System Information** - CPU, memory, OS details
- **Hardware Detection** - CPU cores, architecture, model
- **Memory Management** - Memory utilities and pressure detection
- **Performance Monitoring** - Real-time metrics collection
- **Process Information** - Process management and monitoring
- **Network Information** - Network interfaces and statistics
- **Disk Information** - Disk usage and filesystem details

## Quick Start

```rust
use nebula_system::{SystemInfo, MemoryPressure};

fn main() -> nebula_system::SystemResult<()> {
    // Initialize the system
    nebula_system::init()?;

    // Get system information
    let info = SystemInfo::get();
    println!("CPU: {} cores", info.cpu.cores);
    println!("Memory: {} GB", info.memory.total / (1024 * 1024 * 1024));

    // Check memory pressure
    let pressure = nebula_system::memory::pressure();
    if pressure.is_concerning() {
        println!("Warning: Memory pressure is high!");
    }

    Ok(())
}
```

## Feature Flags

### Default Features
- `memory` - Memory management utilities
- `sysinfo` - System information gathering

### Optional Features
- `process` - Process information and management
- `network` - Network interface information
- `disk` - Disk and filesystem information
- `component` - Hardware component monitoring (temperatures)
- `metrics` - Performance metrics collection
- `serde` - Serialization support
- `async` - Async support with tokio

### Preset Combinations
- `full` - All features enabled
- `minimal` - Only memory utilities

## Usage Examples

### CPU Information

```rust
#[cfg(feature = "sysinfo")]
use nebula_system::cpu;

let info = cpu::info()?;
println!("CPU Model: {}", info.model);
println!("Cores: {} physical, {} logical", info.physical_cores, info.logical_cores);
println!("Usage: {:.2}%", cpu::usage()?);
```

### Memory Information

```rust
#[cfg(feature = "memory")]
use nebula_system::memory;

let info = memory::info()?;
println!("Total: {} MB", info.total / (1024 * 1024));
println!("Available: {} MB", info.available / (1024 * 1024));
println!("Used: {:.2}%", info.usage_percent());

// Check memory pressure
let pressure = memory::pressure();
match pressure {
    MemoryPressure::Normal => println!("Memory is OK"),
    MemoryPressure::Warning => println!("Memory getting high"),
    MemoryPressure::Critical => println!("Memory critical!"),
}
```

### Process Information

```rust
#[cfg(feature = "process")]
use nebula_system::process;

// Get current process info
let current = process::current()?;
println!("PID: {}", current.pid);
println!("Memory: {} MB", current.memory_usage / (1024 * 1024));
println!("CPU: {:.2}%", current.cpu_usage);

// List all processes
for proc in process::list()? {
    println!("{}: {} - {:.2}%", proc.pid, proc.name, proc.cpu_usage);
}
```

### Network Information

```rust
#[cfg(feature = "network")]
use nebula_system::network;

for interface in network::interfaces()? {
    println!("Interface: {}", interface.name);
    println!("  IP: {:?}", interface.ip_addresses);
    println!("  Sent: {} bytes", interface.bytes_sent);
    println!("  Received: {} bytes", interface.bytes_received);
}
```

### Disk Information

```rust
#[cfg(feature = "disk")]
use nebula_system::disk;

for disk in disk::list()? {
    println!("Disk: {}", disk.mount_point);
    println!("  Total: {} GB", disk.total_space / (1024 * 1024 * 1024));
    println!("  Available: {} GB", disk.available_space / (1024 * 1024 * 1024));
    println!("  Usage: {:.2}%", disk.usage_percent());
}
```

## Initialization

Call `nebula_system::init()` once at program startup to initialize caches and prepare system information gathering:

```rust
fn main() -> nebula_system::SystemResult<()> {
    nebula_system::init()?;

    // Your code here

    Ok(())
}
```

## Error Handling

```rust
use nebula_system::{SystemResult, SystemError};

fn get_system_info() -> SystemResult<String> {
    let info = nebula_system::SystemInfo::get();
    Ok(format!("CPU: {} cores", info.cpu.cores))
}
```

## Cross-Platform Support

This crate works on:
- ✅ Linux (x86_64, aarch64)
- ✅ macOS (x86_64, aarch64/M1)
- ✅ Windows (x86_64)

## Performance

- Information is cached where possible
- Minimal overhead for basic queries
- Efficient updates for monitoring scenarios

## Dependencies

See [Cargo.toml](./Cargo.toml) for the full list of dependencies.

## License

Licensed under the same terms as the Nebula project.
