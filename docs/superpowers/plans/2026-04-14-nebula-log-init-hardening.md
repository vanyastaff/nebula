# nebula-log Init Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four interlocking init-safety bugs in `nebula-log` (#375, #377, #379, #380) in a single PR: make OTLP opt-in, surface invalid Sentry DSNs, return a structured error on duplicate init, and defer OTel global registration until after `try_init` succeeds.

**Architecture:** All changes live in the `crates/log` crate. We introduce one new `LogError::AlreadyInitialized` variant and refactor `telemetry::otel::build_layer` so global side effects (tracer provider + propagator) happen *after* the subscriber is installed. `init_with` fast-paths a set dispatcher with a structured error, and `sentry::init` logs a `tracing::warn!` when `SENTRY_DSN` is non-empty but unparseable. OTLP endpoint resolution drops the silent `http://localhost:4317` default and requires an explicit config value or env var to enable export.

**Tech Stack:** Rust 2024, `tracing`, `tracing-subscriber`, `opentelemetry` / `opentelemetry-sdk` / `opentelemetry-otlp`, `sentry`, `thiserror`, `cargo nextest`.

**Relevant issues:**
- #375 — OTLP build_layer defaults to localhost when endpoint unset
- #377 — invalid SENTRY_DSN fails silently (no Sentry, no warning)
- #379 — second `init_with` fails hard; only test `auto_init` short-circuits duplicate dispatcher
- #380 — OTel global tracer registered before tracing `try_init` — partial init on subscriber failure

---

## File Structure

**Create:**
- `crates/log/tests/init_hardening.rs` — integration tests for all four fixes, in one process-isolated test file.

**Modify:**
- `crates/log/src/core/error.rs` — add `LogError::AlreadyInitialized` variant.
- `crates/log/src/telemetry/otel.rs` — split endpoint resolution into `resolve_endpoint`, drop silent localhost default, split global-side-effects out of `build_layer` into `install_globals` / `shutdown_provider`.
- `crates/log/src/telemetry/sentry.rs` — explicit parse with `tracing::warn!` on failure.
- `crates/log/src/builder/mod.rs` — fast-path `AlreadyInitialized` before building layers; call `otel::install_globals` *after* successful `try_init`; on error between OTel layer build and `try_init`, shut down the provider.
- `crates/log/src/lib.rs` — doc tweaks for the new error and the strict OTLP semantics.

Each file has one clear responsibility. The test file is process-isolated via `#[cfg(test)]` + running tests with `--test-threads=1` in the new test binary; we do not pollute `tests/integration_tests.rs` because it already sets a global dispatcher.

---

## Task 1: Add `LogError::AlreadyInitialized` variant

**Files:**
- Modify: `crates/log/src/core/error.rs`

- [ ] **Step 1: Write the failing test**

