# Tier 3 — Quality & Coverage

> Tests, docs, and benchmarks for 6 cross-cutting crates:
> `eventbus`, `telemetry`, `metrics`, `system`, `config`, `log`

**Branch:** `feature/tier-3-quality-coverage`
**Created:** 2026-03-12
**Roadmap tier:** Tier 3 — Quality & Coverage (9 milestones)

---

## Settings

| Setting | Value |
|---|---|
| Testing | Yes — all tasks include test implementation |
| Logging | Verbose — `DEBUG` logs throughout; `tracing::debug!` on all key decision points |
| Docs | Yes — mandatory documentation checkpoint at plan completion |

---

## Roadmap Linkage

**Milestone:** "Tier 3 — Quality & Coverage"
**Rationale:** This plan implements all 9 unchecked Tier 3 items from `.ai-factory/ROADMAP.md`, bringing
5 crates (eventbus, telemetry, metrics, system, config) to full integration test coverage and adding
documented telemetry examples to `nebula-log`.

---

## Codebase Context (Reconnaissance Summary)

### Key Gaps Identified

| Crate | Gap |
|---|---|
| `eventbus` | Zero integration tests; `registry.rs`, `filter.rs`, `scope.rs` have no tests at all |
| `telemetry` | `BufferedRecorder` shutdown drain, channel-full back-pressure, and stress tests missing |
| `metrics` | 14 `NEBULA_RESOURCE_*` constants have zero test coverage; `record_eventbus_stats` edge cases untested |
| `system` | Feature-gated modules (`memory`, `cpu`, `disk`, `network`, `process`) have no tests; no platform support matrix docs |
| `config` | File-watcher polling edge cases (permission-denied → false Deleted, symlinks, rename detection) untested |
| `log` | `nebula-log` + `nebula-telemetry` + OTLP integration example is missing; Sentry config undocumented |

### Architectural Notes (Do Not Fix in This Plan)

- `metrics/adapter.rs`: Typed `record_resource_*()` methods absent — 14 constants present but no typed adapter surface.
  Tracked separately. This plan verifies constants are accessible via generic fallback path.
- `telemetry/service.rs`: `ProductionTelemetry` wraps `BufferedRecorder` but `TelemetryService` impl is only `NoopTelemetry`.
  Full wiring tests for `ProductionTelemetry` are _in scope_ for Tier 3 stress tests.
- `system/disk.rs`: `DiskStats` is defined but `list()` always returns `Default::default()` for I/O counters.
  The platform docs task will document this as a known limitation.

---

## Tasks

### Phase 1: EventBus Quality

---

#### [x] Task 1 — EventBus Integration Test Suite

**Delivers:** `crates/eventbus/tests/integration.rs` with comprehensive integration tests

**Roadmap item:** "EventBus: Integration Test Suite"

**Test scenarios to implement:**

1. **Multi-bus registry tests** (`EventBusRegistry`)
   - `get_or_create` for same key concurrently from 8 threads → all get same `Arc` (`ptr_eq`)
   - `remove` + immediate `get_or_create` race: removed bus stays alive via `Arc`, new call returns fresh bus
   - `prune_without_subscribers` removes buses with zero subscribers; buses with live subscribers survive
   - `stats()` snapshot includes correct aggregate counts across all buses

2. **Concurrent producer/consumer scenarios** (`EventBus`)
   - 16 producers × 4 subscribers: emit 1_000 events each → verify total received across subscribers matches expectations (accounting for lag)
   - `BackPressurePolicy::DropOldest` vs `BackPressurePolicy::DropNewest` with a slow consumer: verify `dropped_count` increments and `sent_count` equals attempts

3. **Subscriber unsubscribe lifecycle**
   - Drop `Subscriber` while bus is emitting → subsequent emissions succeed; `subscriber_count` decrements
   - `lagged_count` accumulation: send `N > buffer_size` events before calling `recv()` once → `lagged_count == N - buffer_size`
   - `FilteredSubscriber` with a filter matching 0 of N events, then bus dropped → `recv()` returns `None`, no hang
   - `is_closed()` returns `true` after all senders dropped

