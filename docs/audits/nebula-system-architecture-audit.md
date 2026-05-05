# nebula-system Architecture & Correctness Audit

Audit date: 2026-05-05

Scope: `crates/system` only, with repository CI and docs where they define the crate contract. This is an architecture and correctness audit, not a patch plan.

Remediation note: this report records the pre-remediation audit snapshot. Initial fixes were started in the same worktree after the audit, so some code paths and line numbers below intentionally describe the original audited state rather than the current edited tree. The first remediation batch flattened `src/core`, split `serde` out of `full`, introduced `Availability<T>`, made several stale/unsupported probe states explicit, and added tests. Findings that require provider abstractions, CPU quota support, privacy policy, and broader platform CI remain open.

Evidence checked:
- Public crate docs and role: `crates/system/src/lib.rs`, `crates/system/README.md`, and the
  crate-local `DOCS.md` file that existed at audit time and was removed during remediation.
- Implementation: `info.rs`, `memory.rs`, `cpu.rs`, `load.rs`, `disk.rs`, `network.rs`, `process.rs`, `core/error.rs`, `core/result.rs`, `utils.rs` as they existed at audit time.
- Tests and CI: `crates/system/tests/integration.rs`, `crates/system/benches/system_load.rs`, `.github/workflows/ci.yml`, `.github/workflows/cross-platform.yml`.
- Pinned backend: `sysinfo = 0.38.4` from `crates/system/Cargo.toml` and `Cargo.lock`.

Validation run locally:
- `cargo check -p nebula-system --no-default-features` passed, with dead-code warnings in the no-sysinfo fallback.
- `cargo check -p nebula-system --no-default-features --features sysinfo|process|network|disk|serde` passed.
- `cargo check -p nebula-system --all-features` passed.
- `cargo check -p nebula-system --target x86_64-unknown-linux-gnu --all-features` passed, with a Linux-only `unused_qualifications` warning in `disk.rs`.
- `cargo test -p nebula-system --all-features` passed on Windows: 12 unit tests, 53 integration tests, 6 doctests.

Passing tests do not prove production safety for this crate. The current tests mostly assert non-panics, range bounds, and current-host sanity; they do not prove backend semantics, container correctness, path-specific disk behavior, unsupported-data honesty, CPU sampling validity, or platform parity.

## Executive Summary

The crate is conceptually useful: Nebula needs a single host-probing surface rather than ad hoc direct OS/sysinfo calls. However, the current implementation mixes cached identity data, fresh probes, derived pressure policy, placeholder fields, and scheduling advice behind APIs that often cannot express freshness, unsupported data, platform differences, or effective container limits.

Nebula should not build hard scheduling, backpressure, or operator health decisions directly on the current API. It is acceptable today only as best-effort diagnostics and human-facing coarse information where false zeros, stale snapshots, and platform gaps are tolerable.

Biggest 3 risks:
- A safe public Linux CPU-affinity API wraps an unsafe `CPU_SET` call without validating caller-provided CPU indexes, contradicting the unsafe block's own invariant in `crates/system/src/cpu.rs:416-428`.
- Host capacity is reported instead of effective runtime capacity. Memory uses `System::total_memory()`/`available_memory()` and CPU uses host CPU count without consulting cgroups, quotas, or Kubernetes limits, even though pinned `sysinfo` exposes Linux cgroup memory limits.
- The public model represents unsupported or unknown data as real-looking zeros, empty vectors, `Low` pressure, `Unknown` strings, or hardcoded values. That makes wrong decisions look valid.

Most important design correction: introduce an explicit snapshot/probe model with `observed_at`, sample validity, `ProbeStatus` or `Availability<T>`, effective host/container limits, and pressure reports with raw evidence. Keep scheduling policy out of `nebula-system`; let engine/resource layers decide from explicit evidence.

## Design Principles

These are the non-negotiable contracts this crate should converge on before Nebula relies on it for scheduling or operator health decisions:

1. Do not lie with zeros. Unsupported, unavailable, permission-denied, and not-implemented data must not be silently represented as `0`, `None`, `false`, `Low`, or an empty `Vec` when callers can interpret that value as real measured data.
2. Snapshot freshness must be explicit. `SystemInfo::get()`, CPU usage, memory pressure, disk/network/process probes, and aggregate load reports must say whether the value is cached, freshly observed, sampled, warming, stale, or unsupported.
3. Container limits matter. Scheduling-facing capacity must use effective runtime limits where available, not only physical host memory and CPU count. Docker/Kubernetes cgroup memory, CPU quota, and cpuset constraints are part of the host truth for Nebula.
4. CPU usage is a sampler, not a stateless read. Any backend that needs previous refresh state must surface warmup, sample interval, sample age, and invalid/stale samples instead of returning a plain percentage that looks immediately trustworthy.
5. Disk pressure is path-specific for persistence safety. Global or aggregate disk health must not be used to decide whether checkpoints, database writes, artifact storage, or workflow state can be persisted.
6. Pressure classification needs evidence. A pressure level alone is not operationally sufficient; reports need level, reason codes, raw values, effective limits, thresholds, source, and timestamp so operators can explain the signal.

Behavioral constraint: do not fix missing platform support by returning defaults. These values are dangerous because callers can read them as real measurements:
- `ip_addresses: vec![]` when IP enumeration is not implemented.
- `thread_count: 0` or `thread_count: 1` when thread count is unknown.
- zero I/O counters on unsupported platforms.
- `uid: None` or `gid: None` without distinguishing unsupported from unavailable.
- `cpu_usage: 0.0` when the backend sample is not ready.

Correct direction: represent unsupported, unavailable, not sampled, stale, and permission-denied data explicitly. The encoding can be `Result`, an availability wrapper, a status field, or platform-specific type semantics, but the public API must make the state impossible to confuse with a real zero/empty/none measurement.

## Critical Findings