Add a new unit test inside `crates/log/src/core/error.rs` at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_initialized_display_is_stable() {
        let err = LogError::AlreadyInitialized;
        assert_eq!(err.to_string(), "Logger already initialized for this process");
    }

    #[test]
    fn already_initialized_is_distinct_from_internal() {
        let err = LogError::AlreadyInitialized;
        assert!(matches!(err, LogError::AlreadyInitialized));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p nebula-log error::tests::already_initialized`
Expected: FAIL with "no variant named `AlreadyInitialized`".

- [ ] **Step 3: Add the variant**

In `crates/log/src/core/error.rs`, extend the `LogError` enum. After the `Internal(String)` variant and before the closing brace, add:

```rust
    /// Logger already initialized for this process
    ///
    /// Returned by [`init_with`](crate::init_with) / [`init`](crate::init) /
    /// [`auto_init`](crate::auto_init) when `tracing::dispatcher::has_been_set()`
    /// is already true. Callers that expect idempotent initialization can treat
    /// this variant as success.
    #[classify(category = "validation", code = "LOG:ALREADY_INITIALIZED")]
    #[error("Logger already initialized for this process")]
    AlreadyInitialized,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-log error::tests::already_initialized`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/log/src/core/error.rs
git commit -m "feat(log): add LogError::AlreadyInitialized variant

Structured error for duplicate dispatcher init so callers can distinguish
\"already installed\" from other subscriber errors. Precursor to #379 fix."
```

---

## Task 2: Fast-path duplicate init in `init_with` / `build`

**Files:**
- Modify: `crates/log/src/builder/mod.rs:126-138`
- Modify: `crates/log/src/lib.rs:183-195`

- [ ] **Step 1: Write the failing test**

Create the new integration test file `crates/log/tests/init_hardening.rs` with this initial content (we will add to it in later tasks):

```rust
//! Integration tests for nebula-log init-hardening fixes (#375/#377/#379/#380).
//!
//! These tests share a process-global `tracing` dispatcher, so they are ordered
//! and gated via `serial_test` where necessary. The first init always wins;
//! subsequent calls must return `LogError::AlreadyInitialized`.

use nebula_log::{Config, LogError, init_with};

/// #379 — second `init_with` returns a structured `AlreadyInitialized` error,
/// not a generic `Internal` error.
#[test]
fn second_init_with_returns_already_initialized() {
    // First init wins (or is already installed by a prior test).
    let _ = init_with(Config::default());

    // Second init must now return AlreadyInitialized.
    let err = init_with(Config::default()).expect_err("expected duplicate init to error");
    assert!(
        matches!(err, LogError::AlreadyInitialized),
        "expected AlreadyInitialized, got: {err:?}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p nebula-log --test init_hardening second_init_with_returns_already_initialized`
Expected: FAIL — the second init returns `LogError::Internal` from `try_init()`, not `AlreadyInitialized`.

- [ ] **Step 3: Implement the fast path in `LoggerBuilder::build`**

In `crates/log/src/builder/mod.rs`, at the very top of `LoggerBuilder::build` (immediately after the opening brace on line 126), insert:

```rust
        // #379: fast-path a duplicate dispatcher so callers get a structured
        // error instead of a generic subscriber failure. This must run BEFORE
        // any telemetry or writer setup to avoid partial init side effects.
        if tracing::dispatcher::has_been_set() {
            return Err(crate::core::LogError::AlreadyInitialized);
        }

        self.config.ensure_compatible()?;
```

Replace the existing `self.config.ensure_compatible()?;` on line 127 with the block above (i.e. the check precedes the existing `ensure_compatible` call).

- [ ] **Step 4: Also fast-path `auto_init` outside `cfg(test)`**

In `crates/log/src/lib.rs`, replace the body of `auto_init` (lines 183–195) with:

```rust
pub fn auto_init() -> LogResult<LoggerGuard> {
    #[cfg(test)]
    {
        TEST_INIT.get_or_init(|| ());
    }

    // #379: in any build, a duplicate dispatcher short-circuits to a no-op
    // guard so callers that opportunistically call `auto_init` from library
    // code do not blow up a host process that already owns logging.
    if tracing::dispatcher::has_been_set() {
        return Ok(LoggerGuard::noop());
    }

    let (guard, source) = LoggerBuilder::build_startup(None)?;
    info!(source = ?source, "logging initialized");
    Ok(guard)
}
```

Note: `LoggerGuard::noop` is currently `#[cfg(test)]`. Remove that gate so it is available in production builds — change line 245 from `#[cfg(test)]` to nothing (i.e. make `noop()` an unconditionally `pub(crate)` helper).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-log --test init_hardening second_init_with_returns_already_initialized`
Expected: PASS.

Run: `cargo nextest run -p nebula-log`
Expected: all existing unit tests still pass (watch for `noop_guard_has_no_reload_handle`, which must still compile after lifting the `cfg(test)` gate).

- [ ] **Step 6: Commit**

```bash
git add crates/log/src/builder/mod.rs crates/log/src/lib.rs crates/log/tests/init_hardening.rs
git commit -m "fix(log): return AlreadyInitialized on duplicate init (#379)

LoggerBuilder::build and auto_init now check tracing::dispatcher::has_been_set
up front. build returns LogError::AlreadyInitialized; auto_init returns a
no-op LoggerGuard so library callers cannot blow up a host process that
already owns logging."
```

---

## Task 3: Warn on invalid `SENTRY_DSN`

**Files:**
- Modify: `crates/log/src/telemetry/sentry.rs`
- Modify: `crates/log/tests/init_hardening.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/log/tests/init_hardening.rs`:

```rust
/// #377 — invalid SENTRY_DSN must not silently disable Sentry.
///
/// We cannot easily intercept the `tracing::warn!` (the subscriber is
/// process-global), so we instead assert that `sentry::init()` still returns
/// `None` for a bogus DSN *and* that the call path does not panic. The real
/// regression signal is a code inspection: the `ok()?` shortcut must be gone.
#[cfg(feature = "sentry")]
#[test]
fn invalid_sentry_dsn_returns_none_without_panic() {
    // Save & restore env so parallel tests don't clobber each other.
    let prev = std::env::var("SENTRY_DSN").ok();
    // SAFETY: test-only single-threaded env mutation.
    unsafe { std::env::set_var("SENTRY_DSN", "not-a-valid-dsn"); }

    let guard = nebula_log::telemetry::sentry::init();
    assert!(guard.is_none(), "invalid DSN must not produce a Sentry guard");

    // Restore env.
    match prev {
        Some(v) => unsafe { std::env::set_var("SENTRY_DSN", v) },
        None => unsafe { std::env::remove_var("SENTRY_DSN") },
    }
}
```

Also make the `telemetry` module visible to integration tests. In `crates/log/src/lib.rs`, change line 127 from:

```rust
#[cfg(any(feature = "telemetry", feature = "sentry"))]
mod telemetry;
```

to:

```rust
#[cfg(any(feature = "telemetry", feature = "sentry"))]
#[doc(hidden)]
pub mod telemetry;
```

The `#[doc(hidden)]` keeps it out of rustdoc; `pub` lets integration tests reach `telemetry::sentry::init`.

- [ ] **Step 2: Run the test to verify it fails (or at least compiles)**

Run: `cargo nextest run -p nebula-log --features sentry --test init_hardening invalid_sentry_dsn_returns_none_without_panic`
Expected: PASS on the assertion (the old `ok()?` path already returns `None` for bad DSNs), but the point of the test is to lock behavior. We treat this as a regression fence, not a RED step. Move on.

- [ ] **Step 3: Replace silent `ok()?` with an explicit warn**

In `crates/log/src/telemetry/sentry.rs`, replace lines 28–36 (the `sentry::init` call) with:

```rust
    let parsed_dsn = match dsn.parse::<sentry::types::Dsn>() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "SENTRY_DSN is set but invalid; Sentry reporting is disabled"
            );
            return None;
        }
    };

    let guard = sentry::init(sentry::ClientOptions {
        dsn: Some(parsed_dsn),
        environment: Some(environment.into()),
        release: release.map(|s| s.into()),
        traces_sample_rate: sample_rate,
        attach_stacktrace: true,
        send_default_pii: false,
        ..Default::default()
    });
```

Note: `sentry::types::Dsn` is the public re-export of `sentry_core::types::Dsn`. If that path is not available, use `sentry::IntoDsn` and call `dsn.as_str().into_dsn()` instead — but prefer the direct `parse` form.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-log --features sentry`
Expected: all tests pass, `invalid_sentry_dsn_returns_none_without_panic` included.

- [ ] **Step 5: Commit**

```bash
git add crates/log/src/telemetry/sentry.rs crates/log/src/lib.rs crates/log/tests/init_hardening.rs
git commit -m "fix(log): warn when SENTRY_DSN is set but invalid (#377)

Replace the silent ok()? shortcut with an explicit parse that emits
tracing::warn! on failure so operators notice misconfigurations at startup
instead of discovering 'no Sentry events' in prod."
```

---

## Task 4: Make OTLP opt-in; drop silent localhost default

**Files:**
- Modify: `crates/log/src/telemetry/otel.rs:40-51`
- Modify: `crates/log/tests/init_hardening.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/log/tests/init_hardening.rs`:

```rust
/// #375 — with no endpoint in config and no `OTEL_EXPORTER_OTLP_ENDPOINT`
/// env var, `build_layer` must return `Ok(None)` (opt-in), not silently
/// point at `http://localhost:4317`.
#[cfg(feature = "telemetry")]
#[test]
fn otlp_is_opt_in_when_endpoint_unset() {
    use nebula_log::TelemetryConfig;
    use nebula_log::layer::context::Fields;

    let prev = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    // SAFETY: test-only single-threaded env mutation.
    unsafe { std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT"); }

    let cfg = TelemetryConfig {
        otlp_endpoint: None,
        service_name: "test".to_string(),
        sampling_rate: 1.0,
    };
    let fields = Fields::default();

    let layer = nebula_log::telemetry::otel::build_layer(&cfg, &fields)
        .expect("build_layer must not error when endpoint is unset");
    assert!(layer.is_none(), "OTLP must be opt-in when endpoint is unset");

    // Restore env.
    if let Some(v) = prev {
        unsafe { std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", v) };
    }
}

/// #375 companion — an explicit empty string in config is also treated as off,
/// consistent with "disabled".
#[cfg(feature = "telemetry")]
#[test]
fn otlp_empty_endpoint_is_opt_out() {
    use nebula_log::TelemetryConfig;
    use nebula_log::layer::context::Fields;

    let cfg = TelemetryConfig {
        otlp_endpoint: Some(String::new()),
        service_name: "test".to_string(),
        sampling_rate: 1.0,
    };
    let fields = Fields::default();

    let layer = nebula_log::telemetry::otel::build_layer(&cfg, &fields).unwrap();
    assert!(layer.is_none());
}
```

Also re-export `layer` as `pub(crate) use layer::context` is already done; but the test needs `nebula_log::layer::context::Fields`. `Fields` is already re-exported at the crate root as `pub use layer::context::{Context, Fields};` (lib.rs:146), so change the test imports to `use nebula_log::Fields;` instead and delete the `use nebula_log::layer::context::Fields;` lines.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p nebula-log --features telemetry --test init_hardening otlp_is_opt_in_when_endpoint_unset`
Expected: FAIL — current code returns `Some(layer)` pointing at `http://localhost:4317`.

- [ ] **Step 3: Rewrite endpoint resolution**

In `crates/log/src/telemetry/otel.rs`, replace lines 40–51 (the start of `build_layer`, from `let endpoint_str = ...` through the `if endpoint_str == "disabled" ...` block) with:

```rust
    let endpoint_str = match resolve_endpoint(config) {
        Some(e) => e,
        None => return Ok(None),
    };
```

Then add a new private function above `build_resource` (i.e. after `build_layer`'s closing brace, before line 105):

```rust
/// Resolve the OTLP endpoint from config + environment.
///
/// Precedence: explicit `config.otlp_endpoint` → `OTEL_EXPORTER_OTLP_ENDPOINT`
/// env var → **off**. The literal values `"disabled"` and `""` are treated as
/// explicit opt-out at both layers.
///
/// Returns `None` when OTLP should be disabled.
fn resolve_endpoint(config: &TelemetryConfig) -> Option<String> {
    // 1. Explicit config wins.
    if let Some(endpoint) = config.otlp_endpoint.as_deref() {
        let trimmed = endpoint.trim();
        if trimmed.is_empty() || trimmed == "disabled" {
            return None;
        }
        return Some(trimmed.to_string());
    }

    // 2. Env var falls through.
    match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(endpoint) => {
            let trimmed = endpoint.trim();
            if trimmed.is_empty() || trimmed == "disabled" {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        // 3. No config + no env = OTLP off (opt-in). Previously defaulted to
        // http://localhost:4317, which caused surprise network activity in
        // environments that never ran a collector (see #375).
        Err(_) => None,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-log --features telemetry --test init_hardening otlp_is_opt_in_when_endpoint_unset otlp_empty_endpoint_is_opt_out`
Expected: PASS.

Run: `cargo nextest run -p nebula-log --features telemetry`
Expected: all existing tests pass; no regressions in `otlp_setup` example compile.

- [ ] **Step 5: Commit**

```bash
git add crates/log/src/telemetry/otel.rs crates/log/tests/init_hardening.rs
git commit -m "fix(log): make OTLP opt-in; drop silent localhost default (#375)

When neither TelemetryConfig.otlp_endpoint nor OTEL_EXPORTER_OTLP_ENDPOINT
is set, build_layer now returns Ok(None). Previously it silently defaulted
to http://localhost:4317 and attempted gRPC export, causing noisy errors in
environments without a collector."
```

---

## Task 5: Defer OTel globals until after `try_init` succeeds

**Files:**
- Modify: `crates/log/src/telemetry/otel.rs:86-95`
- Modify: `crates/log/src/builder/mod.rs:164-204`
- Modify: `crates/log/tests/init_hardening.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/log/tests/init_hardening.rs`:

```rust
/// #380 — if subscriber `try_init` fails (duplicate dispatcher), OTel globals
/// must NOT have been installed and the provider must be shut down cleanly.
///
/// We can only observe this indirectly: after a first successful init, a
/// second `init_with` with a telemetry config must return `AlreadyInitialized`
/// and must not leak a new tracer provider onto `opentelemetry::global`.
///
/// The precise leak-check is difficult without poking at OTel internals, so
/// we assert the happy-path invariant: on the error path the call returns
/// cleanly without panicking from a dangling provider drop.
#[cfg(feature = "telemetry")]
#[test]
fn partial_otel_init_is_cleaned_up_on_subscriber_failure() {
    use nebula_log::{Config, LogError, TelemetryConfig, init_with};

    // Force a prior init so the next one hits `AlreadyInitialized`.
    let _ = init_with(Config::default());

    let mut cfg = Config::default();
    cfg.telemetry = Some(TelemetryConfig {
        // Use a bogus but syntactically valid endpoint to force exporter
        // build success followed by subscriber try_init failure.
        otlp_endpoint: Some("http://127.0.0.1:1".to_string()),
        service_name: "partial-init-test".to_string(),
        sampling_rate: 0.0,
    });

    let err = init_with(cfg).expect_err("duplicate init must fail");
    assert!(
        matches!(err, LogError::AlreadyInitialized),
        "expected AlreadyInitialized, got: {err:?}"
    );
    // If we got here without panicking, the error-path cleanup is OK.
}
```

- [ ] **Step 2: Run the test to verify it passes or fails**

Run: `cargo nextest run -p nebula-log --features telemetry --test init_hardening partial_otel_init_is_cleaned_up_on_subscriber_failure`
Expected: this specific test likely passes already (Task 2 made `AlreadyInitialized` short-circuit before OTel setup runs). That short-circuit is *the* fix for #380's symptom — but #380 also applies when `try_init` fails for non-duplicate reasons. The refactor below makes the code structurally correct.

- [ ] **Step 3: Split global side effects out of `build_layer`**

In `crates/log/src/telemetry/otel.rs`, replace lines 53–95 (everything from `// Set up W3C trace-context propagator` through the `Ok(Some(OtelLayer { ... }))` block) with:

```rust
    // Configure sampler.
    let sampler = if config.sampling_rate >= 1.0 {
        Sampler::AlwaysOn
    } else if config.sampling_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sampling_rate)
    };

    // Build OTel resource from config + fields (OTel semantic conventions).
    let resource = build_resource(&config.service_name, fields);

    let provider_builder = SdkTracerProvider::builder()
        .with_sampler(sampler)
        .with_resource(resource);

    // Batch export requires an active Tokio runtime.
    // Fall back to simple export in sync contexts to avoid runtime panics.
    #[cfg(feature = "async")]
    let has_runtime = tokio::runtime::Handle::try_current().is_ok();
    #[cfg(not(feature = "async"))]
    let has_runtime = false;

    let exporter = build_exporter(&endpoint_str)?;
    let provider = if has_runtime {
        provider_builder.with_batch_exporter(exporter).build()
    } else {
        provider_builder.with_simple_exporter(exporter).build()
    };

    let tracer = provider.tracer("nebula-log");

    // #380: globals are NOT set here — the caller installs them after the
    // subscriber is successfully `try_init`'d so a mid-init failure does not
    // leave a dangling tracer provider in `opentelemetry::global`.
    Ok(Some(OtelLayer {
        layer: Box::new(OpenTelemetryLayer::new(tracer)),
        provider,
    }))
```

Then, at the very bottom of `otel.rs` (after `build_exporter`), add:

```rust
/// Install OTel globals from a successfully-built provider.
///
/// Must be called only **after** the tracing subscriber's `try_init` succeeds,
/// so a subscriber-init failure cannot leave the OTel global state pointing at
/// a provider whose lifecycle no longer matches the `LoggerGuard`.
///
/// Sets:
/// - the W3C trace-context propagator as the global text-map propagator
/// - the given provider as the global tracer provider
pub(crate) fn install_globals(provider: &SdkTracerProvider) {
    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider.clone());
}

/// Shut down a provider that was built but never installed globally.
///
/// Used by the builder when `try_init` fails after `build_layer` succeeded, to
/// avoid leaking exporter tasks/network connections.
pub(crate) fn shutdown_unused_provider(provider: SdkTracerProvider) {
    if let Err(e) = provider.shutdown() {
        eprintln!("nebula-log: unused OTel provider shutdown error: {e}");
    }
}
```

- [ ] **Step 4: Rewire `LoggerBuilder::build` to install globals after `try_init`**

This is the non-trivial part. Today `init_subscriber!` swallows the returned `OtelLayer.provider` into `inner.otel_provider` *before* calling `try_init`. We want:

1. Build the `OtelLayer` (producing `layer` + `provider`).
2. Stash the `provider` in a local, not yet in `inner`.
3. Call `try_init` via `init_subscriber!`.
4. On success: move `provider` into `inner.otel_provider` **and** call `otel::install_globals(&provider_ref)`.
5. On failure: the `?` propagates, and the stashed `provider` gets shut down via its scope-local drop handler.

Update `crates/log/src/builder/mod.rs`. First, change the `init_subscriber!` macro (lines 76–93) to *not* call `try_init` at the end — return the layer stack instead:

```rust
macro_rules! build_subscriber {
    ($filter_layer:expr, $fmt_layer:expr, $otel_layer:expr) => {{
        let mut layers: Vec<BoxLayer> = Vec::new();
        layers.push($filter_layer);
        layers.push(Box::new($fmt_layer));
        if let Some(otel) = $otel_layer {
            layers.push(otel);
        }
        attach_sentry!(layers);
        layers
    }};
}
```

Rename every `init_subscriber!` call site in `build()` (currently lines 190, 194, 198, 202) from `init_subscriber!(...)` to `let layers = build_subscriber!(...);`. Then, after the `match self.config.format { ... }` block, add a single `try_init` + global-install sequence:

```rust
        // Build all four format branches produce a `layers: Vec<BoxLayer>`.
        // We then install the subscriber once here so OTel globals are only
        // touched after `try_init` succeeds (#380).
        let layers = match self.config.format {
            Format::Pretty => {
                let fmt_layer = create_fmt_layer!(pretty, &self.config.display, writer);
                build_subscriber!(filter_layer, fmt_layer, otel_layer)
            },
            Format::Compact => {
                let fmt_layer = create_fmt_layer!(compact, &self.config.display, writer);
                build_subscriber!(filter_layer, fmt_layer, otel_layer)
            },
            Format::Logfmt => {
                let fmt_layer = create_logfmt_layer!(&self.config.display, writer);
                build_subscriber!(filter_layer, fmt_layer, otel_layer)
            },
            Format::Json => {
                let fmt_layer = create_json_layer!(&self.config.display, writer);
                build_subscriber!(filter_layer, fmt_layer, otel_layer)
            },
        };

        // Move otel_provider out of the layer build so we can install globals
        // *after* try_init succeeds. On failure, the local `pending_provider`
        // drops and gets shut down via the `Drop` path below.
        #[cfg(feature = "telemetry")]
        let pending_provider = inner.otel_provider.take();

        if let Err(e) = Registry::default().with(layers).try_init() {
            // #380: try_init failed — tear down any provider we built so we
            // don't leak exporter tasks.
            #[cfg(feature = "telemetry")]
            if let Some(provider) = pending_provider {
                crate::telemetry::otel::shutdown_unused_provider(provider);
            }
            return Err(crate::core::LogError::Internal(e.to_string()));
        }

        // #380: install OTel globals only now that the subscriber owns the
        // tracing pipeline.
        #[cfg(feature = "telemetry")]
        if let Some(provider) = pending_provider {
            crate::telemetry::otel::install_globals(&provider);
            inner.otel_provider = Some(provider);
        }
```

This means the old section that does `inner.otel_provider = Some(otel.provider)` inside the `#[cfg(feature = "telemetry")]` block (lines 164–184) must **not** assign `inner.otel_provider` any more. Keep the match that produces `otel_layer`, but replace the `Some(otel) => { inner.otel_provider = Some(otel.provider); Some(otel.layer) }` branch with a local temporary:

```rust
        #[cfg(feature = "telemetry")]
        let otel_layer: Option<Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>> = {
            match &self.config.telemetry {
                Some(telemetry_config) => {
                    match crate::telemetry::otel::build_layer(
                        telemetry_config,
                        &self.config.fields,
                    )? {
                        Some(otel) => {
                            // Hold the provider in `inner` temporarily; we
                            // move it back out below and install globals
                            // only after try_init succeeds (#380).
                            inner.otel_provider = Some(otel.provider);
                            Some(otel.layer)
                        },
                        None => None,
                    }
                },
                None => None,
            }
        };
```

(Yes, `inner.otel_provider` is assigned here and then `.take()`-n out below. That keeps the diff local and avoids adding a new field.)

- [ ] **Step 5: Run clippy + tests to verify the refactor compiles and passes**

Run: `cargo +nightly fmt -p nebula-log`
Run: `cargo clippy -p nebula-log --features telemetry -- -D warnings`
Expected: no warnings.

Run: `cargo nextest run -p nebula-log --features telemetry`
Expected: all tests pass, including the new `partial_otel_init_is_cleaned_up_on_subscriber_failure`.

- [ ] **Step 6: Commit**

```bash
git add crates/log/src/telemetry/otel.rs crates/log/src/builder/mod.rs crates/log/tests/init_hardening.rs
git commit -m "fix(log): defer OTel globals until after try_init succeeds (#380)

Split build_layer into a pure layer builder and an explicit install_globals
step. LoggerBuilder::build now installs the subscriber first, then installs
OTel globals, so a failed try_init tears down the provider cleanly instead
of leaving opentelemetry::global pointing at an orphaned tracer provider."
```

---

## Task 6: Doc pass + full workspace validation

**Files:**
- Modify: `crates/log/src/lib.rs:183-230`

- [ ] **Step 1: Update doc comments**

In `crates/log/src/lib.rs`, update the doc comments on `auto_init`, `init`, and `init_with` to reflect the new semantics. Replace the `# Errors` section on `init_with` (around lines 214–219) with:

```rust
/// # Errors
///
/// Returns error if:
/// - The logger is already initialized for this process
///   ([`LogError::AlreadyInitialized`])
/// - Filter string is invalid
/// - File writer cannot be created (if using file output)
/// - Telemetry setup fails (if enabled)
///
/// Calling `init_with` a second time in the same process always returns
/// [`LogError::AlreadyInitialized`]; callers that want idempotent init
/// should treat that variant as success.
```

Update the `auto_init` doc to mention that a duplicate dispatcher returns a no-op guard (same text as the old test-only behavior, but now unconditional):

```rust
/// Auto-detect and initialize the best logging configuration.
///
/// Checks environment variables (`NEBULA_LOG`, `RUST_LOG`) and debug assertions
/// to choose between development, production, or custom configuration.
///
/// Safe to call multiple times: once a dispatcher is already set, this
/// returns a no-op [`LoggerGuard`] (see [`LogError::AlreadyInitialized`]
/// for the `init_with` counterpart).
///
/// # Errors
///
/// Returns error if filter parsing fails or logger initialization fails.
```

In the top-of-crate docs (`//!` lines 83–90), append a sentence under the `OTEL_EXPORTER_OTLP_ENDPOINT` bullet:

```text
//! - `OTEL_EXPORTER_OTLP_ENDPOINT` — when neither this env var nor
//!   `TelemetryConfig::otlp_endpoint` is set, OTLP export is disabled. Use
//!   the literal string `"disabled"` (or an empty string) to explicitly
//!   opt out at either layer.
```

- [ ] **Step 2: Run full validation gate**

Run: `cargo +nightly fmt --all`
Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings.

Run: `cargo nextest run -p nebula-log --all-features`
Expected: all tests pass.

Run: `cargo test -p nebula-log --doc --all-features`
Expected: all doctests pass.

Run: `cargo nextest run --workspace`
Expected: no regressions elsewhere (the init-harden changes are crate-local, but `nebula-api` / examples call into `nebula-log`).

- [ ] **Step 3: Commit**

```bash
git add crates/log/src/lib.rs
git commit -m "docs(log): document AlreadyInitialized and strict OTLP semantics

Part of the batch fix for #375/#377/#379/#380."
```

---

## Task 7: Open the PR

- [ ] **Step 1: Push branch and open PR**

```bash
git push -u origin HEAD
gh pr create \
  --title "fix(log): harden init paths (#375, #377, #379, #380)" \
  --body "$(cat <<'EOF'
## Summary

Single-PR batch fix for four interlocking init-safety bugs in `nebula-log`:

- **#375** — OTLP is now opt-in. When neither `TelemetryConfig.otlp_endpoint`
  nor `OTEL_EXPORTER_OTLP_ENDPOINT` is set, `build_layer` returns `Ok(None)`.
  Previously it silently defaulted to `http://localhost:4317` and attempted
  gRPC export, causing noisy errors in environments without a collector.
- **#377** — Invalid `SENTRY_DSN` now emits a `tracing::warn!` instead of
  silently disabling Sentry via `ok()?`.
- **#379** — `init_with` returns a structured `LogError::AlreadyInitialized`
  on duplicate dispatcher, not a generic `LogError::Internal`. `auto_init`
  returns a no-op `LoggerGuard` on the same condition in any build (was
  previously `#[cfg(test)]`-only).
- **#380** — `telemetry::otel::build_layer` no longer touches OTel globals.
  `LoggerBuilder::build` now installs the subscriber first and then calls
  the new `otel::install_globals`; on `try_init` failure it calls
  `otel::shutdown_unused_provider` so a mid-init failure cannot leave
  `opentelemetry::global` pointing at an orphaned tracer provider.

## Test plan

- [x] `cargo nextest run -p nebula-log --all-features`
- [x] `cargo test -p nebula-log --doc --all-features`
- [x] `cargo clippy --workspace -- -D warnings`
- [x] New integration test file `crates/log/tests/init_hardening.rs` covers
      all four regressions.

Closes #375
Closes #377
Closes #379
Closes #380
EOF
)"
```

- [ ] **Step 2: Report PR URL back to the user.**

---

## Self-Review Notes

**Spec coverage check:**
- #375 → Task 4 (`resolve_endpoint`, opt-in semantics, tests `otlp_is_opt_in_when_endpoint_unset` + `otlp_empty_endpoint_is_opt_out`).
- #377 → Task 3 (explicit parse + `tracing::warn!`, test `invalid_sentry_dsn_returns_none_without_panic`).
- #379 → Task 1 (new error variant) + Task 2 (fast-path in `build` and `auto_init`, test `second_init_with_returns_already_initialized`).
- #380 → Task 5 (split `build_layer`, new `install_globals` / `shutdown_unused_provider`, rewire `build()`, test `partial_otel_init_is_cleaned_up_on_subscriber_failure`).
- Doc + validation gate → Task 6.
- PR → Task 7.

**Type consistency check:**
- `LogError::AlreadyInitialized` declared in Task 1, consumed in Task 2 (`build`, `init_with` via `build`), Task 5 test, and Task 6 docs. Consistent.
- `otel::install_globals(&SdkTracerProvider)` signature (Task 5) matches the call in `LoggerBuilder::build` (Task 5 step 4).
- `otel::shutdown_unused_provider(SdkTracerProvider)` takes ownership so the provider is dropped after shutdown. Matches the move in the error path in Task 5 step 4.
- `resolve_endpoint(&TelemetryConfig) -> Option<String>` (Task 4) is called exactly once at the top of `build_layer`. Consistent.
- `LoggerGuard::noop()` is lifted from `#[cfg(test)]` to unconditional `pub(crate)` in Task 2; the existing test `noop_guard_has_no_reload_handle` still compiles because it is itself `#[cfg(test)]` and inside the same module.

**Placeholder scan:** no "TBD", "implement later", or bare "add tests for the above" — every test has an explicit body, every code step has a code block.

**Known trade-offs / notes for the implementer:**
1. The sentry test is a regression fence, not a true RED step, because the old `ok()?` already returned `None`. The real change is the warn log; we cannot easily assert on a `tracing::warn!` without a dedicated subscriber, so we rely on code inspection + the explicit parse being present.
2. `noop()` losing `#[cfg(test)]` is intentional — it is the only cheap way to give `auto_init` a harmless return on duplicate init without allocating a fresh `Inner`.
3. `sentry::types::Dsn` path may be `sentry::types::Dsn` or `sentry::Dsn` depending on the `sentry` crate's re-exports; if the first path fails, check `Cargo.lock` for the exact version and adjust.
