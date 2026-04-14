# nebula-system Overhaul Plan

**Goal:** Превратить nebula-system из набора полуреализованных модулей в надёжный infrastructure crate для трёх контуров мониторинга: sandbox per-process, system-wide health, adaptive load shedding.

**Motivation:** Runtime/engine будут использовать этот крейт для:
- Отслеживания CPU/memory sandbox'ных процессов (Phase 3 OS-process sandbox)
- Мониторинга нагруженности всего приложения
- Адаптивного управления воркерами (отключать/уменьшать при высокой нагрузке)

**Tech Stack:** Rust 1.93, edition 2024, sysinfo 0.38, parking_lot, no new deps.

---

## Phase 1: Cleanup — удалить broken и dead code

**Goal:** Убрать всё что сломано или не используется. Уменьшить maintenance surface.

### Task 1.1: Удалить broken `memory::management` подмодуль

`allocate()` leaks через `mem::forget`, `free()` всегда Err, `lock()` дропает guard немедленно.
Никто не использует. `protect`/`query` — тонкие обёртки над `region` без добавочной ценности.

**Действие:**
- Удалить `memory::management` модуль полностью
- Удалить `pub use region::Protection as MemoryProtection`
- Убрать `region` и `libc` из зависимостей
- Убрать feature `memory` (модуль `memory` компилируется когда есть `sysinfo`)
- Обновить `prelude.rs`, `lib.rs`

**Файлы:** `src/memory.rs`, `Cargo.toml`, `src/lib.rs`, `src/prelude.rs`

### Task 1.2: Удалить dead feature flags

**Действие:**
- Удалить из `Cargo.toml`: `component`, `metrics`, `async` features и dep `tokio`
- Убрать `once_cell` dep (заменён на `std::sync::LazyLock` везде кроме `network.rs`)
- Обновить `full` preset
- Обновить `minimal` preset (станет просто `default = ["sysinfo"]`)

**Файлы:** `Cargo.toml`

### Task 1.3: Удалить dead globals и inconsistencies в `info.rs`

**Действие:**
- Удалить `SYSTEM_INFO_CACHE` (никогда не читается, `refresh()` нигде не вызывается)
- Удалить `SystemInfo::refresh()` (возвращаем когда будет реальный use case)
- Заменить `once_cell::sync::Lazy` → `std::sync::LazyLock` в `network.rs`

**Файлы:** `src/info.rs`, `src/network.rs`

### Task 1.4: Удалить broken network API

**Действие:**
- Удалить `connections()`, `connections_for_process()` (всегда `[]`, нет реализации)
- Удалить `is_online()` (broken — зависит от `ip_addresses` которые всегда empty)
- Удалить `detect_gateway()`, `detect_dns_servers()` (platform stubs, subprocess spawning из library code)
- Удалить `NetworkConfig` struct и `config()` fn (зависит от удалённого)
- Оставить: `interfaces()`, `usage()`, `total_stats()`, `get_interface()`

**Файлы:** `src/network.rs`

### Task 1.5: Почистить process module

**Действие:**
- Удалить `set_priority()` (unsafe libc, не тестируется, не используется)
- Удалить `kill()` (опасный API без потребителей — вернуть когда sandbox потребует)
- Пометить `cmd`, `environ` как `#[doc(hidden)]` или удалить из struct — они всегда empty
- Оставить: `current()`, `list()`, `get_process()`, `find_by_name()`, `stats()`, `children()`, `tree()`

**Файлы:** `src/process.rs`

### Task 1.6: Fix `process::stats()` stale read

`stats()` читает через read lock без refresh — видит stale data от последнего `list()`.

**Действие:** `stats()` должен вызывать `refresh_processes()` перед чтением (как `list()` делает).

**Файлы:** `src/process.rs`

### Task 1.7: Fix NUMA inconsistency