4. **Back-pressure policy combinations**
   - `DropOldest`: ring-buffer fills → oldest events are evicted → newest events win
   - `DropNewest`: ring-buffer fills → new events are rejected → oldest events preserved
   - `Block`: not yet implemented in current codebase — add a `#[ignore]` placeholder test with a TODO comment

5. **Graceful shutdown propagation**
   - `EventBusRegistry::clear()` while subscribers hold `Arc<EventBus>` → existing subscribers continue to drain, new `get_or_create` returns a fresh bus
   - `EventBus` dropped while `Subscriber::recv()` is polling → `recv()` returns `None`

**Logging requirements:**
- `tracing::debug!` at start and end of each test (test name + scenario)
- `tracing::debug!` when verifying counts (log expected vs actual)
- Use `tracing_subscriber::fmt().with_test_writer()` in a `test_helpers::init_log()` helper

**Files to create:**
- `crates/eventbus/tests/integration.rs`
- `crates/eventbus/tests/helpers.rs` (shared test utilities: `make_bus`, `init_log`)

**Dependency:** None

---

#### [x] Task 2 — EventBus Subscriber Documentation

**Delivers:** Inline docs update in `src/subscriber.rs`, `src/filtered_subscriber.rs`, `src/registry.rs` + architecture note in `src/lib.rs`

**Roadmap item:** "EventBus: Subscriber Documentation"

**Documentation to add:**

1. **`src/lib.rs`** — add `## Subscriber Lifecycle` section to module-level doc:
   - Explain what happens when a slow subscriber lags: events silently skipped, `lagged_count` incremented
   - Explain buffer overflow recovery: subscriber automatically re-positions to latest event (no reconnect needed)
   - Explain why in-memory only (Phase 2): persistence is deferred to Phase 3 — add `## Architecture Note: Persistence`
   - Add working example: `subscribe()` → `recv()` loop with lag check

2. **`src/subscriber.rs`** — add `# Lifecycle` section to `Subscriber` doc:
   - Drop behaviour: channel closed decrement (no explicit unsubscribe needed)
   - `close(self)` is equivalent to `drop(self)` — exists for semantic clarity
   - Lag recovery: how to detect lag via `lagged_count()`

3. **`src/filtered_subscriber.rs`** — add `# Filter Behaviour` section:
   - Filter discards don't appear in `lagged_count` — only ring-buffer drops are counted
   - Infinite-filter anti-pattern: filter matching 0 events will spin until channel closes

4. **`src/registry.rs`** — add `# Concurrency` section to `EventBusRegistry` doc:
   - Double-checked locking for `get_or_create`
   - Note that `prune_without_subscribers` is best-effort (subscriber created between check and prune survives)

5. **`examples/`** — add `subscriber_patterns.rs` example showing:
   - Basic subscribe + recv loop
   - Filtered subscription
   - Lag monitoring with `lagged_count()`
   - Multi-bus registry pattern

**Logging requirements:** No runtime logging; doc examples should call `tracing::debug!` at key points

**Files to modify:**
- `crates/eventbus/src/lib.rs`
- `crates/eventbus/src/subscriber.rs`
- `crates/eventbus/src/filtered_subscriber.rs`
- `crates/eventbus/src/registry.rs`

**Files to create:**
- `crates/eventbus/examples/subscriber_patterns.rs`

**Dependency:** Can run in parallel with Task 1

---

> **COMMIT CHECKPOINT 1** (after Tasks 1–2)
> ```
> test(eventbus): add integration test suite and subscriber documentation
> ```

---

### Phase 2: Telemetry Quality

---

#### [x] Task 3 — Telemetry Trace Module Tests

**Delivers:** Unit tests covering all trace-related types in `crates/telemetry/`

**Roadmap item:** "Telemetry: Trace Module Tests"

**Test scenarios to implement (in `src/recorder.rs` and `src/trace.rs` `#[cfg(test)]` blocks):**

1. **`CallRecord` edge cases**
   - `CallRecord` with empty `inputs`/`outputs` (`serde_json::Value::Null`)
   - `CallBody::redacted()` → verify `body` field reads `"[REDACTED]"` not original value
   - `CallPayload` with oversized content (>64 KB string) — verify no truncation panic; body stored as-is
   - `CallStatus::Error { code, message }` with empty strings
   - `Duration::ZERO` for `elapsed` field — verify serialises as `0` not NaN/inf

