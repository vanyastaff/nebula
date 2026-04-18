---
name: nebula-system
role: Host Probes (CPU, memory, network, disk pressure detection)
status: partial
last-reviewed: 2026-04-17
canon-invariants: []
related: [nebula-metrics]
---

# nebula-system

## Purpose

The engine needs to make scheduling decisions and surface operator-visible health signals based on
host resource pressure — whether memory is approaching a critical threshold, whether CPU is
saturated, or whether disk space is low. Without a shared cross-platform abstraction, each
component would call OS APIs directly with varying semantics on Linux, macOS, and Windows.
`nebula-system` provides a unified interface for host probes: structured reads of CPU, memory,
disk, network, and process information, with a `MemoryPressure` classifier that returns actionable
thresholds rather than raw bytes.

## Role

**Host Probes** — the cross-platform system information and pressure detection layer. Cross-cutting
infrastructure (no upward dependencies). Backed by the `sysinfo` crate for cross-platform
compatibility. This crate does not emit metrics; metric recording from system data is the
responsibility of the caller (typically wired through `nebula-metrics`).

## Public API

- `init() -> SystemResult<()>` — one-time initialization; call once at process startup.
- `SystemInfo::get() -> SystemInfo` — snapshot of CPU, memory, OS, and hardware info.
- `memory::info() -> SystemResult<MemoryInfo>` — detailed memory statistics.
- `memory::pressure() -> MemoryPressure` — classified pressure level (`Normal`, `Warning`, `Critical`).
- `MemoryPressure::is_concerning() -> bool` — convenience predicate.
- `cpu::info() -> SystemResult<CpuInfo>`, `cpu::usage() -> SystemResult<f32>` — CPU stats.
- `SystemError`, `SystemResult<T>` — typed error and result alias.
- Optional modules (feature-gated): `process`, `network`, `disk`, `load`.

## Contract

No L2 canon invariants are directly assigned to this crate's seams. The crate is a utility layer;
its correctness contract is: return accurate host data on all three supported platforms, or return
a typed `SystemError` rather than silently returning incorrect values.

## Non-goals

- Not a metrics recorder — readings from this crate feed metric call sites in consumer code; metric storage and export live in `nebula-telemetry` / `nebula-metrics`.
- Not a process manager or scheduler — it reads process information, it does not control processes.
- Not a security boundary — CPU affinity on Linux requires `unsafe` (the crate's `#[allow(unsafe_code)]` annotation is intentional and documented).

## Maturity

See `docs/MATURITY.md` row for `nebula-system`.

- API stability: `partial` — core memory and CPU probes are functional; several platform limitations are documented in the `lib.rs` platform support matrix (e.g. `ip_addresses` always empty in `network`, `thread_count` hardcoded in `process`, `io_stats()` Linux-only).
- Test coverage is limited; pressure thresholds and aggregation logic would benefit from unit tests.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.6 (Observability — operators must be able to explain resource pressure from logs and metrics alone).
- Siblings: `nebula-metrics` (typical consumer for recording system metrics).

## Appendix: Feature flags and platform support

| Feature | Default | What it enables |
|---|---|---|
| `sysinfo` | yes | CPU, memory, OS info via `sysinfo` crate |
| `process` | no | Process information and monitoring |
| `network` | no | Network interface statistics |
| `disk` | no | Disk usage and filesystem details |
| `serde` | no | Serialization support for data types |

Platform support matrix (from `lib.rs`):

| Module | Linux | macOS | Windows | Notes |
|---|---|---|---|---|
| `memory` | yes | yes | yes | Via `sysinfo` |
| `cpu` | yes | yes | yes | SSE/AVX feature detection x86 only |
| `disk` | yes | yes | yes | I/O counters Linux-only (`io_stats()`) |
| `network` | yes | yes | yes | `ip_addresses` always empty |
| `process` | yes | yes | yes | `thread_count` hardcoded; `uid`/`gid` always `None` |
