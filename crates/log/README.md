# Nebula Log

A simple, fast, and beautiful logging library for Rust built on top of the `tracing` ecosystem.

## Features

- 🚀 **Fast and lightweight** - minimal dependencies, maximum performance
- 🎨 **Beautiful output** - colorful and readable logs with emojis
- ⏱️ **Built-in timing** - measure execution time with simple macros
- 🔧 **Easy configuration** - fluent API for quick setup
- 📊 **Multiple formats** - Pretty, Compact, and JSON output
- 🎯 **Structured logging** - full tracing support with spans and fields
- 📈 **Unified Observability** - Events, hooks, and metrics integration (NEW!)

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-log = "0.1.0"
```

## Usage

### Simple Setup

```rust
use nebula_log::{info, warn, error, Logger};

// Initialize with defaults
Logger::init();

// Log some messages
info!("Application started");
warn!(user_id = "123", "Invalid input detected");
error!("Failed to connect to database");
```

### Development Setup

```rust
use nebula_log::Logger;

// Pretty format with colors, debug level, and source locations
Logger::init_dev().unwrap();

// Now all your logs will be colorful and detailed
info!("This will show file and line number");
debug!("Debug info is now visible");
```

### Production Setup

```rust
use nebula_log::Logger;

// JSON format, info level, no colors
Logger::init_production().unwrap();

// Logs will be in JSON format for machine processing
info!("Service started");
```

### Custom Configuration

```rust
use nebula_log::{Logger, Format};

Logger::new()
    .level("debug")
    .format(Format::Pretty)
    .with_colors(true)
    .with_source(true)
    .with_target(false)
    .init()
    .unwrap();
```

## Timing

Measure execution time easily:

```rust
use nebula_log::{Timer, timed, info};
use std::time::Duration;

// Manual timer
let timer = Timer::new("database_query");
let result = perform_query().await;
timer.finish(); // Logs: "⚡ Timer 'database_query' finished in 25ms"

// Macro timer
let result = timed!("complex_calculation", {
    expensive_operation()
});

// Timer with checkpoints
let timer = Timer::new("multi_step");
step_1();
timer.checkpoint("step_1_done");
step_2(); 
timer.checkpoint("step_2_done");
timer.finish();
```

Timing automatically categorizes operations:
- ⚡ **Very fast** (0-10ms) - Debug level, green
- 🏃 **Fast** (11-100ms) - Debug level, cyan
- 🚶 **Medium** (101-1000ms) - Info level, yellow
- 🐌 **Slow** (1000ms+) - Warn level, red

## Output Formats

### Pretty Format (Default)
```
2024-08-06T10:30:45.123456Z  INFO example: 🚀 Application started
2024-08-06T10:30:45.123789Z  WARN example: ⚠️ Database connection slow
    at examples/simple.rs:42
2024-08-06T10:30:45.124000Z ERROR example: 💥 Failed to process request
    with user_id: "user123"
    with error: "Connection timeout"
    at examples/simple.rs:45
```

### Compact Format
```
2024-08-06T10:30:45Z INFO example: Application started user_id="user123"
2024-08-06T10:30:45Z WARN example: Database connection slow  
2024-08-06T10:30:45Z ERROR example: Failed to process request error="timeout"
```

### JSON Format
```json
{"timestamp":"2024-08-06T10:30:45.123456Z","level":"INFO","target":"example","message":"Application started","user_id":"user123"}
{"timestamp":"2024-08-06T10:30:45.123789Z","level":"WARN","target":"example","message":"Database connection slow"}
{"timestamp":"2024-08-06T10:30:45.124000Z","level":"ERROR","target":"example","message":"Failed to process request","error":"timeout"}
```

## Presets

Several presets are available for common scenarios:

```rust
// Development - pretty, colorful, debug level, shows source
Logger::init_dev().unwrap();

// Production - JSON, info level, no colors  
Logger::init_production().unwrap();

// Minimal - compact, warn level, no extras (for CLI tools)
Logger::init_minimal().unwrap();

// Compact - single line format
Logger::init_compact().unwrap();

// JSON - machine readable format
Logger::init_json().unwrap();
```

## Environment Variables

You can control logging via environment variables:

```bash
# Set log level
RUST_LOG=debug cargo run

# Filter by target
RUST_LOG=myapp::database=trace cargo run

# Complex filtering  
RUST_LOG="myapp=debug,hyper=warn,sqlx=error" cargo run
```

## Features

- `colors` (default) - Enable colored output
- `json` - Enhanced JSON formatting support

## Examples

Run the examples to see different logging styles:

```bash
# Basic example
cargo run --example simple

# See all timing categories
cargo run --example simple 2>&1 | grep Timer
```

## Performance

Nebula Log is designed for performance:
- Minimal overhead when logging is disabled
- Efficient structured data handling
- Fast JSON serialization
- Lazy evaluation of log messages

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.