# nebula-system
Cross-platform system information — CPU, memory, OS, process, network, disk.
Infrastructure crate for sandbox monitoring, system health, and adaptive load shedding.

## Invariants
- Only `unsafe` is `cpu::affinity::set_current_thread()` on Linux (libc sched_setaffinity).
- `init()` should be called once at startup to warm caches; not strictly required.
- `SystemInfo::get()` returns immutable `Arc<SystemInfo>` — never changes after first access.
- `SYSINFO_SYSTEM` (global `RwLock<sysinfo::System>`) is the single mutable backend; all live reads (`cpu::usage`, `memory::current`, `process::list`) take write locks because sysinfo requires `&mut self` for refresh.
- `cpu::features()` cached via `LazyLock` — CPU features don't change at runtime.
- `page_size()` returns hardcoded 4096 (correct for x86_64; was `region::page::size()` before region dep removal).

## Key Decisions
- Single default feature: `sysinfo`. Modules `memory`, `cpu`, `load` gated on it. `process`, `network`, `disk` are opt-in.
- Removed features: `component`, `metrics`, `async`, `memory` (as standalone feature).
- Removed deps: `region`, `libc` (non-Linux), `once_cell`, `tokio`.
- `memory::management` submodule deleted (allocate/free was broken by design — region RAII).
- Network: `connections()`, `is_online()`, `config()` deleted (broken stubs). Kept: `interfaces()`, `usage()`, `total_stats()`, `get_interface()`.
- Process: `kill()`, `set_priority()` deleted (dangerous, no consumers). `cmd`/`environ` fields removed (always empty stubs).
- `SYSTEM_INFO_CACHE` and `SystemInfo::refresh()` deleted (dead code — never called).
- `cpu::pressure()` computes average directly without allocating `Vec<f32>`.
- `ProcessMonitor` — per-PID tracking for sandbox monitoring (sample, peak_memory, elapsed).
- `SystemLoad` + `system_load()` — combined CPU+memory health signal for adaptive worker scaling (`can_accept_work()`, `headroom()`).

## Traps
- Platform gaps (always returns empty/zero):
  - `network::ip_addresses` always `vec![]`
  - `disk::DiskStats` I/O counters always zero via `list()` path (use `io_stats()` on Linux)
  - `process::thread_count` always `1`, `uid`/`gid` always `None`
  - CPU feature flags (SSE/AVX) are x86 only; non-x86 returns `CpuFeatures::default()`
- `sysinfo` calls are expensive (~1ms) — don't call `system_load()` or `cpu::usage()` more than every 100ms.
- `network::usage()` first-tick returns `rx_rate=0.0, tx_rate=0.0` (no previous snapshot).
- `info::detect_numa_nodes()` reads `/sys/` on Linux only; fallback `1` on other OSes.

## Relations
- No nebula deps. Used by nebula-memory (pressure detection, format utils, SystemInfo).
- nebula-log does NOT depend on this (confirmed).

<!-- reviewed: 2026-04-03 — full overhaul: cleanup, test hardening, ProcessMonitor, SystemLoad -->

<!-- reviewed: 2026-04-07 -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
