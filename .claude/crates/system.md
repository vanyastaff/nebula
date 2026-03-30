# nebula-system
Cross-platform system information — CPU, memory, OS, process, network, disk.

## Invariants
- Uses `unsafe` internally (system-level memory management). This is by design.
- `init()` must be called once at startup to initialize caches and prepare info gathering.

## Key Decisions
- All modules are feature-gated: `memory` + `sysinfo` are default; `process`, `network`, `disk`, `component`, `metrics` are optional.
- `MemoryPressure` is the pressure signal used by nebula-memory's monitoring.

## Traps
- Several platform gaps (always returns empty/zero for some fields):
  - `network::connections()` always returns `[]` (not implemented)
  - `disk::DiskStats` I/O counters always zero on the `sysinfo` path
  - `process`: `cmd`, `environ`, `thread_count`, `uid`, `gid` always zero/default
  - CPU feature flags (SSE/AVX) are x86 only
- `sysinfo` calls can be expensive — cache results rather than calling repeatedly.

## Relations
- No nebula deps. Used by nebula-memory (pressure detection), optionally by nebula-log (metrics feature).

<!-- reviewed: 2026-03-30 — dep bump: sysinfo 0.38.4 -->
