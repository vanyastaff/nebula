# nebula-system

Cross-platform system information — CPU, memory, OS, process, network, disk.
Infrastructure crate for sandbox monitoring, health, and adaptive load shedding.

## Invariants

- Only `unsafe` block: `cpu::affinity::set_current_thread()` on Linux (libc `sched_setaffinity`).
- `init()` should be called once at startup to warm caches (not strictly required).
- `SystemInfo::get()` returns an immutable `Arc<SystemInfo>` — never changes after first access.
- `SYSINFO_SYSTEM` (global `RwLock<sysinfo::System>`) is the single mutable backend; all live reads (`cpu::usage`, `memory::current`, `process::list`) take write locks because sysinfo needs `&mut self` for refresh.
- `cpu::features()` cached via `LazyLock` — CPU features don't change at runtime.
- `page_size()` returns a hardcoded 4096 (correct for x86_64).
- Features: `sysinfo` (default) gates `memory`/`cpu`/`load`. `process`, `network`, `disk` are opt-in.
- `ProcessMonitor` — per-PID tracking for sandbox monitoring (sample, peak_memory, elapsed).
- `SystemLoad` + `system_load()` — combined CPU + memory health signal for adaptive worker scaling (`can_accept_work()`, `headroom()`).

## Traps

- **Platform gaps** (always returns empty / zero):
  - `network::ip_addresses` always `vec![]`.
  - `disk::DiskStats` I/O counters always zero via `list()` path (use `io_stats()` on Linux).
  - `process::thread_count` always `1`, `uid` / `gid` always `None`.
  - CPU feature flags (SSE/AVX) are x86-only; non-x86 returns `CpuFeatures::default()`.
- **Don't call more than every 100 ms.** `sysinfo` calls are ~1 ms; `system_load()` and `cpu::usage()` amplify that.
- `network::usage()` first tick returns `rx_rate = 0.0, tx_rate = 0.0` (no previous snapshot to diff).
- `info::detect_numa_nodes()` reads `/sys/` on Linux only; falls back to `1` elsewhere.

## Relations

No nebula deps. `nebula-memory` (former consumer) has been removed from the workspace — no current consumers are wired through. Available for use by sandbox monitoring, engine health, and adaptive load shedding.