| ID | Severity | Area | Problem | Failure Scenario | Recommended Fix |
|---|---|---|---|---|---|
| NSYS-C01 | Critical | Unsafe CPU affinity | `affinity::set_current_thread(cpus: &[usize])` is safe, but the unsafe block says `CPU_SET` is safe because "cpus validated by caller"; no validation exists before `CPU_SET(cpu, &mut set)`. | A config bug passes a CPU index larger than `CPU_SETSIZE`; the safe API may cause memory corruption before the syscall can fail. | Patch: validate non-empty CPU indexes against `CPU_SETSIZE` and effective CPU count before entering unsafe code, or make the unchecked primitive private/unsafe. |
| NSYS-C02 | Critical | Container/cgroup limits | Memory and CPU capacity are host-based. `SystemInfo::current_memory()` reads `sys.total_memory()` and `sys.available_memory()`; `CpuInfo` uses `physical_core_count()` and `cpus.len()`. No cgroup or quota path is used. | Docker/Kubernetes pod has 2 GB and 1 CPU on a 64 GB/16 CPU node. Nebula sees host capacity and over-schedules until cgroup OOM or CPU throttling. | Architecture correction: add `EffectiveHostLimits`/`ContainerLimits` and make pressure and capacity use effective limits where available. |
| NSYS-C03 | Critical | Disk pressure | `disk::pressure(Some(...))` accepts a mount point, not an arbitrary data path, and returns `DiskPressure::Low` if no exact mount is found. `has_enough_space` uses string prefix matching. | Checkpoint/database directory is on a nearly full mounted volume, but caller passes a path that is not exactly `disk.mount_point`; crate returns low pressure or false data. | API correction: replace mount-only APIs with `DiskProbe::for_path(Path)` returning a typed report or error; missing/unsupported must not become `Low`. |
| NSYS-H01 | High | Snapshot/cache semantics | `SystemInfo::get()` is a forever-cached `Arc<SystemInfo>`, while docs call it a "snapshot". `init()` refreshes the backing `sysinfo::System` but never rebuilds the cached `SYSTEM_INFO`. | Scheduler or status endpoint reads `SystemInfo::get().memory.available` in a loop and believes it is live. It is the startup value. | Architecture correction: split immutable host identity from live `SystemSnapshot { observed_at, sample_age }`; document `init()` as optional warmup, not freshness. |
| NSYS-H02 | High | Role boundary | The crate role says host probes and no scheduling decisions, but `load::SystemLoad::can_accept_work()` directly encodes admission policy and docs say to spawn workers from it. | Engine treats `can_accept_work()` as canonical policy even though CPU/memory inputs can be stale, host-scoped, or first-sample invalid. | Architecture correction: expose pressure evidence only; move acceptance/throttling policy to engine/resource layers. |
| NSYS-H03 | High | CPU usage sampling | `cpu::usage()`, `cpu::pressure()`, and `load::system_load()` call `sys.refresh_cpu_usage()` directly. Pinned sysinfo documents first CPU usage calls as inaccurate and requiring two samples separated by `MINIMUM_CPU_UPDATE_INTERVAL`. | Nebula starts, polls CPU once, sees a meaningless value, and accepts or rejects work incorrectly. Hot polling faster than 200 ms can reuse stale backend state. | API correction: introduce stateful `CpuSampler` with warmup, `observed_at`, `valid_after`, and explicit invalid/insufficient-sample status. |
| NSYS-H04 | High | Memory pressure | `MemoryPressure` is a fixed enum derived from `(total - available) / total` with thresholds `>50`, `>70`, `>85`. It ignores cgroups, swap, process RSS, OOM risk, configured limits, hysteresis, and reasons. | 512 MB VPS and 64 GB workstation get the same thresholds; macOS compression or Linux cache reclaim makes "used" misleading; swap thrash is invisible. | API correction: replace bare enum with `MemoryPressureReport { level, reasons, raw, effective_limit, thresholds }` and configurable classifier. |
| NSYS-H05 | High | Unsupported data honesty | No-sysinfo `SystemInfo` returns zeros for memory and unknown strings. Network IPs are always `vec![]`; process `thread_count` is always `1`; `uid`/`gid` are always `None`; missing disk pressure becomes `Low`. | UI displays "no IPs" or "1 thread"; scheduler interprets zero memory or low disk pressure as real state. | API correction: introduce `Availability<T>`/`ProbeStatus` so unsupported, unavailable, permission denied, unknown, and zero are distinct. |
| NSYS-H06 | High | Network probe | `interfaces()` sets `ip_addresses: vec![]`, `is_up: true`, `mtu: None`, `speed: None`, and name-based loopback detection. It also wraps `mac_address()` in `Some(...)` unconditionally. | UI/operator sees "up, no IPs" for every interface; virtual/container NICs and nonstandard loopbacks are misclassified. | API correction: use sysinfo IP network metadata where available, and represent missing metadata explicitly. |
| NSYS-H07 | High | Process probe | Process fields are lossy and misleading: `thread_count: 1`, `uid/gid: None`, no PID start-time identity, no command-line privacy policy, and all-process refresh includes Linux tasks by default. | Operator trusts thread count; PID is reused after worker exit; process listing is expensive in a hot path; logs expose paths without policy. | API correction: make unsupported fields explicit, include process identity/start time where possible, add privacy-safe projection, and avoid full task refresh unless requested. |
| NSYS-H08 | High | Error model | `SystemError` has useful-looking variants, but many fallible probes return `Vec`, `Option`, booleans, `Low`, or default values. Errors are stringly typed and mostly lose path/interface/PID/backend context. | Engine cannot decide whether to retry, degrade, warn, or fail because unsupported, unavailable, permission denied, and "not found" collapse into `None` or defaults. | API correction: make probe APIs return typed status/result reports with structured context and source error retention. |
| NSYS-H09 | High | Platform/data semantics | Hardware/topology values are hardcoded defaults: page size `4096`, cache line `64`, huge page `Some(2MB)` on Linux/Windows, non-Linux cache sizes are "typical" values, packages always `1`. | Scheduler or allocator treats guessed NUMA/cache/hugepage data as real platform capabilities. | API correction: return `Option`/`Availability` for unproven hardware fields; never encode guesses as measured values. |
| NSYS-H10 | High | Load API mismatch | The `load` module does not expose OS load average even though host probes list load information "if present". It exposes CPU+memory scheduling aggregation instead. | Operators expect Linux/macOS load average, but dashboards get a crate-specific admission proxy with different semantics. | API correction: add a separate load-average probe or rename current aggregate to avoid semantic confusion. |
| NSYS-H11 | High | CI/platform coverage | Cross-platform smoke tests omit `nebula-system`; CI no-default feature checks omit it too. Local Windows tests pass, but there is no required Linux/macOS/Windows system-probe matrix for this crate. | A sysinfo/platform regression ships because Ubuntu-only workspace checks do not exercise macOS/Windows semantics or feature combinations for host probes. | Documentation/test only: add `nebula-system` to cross-platform smoke and add feature/target matrix jobs. |
| NSYS-H12 | High | Testability architecture | Direct global `sysinfo` calls and lazy globals make core behavior hard to test deterministically. Pressure classifiers cannot be fed fixture data except through the live host. | Boundary cases like `total=0`, `available>total`, cgroup limits, swap pressure, first CPU sample, and permission errors remain unproven. | Refactor: introduce `SystemProvider`/`FakeSystemProvider` and pure classifiers over explicit raw inputs. |

## Architecture Risks

`nebula-system` is not one abstraction today; it is three:
- Cached host identity: `SystemInfo::get()` returns `Arc::clone(&SYSTEM_INFO)` from `info.rs:115-117`.
- Fresh live probes: `SystemInfo::current_memory()` takes a write lock and calls `refresh_memory()` in `info.rs:123-132`; CPU and load refresh in `cpu.rs:139-140`, `cpu.rs:197-198`, and `load.rs:73-75`.
- Derived policy: `MemoryPressure`, `CpuPressure`, `DiskPressure`, and `SystemLoad::can_accept_work()`.

