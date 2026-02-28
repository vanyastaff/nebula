# Quickstart: Nebula Log Production Hardening Validation

## Goal

Validate that planned hardening behavior for `nebula-log` is testable end-to-end before implementation tasks are generated.

## Prerequisites

- Workspace builds on Rust 1.92+.
- Feature docs available under `docs/crates/log`, `docs/crates/telemetry`, and `docs/crates/metrics`.
- Spec artifacts exist in `specs/001-log-crate-spec/`.

## Validation Flow

1. **Configuration precedence validation**
   - Define scenarios with explicit config + environment + preset defaults.
   - Verify resolved profile is deterministic for every scenario.

2. **Multi-destination behavior validation**
   - Run scenarios with multiple destinations and injected sink failure.
   - Verify behavior for `FailFast`, `BestEffort`, `PrimaryWithFallback`.

3. **Hook isolation validation**
   - Register healthy, panicking, and slow hooks.
   - Verify panic isolation and bounded behavior under load.

4. **Rolling behavior validation**
   - Validate size-based rolling activates at configured threshold.
   - Verify continuity of writes across rotated outputs.

5. **Compatibility validation**
   - Run config schema compatibility checks against existing supported minor-version inputs.
   - Verify migration guidance exists for any contract expansion.

6. **Performance validation**
   - Run benchmark scenarios for emission path, context propagation, and hook dispatch.
   - Confirm latency impact remains within plan target budget.

## Quality Gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test -p nebula-log
cargo test -p nebula-log --features file --test writer_fanout
cargo bench -p nebula-log --bench log_hot_path
cargo doc --no-deps --workspace
```

## Expected Result

- All validations pass.
- No unresolved clarification markers remain in planning artifacts.
- Artifacts are ready for `/speckit.tasks`.