2. **`ResourceUsageRecord` edge cases**
   - All numeric fields at `u64::MAX` / `f64::MAX` — verify `serde_json` serialises without loss
   - Zero-usage record (all zeros) — verify equality and round-trip through JSON

3. **`RecordEntry` enum**
   - `RecordEntry::Call(Box::new(...))` and `RecordEntry::Usage(...)` distinguish via pattern match
   - `RecordEntry` clone if derived; if not, verify `Debug` output doesn't panic on large payloads

4. **`CallStatus` coverage**
   - `is_success()` / `is_error()` predicates for all variants
   - `CallStatus::Timeout` → `is_error() == true`
   - `CallStatus::Cancelled` → verify it's neither success nor hard error (if such semantics exist)

5. **`Recorder` trait**
   - `NoopRecorder::record_usage` and `record_call` are no-ops (no panic, no log)
   - `Arc<dyn Recorder>` is object-safe — verify via `let _: Arc<dyn Recorder> = Arc::new(NoopRecorder);`

6. **`TraceContext` (existing + gaps)**
   - `TraceContext::new()` generates unique `trace_id` + `span_id` on each call
   - W3C `traceparent` round-trip: `format_traceparent()` → `parse_traceparent()` → same IDs
   - Invalid `traceparent` strings (wrong version, wrong length, non-hex) → `Err(...)`

**Logging requirements:**
- `tracing::debug!("testing {:?}", entry)` at start of each scenario in tests

**Files to modify:**
- `crates/telemetry/src/recorder.rs` (add `#[cfg(test)] mod tests`)
- `crates/telemetry/src/trace.rs` (extend existing test block)

**Dependency:** None

---

#### [x] Task 4 — Telemetry Concurrent Stress Tests

**Delivers:** High-throughput concurrent tests in `crates/telemetry/tests/integration.rs`

**Roadmap item:** "Telemetry: Concurrent Stress Tests"

**Test scenarios:**

1. **100+ concurrent emitters, 50+ subscribers** (`src/integration.rs`)
   - Spawn 100 `tokio::task` emitters, each pushing 100 `ExecutionEvent::WorkflowStarted` events
   - Spawn 50 subscribers all listening on the same bus
   - Assert total received across all subscribers ≥ (events_sent × subscribers - lag_budget)
   - Use `tokio::time::timeout(Duration::from_secs(10), ...)` to fail fast on hang

2. **Histogram thread safety under contention** (`MetricsRegistry`)
   - 20 threads each calling `histogram.observe(rng.gen::<f64>())` 10_000 times concurrently
   - After all threads join: verify `histogram.count()` == 200_000 (no lost increments)
   - Verify `histogram.percentile(0.99)` returns a finite `f64` (not NaN)
   - Verify `histogram.sum()` is finite and > 0

3. **`BufferedRecorder` shutdown drain**
   - Start `BufferedRecorder` with `buffer_size=100`, `flush_interval=Duration::from_secs(60)`
   - Send 50 `RecordEntry::Usage(...)` records
   - Call `recorder.shutdown().await`
   - Assert `LogSink` (or mock sink) received exactly 50 entries (no records lost on shutdown)
   - Use `tokio::time::pause()` to skip the 60-second flush interval

4. **BufferedRecorder channel-full drop**
   - Start `BufferedRecorder` with `buffer_size=5`
   - Block the background flush task (do not start it / use `mpsc::channel(0)` variant)
   - Send 10 records via `record_usage()`
   - Assert 5 are accepted, 5 are silently dropped
   - Assert `tracing::warn!` is emitted for each drop (capture via `tracing_subscriber::fmt::TestWriter`)

5. **`ProductionTelemetry` multi-Arc sharing**
   - Create `ProductionTelemetry` (using `NoopTelemetry` until `ProductionTelemetryBuilder` is wired)
   - Wrap in `Arc<dyn TelemetryService>`
   - Clone `Arc` 20 times, each clone calls `metrics().counter("c").inc()` in separate threads
   - Assert final counter value == 20

**Logging requirements:**
- `tracing::debug!("stress test: started {} emitters, {} subscribers", e, s)` before spawn
- `tracing::debug!("stress test: emitter {} complete", id)` per task completion
- `tracing::info!("stress test: total received={}", total)` on assertion