That mix is dangerous because the API does not identify which values are cached, which are fresh, which are estimates, and which are decisions.

`init()` is idempotent but not required for correctness because `LazyLock` initializes on first use. It also does not refresh the cached `SYSTEM_INFO` if that cache already exists. Evidence:
- `init()` calls `SystemInfo::get()` first, then refreshes `SYSINFO_SYSTEM` in `info.rs:316-327`.
- `SYSTEM_INFO` is a separate lazy `Arc<SystemInfo>` built once in `info.rs:144-145`.
- The test only proves repeated `init()` does not fail (`tests/integration.rs:472-476`), not that it changes freshness.

The crate also lacks backend/provider separation. Direct calls to `SYSINFO_SYSTEM`, `Disks::new_with_refreshed_list()`, `Networks::new_with_refreshed_list()`, `/sys`, and `statvfs` are embedded in public API functions. This blocks deterministic testing of edge cases and platform failures.

Most important correction: separate `HostIdentity` from `SystemSnapshot`, and make snapshots include `observed_at`, `source`, and per-field availability/status. Then build optional samplers/classifiers around snapshots.

## Host Data Semantics Risks

Memory:
- `info::MemoryInfo` has `total`, `available`, page size, and swap fields (`info.rs:81-92`), but `memory::MemoryInfo` drops swap and page size (`memory.rs:42-53`).
- `memory::current()` calculates `used = total.saturating_sub(available)` in `memory.rs:58-59`. This prevents underflow but hides impossible backend states such as `available > total`.
- `usage_percent` becomes `0.0` when `total == 0` (`memory.rs:65-77`), making unavailable data look idle.

CPU:
- `CpuInfo` stores `cores` and `threads` from sysinfo in `info.rs:171-183`, but there is no distinction between host logical CPUs and effective scheduler capacity.
- CPU frequency is taken from the first CPU only (`info.rs:179`), which may be stale, zero, or unrepresentative on heterogeneous/frequency-scaling systems.
- `CpuPressure` is an average over all host CPUs (`cpu.rs:205-206`), not quota-adjusted capacity.

Disk:
- `DiskInfo.available_space` is sysinfo `available_space()` (`disk.rs:110-127`). The API does not explain whether this is user-available, free including reserved blocks, or volume-specific across platforms.
- `total_usage()` sums all disks (`disk.rs:157-175`), which is not meaningful for any one data path and can double-count or mix root, bind mounts, removable, network, and ephemeral filesystems.

Network:
- Counters use cumulative totals (`network.rs:117-126`, `network.rs:170-179`), while `usage()` derives rates from a global cache (`network.rs:184-185`).
- Counter resets/interface restarts are treated as zero delta through `saturating_sub` (`network.rs:210-211`) without a reset status.

Process:
- Resident memory and virtual memory are bytes (`process.rs:34-37`, `process.rs:115-116`), but virtual memory has platform-specific limitations. Pinned sysinfo documents macOS virtual memory values as often hundreds of GB per process.
- `ProcessStats.total_cpu` sums per-process percentages (`process.rs:201-217`), so values can exceed 100 and do not mean "system CPU percent".

OS/hardware:
- Unknown OS data is represented as `"Unknown"` strings in `info.rs:163-166` and no-sysinfo fallback `info.rs:213-230`, not as unavailable data.
- Page size and cache line size are constants, not probes (`info.rs:260-267`).

## Platform Compatibility Risks

Linux:
- Cgroup memory support is not used. Pinned sysinfo explicitly documents that `System::total_memory()` is host memory and points Linux callers to `cgroup_limits()` for cgroup-limited values.
- CPU usage uses `/proc/stat` through sysinfo and requires elapsed time between refreshes.
- Disk `io_stats(device)` reads `/sys/block/<device>/stat` and assumes a simple block device basename (`disk.rs:217-233`). Partitions, device mapper, NVMe names, and containerized `/sys` visibility need platform tests.
- NUMA node count is read from `/sys/devices/system/node/` in `info.rs:278-300`, but NUMA node memory is reported as zero in `cpu.rs:344-348`.

macOS:
- There are no macOS-specific tests in CI for `nebula-system`. Cross-platform smoke exists but does not include this crate (`.github/workflows/cross-platform.yml:50-54`).
- The crate returns typical cache sizes on non-Linux (`cpu.rs:271-280`) and does not expose macOS memory compression semantics.
- Disk I/O stats return `None`; the API does not encode "unsupported on macOS" except doc comments.

Windows:
- Local all-features tests pass on Windows, but semantics remain weak: free vs available memory differs from Linux, UID/GID are intentionally unavailable, and drive/mount path handling is string-based.
- CPU affinity returns a typed unsupported error on non-Linux (`cpu.rs:442-448`), which is the right pattern and should be extended to other unsupported fields.

Unsupported targets:
- `info.rs` no-sysinfo fallback returns zeros/unknowns instead of unsupported status (`info.rs:209-245`). This violates the README contract that inaccurate data should become a typed error (`README.md:40-44`).

Evidence missing: no CI or checked artifact proves macOS runtime behavior for memory, disk, network, or process probes. A macOS and Windows matrix running the system integration tests with default and all features would be the minimum proof.

## Container / Cgroup Risks

This is a major architecture risk.

Current code:
- `SystemInfo::current_memory()` uses host `sys.total_memory()` and `sys.available_memory()` in `info.rs:126-132`.
- `memory::current()` and `load::system_load()` classify pressure from those host totals in `memory.rs:58-87` and `load.rs:86-112`.
- `CpuInfo` uses host physical/logical counts in `info.rs:171-183`.
- No code references `cgroup`, `cpu.max`, `cpu.cfs_quota_us`, `cpuset.cpus`, Kubernetes Downward API, or container memory paths.

Pinned sysinfo 0.38.4 has Linux `System::cgroup_limits()` and `CGroupLimits { total_memory, free_memory, free_swap, rss }`, but `nebula-system` does not call it. That means Nebula running inside Docker/Kubernetes may see host memory and host CPU count.

Impact:
- Memory backpressure false negative: 2 GB pod on 64 GB host can report low pressure until cgroup OOM.
- CPU scheduling false negative: 1 CPU quota on 16-core node can report 16 threads and low average CPU.
- Load average is host-level and not useful for container admission even if later added.
- Disk free space may be correct for a mounted volume if the path is checked correctly, but current APIs do not reliably check the relevant path.

Design path:
- Add `EffectiveLimits` with fields for host memory, cgroup memory, effective memory limit, host CPU count, cpuset count, CPU quota cores, and confidence/status.
- On Linux, read cgroup v2 and v1 for memory and CPU. Reuse sysinfo cgroup memory where sufficient, but add CPU quota/cpuset support because sysinfo memory limits alone do not solve scheduling.
- Make pressure classifiers consume effective limits, not raw host totals.
- Expose whether the process is container-limited and whether limits were unavailable, unlimited, or unreadable.

## Memory Pressure Classifier Review