`info::detect_numa_nodes()` всегда `1`, `cpu::detect_numa_nodes()` читает `/sys/` на Linux. `HardwareInfo.numa_nodes` всегда `1` даже на NUMA-машинах.

**Действие:** `info::detect_numa_nodes()` делегирует в `cpu::detect_numa_nodes()`.len() (или shared function).

**Файлы:** `src/info.rs`, `src/cpu.rs`

### Task 1.8: Обновить lib.rs, prelude, examples

**Действие:**
- Обновить doc comments и platform support matrix
- Обновить prelude re-exports
- Обновить/удалить examples под новый API surface
- `cargo fmt && cargo clippy && cargo nextest run && cargo test --doc`

**Файлы:** `src/lib.rs`, `src/prelude.rs`, `examples/*`

---

## Phase 2: Test hardening

**Goal:** Покрыть все оставшиеся public API тестами. Boundary values, error paths, documented stubs.

### Task 2.1: `cpu` module tests

- `CpuPressure::from_usage()` — boundary values: 0.0, 50.0, 50.1, 70.0, 70.1, 85.0, 85.1, 100.0
- `features()` — second call returns same result (LazyLock cache)
- `topology()` — `threads_per_core >= 1`, `cores_per_package >= 1`
- `optimal_thread_count()` — returns `> 0`
- `cache_info()` — `line_size > 0`

**Файлы:** `tests/integration.rs`

### Task 2.2: `memory` module tests

- `current()` — usage_percent in [0, 100], used = total - available (±1 for race)
- `MemoryPressure` — boundary values как cpu
- Pressure ordering: `Low < Medium < High < Critical`

**Файлы:** `tests/integration.rs`

### Task 2.3: `disk` module tests

- `DiskPressure::from_usage()` — boundary values
- `has_enough_space()` — mount-point prefix matching
- `total_usage()` — in [0, 100]

**Файлы:** `tests/integration.rs`

### Task 2.4: `network` module tests

- `interfaces()` — has at least 1 interface
- `get_interface()` — returns None for nonexistent
- `total_stats()` — rx/tx >= 0

**Файлы:** `tests/integration.rs`

### Task 2.5: `process` module tests

- `list()` — non-empty, contains current process
- `stats()` — `total > 0`, `running > 0`
- `find_by_name()` — finds current process
- `children()` — returns vec (may be empty)
- `get_process(u32::MAX)` — returns Err

**Файлы:** `tests/integration.rs`

### Task 2.6: `info` module tests

- `SystemInfo::get()` — returns same Arc content on repeated calls
- `summary()` — non-empty string, contains OS name
- `OsFamily` — matches `std::env::consts::OS`
- `init()` — idempotent, succeeds on second call

**Файлы:** `tests/integration.rs`

---

## Phase 3: Per-process monitoring API

**Goal:** API для sandbox/runtime чтобы следить за конкретным процессом по PID.

### Task 3.1: `ProcessMonitor` struct

```rust
/// Tracks resource usage of a specific OS process over time.
///
/// Designed for sandbox monitoring: create when spawning a worker,
/// poll periodically, drop when worker exits.
pub struct ProcessMonitor {
    pid: u32,
    /// CPU usage from previous sample (for delta computation)
    prev_cpu: f32,
    /// Memory high-water mark
    peak_memory: usize,
    /// Creation time
    created_at: Instant,
}

impl ProcessMonitor {
    /// Create a monitor for the given PID.
    pub fn new(pid: u32) -> SystemResult<Self>;

    /// Sample current process metrics.
    /// Returns None if the process has exited.
    pub fn sample(&mut self) -> Option<ProcessSample>;

    /// Peak memory usage observed across all samples.
    pub fn peak_memory(&self) -> usize;

    /// How long this monitor has been tracking.
    pub fn elapsed(&self) -> Duration;
}

pub struct ProcessSample {
    pub pid: u32,
    pub cpu_usage: f32,
    pub memory: usize,
    pub virtual_memory: usize,
    pub status: ProcessStatus,
    pub is_alive: bool,
}
```

