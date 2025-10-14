# nebula-system Documentation

## For AI Agents

Structured information about the `nebula-system` crate for AI agents and automated tools.

## Crate Purpose

`nebula-system` provides **cross-platform system information** for Nebula. It offers:
1. Hardware detection (CPU, memory, disks)
2. Process monitoring
3. Network interface information
4. Memory pressure detection
5. Performance metrics

## Module Structure

```
nebula-system/
├── src/
│   ├── lib.rs          # Main exports and initialization
│   ├── core/           # Core error types
│   ├── info.rs         # SystemInfo aggregator
│   ├── cpu.rs          # CPU information
│   ├── memory.rs       # Memory management
│   ├── process.rs      # Process information
│   ├── network.rs      # Network interfaces
│   ├── disk.rs         # Disk/filesystem info
│   ├── utils.rs        # Utility functions
│   └── prelude.rs      # Common imports
└── Cargo.toml
```

## Key Types

### SystemInfo

```rust
pub struct SystemInfo {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub os: OsInfo,
}
```

**Usage**: One-stop access to all system information.

### CpuInfo

```rust
pub struct CpuInfo {
    pub model: String,
    pub physical_cores: usize,
    pub logical_cores: usize,
    pub frequency_mhz: u64,
}
```

### MemoryInfo

```rust
pub struct MemoryInfo {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

impl MemoryInfo {
    pub fn usage_percent(&self) -> f64;
}
```

### MemoryPressure

```rust
pub enum MemoryPressure {
    Normal,    // < 70% used
    Warning,   // 70-90% used
    Critical,  // > 90% used
}

impl MemoryPressure {
    pub fn is_concerning(&self) -> bool;
}
```

## Feature Flags

### Core Features
- `memory` (default) - Memory utilities
- `sysinfo` (default) - System information

### Extended Features
- `process` - Process monitoring
- `network` - Network interfaces
- `disk` - Disk information
- `component` - Temperature sensors
- `metrics` - Metrics collection
- `serde` - Serialization
- `async` - Tokio support

### Presets
- `full` - All features
- `minimal` - Only memory

## Common Patterns

### Getting System Info

```rust
use nebula_system::SystemInfo;

let info = SystemInfo::get();
println!("CPU: {} cores", info.cpu.cores);
println!("Memory: {} MB", info.memory.total / (1024 * 1024));
```

### Monitoring Memory

```rust
use nebula_system::memory;

let pressure = memory::pressure();
if pressure.is_concerning() {
    // Take action (GC, free caches, etc.)
}
```

### Process Monitoring

```rust
#[cfg(feature = "process")]
use nebula_system::process;

let current = process::current()?;
println!("Memory: {} MB", current.memory_usage / (1024 * 1024));

// List all processes
for proc in process::list()? {
    if proc.cpu_usage > 50.0 {
        println!("High CPU: {} ({}%)", proc.name, proc.cpu_usage);
    }
}
```

## Initialization

**IMPORTANT**: Call `init()` once at startup:

```rust
fn main() -> nebula_system::SystemResult<()> {
    nebula_system::init()?;

    // Now safe to use all functions
    let info = SystemInfo::get();

    Ok(())
}
```

This initializes:
- System information cache
- sysinfo backend
- Memory monitoring

## Platform-Specific Notes

### Linux
- Uses `/proc` filesystem
- Requires read access to `/proc/meminfo`, `/proc/cpuinfo`
- Process info from `/proc/[pid]/`

### macOS
- Uses `sysctl` system calls
- Process info from `libproc`
- May require elevated permissions for some info

### Windows
- Uses Windows API (`winapi` crate)
- Process info from `GetProcessMemoryInfo`
- Some info requires admin privileges

## Error Handling

```rust
pub enum SystemError {
    InitializationFailed(String),
    InformationUnavailable(String),
    PermissionDenied(String),
    PlatformNotSupported(String),
}

pub type SystemResult<T> = Result<T, SystemError>;
```

## Integration Points

### Used By
- Workflow execution engine (resource limits)
- Monitoring systems
- Performance optimization
- Resource allocation

### Uses
- `sysinfo` - Cross-platform system info
- `region` - Memory management

## When to Use

Use `nebula-system` when you need:
1. ✅ Cross-platform system information
2. ✅ CPU/memory/disk monitoring
3. ✅ Process information
4. ✅ Memory pressure detection
5. ✅ Performance metrics

## When NOT to Use

❌ Don't use for:
- Low-level hardware control
- Real-time OS operations
- Kernel-level operations
- Platform-specific optimizations (use platform APIs directly)

## Performance Considerations

- **Caching**: System info is cached and refreshed periodically
- **Overhead**: Minimal for basic queries (~1μs)
- **Refresh**: Memory/CPU info updated every 200ms by default
- **Process List**: Can be expensive (~10ms for 100+ processes)

## Thread Safety

- All types are `Send + Sync`
- Internal caching uses `parking_lot::RwLock`
- Safe to call from multiple threads
- Initialization is thread-safe (uses `once_cell`)

## Best Practices

1. **Initialize once** - Call `init()` at startup
2. **Use SystemInfo** - For general info
3. **Check features** - Use `#[cfg(feature = "...")]`
4. **Handle errors** - Some info may be unavailable
5. **Cache results** - Don't poll too frequently

## Examples

### Basic Monitoring

```rust
use nebula_system::{SystemInfo, memory};

fn monitor_system() -> nebula_system::SystemResult<()> {
    let info = SystemInfo::get();

    // Check CPU
    if info.cpu.cores < 4 {
        println!("Warning: Low CPU count");
    }

    // Check memory
    let pressure = memory::pressure();
    if pressure.is_concerning() {
        println!("Warning: High memory usage");
    }

    Ok(())
}
```

### Resource Limits

```rust
use nebula_system::memory;

fn check_can_allocate(size: usize) -> bool {
    let info = memory::info().ok()?;
    info.available > size as u64
}
```

## Testing

```bash
# Run tests
cargo test -p nebula-system

# Run with all features
cargo test -p nebula-system --all-features

# Run specific feature tests
cargo test -p nebula-system --features process
```

## Version

Current version: See [Cargo.toml](./Cargo.toml)
