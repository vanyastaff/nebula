# Resource Adapter Guide

This guide explains how to implement a driver crate for `nebula-resource`.

## Goal

- keep `nebula-resource` generic and policy-focused
- move transport/protocol specifics into adapter crates
- give downstream crates stable acquisition contracts

## Recommended crate layout

```text
crates/drivers/resource-<driver>/
  Cargo.toml
  src/lib.rs
```

## Minimal adapter contract

1. Define `Config`:
   - include driver-specific fields (`dsn`, tls, timeouts)
   - implement `Config::validate` with fail-fast checks

2. Define runtime `Instance`:
   - keep instance focused on operational needs
   - avoid leaking secrets in debug output

3. Implement `Resource`:
   - `metadata()` returns canonical `ResourceKey`
   - `create()` builds instance from validated config/context
   - optional `is_valid/recycle/cleanup` for driver lifecycle

## Example: `nebula-resource-postgres`

Reference adapter exists at:
- `crates/drivers/resource-postgres`

It demonstrates:
- typed config + validation
- `Resource` implementation
- end-to-end register/acquire via `Manager` test

## Integration checklist

- `cargo check -p <adapter-crate>`
- `cargo test -p <adapter-crate>`
- register via `Manager::register` in integration test
- verify downcast to typed instance

## Design rules

- adapters own protocol details, not orchestration policy
- retry policy remains caller/resilience concern
- scope/quarantine/health enforcement remains in `nebula-resource`
