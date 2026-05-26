# PR3 (OTLP) — Reviewer Report

**Commits reviewed:** `ddcae4dc`, `5890d699`, `799b401844a35dd6d39674bb2d52e3dd735dc94e`
(task brief named the third commit as `c08fd670`; the actual HEAD on
`feat/api-otlp-e2e` is `799b4018`. Worker report cites the same stale SHA.
Both refer to the same change set per `git diff 85be16e2..HEAD --stat`; the
hash discrepancy is metadata, not a code drift.)

**Verdict:** **request-changes** (one HARD blocker + one operator-stack
correctness blocker; both small fixes).

## Verdict justification

The Rust implementation is solid: env-gated trace install mirrors the
`nebula-log` convention, the OTLP metrics module is a clean flat seam over
`MetricsRegistry::snapshot_*` per ADR-0046, shutdown discipline is correct
both on the happy path and on the subscriber-already-installed edge, no
`unwrap`/`expect`/`panic!` escape outside `#[cfg(test)]`, all four feature
combos (`-p nebula-api` ±postgres, `-p nebula-metrics`, `-p nebula-server`)
build clippy-clean with `-D warnings`, and the hermetic integration test
runs in 0.57s against in-memory OTel exporters. **However**, two issues
block: (1) a planning-vocab leak in the newly-added test file —
`crates/api/tests/otlp_one_root_span.rs:15` cites `docs/plans/recon/m3-otlp-state.md`
which is exactly the pattern PR2 cleanup made a HARD zero-tolerance rule;
and (2) `deploy/docker/otel-collector-config.yaml` exports OTLP-encoded
traces to `jaeger:14250`, but Jaeger 1.62's port 14250 speaks the Jaeger
native `model.proto` gRPC (not OTLP), and `COLLECTOR_OTLP_ENABLED=false`
explicitly disables Jaeger's OTLP receivers — the operator validation path
in `deploy/docker/README.md` (Jaeger UI shows the span) cannot succeed as
configured. Both fixes are one-line edits.

## Blocker findings (must fix before push)