**Files to modify:**
- `crates/telemetry/tests/integration.rs` (append new test functions)

**Dependency:** Task 3 (should be done first to validate individual types before stress testing them)

---

> **COMMIT CHECKPOINT 2** (after Tasks 3–4)
> ```
> test(telemetry): add trace module unit tests and concurrent stress tests
> ```

---

### Phase 3: Metrics Quality

---

#### [x] Task 5 — Metrics Resource Domain Tests

**Delivers:** Tests in `crates/metrics/src/adapter.rs` and new `crates/metrics/tests/integration.rs`

**Roadmap item:** "Metrics: Resource Domain Tests"

**Test scenarios:**

1. **All 14 `NEBULA_RESOURCE_*` constants are accessible**
   - For each constant, verify: `!CONSTANT.is_empty()`, starts with `"nebula_resource_"`, lowercase ASCII only
   - Verify constants are unique (collect into `HashSet`, assert len == 14)
   - Verify each can be used as a key via `MetricsRegistry::counter(CONSTANT)` without panic

2. **`TelemetryAdapter` + resource-domain round-trip** (generic fallback path)
   - Create `MetricsRegistry`, create `TelemetryAdapter`
   - Call `adapter.counter(NEBULA_RESOURCE_CREATE_TOTAL).inc()` → verify `snapshot()` contains the counter
   - Call `adapter.histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS).observe(0.5)` → verify bucket/sum in snapshot

3. **`record_eventbus_stats` edge cases**
   - `sent=0, dropped=0` → `drop_ratio() = 0.0` → `ppm = 0` (NaN guard exercised)
   - `sent=1_000_000, dropped=1_000_000` → `drop_ratio() = 1.0` → `ppm = 1_000_000` (max value)
   - `sent=u64::MAX, dropped=0` → `clamp_u64_to_i64` → counter stored as `i64::MAX`
   - `subscriber_count=usize::MAX` → `clamp_usize_to_i64` → stored as `i64::MAX`
   - `sent=3, dropped=1` → `ppm = 333_333` (fractional ratio, round not truncate)
   - Zero-value `EventBusStats::default()` → no panic, all metrics remain at 0

**Logging requirements:**
- `tracing::debug!("testing constant: {}", CONSTANT)` in constant accessibility loop
- `tracing::debug!("record_eventbus_stats test: sent={} dropped={} ppm_expected={}", ...)` per edge case

**Files to modify:**
- `crates/metrics/src/adapter.rs` (add `#[cfg(test)] mod tests` block)
- `crates/metrics/src/naming.rs` (add `#[cfg(test)] mod tests` block for constant validation)

**Files to create:**
- `crates/metrics/tests/integration.rs`

**Dependency:** None

---

> **COMMIT CHECKPOINT 3** (after Task 5)
> ```
> test(metrics): add resource domain tests and record_eventbus_stats edge case coverage
> ```

---

### Phase 4: System Quality

---

#### [x] Task 6 — System Platform Documentation

**Delivers:** Inline docs update in all 5 feature-gated modules + updated `src/lib.rs`

**Roadmap item:** "System: Platform Documentation"

**Documentation to add:**

1. **`src/lib.rs`** — add `## Platform Support Matrix` section to module-level doc:

   | Module | Linux | macOS | Windows | Notes |
   |---|---|---|---|---|
   | `memory` | ✓ | ✓ | ✓ | Via `sysinfo`; `management` submodule stubs only |
   | `cpu` | ✓ | ✓ | ✓ | Via `sysinfo`; SSE/AVX feature flags x86 only |
   | `disk` | ✓ | ✓ | ✓ | `DiskStats` I/O counters always zero (not populated by sysinfo path) |
   | `network` | ✓ | ✓ | ✓ | `connections()` always returns `[]` (not yet implemented) |
   | `process` | ✓ | ✓ | ✓ | `cmd`, `environ`, `thread_count`, `uid`, `gid` are always default/zeroed |