**Файлы:** `src/process.rs`

### Task 3.2: Tests для `ProcessMonitor`

- Создать monitor для текущего процесса → sample() возвращает Some
- sample() для несуществующего PID → None
- peak_memory >= любого отдельного sample.memory
- elapsed() > Duration::ZERO

**Файлы:** `tests/integration.rs`

---

## Phase 4: System health / load signal API

**Goal:** Единый API для runtime/engine: "можно ли спавнить ещё воркеров?"

### Task 4.1: `SystemLoad` struct и `system_load()` fn

```rust
/// Aggregated system load snapshot.
///
/// Designed for adaptive worker scaling: poll periodically,
/// use `can_accept_work()` to decide whether to spawn more workers.
#[derive(Debug, Clone)]
pub struct SystemLoad {
    pub cpu: CpuPressure,
    pub memory: MemoryPressure,
    pub cpu_usage_percent: f32,
    pub memory_usage_percent: f64,
}

impl SystemLoad {
    /// Quick check: is the system healthy enough to accept more work?
    ///
    /// Returns `false` when CPU OR memory pressure is High or Critical.
    pub fn can_accept_work(&self) -> bool {
        !self.cpu.is_concerning() && !self.memory.is_concerning()
    }

    /// More nuanced: how much headroom is available?
    /// Returns a value in [0.0, 1.0] where 1.0 = fully idle, 0.0 = at capacity.
    pub fn headroom(&self) -> f64 {
        let cpu_headroom = (100.0 - self.cpu_usage_percent as f64) / 100.0;
        let mem_headroom = (100.0 - self.memory_usage_percent) / 100.0;
        cpu_headroom.min(mem_headroom).max(0.0)
    }
}

/// Get current system load (CPU + memory combined).
///
/// This acquires write locks on the sysinfo backend — avoid calling
/// more often than every 100ms in production.
pub fn system_load() -> SystemLoad;
```

**Файлы:** новый `src/load.rs`, обновить `src/lib.rs`

### Task 4.2: Tests для `SystemLoad`

- `system_load()` — cpu_usage_percent in [0, 100], memory_usage_percent in [0, 100]
- `headroom()` — in [0.0, 1.0]
- `can_accept_work()` — returns bool without panic
- Boundary: `SystemLoad` with Critical pressure → `can_accept_work() == false`

**Файлы:** `tests/integration.rs`

---

## Phase 5: Cleanup и финализация

### Task 5.1: Обновить `.claude/crates/system.md`

Отразить:
- Удалённые API (management, connections, kill, set_priority)
- Новые API (ProcessMonitor, SystemLoad, system_load)
- Новые инварианты (features cached, single-pass loops, no region dep)

### Task 5.2: Удалить `crates/system/design/` папку

Старые design docs полностью заменены этим планом.

```bash
rm -rf crates/system/design/
```

### Task 5.3: Full validation

```bash
cargo fmt
cargo clippy -p nebula-system --all-features -- -D warnings
cargo nextest run -p nebula-system --all-features
cargo test -p nebula-system --doc
cargo check -p <archived-memory-crate>  # historical consumer validation command
```

---

## Что НЕ делаем в этом плане

- **Metrics export (OpenTelemetry/Prometheus)** — отдельный план когда nebula-telemetry стабилизируется
- **Async wrappers** — потребитель (runtime) сам обернёт в `spawn_blocking`
- **Network connections** — требует `netstat2` или platform-specific, отложено
- **Component/temperature monitoring** — нет use case
- **Configurable refresh intervals** — отложено до реального profiling в engine
- **NUMA-aware worker placement** — Phase 3+ sandbox, отдельный план

## Зависимости

```
Phase 1 (cleanup) → Phase 2 (tests) → Phase 3 (ProcessMonitor) → Phase 4 (SystemLoad) → Phase 5 (finalize)
```

Phases 3 и 4 могут идти параллельно после Phase 2.
