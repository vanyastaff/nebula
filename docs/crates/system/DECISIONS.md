# Decisions

## D001: Use sysinfo for Cross-Platform System Info

**Status:** Adopt

**Context:** Need CPU, memory, process, network, disk info across Linux, macOS, Windows.

**Decision:** Use `sysinfo` crate as primary backend; feature-gate optional subsystems.

**Alternatives considered:** heim (heavier), raw `/proc`/sysctl/WinAPI (maintenance burden).

**Trade-offs:** Some platform limitations (e.g., NUMA, temperature); acceptable for workflow use case.

**Consequences:** sysinfo API changes may require adaptation; track upstream releases.

**Migration impact:** None.

**Validation plan:** CI on Linux, macOS, Windows; integration tests for core APIs.

---

## D002: Sync-Only API

**Status:** Adopt

**Context:** System info is typically polled; async adds complexity.

**Decision:** All public APIs are synchronous.

**Alternatives considered:** Async wrappers; tokio-based polling.

**Trade-offs:** Callers doing high-frequency polling should batch/throttle; async can wrap in consumers.

**Consequences:** No tokio dependency by default; `async` feature optional.

**Migration impact:** None.

**Validation plan:** N/A.

---

## D003: Pressure Thresholds (50/70/85%)

**Status:** Adopt

**Context:** Need actionable pressure levels for backpressure and health checks.

**Decision:** Low (<50%), Medium (50–70%), High (70–85%), Critical (>85%) for memory; similar for CPU/disk.

**Alternatives considered:** Configurable thresholds; single boolean.

**Trade-offs:** Fixed thresholds may not fit all workloads; `is_concerning()` covers common case.

**Consequences:** Documented in API; consumers can use raw percentages if needed.

**Migration impact:** None.

**Validation plan:** Unit tests for boundary conditions.

---

## D004: Feature-Gated Modules

**Status:** Adopt

**Context:** Minimal binary size; not all consumers need process/network/disk.

**Decision:** `sysinfo`, `memory` default; `process`, `network`, `disk`, `component` optional.

**Alternatives considered:** Single monolithic build; separate crates per domain.

**Trade-offs:** More feature combinations to test; clearer dependency graph.

**Consequences:** Consumers must enable features; `minimal` preset for memory-only.

**Migration impact:** None.

**Validation plan:** CI with `default`, `full`, `minimal` feature sets.
