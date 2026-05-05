---
name: nebula-system
role: Host Probes (CPU, memory, disk, network, process, and pressure signals)
status: partial
last-reviewed: 2026-05-05
canon-invariants: []
related: [nebula-metrics]
---

# nebula-system

Internal Nebula workspace crate. It is not published as a standalone public crate; its
versioning, documentation, and compatibility expectations follow the Nebula repository.

## Purpose

Nebula needs trustworthy host information for scheduling, backpressure, diagnostics, and
operator-visible health signals. `nebula-system` is the shared host-probing layer for CPU,
memory, disk, network, process, OS, hardware, and aggregate load information across Linux,
macOS, and Windows.

The crate is backed by `sysinfo`, but it does not assume every platform exposes the same
semantics. Unsupported, unavailable, stale, not-sampled, and not-implemented values are
represented explicitly where a normal zero, `None`, or empty vector would be misleading.

## Role

**Host Probes** - cross-cutting infrastructure for reading system state and classifying
resource pressure. It does not emit metrics, make durable scheduling decisions, manage
processes, or act as a security boundary. Metrics recording belongs in callers such as
`nebula-metrics`; scheduling policy belongs in engine/resource layers.

## Cargo Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `sysinfo` | yes | Enables CPU, memory, OS, hardware, and aggregate load probes through `sysinfo`. |
| `process` | no | Enables process listing, lookup, stats, and `ProcessMonitor`. |
| `network` | no | Enables network interface metadata, counters, and sampled traffic rates. |
| `disk` | no | Enables disk usage, path-to-mount lookup, filesystem info, and disk pressure probes. |
| `serde` | no | Enables serde support for data models. Kept deliberately separate from `full`. |
| `full` | no | Convenience alias for normal probe features: `sysinfo`, `process`, `network`, and `disk`. |

## Workspace API

- `init() -> SystemResult<()>` - idempotent backend warmup. Lazy initialization still works if
  callers forget it.
- `SystemInfo::get() -> Arc<SystemInfo>` - cached host identity/capacity snapshot with
  `SnapshotMetadata` describing source and freshness.
- `Availability<T>` / `AvailabilityStatus` - field-level status wrapper for values that may be
  unsupported, unavailable, permission-denied, not implemented, not sampled, or stale.
- `memory::current() -> MemoryInfo` - current effective memory snapshot. Scheduling-facing
  fields use effective capacity, including Linux cgroup memory limits when sysinfo reports them.
- `memory::pressure() -> MemoryPressure` and `memory::pressure_report()` - pressure level plus
  raw evidence, thresholds, capacity source, swap state, and reason codes.
- `cpu::usage() -> CpuUsage` and `cpu::pressure_report()` - CPU usage with explicit sample
  freshness, observation time, and backend minimum sample interval.
- `load::system_load() -> SystemLoad` - aggregate CPU/memory signal with availability-aware
  usage percentages, headroom, and work-admission hints. Treat it as probe evidence, not the
  engine's scheduling policy.
- `disk::disk_for_path(path)` and `disk::pressure_for_path(path)` - path-specific disk lookup and
  pressure for persistence directories, checkpoint storage, and database volumes.
- `disk::io_stats(device)` and `disk::filesystem_info(path)` - platform-specific details exposed
  as `Availability<T>` rather than fake zero counters or silent `None`; Linux I/O stats require a
  sysfs block-device basename such as `sda` or `nvme0n1`.
- `network::interfaces()` and `network::usage()` - interface metadata and sampled counter rates.
  First samples, counter resets, and unimplemented metadata are explicit.
- `process::current()`, `process::get_process(pid)`, `process::list()`, `process::stats()`, and
  `ProcessMonitor` - process probes with explicit availability for CPU usage, UID/GID, and
  thread/task count.
- `SystemError`, `SystemResult<T>`, and `SystemResultExt` - typed crate error surface and result
  helpers.

## Contract

- Do not lie with zeros: unsupported or unavailable probe data must not be silently represented as
  `0`, `false`, `None`, `Low`, or an empty collection when callers may treat that as measured data.
- Freshness must be visible: cached snapshots, warmed samples, stale samples, and unavailable
  backend data must be distinguishable.
- Scheduling-facing memory uses effective runtime capacity where available, not blindly host
  physical capacity.
- CPU usage is sampler-backed. First and too-frequent samples are status-bearing readings, not
  guaranteed trustworthy percentages.
- Disk pressure for persistence safety should be path-specific.
- Pressure signals should carry evidence that operators can explain from logs and metrics.

## Platform Notes

| Module | Linux | macOS | Windows | Notes |
|--------|-------|-------|---------|-------|
| `memory` | yes | yes | yes | Uses sysinfo; Linux cgroup memory limits are used for effective capacity when reported. |
| `cpu` | yes | yes | yes | CPU usage freshness is explicit; x86 feature detection is architecture-specific. |
| `disk` | yes | yes | yes | Disk I/O counters are Linux-only; unsupported platforms return `Availability::Unsupported`. |
| `network` | yes | yes | yes | Metadata support varies; unimplemented fields use `Availability<T>`. |
| `process` | yes | yes | yes | UID/GID/thread/task metadata varies by platform and permissions. |
| `load` | yes | yes | yes | Current module is CPU+memory aggregate load, not OS load average. |

## Non-goals

- Not a metrics recorder.
- Not a scheduler or admission controller.
- Not a process manager.
- Not a security boundary.
- Not a replacement for platform-specific diagnostics where an integration needs deeper OS data.

## Maturity

See `docs/MATURITY.md` row for `nebula-system`.

- API stability: `partial` - the current surface is usable for diagnostics and conservative
  pressure signals, but some audit findings remain open.
- Known open areas: CPU cgroup quota/cpuset awareness, deterministic provider/fake-provider
  testing, PID reuse identity, process privacy policy, richer hardware availability, and broader
  CI/platform coverage.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.6 (operators must be able to explain resource pressure from
  logs and metrics alone).
- Audit: `docs/audits/nebula-system-architecture-audit.md`.
- Changelog: `crates/system/CHANGELOG.md`.
- Sibling: `nebula-metrics`, the typical consumer for recording probe values.

```bash
# Verify locally
cargo check -p nebula-system --all-features --all-targets
cargo test -p nebula-system --all-features
cargo clippy -p nebula-system --all-features --all-targets -- -D warnings
cargo check -p nebula-system --no-default-features
cargo check -p nebula-system --no-default-features --features "full serde"
```