2. **`src/process.rs`** — add `# Known Limitations` to `ProcessInfo` doc:
   - `cmd: Vec::new()` — not populated (performance: sysinfo arg parsing is per-process and expensive)
   - `environ: HashMap::new()` — skipped (security: environ can be large and sensitive; add opt-in feature if needed)
   - `thread_count: 1` — hardcoded (sysinfo 0.37 does not expose thread count on all platforms)
   - `uid: None, gid: None` — Unix-only fields; Windows returns `None`

3. **`src/network.rs`** — add `# Known Limitations` to module doc:
   - `connections()` is defined in the type system but always returns `[]` — full implementation requires `netstat2` crate (Tier 4)
   - Rate tracking via `NETWORK_STATS` lazy global may not reflect first-tick measurements accurately

4. **`src/disk.rs`** — add `# Known Limitations` to module doc:
   - `DiskStats` (I/O read/write bytes/ops counters) is always `Default::default()` in current implementation
   - `detect_disk_type` maps `HDD`/`SSD` only; `Network`, `Removable`, `RamDisk` → `Unknown`
   - Workaround: read `/sys/block/*/stat` directly on Linux for I/O counters

5. **`src/memory.rs`** — add `# Known Limitations`:
   - `management` submodule functions (`allocate`, `free`) always return `Err("not supported")` — it's a stub placeholder for future WASM memory management hooks

**Logging requirements:** No runtime logging; docs only

**Files to modify:**
- `crates/system/src/lib.rs`
- `crates/system/src/process.rs`
- `crates/system/src/network.rs`
- `crates/system/src/disk.rs`
- `crates/system/src/memory.rs`

**Dependency:** None

---

#### [x] Task 7 — System Integration Tests

**Delivers:** `crates/system/tests/integration.rs` with platform-gated tests

**Roadmap item:** "System: Integration Tests"

**Test scenarios (all gated with feature flags and `#[cfg(target_os)]` as appropriate):**

1. **Memory pressure detection** (`#[cfg(feature = "memory")]`)
   - `memory::current()` returns `MemoryInfo` with `total > 0` and `available <= total`
   - `memory::pressure()` returns one of the four variants without panicking
   - `MemoryPressure::is_critical()` returns `true` only for `Critical` variant
   - `MemoryInfo::usage_percent` is in `[0.0, 100.0]` range

2. **CPU info retrieval** (`#[cfg(feature = "cpu")]`)
   - `cpu::usage()` returns `CpuUsage` with `cores_count >= 1`
   - `cpu::pressure()` does not panic
   - `CpuUsage::average` is in `[0.0, 100.0]`
   - `cpu::features()` on x86_64: `CpuFeatures` struct is populated (at minimum `is_64bit == true`)

3. **Disk stats** (`#[cfg(feature = "disk")]`)
   - `disk::list()` returns at least one `DiskInfo` on any non-empty OS
   - Each `DiskInfo` has `total_space > 0` and `mount_point` is non-empty string
   - `disk::total_usage().usage_percent` is in `[0.0, 100.0]`

4. **Network interfaces** (`#[cfg(feature = "network")]`)
   - `network::interfaces()` returns at least one entry (loopback at minimum)
   - Loopback interface (`is_loopback == true`) identified on Linux/macOS/Windows
   - `#[cfg(not(target_os = "windows"))]` guard for Unix-specific MAC address checks

5. **Process info** (`#[cfg(feature = "process")]`)
   - `process::current()` returns `Ok(ProcessInfo)` for own PID
   - `ProcessInfo.pid` equals `std::process::id()`
   - `process::get_process(0)` on Linux returns error (PID 0 is kernel) or Ok (system process)
   - `process::get_process(u32::MAX)` returns `Err(resource_not_found(...))`

**Notes:**
- All tests should use `#[cfg(not(miri))]` (sysinfo uses system calls incompatible with Miri)
- Use `tracing::debug!` in each test to log the returned values (verbose mode)
- Mark any test that is inherently racy (CPU usage on first call) with `#[cfg(not(ci))]` or `#[allow(..)]` comment

**Logging requirements:**
- `tracing::debug!("memory info: {:?}", info)` in each test before assertions
- `tracing::debug!("test passed: {}", test_name)` at end

**Files to create:**
- `crates/system/tests/integration.rs`

**Dependency:** Task 6 (docs establish what stubs exist; tests should assert known stubs return their documented defaults)