- **B1 — Planning-vocabulary leak in test file (project HARD rule).**
  `crates/api/tests/otlp_one_root_span.rs:15` in the module docstring:
  > `//! The recon (`docs/plans/recon/m3-otlp-state.md`) calls for an in-process collector mock.`
  This violates the zero-tolerance rule established in PR2 cleanup ("plan
  paths" and `M3.x` are explicitly forbidden). The reference to a plan path
  AND the `m3-` token both trip the rule. Fix: drop the sentence or
  rephrase to a vendor-neutral statement (e.g. "the test asserts the
  end-to-end propagation contract using in-memory OTel exporters; the
  reference operator path is documented in `deploy/docker/README.md`"). No
  ADR/canon citation needed.

  Evidence (touched-files-only scan, planning glob from task brief):
  ```
  $ grep -iEn 'docs/plans|recon|oracle\b|\bPR[0-9]|\bM[0-9]\.|wave [0-9]|box [0-9]|verdict|§Risk' <16 in-scope files>
  crates/api/tests/otlp_one_root_span.rs:15://! The recon (`docs/plans/recon/m3-otlp-state.md`) calls for an in-process collector mock.
  ```
  All other hits are either pre-existing (out of PR3 scope —
  `crates/api/tests/trace_w3c_smoke.rs`, `crates/api/src/transport/webhook/mod.rs`,
  `crates/engine/src/runtime/runtime.rs`, last touched by #695 / sandbox plan)
  or are the literal word "Backends" in `crates/metrics/src/otlp.rs:40` (not
  planning vocab). The PR3-introduced file is the only new offender.

- **B2 — Operator stack: OTLP exporter pointed at Jaeger's non-OTLP port.**
  `deploy/docker/otel-collector-config.yaml:28-31`:
  ```yaml
  otlp/jaeger:
    endpoint: jaeger:14250
    tls:
      insecure: true
  ```
  In `jaegertracing/all-in-one:1.62`, port `14250` is the Jaeger native
  `model.proto` gRPC collector (used by `jaeger-agent` and the deprecated
  `jaeger` otelcol exporter). It does **not** accept OTLP-encoded data. The
  `otlp` exporter type in otelcol-contrib speaks OTLP. The compose stack
  also sets `COLLECTOR_OTLP_ENABLED: "false"` on Jaeger
  (`deploy/docker/docker-compose.observability.yml:38-41`), which disables
  Jaeger's `4317`/`4318` OTLP receivers. Net effect: spans leave the
  collector, hit `jaeger:14250` as OTLP, get rejected, never appear in the
  UI. The "Open `http://localhost:16686`, … the span should appear within
  ~1s" step in `deploy/docker/README.md:33-39` cannot succeed.

  Also relevant: the otelcol-contrib `jaeger` exporter (which DID speak
  Jaeger native gRPC) was deprecated in v0.85 and **removed in v0.105**; this
  PR pins `otel/opentelemetry-collector-contrib:0.111.0`, which no longer
  ships it. So "switch back to the `jaeger` exporter" is not a viable fix.

  Fix (minimal): enable OTLP on Jaeger and target its OTLP port.
  - In `docker-compose.observability.yml`: remove
    `COLLECTOR_OTLP_ENABLED: "false"` (or set it to `"true"`) and expose/use
    Jaeger's `4317` internally; the host port mapping for `4317` already
    belongs to the collector, so simply do not publish Jaeger's `4317` on
    the host — let the collector reach it through the compose network.
  - In `otel-collector-config.yaml`, change the exporter to:
    ```yaml
    otlp/jaeger:
      endpoint: jaeger:4317
      tls:
        insecure: true
    ```
  - In `deploy/docker/README.md`, drop the `14250` row in the ports table
    (or relabel as "internal collector → jaeger OTLP gRPC :4317").

  Worker self-report says "`task obs:up` succeeds (smoke-test by hand once
  locally; document in README)". Either the hand smoke-test was never run
  end-to-end through the Jaeger UI, or the verification step asserted only
  "containers up", not "spans visible". Either way the operator path the
  PR ships is broken.

## Nit findings (nice-to-fix; not blocking)

- **N1 — `deploy/docker/README.md` claims an "in-process gRPC stub".**
  Lines 46-52 say the test "exercises … against an in-process gRPC stub".
  The test actually uses `opentelemetry_sdk::testing`'s
  `InMemorySpanExporter` and `InMemoryMetricExporter` — pure in-memory, no
  gRPC anywhere in the test path. Worker's own report correctly describes
  this; the README drifted. Recommend: "against in-memory OTel exporters
  (no collector required)".

- **N2 — `deploy/docker/README.md` claims test reads `OTEL_EXPORTER_OTLP_ENDPOINT`.**
  Lines 49-52: "Operators who want to validate against the real
  otelcol-contrib + Jaeger should set `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317`
  in addition to `OTEL_E2E_TEST=1`; the test prefers its own in-process
  collector when both are present". The test code in
  `crates/api/tests/otlp_one_root_span.rs` never reads
  `OTEL_EXPORTER_OTLP_ENDPOINT` — it always uses the in-memory exporters.
  Setting that env var has no effect on the test. Recommend: remove the
  "in addition to … prefers its own" sentence; point operators at the
  binary path (`OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 cargo run
  -p nebula-server`) for collector validation, not at the test.

- **N3 — Trace-id assertion (2) is tautological.**
  `crates/api/tests/otlp_one_root_span.rs:243-256` builds a set of trace
  ids from the spans **already filtered by `trace_id == inbound_trace_id`**,
  then asserts the set has cardinality 1. After the filter that is
  vacuously true for any non-empty input, and assertion (1) already
  guarantees non-empty. The comment ("all spans sharing the inbound trace
  id form one root tree") and the assertion don't match the production
  contract that "no engine-emitted span drifts onto a fresh trace id".
  Stronger and aligned with the worker's own concern: assert
  `spans.iter().all(|s| format!("{:032x}", s.span_context.trace_id()) ==
  inbound_trace_id)` after first asserting `spans.len() > 0` (or > 1 if
  you want the chain assertion to bite). Worker's report flagged this in
  open-question (e).

- **N4 — Redundant `nebula-metrics` dev-dep duplicate.**
  `crates/api/Cargo.toml:25` already lists `nebula-metrics` as a normal
  dep; line 138 re-lists it as a dev-dep. Cargo tolerates this (dev-deps
  union with normal deps for tests), but it is noise and will confuse the
  next reader. Drop line 138.

- **N5 — `install_with_exporter` is unconditional public surface.**
  Worker's open-question (d). Acceptable as-is (see Decision below), but
  consider `#[doc(hidden)]` (not `cfg(test)`) so it stays linkable from
  out-of-crate test harnesses without inflating the rendered rustdoc
  surface for production consumers.

- **N6 — `OTLP_ENDPOINT_ENV` resolution duplication.**
  `crates/api/src/telemetry_init.rs::resolve_otlp_endpoint` is invoked
  TWICE per startup: once inside `build_tracer_provider` (Wave 1), and
  again inside `TelemetryGuard::attach_metrics_exporter` (Wave 2). Same
  env, same normalisation. If an operator's env changes between the two
  calls (extremely unlikely but possible under tooling that mutates env
  mid-process) the trace and metrics exporters can disagree. Trivial fix:
  resolve once in `init_api_telemetry` and pass into `attach_metrics_exporter`,
  or cache via `OnceLock`. Not blocking — current behaviour is well-defined
  for any sane invocation.

- **N7 — `Duration::from_mins(1)` is currently stable but unusual.**
  `crates/api/src/telemetry_init.rs:65` and `crates/metrics/src/otlp.rs:73`
  use `Duration::from_mins`. This is in stable Rust today (MSRV 1.95) but
  using `Duration::from_secs(60)` matches the rest of the codebase and
  reads identically. Cosmetic only.

## Planning-vocabulary check

`rg` over the 16 in-scope files (excluding `docs/plans/`):

```
crates/api/tests/otlp_one_root_span.rs:15://! The recon (`docs/plans/recon/m3-otlp-state.md`) calls for an in-process collector mock.
crates/metrics/src/otlp.rs:40://! Backends that consume the OTLP / Prometheus convention reconstruct the histogram from these
```

- Line 1 = **B1 above** (BLOCKER).
- Line 2 = false positive: the word "Backends" is a generic term about
  monitoring backends consuming OTLP/Prometheus output. Not planning vocab.

Pre-existing references in `crates/api/tests/trace_w3c_smoke.rs`,
`crates/engine/src/runtime/runtime.rs`, `crates/api/src/transport/webhook/mod.rs`
last touched by #695 / earlier sandbox work — out of PR3 scope; not new
debt introduced by these commits.

## Per-wave audit

### Wave 1 — Traces exporter (`crates/api/src/telemetry_init.rs`)

- **Env-gating correctness:** `normalise_otlp_endpoint`
  (`telemetry_init.rs:294-302`) accepts empty, whitespace-only, and the
  case-insensitive literal `"disabled"` as opt-out. Matches the convention
  in `nebula_log::telemetry::otel::resolve_endpoint_from`. Three unit tests
  (`telemetry_init.rs:307-329`) pin the behaviour. ✓
- **`SpanExporter` build path:**
  `opentelemetry_otlp::SpanExporter::builder().with_tonic().with_endpoint(...)`
  at `telemetry_init.rs:259-264`. Workspace pin
  `opentelemetry-otlp = { version = "0.31.1", features = ["grpc-tonic",
  "trace", "metrics"] }` (`Cargo.toml:153`) supplies the required cargo
  features. `cargo check -p nebula-log` passes — no side-effect from the
  workspace feature addition. ✓
- **Batch vs simple exporter:** `telemetry_init.rs:234-248` selects
  `with_batch_exporter` when `tokio::runtime::Handle::try_current().is_ok()`,
  otherwise `with_simple_exporter`. Mirrors the `nebula-log` runtime-detect
  fallback. ✓
- **`TelemetryGuard` shutdown discipline:**
  - `Drop` calls `shutdown` (`telemetry_init.rs:160-164`). ✓
  - `shutdown` flushes metrics first, then traces
    (`telemetry_init.rs:140-156`). ✓
  - Subscriber-install-failure path: `telemetry_init.rs:200-216` shuts the
    just-built provider down immediately when `try_init` returns `Err`,
    avoiding a leaked batch processor task. Matches the
    `nebula_log::telemetry::otel::shutdown_unused_provider` edge. ✓
- **`OTEL_SERVICE_NAME` default:** `resolve_service_name` at
  `telemetry_init.rs:277-290` defaults to `"nebula-api"`. Empty values fall
  back to the default. ✓
- **Double-install guard:** `try_init` (not `init`) is used, so a second
  call after a test pre-installed a subscriber returns quietly instead of
  panicking — and the just-built provider gets shutdown so no batch
  processor leaks. ✓

### Wave 2 — Metrics exporter (`crates/metrics/src/otlp.rs` + composition)

- **Flat module layout (ADR-0046):** single new file
  `crates/metrics/src/otlp.rs` (595 LOC inc. tests/doc), `pub mod otlp;` at
  `crates/metrics/src/lib.rs:43`. No submodule tree. ✓
- **Documented public entry point:** `OtlpMetricsExporter::install(registry,
  config) -> Result<OtlpMetricsGuard, OtlpInitError>` at
  `crates/metrics/src/otlp.rs:215-226`. ✓
- **`MetricsRegistry::snapshot_*` signatures unchanged:**
  `git diff 85be16e2..HEAD -- crates/metrics/src/registry.rs` returns
  empty. ✓
- **Histogram decomposition:** `_sum` as `f64_observable_counter`, `_count`
  as `u64_observable_counter`, `_bucket` as `u64_observable_counter` with
  `le` attribute (`otlp.rs:439-525`). Cumulative-per-labelset semantics
  hold (`Histogram::snapshot().cumulative_buckets()` already returns
  cumulative counts per labelset; OTel counter contract requires
  monotonic-per-attribute-set, which cumulative bucket counts satisfy as
  the `le` upper bound varies — every `(name, labels, le)` tuple is its
  own series, and within each, the value is monotonic). ✓
- **`+Inf` handling:** `otlp.rs:506-513` renders the unbounded bucket as
  the literal string `"+Inf"`. Matches the Prometheus text-format
  convention (Prometheus exposition uses `+Inf`). ✓
- **Discovery interval floor:** `spawn_discovery_task` (`otlp.rs:316-324`)
  uses `export_interval / 2` with a 1s floor — prevents tight loops on
  absurdly short test intervals. ✓
- **Dedup:** `(name, role)` is inserted into the `seen` HashSet inside the
  `Mutex` (`otlp.rs:351-357`). A poisoned mutex degrades to "no new
  registrations" — safer than risking a double-register on a panicked
  thread. ✓
- **`LabelAllowlist::apply` invocation:** `ExporterInner::build_attributes`
  (`otlp.rs:189-201`) calls `allowlist.apply(labels, interner)` before
  emitting any KeyValue. Called from every observable callback (counter,
  gauge, histogram-sum, histogram-count, histogram-bucket). Unit test
  `cardinality_allowlist_strips_unlisted_keys_before_emission`
  (`otlp.rs:573-583`) pins the behavior. ✓
- **`OtlpInitError` is `thiserror`-derived:** `otlp.rs:144-148`. No
  `Box<dyn Error>` escape. ✓
- **Composition root wiring:**
  - `Arc<MetricsRegistry>` constructed once in
    `compose.rs::ServerRuntime::run_transport` (`compose.rs:152`).
  - Attached via `state.with_metrics_registry(metrics_registry)` at
    `compose.rs:267-269` AND
    `guard.attach_metrics_exporter(Arc::clone(&metrics_registry))` at
    `compose.rs:153-156`. Same `Arc` → same source of truth between
    Prometheus `/metrics` handler and OTLP push pipeline. ✓
  - `ServerRunError::MetricsExporter(OtlpInitError)` variant
    (`compose.rs:41-49`) — typed propagation, no silent fallback. ✓
- **`main.rs` guard lifetime:** `apps/server/src/main.rs:22-34` builds the
  guard before `Cli::parse()`, moves it into `run_transport`. The guard is
  bound to the parameter on `ServerRuntime::run_transport`
  (`compose.rs:144-146`); it lives until the function returns (after
  `app::serve(app, bind_address).await?` at `compose.rs:184`). `Drop` runs
  before the `main` return. ✓
- **Direct `nebula-metrics` dep on `apps/server`:** justified — the binary
  now constructs the registry directly (`compose.rs:152`). See N4 for the
  redundant dev-dep duplicate inside `crates/api/Cargo.toml`.

### Wave 3 — Compose stack + integration test

**Compose stack:**

- Image tags exist on Docker Hub:
  - `otel/opentelemetry-collector-contrib:0.111.0` — released
    2024-09-26, real.
  - `jaegertracing/all-in-one:1.62.0` — released 2024-09-26, real. ✓
- YAML is syntactically valid (`docker compose config -f
  deploy/docker/docker-compose.observability.yml` not run from this
  worktree, but `yaml.safe_load` semantics are satisfied — there are no
  duplicate keys, tabs, or inconsistent block-style indents). ✓
- **`otel-collector-config.yaml` exporter mismatch — see B2.** ✗
- `deploy/.env.example`: `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317`
  matches what `telemetry_init.rs::resolve_otlp_endpoint` reads (env var
  name constant at `telemetry_init.rs:51`). `NEBULA_METRICS_OTLP_INTERVAL_SECS=10`
  matches the constant at `telemetry_init.rs:55`. ✓
- `Taskfile.yml:286-289` `obs:up` targets `deploy/docker/docker-compose.observability.yml`,
  which is the file added by this PR. Wiring lines up. ✓
- `deploy/docker/README.md`: see N1, N2 for documentation drift; B2 for the
  Jaeger UI verification step that won't actually work.

**Integration test (`crates/api/tests/otlp_one_root_span.rs`):**

- **Gate:** `OTEL_E2E_TEST=1`. `e2e_enabled()`
  (`otlp_one_root_span.rs:54-65`) returns false for unset, `""`, `"0"`,
  `"false"` (case-insensitive). Early-return path
  (`otlp_one_root_span.rs:69-74`) skips with `eprintln!`. ✓ Verified —
  `cargo nextest run -p nebula-api` shows the test as "skipped"
  (442/442 + 1 skipped on the postgres run).
- **In-memory exporter approach:** `InMemorySpanExporter::default()` +
  `InMemoryMetricExporter::default()` from
  `opentelemetry_sdk::testing` (dev-only via `opentelemetry_sdk =
  { workspace = true, features = ["testing", "metrics"] }` in
  `crates/api/Cargo.toml:142`). Cheap, hermetic. ✓ Accept (see Decision 1).
- **Runtime claim:** worker says "<1s end-to-end". I measured
  `OTEL_E2E_TEST=1 cargo test -p nebula-api --test otlp_one_root_span` at
  0.57s on this Windows host (Rust 1.95, target dir warm). ✓
- **Engine seam detached shutdown:** `drop(tokio::spawn(async move {
  engine_seam.shutdown().await; }))` at `otlp_one_root_span.rs:215-217`.
  In a `#[tokio::test]`, the orphan task is bounded by the test runtime's
  lifetime (`Runtime::drop` aborts all tasks). No leak across tests
  (each `#[tokio::test]` in its own integration-test file uses its own
  process under cargo's default scheduler; nextest enforces per-test
  isolation). ✓ Accept (see Decision 3).
- **Assertion strength:** see N3.
- **Subscriber install hygiene:** `tracing_subscriber::registry().with(otel_layer).try_init()`
  at `otlp_one_root_span.rs:97`. The integration-test file is its own
  binary under cargo, so no other subscriber is pre-installed. If
  `try_init` were to fail, assertion (1) would catch it (zero matching
  spans). ✓

## Worker deviation decisions

1. **In-memory exporter for integration test: ACCEPT.**
   The recon's "real in-process gRPC collector mock" alternative would have
   added ~300 LOC of fixture code reproducing what `opentelemetry_sdk`'s
   `testing` feature already provides on the trace side. The test asserts
   the registry → OTel observable bridge and the propagation contract end
   to end; an in-memory exporter is the smallest fixture that does this
   honestly. The reference operator path against real otelcol-contrib +
   Jaeger is still documented in `deploy/docker/README.md` (modulo B2 / N1
   / N2). No reason to over-engineer.

2. **`attach_metrics_exporter` separate from `init_api_telemetry`: ACCEPT.**
   The hybrid third option is correct. `init_api_telemetry()` stays
   argument-free so existing test/example call sites don't churn, and the
   composition root that already owns `Arc<MetricsRegistry>` (Prometheus
   exporter shares it) explicitly attaches the OTLP metrics pipeline.
   Wires the trace and metric pipelines through the same `TelemetryGuard`
   for unified shutdown — clean. The plan named both alternatives; this
   composition is honest about the dependency direction (the binary owns
   the registry, the library only owns the trace pipeline).

3. **Engine seam detached shutdown: ACCEPT WITH NOTE.**
   Safe in this `#[tokio::test]` because the runtime aborts orphan tasks
   on drop. The slow action's 30s sleep doesn't outlive the test process,
   doesn't affect other test binaries (separate processes under cargo),
   and the trace pipeline's assertions complete before the seam's
   shutdown signal even propagates. The comment at
   `otlp_one_root_span.rs:206-215` correctly identifies the trade-off:
   terminate cancellation isn't routed into the action ctx in the seam
   harness, so awaiting `engine_seam.shutdown()` would wait the full 30s.
   The detach is correct given that gap. Follow-up (not for this PR):
   route terminate cancellation into the action ctx in the seam helper
   so future tests can shut down cleanly.

## Worker open-question decisions

4. **Feature-gate `install_with_exporter`: NO.**
   Rationale: the install logic is identical regardless of which
   `PushMetricExporter` is supplied; gating behind `cfg(test)` or a
   `testing` feature would force a duplicate cfg-branch API surface for
   nothing. An operator who wants a custom on-disk sink can supply one,
   and that's a legitimate (if rare) operator path. Mitigation if surface
   bloat becomes a concern: `#[doc(hidden)]` on the method. Strong-typed
   surface today; no urgency to gate.

5. **Tighten "all spans share one trace id" assertion: YES.**
   See N3. The current assertion is tautological after filter. Recommend
   strengthening to: `spans.iter().all(|s| format!("{:032x}",
   s.span_context.trace_id()) == inbound_trace_id)` so a regression that
   drops some engine-emitted span onto a fresh trace id fails the test
   instead of silently passing. Nit because the existing assertion (1) is
   the load-bearing contract; the chain-integrity assertion just needs to
   match its claimed semantics. Not a blocker.

6. **Engine seam separate `MetricsRegistry::new()`: ACCEPT.**
   The seam in `crates/api/tests/common/mod.rs:1018` constructs its own
   `MetricsRegistry::new()` for the engine's `ActionRuntime`. Pre-existing
   pattern, last touched by #695 — `git log -1` on `common/mod.rs` returns
   `a3f0ec9b`, no PR3 diff. Consolidating the engine and API registries is
   a sensible follow-up (the engine seam should observe the same counters
   the API exposes for cross-layer correlation), but it's beyond PR3
   scope and would expand the diff materially without changing any
   asserted contract.

## Re-run evidence

All commands run from `C:/Users/vanya/RustroverProjects/nebula/.worktrees/otlp-e2e`.

- `cargo fmt -p nebula-api -p nebula-metrics -p nebula-server -- --check`: **pass** (exit 0, no output).
- `cargo clippy -p nebula-api --all-targets -- -D warnings`: **pass** (Finished, no warnings).
- `cargo clippy -p nebula-api --all-targets --features postgres -- -D warnings`: **pass**.
- `cargo clippy -p nebula-metrics --all-targets -- -D warnings`: **pass**.
- `cargo clippy -p nebula-server --all-targets -- -D warnings`: **pass**.
- `cargo nextest run -p nebula-api`: **437 passed, 1 skipped** (matches worker claim). New test
  `nebula-api::otlp_one_root_span::otlp_one_root_span_across_api_control_queue_engine_action`
  is correctly skipped when env unset.
- `cargo nextest run -p nebula-api --features postgres`: **442 passed, 1 skipped** (matches).
- `cargo nextest run -p nebula-metrics`: **77 passed, 0 skipped** (matches). New OTLP tests
  present: `otlp::tests::config_defaults_to_cumulative_temporality_and_60s_interval`,
  `otlp::tests::config_builder_overrides_apply`,
  `otlp::tests::cardinality_allowlist_strips_unlisted_keys_before_emission`,
  `otlp::tests::build_attributes_appends_extras_after_resolved_pairs`,
  `otlp::tests::install_then_shutdown_is_safe`.
- `cargo test -p nebula-server`: **3 passed** (matches).
- `OTEL_E2E_TEST=1 cargo test -p nebula-api --test otlp_one_root_span`: **1 passed**, finished
  in **0.57s** (matches worker's "<1s" claim).
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps`: **pass**.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-metrics --no-deps`: **pass**.
- `rg "unwrap\(\)|expect\(|panic!"` in
  `crates/api/src/telemetry_init.rs`, `crates/metrics/src/otlp.rs`,
  `apps/server/src/compose.rs`, `apps/server/src/main.rs` outside tests:
  **0 hits**. (All raw hits live inside `#[cfg(test)] mod tests {}` blocks
  in `otlp.rs` + `compose.rs`, or are the new `expect` lint attribute on
  the existing `ContextFactory` variant — not the `expect()` method.)
- `cargo check -p nebula-log`: **pass** (workspace OpenTelemetry feature
  additions did not break `nebula-log`'s existing `["rt-tokio"]` /
  `["grpc-tonic", "trace"]` consumption — features are additive).
- `cargo deny check`: **advisories ok, bans ok, licenses ok, sources ok.**
  Pre-existing unmatched-wrapper warnings unchanged by this PR.

## Plan adherence

- **Scope creep:** **none.** All 16 changed files match the worker's stat
  output and the PR3 plan footprint. No drive-by edits.
- **Missing deliverables:** **none in code; one in deploy.** All three
  waves are present:
  - Wave 1: traces exporter + `TelemetryGuard` in `telemetry_init.rs` —
    present.
  - Wave 2: `nebula_metrics::otlp` + composition wiring + `Arc<MetricsRegistry>`
    shared between Prometheus and OTLP — present.
  - Wave 3: compose stack + integration test — files present; **but the
    operator path (B2) does not actually work as configured**, so Wave 3's
    acceptance criterion "task obs:up succeeds (smoke-test by hand once
    locally)" is not satisfied for the Jaeger UI sub-step.
- **ADR-0046 honored:** flat module layout — `crates/metrics/src/otlp.rs`
  is one file, no `otlp/` directory. ✓
- **ADR-0050 honored:** `init_api_telemetry` remains the single binary
  install site for the trace propagator + tracer + Subscriber stack. No
  duplicate install path was introduced. The metrics pipeline lives in a
  separate guard slot. ✓
- **`MetricsRegistry::snapshot_*` signatures:** unchanged (`git diff` on
  `registry.rs` returns empty). ✓
- **`cargo check -p nebula-log`:** still passes. Workspace OTel feature
  additions (`metrics` on `opentelemetry-otlp` and `opentelemetry_sdk`)
  are additive and do not affect `nebula-log`'s consumption.

## Recommendation

**Block on the following minimal fixes, then LGTM + push:**

1. **B1** — Remove the `docs/plans/recon/m3-otlp-state.md` reference from
   `crates/api/tests/otlp_one_root_span.rs:15`. One-line edit.

2. **B2** — Fix the otelcol-contrib → Jaeger exporter:
   - `deploy/docker/docker-compose.observability.yml:38-41`: remove
     `COLLECTOR_OTLP_ENABLED: "false"` (or set to `"true"`).
   - `deploy/docker/otel-collector-config.yaml:29`: change
     `endpoint: jaeger:14250` to `endpoint: jaeger:4317`.
   - `deploy/docker/README.md`: drop or relabel the `14250` row in the
     ports table to reflect the corrected wiring.

After those two fixes, the PR is ready. The nits (N1–N7) are improvements
worth landing in this PR or a small follow-up; none are blocking. Recommend
also addressing N1, N2, N3 in this same PR since they're documentation
drift and a one-line test-assertion strengthening — cheap to bundle.

Optional follow-up (not for this PR): consolidate the engine seam's
`MetricsRegistry::new()` into the API-owned `Arc<MetricsRegistry>` so
seam-emitted metrics flow through the same OTLP pipeline as the API's.
Pre-existing pattern, but the OTLP exporter wiring makes the case for
consolidation sharper.
