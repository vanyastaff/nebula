# nebula-resilience

`nebula-resilience` provides resilience patterns for Nebula services: retry, circuit breaker,
timeout, bulkhead, rate limiting, fallback, hedge, composition, and observability hooks.

## Source of Truth Documentation

Crate-local docs are now the canonical source and live under `crates/resilience/docs/`.

- Overview: [`docs/README.md`](docs/README.md)
- Pattern guide: [`docs/PATTERNS.md`](docs/PATTERNS.md)
- API surface: [`docs/API.md`](docs/API.md)
- Reliability: [`docs/RELIABILITY.md`](docs/RELIABILITY.md)
- Migration / compatibility: [`docs/MIGRATION.md`](docs/MIGRATION.md)

## Quick Use

```toml
[dependencies]
nebula-resilience = { path = "../crates/resilience" }
```

See crate-level docs in `src/lib.rs` and the guides in `docs/` for end-to-end examples.

## Verify Locally

```bash
cargo check -p nebula-resilience --all-features
cargo test -p nebula-resilience
cargo clippy -p nebula-resilience -- -D warnings
cargo doc --no-deps -p nebula-resilience
```

## Benchmarks

```bash
cargo bench -p nebula-resilience
```

Benchmark suites include `manager`, `rate_limiter`, `circuit_breaker`, `retry`, `compose`,
`timeout`, `bulkhead`, `fallback`, `hedge`, and `observability`.