---

> **COMMIT CHECKPOINT 4** (after Tasks 6–7)
> ```
> docs(system): add platform support matrix and known limitations
> test(system): add platform-gated integration tests for all 5 system modules
> ```

---

### Phase 5: Config Quality

---

#### Task 8 — Config Loader Edge Case Tests

**Delivers:** Additional test coverage in `crates/config/tests/` and targeted watcher tests

**Roadmap item:** "Config: Loader Edge Case Tests"

**Test scenarios:**

1. **Deeply nested TOML** (add to `integration_test.rs`)
   - 10-level nested table: `a.b.c.d.e.f.g.h.i.j = "deep"` → `config.get::<String>("a.b.c.d.e.f.g.h.i.j")` returns `"deep"`
   - Array of tables: `[[servers]]` with 50 entries → `config.get::<Vec<...>>("servers")` length == 50

2. **Unicode keys and values** (add to `integration_test.rs` under `#[cfg(test)]`)
   - TOML key with unicode: `"café" = "latté"` → retrieve via exact unicode key
   - JSON value with emoji and CJK characters: `{"title": "日本語テスト 🎉"}` → round-trip without corruption
   - YAML value with null-byte boundary: ensure no truncation for `\u0000` sequences (if supported; else assert error)

3. **Large config files (>1MB)**
   - Generate 10_000-line TOML with unique keys in test fixture (`build.rs` or `tempfile` in test)
   - `FileLoader::load()` succeeds without timeout (enforce with `tokio::time::timeout(5s, ...)`)
   - `Config::get::<String>(last_key)` retrieves correct value