Current classifier:
- `Low`: usage <= 50
- `Medium`: usage > 50
- `High`: usage > 70
- `Critical`: usage > 85
- Usage is `(total - available) / total * 100` (`memory.rs:58-87`).

Problems:
- No cgroup/container limit support.
- Swap total/free is collected in `info::MemoryInfo` but discarded by `memory::MemoryInfo`; pressure ignores swap entirely.
- No absolute free-memory floor. On a 512 MB VPS, 20 percent free may be too little; on a 64 GB machine it may be fine.
- No hysteresis or smoothing, so pressure can flap at 70/85 percent.
- No raw evidence or reason field. Operators cannot tell whether pressure came from low available memory, high RSS, swap exhaustion, cgroup limit, or host pressure.
- `total == 0` becomes `0.0 percent` and `Low`, silently treating unavailable memory as safe.
- `used = saturating_sub` hides backend anomalies such as `available > total`.
- Mac memory compression and Windows memory reporting are not represented in the model.

Required scenarios:
- 512 MB VPS: fixed percent thresholds can start work with dangerously low absolute bytes.
- 2 GB container on 64 GB host: pressure is based on host memory, not pod memory.
- 64 GB developer machine: fixed thresholds can over-warn despite ample absolute headroom.
- Heavy Linux file cache: available memory may be okay, but "used" naming suggests pressure; report should state available/reclaimable semantics.
- Swap disabled: no special reason or risk bit.
- Swap heavily used: pressure may remain low if RAM available looks acceptable.
- Fluctuation around threshold: `is_concerning()` can flap every poll.
- Windows: free vs available differs; docs do not define the contract.
- macOS compression: compressed memory is not exposed, so pressure may be misleading.
- Linux cgroup v1/v2: not considered.

Conclusion: `MemoryPressure` can be used as a coarse diagnostic badge, not a scheduling gate. Nebula needs a `MemoryPressureReport` with raw bytes, effective limit, swap state, reasons, and classifier version.

## CPU Probe Review

Current CPU API:
- `cpu::usage() -> CpuUsage`, not the README/DOCS claim of `SystemResult<f32>`.
- `cpu::pressure() -> CpuPressure`.
- `load::system_load()` refreshes CPU and memory in one lock.

Risks:
- First sample invalid: sysinfo documents `refresh_cpu_usage()` as likely inaccurate on first call and requiring two calls separated by about 200 ms. `cpu.rs` does not surface that requirement.
- Fast polling can be stale: sysinfo skips updates faster than `MINIMUM_CPU_UPDATE_INTERVAL` on Linux; the crate returns whatever remains in the backend.
- Global average ambiguity: `CpuUsage.average` averages host per-core values (`cpu.rs:150-166`), not an effective quota-normalized usage.
- Cgroup CPU quotas/cpuset are ignored.
- `CpuInfo.frequency_mhz` from first CPU is not reliable capacity data.
- `CpuPressure::Low` is returned when no CPUs are present (`cpu.rs:200-203`) or when sysinfo is disabled (`cpu.rs:209-212`), making unavailable CPU data look safe.
- `cache_info()` returns typical non-Linux values, and `topology()` assumes one package (`cpu.rs:271-280`, `cpu.rs:313-315`).
- The APIs use synchronous global locks. They do not sleep, but they can block async runtime worker threads while probing.

Can the scheduler use `cpu::usage()` directly? Not safely. It needs a stateful sampler with validity and effective CPU capacity.

## Disk Probe Review

Current disk API:
- `list() -> Vec<DiskInfo>` creates a fresh `Disks::new_with_refreshed_list()` (`disk.rs:97-134`).
- `pressure(mount_point: Option<&str>) -> DiskPressure` returns low on missing mount (`disk.rs:275-284`).
- `has_enough_space(path, required_bytes)` chooses the longest `path.starts_with(&disk.mount_point)` match (`disk.rs:313-331`).
- `filesystem_info(path)` is Unix-only and returns `None` on failure (`disk.rs:334-366`).

Risks:
- Global disk usage is not meaningful for Nebula storage safety.
- Exact mount-point APIs do not support a normal data directory path.
- String prefix matching is incorrect for paths such as `/var/lib2` vs `/var/lib`; it also ignores Windows case/normalization and symlinks.
- Missing disk becomes `Low` pressure.
- Network/removable/RAM disk kinds collapse to `Unknown` in `detect_disk_type()` (`disk.rs:142-147`) even though `DiskType` has variants for them.
- Linux I/O stats parse failures become zero fields through `unwrap_or(0)` (`disk.rs:226-232`) or `None` without error context.
- Filesystem `is_case_sensitive: cfg!(unix)` is a broad guess; macOS APFS can be case-insensitive.

Required scenarios:
- Database directory on separate mount: current API only works if caller already knows and passes exact mount point.
- Docker volume nearly full: `pressure(None)` can be healthy because it sums all disks.
- Root healthy but workflow storage full: global pressure can be low.
- Windows C: with multiple volumes: string matching and exact mount assumptions are fragile.
- Linux reserved blocks: docs do not define whether available or free is used for write safety.
- Network filesystem unavailable: `Vec`/`Option` gives weak operator diagnostics.

## Network Probe Review

Current network API:
- `interfaces() -> Vec<NetworkInterface>` uses sysinfo `Networks::new_with_refreshed_list()`.
- `ip_addresses` is always empty (`network.rs:131`), despite sysinfo 0.38.4 having IP network metadata.
- `is_up` is always true (`network.rs:132`).
- `is_loopback` is name-only (`network.rs:133`).
- `mtu` and `speed` are always `None`.
- `usage()` derives rates from a crate-global cache and returns zero rates on first sample (`network.rs:153-191`).

Risks:
- Empty IP list means "not implemented", not "no IPs"; the type cannot distinguish the two.
- MAC address is always `Some(network.mac_address().to_string())`; an unavailable/zero MAC may be displayed as real.
- Virtual interfaces, Docker/Kubernetes NICs, VPNs, loopbacks, and Wi-Fi/Ethernet differences are not classified.
- Counter resets are hidden as zero deltas.
- The API returns metadata plus counters plus rates, but only rates need state.

Correct direction: split interface metadata from traffic counters/rates, add `observed_at`, and encode unsupported fields explicitly.

## Process Probe Review

Current process API:
- `current()` delegates to `get_process(std::process::id())`.
- `get_process(pid)` refreshes one PID through the global sysinfo lock (`process.rs:140-153`).
- `list()` and `stats()` refresh all processes with `ProcessesToUpdate::All` (`process.rs:163-231`).
- `process_from_sysinfo()` hardcodes `thread_count: 1`, `uid: None`, `gid: None` (`process.rs:107-121`).

Risks:
- Sysinfo exposes user/group and Linux tasks, but the wrapper discards them.
- `thread_count == 1` is actively misleading.
- `ProcessMonitor` tracks only PID; PID reuse is not detected because start time/creation identity is not stored.
- `sample()` maps all errors to `None` (`process.rs:361-374`), losing permission denied vs process exited.
- `list()` can be expensive because sysinfo default process refresh includes tasks on Linux.
- Process info includes `exe_path` and `cwd`; command line/env are not exposed today, but privacy rules should be explicit before adding them.

