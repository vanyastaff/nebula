# Changelog

All notable changes to `nebula-system` will be documented in this file.

`nebula-system` is an internal Nebula workspace crate (`publish = false`).
Its version follows the workspace version, and compatibility expectations are
managed inside the Nebula repository rather than through crates.io releases.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-05-05

Initial implementation of the internal Nebula host-probing layer.

### Added

#### System Snapshot

- Added cached `SystemInfo` host snapshot with OS, CPU, memory, and hardware data.
- Added snapshot metadata describing observation time, freshness, and backend source.
- Added idempotent `init()` backend warmup and `summary()` helper for human-readable diagnostics.

#### Availability Model

- Added `Availability<T>` and `AvailabilityStatus` for probe values that may be available,
  unsupported, unavailable, permission-denied, not implemented, not sampled, or stale.
- Added explicit availability semantics for fields where a normal zero, `None`, or empty
  collection could be mistaken for real measured data.

#### Memory

- Added current memory snapshots with effective total, available, used, and usage percentage.
- Added host memory, swap, and effective-capacity source fields.
- Added Linux cgroup memory-limit support when the sysinfo backend reports cgroup limits.
- Added `MemoryPressure`, `MemoryPressureReport`, `MemoryPressureReason`, and threshold evidence
  for operator-visible memory pressure classification.

#### CPU

- Added CPU usage snapshots with per-core usage, average usage, peak usage, and cores under
  pressure.
- Added explicit CPU sample freshness, observation time, and backend minimum sample interval.
- Added CPU pressure classification and pressure reports with raw usage evidence.
- Added CPU feature detection, topology helpers, cache information, optimal thread count helper,
  and Linux CPU affinity support.

#### Disk

- Added disk listing, aggregate usage, disk type classification, and SSD detection helpers.
- Added disk pressure classification for mount points and path-specific disk pressure helpers.
- Added path-to-disk lookup for persistence directories, checkpoint storage, and database volumes.
- Added filesystem information and Linux disk I/O statistics with explicit availability status for
  unsupported platforms and failed probes.

#### Network

- Added network interface listing with counters, MAC address, IP network metadata, MTU, loopback
  detection, and explicit availability for unsupported metadata.
- Added aggregate network counters and sampled RX/TX rate calculation.
- Added explicit not-sampled, stale, and counter-reset states for network traffic rates.

#### Process

- Added current-process lookup, arbitrary PID lookup, process listing, process statistics, process
  tree helpers, child lookup, and process-name search.
- Added `ProcessMonitor` for repeated process samples and peak resident-memory tracking.
- Added explicit availability for process CPU usage, UID/GID, and thread/task count.

#### Load

- Added aggregate `SystemLoad` snapshot combining CPU and memory pressure.
- Added availability-aware CPU and memory usage percentages, headroom calculation, and conservative
  `can_accept_work()` helper.

#### Errors and Results

- Added `SystemError`, `SystemResult<T>`, `SystemResultExt`, and IO result helpers for typed
  system-probe diagnostics.

#### Features and Serialization

- Added default `sysinfo` feature.
- Added optional `process`, `network`, `disk`, and `serde` features.
- Added `full` as the convenience feature set for normal probe features while keeping `serde`
  opt-in.
- Added serde derives for public data models behind the `serde` feature.

#### Documentation and Verification

- Added crate README covering purpose, workspace role, feature flags, API entry points, platform
  notes, contracts, and verification commands.
- Added architecture and correctness audit for production-safety review.
- Added integration tests, doctests, examples, and benchmarks for the host-probing surface.