4. **Symlink following in `PollingWatcher`**
   - `#[cfg(unix)]`: Create a real TOML file, symlink it, watch the symlink path
   - Modify the real file → verify `Change` event fires (symlink's target mtime changes)
   - Delete the real file → verify `Deleted` event fires (symlink becomes dangling, metadata fails)

5. **`PollingWatcher` edge cases** (new file `tests/watcher_polling_test.rs`)
   - **Permission denied**: Use `tempfile`, `chmod 000`, watch → subsequent tick emits no false `Deleted`
     (skip on Windows: `#[cfg(unix)]`)
   - **0-byte file**: Create 0-byte file, watch, then write content → mtime changes → `Change` fired
   - **File renamed** (old name seen as Deleted, new name as Created): verify no `Renamed` event emitted
     (document as known limitation matching docs from Task 6-equivalent in system)
   - **Already watching**: call `start_watching()` twice → second call returns `Err`

6. **`FileWatcher` stop-while-in-flight edge case** (`tests/watcher_file_test.rs`)
   - Start `FileWatcher`, emit 5 events, call `stop_watching()` mid-delivery
   - Assert `stop_watching()` returns `Ok(())` without hang
   - Assert `is_watching()` is `false` after stop
   - No panic if further events from `notify` arrive after stop (channel send error is silently dropped)

**Logging requirements:**
- `tracing::debug!("loading config from: {:?}", path)` in each test before load
- `tracing::debug!("watcher event: {:?}", event)` on each received event

**Files to modify:**
- `crates/config/tests/integration_test.rs` (add sections 1–3 above)

**Files to create:**
- `crates/config/tests/watcher_polling_test.rs`
- `crates/config/tests/watcher_file_test.rs`
- `crates/config/tests/fixtures/large_config.toml` (if generated statically) or generate in test

**Dependency:** None

---

> **COMMIT CHECKPOINT 5** (after Task 8)
> ```
> test(config): add edge case tests for nested TOML, Unicode, large files, and file watchers
> ```

---

### Phase 6: Log Quality

---

#### Task 9 — Log Telemetry Integration Examples

**Delivers:** New examples and documentation linking `nebula-log` + `nebula-telemetry` + optional OTLP/Sentry

**Roadmap item:** "Log: Telemetry Integration Examples"

**Deliverables:**

1. **`crates/log/examples/telemetry_integration.rs`** — documented end-to-end example:
   ```
   nebula-log init → nebula-telemetry NoopTelemetry → EventBus subscription → emit ExecutionEvent →
   assert subscriber received it + log output captured
   ```
   - Self-contained: uses `NoopTelemetry` so no real metrics backend required
   - Shows how to wire `nebula-log`'s `ObservabilityHook` with `nebula-telemetry`'s `ExecutionEvent`
   - Comment blocks explaining each step
   - Builds with `cargo run --example telemetry_integration -p nebula-log`

2. **`crates/log/examples/otlp_setup.rs`** — configuration-only example (marked `no_run`):
   - Shows how to enable `nebula-log` OTLP feature flag in `Cargo.toml`
   - Shows `LoggerBuilder` with `.with_otlp_endpoint("http://localhost:4317")` (doc-comment only if API not yet stabilised)
   - Links to `crates/log/docs/Integration.md` for full rationale
   - Must compile with `cargo check --example otlp_setup -p nebula-log --features otlp` (or mark `no_run`)

3. **`crates/log/examples/sentry_setup.rs`** — Sentry configuration example:
   - Shows `LoggerBuilder::with_sentry_dsn("https://...")` (or equivalent API)
   - Explains `SENTRY_DSN` env var fallback
   - Documents which event types are forwarded to Sentry (WARN/ERROR only by default)
   - Marks as `no_run` (requires live Sentry DSN)

4. **`crates/log/docs/Integration.md`** — update existing file with:
   - `## nebula-log + nebula-telemetry` section: wiring steps, event bus subscription pattern
   - `## nebula-log + OpenTelemetry OTLP` section: feature flags, collector endpoint config, trace correlation
   - `## nebula-log + Sentry` section: DSN setup, filter policy, breadcrumb forwarding
   - `## Feature Flags Reference` table: each feature flag with what it enables and its dependencies

5. **`crates/log/src/lib.rs`** — verify both `///` Quick Start examples are valid doctests:
   - If they call `auto_init()`, ensure `tracing::dispatcher::has_been_set()` guard is present
   - If not already `no_run`, run `cargo test --doc -p nebula-log` to confirm they compile and pass

**Logging requirements:**
- All examples use `tracing::debug!` and `tracing::info!` to demonstrate log output
- Examples should show actual log output in doc comments (`# Output:` section)

**Files to create:**
- `crates/log/examples/telemetry_integration.rs`
- `crates/log/examples/otlp_setup.rs`
- `crates/log/examples/sentry_setup.rs`

**Files to modify:**
- `crates/log/docs/Integration.md`
- `crates/log/src/lib.rs` (doctest verification only — minimal changes)

**Dependency:** None (can start independently; no dependency on prior tasks)

---

> **COMMIT CHECKPOINT 6** (after Task 9 + docs checkpoint)
> ```
> docs(log): add telemetry integration examples and update Integration.md
> ```

---

## Docs Checkpoint (Mandatory)

After all tasks are complete, run:

```bash
cargo doc --no-deps --workspace 2>&1 | grep -E "^warning:"
```

Verify zero doc warnings across all 6 crates. Then route any doc changes through the docs pipeline.

---

## Commit Plan

| Checkpoint | After | Message |
|---|---|---|
| 1 | Tasks 1–2 | `test(eventbus): add integration test suite and subscriber documentation` |
| 2 | Tasks 3–4 | `test(telemetry): add trace module unit tests and concurrent stress tests` |
| 3 | Task 5 | `test(metrics): add resource domain tests and record_eventbus_stats edge case coverage` |
| 4 | Tasks 6–7 | `docs(system): platform matrix; test(system): platform-gated integration tests` |
| 5 | Task 8 | `test(config): add edge case tests for nested TOML, Unicode, large files, and file watchers` |
| 6 | Task 9 + docs | `docs(log): add telemetry integration examples and update Integration.md` |

---

## CI Checklist

Before opening a PR, verify:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
```

Additionally, run:
- `cargo test -p nebula-eventbus` — verify all 25 unit + new integration tests pass
- `cargo test -p nebula-telemetry` — verify all 44 unit + 6 integration + new stress tests pass
- `cargo test -p nebula-metrics` — verify coverage of all 14 resource constants
- `cargo test -p nebula-system` — verify platform-gated tests run on current OS
- `cargo test -p nebula-config` — verify all 95+ tests pass including watcher edge cases
- `cargo test -p nebula-log` — verify examples compile; run `cargo test --doc -p nebula-log`