Can Nebula use this for self-monitoring? Only for coarse current-process memory and presence. It is not safe as a complete operator process inventory or sandbox enforcement model yet.

## Error Model Review

`SystemError` has variants for platform, unsupported, not found, permission denied, memory, parse, timeout, and hardware (`core/error.rs:11-44`). The problem is not the enum names; it is that most probe APIs do not use them.

Examples:
- `init() -> SystemResult<()>` currently cannot report a backend failure; it forces lazy initialization, refreshes sysinfo, and returns `Ok(())` (`info.rs:316-327`).
- `memory::current()` cannot report failure; it returns data and may classify unavailable memory as `Low`.
- `cpu::usage()` cannot report invalid first sample or unsupported.
- `disk::list()` returns an empty vector if the module-internal no-feature branch were compiled; missing mount pressure returns `Low`.
- `network::interfaces()` returns an empty vector for no-feature branch and placeholder fields for unsupported metadata.
- `filesystem_info()` and `io_stats()` return `None` with no path/device/error context.
- `ProcessMonitor::sample()` returns `None` for every `get_process` error.

Nebula cannot reliably choose retry/degrade/warn/fail from these results. Add structured status:
- `Available(T)`
- `Unsupported { platform, feature, field }`
- `Unavailable { reason, source }`
- `PermissionDenied { operation, target }`
- `Stale { observed_at, age }`
- `InvalidSample { required_warmup }`

## Observability Fit Review

The crate does not emit metrics, which matches its stated role. The risk is that its data model is not yet safe as a metrics source:
- Pressure enums do not carry reasons or raw evidence, so an alert cannot explain whether pressure came from low available bytes, swap exhaustion, cgroup limits, stale CPU samples, or missing probe data.
- Live readings generally lack timestamps, sample age, backend/source, and validity status.
- Field names document units in comments, but serialized data with the `serde` feature does not carry units or availability status.
- Missing values are often indistinguishable from zero/empty values, so dashboards can show "0" rather than "unsupported".
- Labels/dimensions are not obvious for disk and network data: global disk totals are not tied to a data path; network totals are per-interface in one API and aggregate in another.

For `nebula-metrics`, the minimum usable shape is `*_bytes`/`*_percent` unit-suffixed fields, `observed_at`, `source`, `status`, and reason enums suitable for stable labels.

## Feature Flag Review

Feature definitions:
- `default = ["sysinfo"]`.
- `process = ["sysinfo/system"]`.
- `network = ["sysinfo/network"]`.
- `disk = ["sysinfo/disk"]`.
- `full = ["sysinfo", "process", "network", "disk", "serde"]`.

Local compile results:
- Required combinations passed locally on Windows: default, no-default, sysinfo-only, process-only, network-only, disk-only, serde-only, all-features.
- Linux all-features cross-check passed with a warning.

Risks:
- CI no-default feature checks omit `nebula-system` (`.github/workflows/ci.yml:113-119`).
- Cross-platform smoke tests omit `nebula-system` (`.github/workflows/cross-platform.yml:50-54`).
- The public docs mention APIs/features that do not exist or no longer match: `memory::info()`, `Normal/Warning/Critical`, `cpu::usage() -> SystemResult<f32>`, `component`, `metrics`, `async`, and `minimal` appear in docs but not current code/features.
- Feature-gated modules contain no-feature fallback branches even though the modules themselves are gated, which suggests historical API drift.
- `serde` only compiles while system data falls back to zeros/unknowns; that can serialize misleading snapshots.

## Unsafe Code Review

Crate-level `#![allow(unsafe_code)]` is broader than the actual unsafe needs (`lib.rs:3`). Unsafe appears in:
- Linux CPU affinity: `cpu.rs:423-437`.
- Unix `statvfs`: `disk.rs:346` and `disk.rs:351-362`.

CPU affinity:
- The unsafe comment documents a caller validation invariant for CPU indexes (`cpu.rs:416-421`).
- The safe public function does not enforce that invariant.
- `CPU_SET(cpu, &mut set)` is called for each caller-provided index (`cpu.rs:427-428`).
- OS return codes are checked after `sched_setaffinity`.
- Empty CPU sets and out-of-range CPU indexes are not rejected before unsafe code.

Verdict: Critical. A safe function must not rely on undocumented caller validation for memory safety.

`statvfs`:
- `CString::new(path).ok()?` handles interior NUL.
- The struct is zeroed and passed to `statvfs`.
- Return code is checked, but failure loses errno and path context by returning `None`.

Verdict: Memory safety appears acceptable, but error honesty is weak.

## Missing Invariants

| Invariant | Currently encoded in types? | Currently tested? | Risk |
|---|---:|---:|---|
| Unsupported data must never be represented as a real zero value. | No | No | High: no-sysinfo memory, hardware guesses, missing CPU/disk/network data look valid. |
| Probe snapshots must have clear freshness semantics. | No | Partially | High: `SystemInfo::get()` cached forever, live probes refreshed ad hoc. |
| Memory pressure must use the effective memory limit, not necessarily host memory. | No | No | Critical: container OOM false negatives. |
| CPU capacity must respect effective runtime limits where possible. | No | No | Critical: Kubernetes CPU quota over-scheduling. |
| Disk pressure must be evaluated for the relevant storage path. | No | No | Critical: checkpoint writes can fail despite low global pressure. |
| First CPU sample must not be treated as trustworthy when backend requires warmup. | No | No | High: startup decisions can be wrong. |
| Platform limitations must be explicit in API, docs, or errors. | Partially in docs | No | High: fake fields and empty vectors mislead operators. |
| Feature-gated APIs must compile and behave consistently across feature combinations. | Partially | Locally only | Medium: CI does not enforce the matrix. |
| Unsafe OS calls must have documented and enforced safety invariants. | Documented, not enforced | No | Critical: safe API can violate unsafe preconditions. |
| Process data must not leak secrets accidentally. | No | No | Medium now, High if cmd/env are added. |
| Pressure decisions must include raw evidence and reason codes. | No | No | High: operators cannot explain alerts. |
| Missing disk/network/process data must distinguish not found, unsupported, permission denied, and not implemented. | No | No | High: wrong degradation behavior. |
| Path matching must use canonical path/mount APIs, not string prefixes. | No | No | High: wrong disk selected. |
| Network counter resets must be visible to consumers. | No | No | Medium: dashboards show false zero rate. |
| PID reuse must be detectable for long-lived process monitors. | No | No | High for sandbox monitoring. |

## Real Nebula Scenarios

| Scenario | Current behavior | Caller expectation | What can go wrong | What API should make explicit |
|---|---|---|---|---|
| 1. Memory pressure prevents starting new workflow executions. | `memory::pressure()` uses host `total/available` and fixed thresholds. | Pressure reflects whether this Nebula process can safely allocate. | False low pressure in containers; swap/OOM risk hidden. | Effective limit, available bytes, swap state, reasons, confidence. |
| 2. CPU saturation slows scheduler polling loops. | `cpu::pressure()` averages host CPUs from last sysinfo sample. | CPU pressure reflects effective runtime quota. | 1-core pod on 16-core host looks healthy. | Quota-adjusted usage and valid sample age. |
| 3. Disk low prevents checkpoint persistence. | `pressure(None)` uses aggregate all disks; `pressure(Some)` needs exact mount and missing means Low. | Check the path where checkpoints are written. | Writes fail on separate full mount. | Path-based disk report or typed not-found/unsupported error. |
| 4. Docker container has 2 GB limit but host has 64 GB. | Host memory is used. | Container limit is used. | OOMKilled after accepting too much work. | cgroup-aware effective memory. |
| 5. Kubernetes pod has CPU quota of 1 core on 16-core node. | Host threads and host CPU average are used. | 1 effective CPU. | Overscheduling and throttling. | CPU quota/cpuset capacity report. |
| 6. Network interface data is missing but UI displays "no IPs". | `ip_addresses` is `vec![]`. | Missing vs none distinguished. | Operator diagnoses wrong network issue. | `Availability<Vec<IpAddress>>`. |
| 7. Process `thread_count` is hardcoded but operator trusts it. | Always `1`. | Real thread/task count or unsupported. | False diagnostics. | `Availability<usize>` or remove field. |
| 8. Disk IO stats unavailable on macOS but API returns zero. | `io_stats()` returns `None`; list path docs mention zero stats historically. | Unsupported is explicit. | Dashboards show no IO instead of unsupported. | `UnsupportedPlatform` status. |
| 9. `SystemInfo::get()` called in hot scheduler loop. | Cheap cached Arc clone; values stale. | Fresh snapshot. | Scheduler never sees changing memory. | Rename/cache docs or `snapshot()` API. |
| 10. `init()` not called before `memory::pressure()`. | Lazy globals still initialize; works. | Docs imply init required. | Confusion about startup contracts. | Document optional warmup/idempotence. |
| 11. Sysinfo first CPU sample is meaningless. | API returns it as normal `CpuUsage`. | Invalid/warming state. | Bad initial admission decision. | `CpuSampleStatus::Warming`. |
| 12. macOS memory compression affects used memory. | Not represented. | Pressure explains platform semantics. | False pressure or false safety. | Platform-specific evidence fields/status. |
| 13. Windows reports memory/disk differently than Linux. | Types look identical. | Differences documented/encoded. | Cross-platform dashboards compare unlike values. | Platform support matrix in types/docs. |
| 14. Nebula runs in CI with restricted host probes. | Empty/unknown/defaults may be returned. | Restricted probes are marked unavailable. | CI health looks healthy with zeros. | Permission/unavailable statuses. |
| 15. Nebula runs non-root and process info is partial. | Missing fields become `None` or process not found strings. | Permission-denied vs absent known. | Wrong operator message/retry behavior. | Structured error context. |
| 16. Memory fluctuates around 70/85 percent. | No hysteresis; pressure flips immediately. | Stable control signal. | Scheduling flaps. | Hysteresis/smoothing in engine policy. |
| 17. Network interface restarts. | Counter reset becomes zero rate by `saturating_sub`. | Reset/restart signaled. | Dashboard hides traffic loss/restart. | Counter reset status. |
| 18. PID reused after sandbox worker exits. | `ProcessMonitor` tracks only PID. | Monitor follows original process only. | Samples unrelated process. | PID plus start time/process identity. |
| 19. Root disk healthy, data disk full. | Aggregate pressure can be low. | Data disk pressure. | Checkpoint/persistence failure. | Storage path probe. |
| 20. Safe CPU affinity called with invalid CPU index. | Unsafe block executes with invalid index. | Error before unsafe syscall. | Memory unsafety. | Input validation before unsafe. |

## API Misuse Cases

| Misuse | Why current API permits it | Prevention |
|---|---|---|
| Treating `SystemInfo::get()` as live data. | Name/docs say snapshot; return type lacks timestamp. | Rename to `host_identity()` or add `SystemSnapshot::capture()`. |
| Treating unsupported as zero. | Fields are plain numbers. | `Availability<T>` and typed statuses. |
| Treating empty `ip_addresses` as no IPs. | Plain `Vec`. | `Availability<Vec<IpAddress>>`. |
| Calling CPU probes in hot async loops. | Synchronous functions hide refresh and lock behavior. | Sampler with recommended interval and sample age. |
| Relying on host memory in containers. | No effective-limit concept. | `EffectiveMemory` report. |
| Using global disk pressure for storage safety. | `pressure(None)` is convenient. | Deprecate global pressure for write admission; require path. |
| Assuming first CPU usage is valid. | Return type has no validity status. | `CpuSample { status }`. |
| Using bare pressure enum for hard throttling. | `is_concerning()` makes it easy. | Move policy to engine; report reasons/evidence only. |
| Displaying hardcoded process fields. | Fields look real. | Remove or mark unsupported. |
| Ignoring errors. | Many APIs cannot return errors. | Make failures explicit in return type. |
| Logging process paths without policy. | Process model exposes `exe_path`/`cwd`. | Privacy-safe process projection and docs. |
| Assuming feature modules exist. | Docs list stale features/APIs. | Accurate feature docs and compile examples per feature. |

## Recommended Test Plan

P0: must add before relying on this crate
- Pure memory pressure tests with fake inputs: thresholds, `total=0`, `available>total`, tiny machines, huge machines, swap disabled/full, cgroup limit smaller than host.
- CPU sampler tests with fake backend: first sample invalid, second sample valid after interval, fast polling stale/invalid status.
- Linux cgroup fixture tests for v1/v2 memory and CPU quota/cpuset parsing.
- Disk path tests using temp directories/mocked mount table: exact mount, nested mount, prefix collision, missing path, unavailable mount.
- Unsafe affinity validation tests on Linux for empty and out-of-range CPU lists; property tests for accepted indexes.
- Unsupported value tests: no unsupported field may serialize as a real zero/empty without status.
- Feature matrix in CI: default, no-default, sysinfo-only, process-only, network-only, disk-only, serde-only, all-features.

P1: should add soon
- Platform CI for `nebula-system` on Linux, macOS, Windows with default and all-features.
- Process monitor PID reuse/start-time tests with a fake provider.
- Network counter reset/rate tests and first-sample status tests.
- Serde roundtrip for reports including unsupported/unavailable statuses.
- Error variant coverage for permission denied, unsupported, not found, parse, and backend failure.
- Documentation examples compiled for each feature.

P2: nice to have
- Benchmarks with contention and realistic polling intervals.
- Cross-platform golden snapshots for representative host shapes.
- Integration tests in Docker with memory/CPU limits.
- Tests for filesystem case sensitivity and network/removable disk classification where supported.

## Recommended Refactor Plan

Phase 1: clarify contracts, units, freshness, and unsupported values
- Document current behavior honestly.
- Rename or document `SystemInfo::get()` as cached identity.
- Add `observed_at`/`sample_age` to live snapshots.
- Introduce `Availability<T>` or `ProbeStatus` for unsupported/unavailable fields.
- Remove or deprecate `Low`/zero fallbacks for missing data.

Phase 2: fix pressure classifier and platform honesty issues
- Replace `MemoryPressure` as the scheduling surface with `MemoryPressureReport`.
- Include raw totals, available bytes, swap, effective limits, classifier thresholds, and reasons.
- Remove fake hardware/topology guesses or mark them as estimates.
- Make network/process placeholder fields explicit.

Phase 3: introduce testable provider/sampler abstractions if needed
- Add `SystemProvider` with a `SysinfoProvider`.
- Add `FakeSystemProvider` for deterministic tests.
- Add `CpuSampler` and `NetworkRateSampler` as stateful samplers.
- Keep pure classifier functions independent from sysinfo.

Phase 4: improve container/cgroup correctness
- Add Linux cgroup v1/v2 memory and CPU limit detection.
- Respect cpuset and CPU quota in capacity reporting.
- Mark unsupported/unavailable container status explicitly on macOS/Windows.
- Add Docker/Kubernetes CI scenarios.

Phase 5: improve observability-facing data models
- Add reason codes and labels suitable for `nebula-metrics`.
- Include platform/source/backend metadata.
- Include reset/stale/permission status for rates and process data.
- Add redacted process projections for operator views.

Phase 6: expand platform and feature CI
- Add `nebula-system` to cross-platform smoke.
- Add feature matrix jobs for required combinations.
- Add docs.rs examples per feature.
- Add Linux container-limited integration tests.

## Proposed Canon Invariants

Since `canon-invariants` is currently empty in `crates/system/README.md:6`, these are candidate L2 invariants.

| Proposed Invariant | Why Nebula Needs It | How To Encode | How To Test |
|---|---|---|---|
| `nebula-system` never represents unsupported, unavailable, or permission-denied probe data as a real zero/empty value. | Scheduling and operator health must not confuse "unknown" with "healthy". | `Availability<T>`/`ProbeStatus` in public models. | Fake provider tests for every unsupported field; serde snapshots include status. |
| Every live probe snapshot includes `observed_at` and validity/freshness status. | Engine polling needs to know whether data is fresh enough to act on. | `SystemSnapshot`, `CpuSample`, `NetworkUsage` timestamps/status. | Tests inject stale samples and assert policy layers reject/ignore them. |
| Memory pressure is classified against the effective memory limit when one is known. | Containers and Kubernetes are first-class deployment modes. | `EffectiveMemoryLimit` in pressure input/report. | cgroup v1/v2 fixture tests and Docker-limited integration. |
| CPU capacity reports include effective CPU capacity separate from host logical CPU count. | Scheduler must not oversubscribe CPU-limited pods. | `CpuCapacity { host_logical, cpuset, quota_cores, effective }`. | cgroup CPU quota/cpuset fixture tests. |
| Disk pressure for persistence must be path-specific. | Checkpoints fail based on the storage volume, not aggregate disk state. | `DiskProbe::for_path(Path)` with canonical mount resolution. | Temp/mock mount tests for nested mounts and prefix collisions. |
| CPU usage samples are invalid until backend warmup requirements are met. | Startup scheduling must not trust meaningless CPU readings. | `CpuSampleStatus::Warming/Valid/Stale`. | Fake sampler and sysinfo-backed timing tests. |
| Unsafe OS calls enforce their safety preconditions inside the safe wrapper. | Safe public API must remain memory-safe. | Validation before unsafe blocks; private unsafe primitives. | Linux tests for out-of-range CPU indexes and empty sets. |
| Process monitors identify a process by PID plus creation/start identity where available. | Long-lived monitors must not follow PID reuse. | `ProcessIdentity { pid, start_time }`. | Spawn/exit/reuse fake provider tests. |
| Pressure reports include machine-readable reasons and raw evidence. | Operators must explain resource pressure from logs/metrics. | `PressureReason` enum and raw byte/percent fields. | Snapshot tests for reason coverage. |
| Platform limitations are encoded in API or required docs, not only comments in module internals. | Cross-platform callers need compile-time/runtime honesty. | `PlatformSupport` and per-field status docs. | Docs tests and platform matrix checks. |

## GitHub Issues

### Issue: Make Linux CPU affinity safe wrapper enforce CPU index invariants

Severity: Critical

Fix class: Patch

Body:
The safe public `nebula_system::cpu::affinity::set_current_thread(cpus: &[usize])` enters an unsafe block and calls `CPU_SET(cpu, &mut set)` for caller-provided indexes. The unsafe comment in `crates/system/src/cpu.rs:416-421` says CPU indexes are validated by caller, but the safe function does not validate them before `cpu.rs:427-428`.

This violates Rust safe API expectations and may cause memory corruption for out-of-range CPU indexes. Add validation for non-empty CPU sets and indexes below `CPU_SETSIZE`/effective CPU count before entering unsafe code, or move unchecked behavior behind a private/unsafe primitive. Add Linux tests for invalid CPU indexes.

### Issue: Add effective host/container limits for memory and CPU probes

Severity: Critical

Fix class: Architecture correction

Body:
`nebula-system` reports host memory/CPU capacity through `SystemInfo::current_memory()` and `CpuInfo` but does not account for Docker/Kubernetes cgroup limits. Evidence: memory uses `sys.total_memory()` and `sys.available_memory()` in `crates/system/src/info.rs:126-132`; CPU uses `physical_core_count()` and `cpus.len()` in `info.rs:171-183`.

In a 2 GB pod on a 64 GB node or a 1-CPU quota on a 16-core node, Nebula can over-schedule until cgroup OOM/throttling. Introduce `EffectiveHostLimits` covering cgroup v1/v2 memory, CPU quota, and cpuset. Pressure and capacity reports should use effective limits when known and expose confidence/status.

### Issue: Replace mount/global disk pressure with path-specific disk probe reports

Severity: Critical

Fix class: API correction

Body:
`disk::pressure(Some(...))` requires an exact mount point and returns `DiskPressure::Low` when no disk is found (`crates/system/src/disk.rs:275-284`). `has_enough_space` uses string prefix matching (`disk.rs:313-331`). `pressure(None)` sums all disks (`disk.rs:157-175`), which is not useful for checkpoint/database safety.

Add a path-based disk probe such as `DiskProbe::for_path(path)` returning a report with canonical mount, total/available bytes, status, and errors. Missing/unavailable mounts must never become `Low` pressure.

### Issue: Clarify SystemInfo cache and init freshness semantics

Severity: High

Fix class: Architecture correction

Body:
`SystemInfo::get()` returns a clone of a lazily initialized cached `Arc<SystemInfo>` (`crates/system/src/info.rs:115-117`, `info.rs:144-145`). `init()` forces the cache and refreshes `SYSINFO_SYSTEM`, but does not rebuild the cached `SystemInfo` (`info.rs:316-327`). Docs describe `SystemInfo::get()` as a snapshot and `init()` as required initialization.

Split immutable host identity from live snapshots, add `observed_at`/sample age to live data, and document `init()` as optional/idempotent warmup. Add tests proving cache/freshness semantics.

### Issue: Remove scheduling policy from nebula-system load aggregation

Severity: High

Fix class: Architecture correction

Body:
The project role says `nebula-system` is host probes and should not make scheduling decisions. However `SystemLoad::can_accept_work()` directly rejects work when CPU or memory pressure is High/Critical (`crates/system/src/load.rs:44-51`), and module docs say runtime/engine components can use it to accept more work or shed load (`load.rs:1-5`, `load.rs:26-30`).

Expose pressure evidence only and move admission/backpressure policy into the engine/resource layer. If retained temporarily, mark it advisory and not canonical scheduling policy.

### Issue: Introduce stateful CPU sampler with warmup and sample validity

Severity: High

Fix class: API correction

Body:
`cpu::usage()`, `cpu::pressure()`, and `load::system_load()` call `sys.refresh_cpu_usage()` directly (`crates/system/src/cpu.rs:139-140`, `cpu.rs:197-198`, `load.rs:73-75`). Pinned sysinfo 0.38.4 documents first CPU usage as likely inaccurate and requiring two samples separated by `MINIMUM_CPU_UPDATE_INTERVAL`.

Add `CpuSampler` returning `CpuSample { usage, observed_at, status }`, where status can be `Warming`, `Valid`, or `Stale`. Avoid exposing first-sample data as valid scheduling input.

### Issue: Replace bare MemoryPressure enum with evidence-rich pressure report

Severity: High

Fix class: API correction

Body:
`MemoryPressure` is derived from `(total - available) / total` with fixed thresholds in `crates/system/src/memory.rs:58-87`. It ignores cgroup limits, swap, process RSS, hysteresis, absolute free bytes, macOS compression, and Windows memory differences. It also classifies `total == 0` as 0 percent/Low.

Introduce `MemoryPressureReport` with level, reasons, raw bytes, effective limit, swap state, thresholds, observed time, and classifier version. Keep a compatibility enum only as a lossy view.

### Issue: Stop representing unsupported probe fields as zeros, empty vectors, and fake values

Severity: High

Fix class: API correction

Body:
Several public fields encode unsupported/unavailable data as normal-looking values: no-sysinfo memory zeros (`crates/system/src/info.rs:232-238`), network `ip_addresses: vec![]` (`network.rs:131`), process `thread_count: 1` and `uid/gid: None` (`process.rs:118-120`), disk missing -> `Low` (`disk.rs:275-284`), hardware guesses (`info.rs:260-267`, `cpu.rs:271-280`).

Add `Availability<T>` or `ProbeStatus` to distinguish real zero/none from unsupported, unavailable, permission denied, and not implemented.

### Issue: Make network interface metadata honest and complete where available

Severity: High

Fix class: API correction

Body:
`network::interfaces()` returns `ip_addresses: vec![]`, `is_up: true`, `mtu: None`, `speed: None`, and name-only loopback detection (`crates/system/src/network.rs:128-136`). This makes "not implemented" look like "no IPs" and "always up".

Use sysinfo IP network metadata where available, mark unavailable metadata explicitly, and split interface metadata from traffic counters/rates. Add tests for empty vs unsupported IPs and counter reset status.

### Issue: Make process probe fields truthful and PID-reuse aware

Severity: High

Fix class: API correction

Body:
`process_from_sysinfo()` hardcodes `thread_count: 1`, `uid: None`, and `gid: None` (`crates/system/src/process.rs:107-121`). `ProcessMonitor` tracks only PID and maps all sample errors to `None` (`process.rs:361-374`). Full process refresh can be expensive on Linux because sysinfo includes tasks by default.

Expose unsupported fields explicitly, use sysinfo user/group/tasks where available, track PID plus start time/identity, and preserve error reasons from sampling. Add privacy-safe operator projection before adding command/env fields.

### Issue: Redesign probe error/status model for operator diagnostics

Severity: High

Fix class: API correction

Body:
`SystemError` has variants, but many fallible operations return `Vec`, `Option`, boolean, `Low`, or default data. Examples include `filesystem_info() -> Option`, `io_stats() -> Option`, `network::interfaces() -> Vec`, `memory::current() -> MemoryInfo`, and `ProcessMonitor::sample() -> Option`.

Introduce report/status types that preserve operation, target path/interface/PID, platform, backend/source error, and distinction between unsupported, unavailable, permission denied, not found, stale, and invalid sample.

### Issue: Replace fake hardware/topology defaults with explicit availability

Severity: High

Fix class: API correction

Body:
The crate reports hardcoded or typical hardware data as real values: page size 4096 (`crates/system/src/info.rs:260-263`), cache line 64 (`info.rs:265-267`), huge page size `Some(2MB)` for Linux/Windows (`info.rs:303-312`), typical non-Linux cache sizes (`cpu.rs:271-280`), and `packages: 1` (`cpu.rs:313-315`).

Return measured values only, or encode estimates/unavailable status explicitly. Add platform tests for page size/cache/topology where feasible.

### Issue: Add real load average API or rename load aggregation

Severity: High

Fix class: API correction

Body:
The crate role includes load information if present, but `crates/system/src/load.rs` does not expose OS load average. It exposes a CPU+memory aggregate with admission helpers. There is no `System::load_average()` usage in `crates/system`.

Add a distinct `load_average()` probe where supported, with platform/container caveats, or rename the current module/types to avoid implying OS load average.

### Issue: Add nebula-system to platform and feature CI matrices

Severity: High

Fix class: Documentation/test only

Body:
CI checks workspace all-features on Ubuntu, but no-default checks omit `nebula-system` (`.github/workflows/ci.yml:113-119`). Cross-platform smoke omits `nebula-system` (`.github/workflows/cross-platform.yml:50-54`).

Add `nebula-system` default and all-features tests to Linux/macOS/Windows smoke. Add feature-combination checks for default, no-default, sysinfo-only, process-only, network-only, disk-only, serde-only, and all-features.

### Issue: Introduce provider/fake abstractions for deterministic host-probe tests

Severity: High

Fix class: Refactor

Body:
Core behavior is coupled directly to global `SYSINFO_SYSTEM`, `Disks::new_with_refreshed_list()`, `Networks::new_with_refreshed_list()`, `/sys`, and `statvfs`. This prevents deterministic tests for pressure boundaries, cgroup limits, CPU warmup, disk path mapping, process errors, and unsupported fields.

Add a `SystemProvider` trait with `SysinfoProvider` and `FakeSystemProvider`. Move pressure classifiers into pure functions over raw inputs. Use fakes for P0 contract tests and retain live sysinfo tests as smoke tests.
