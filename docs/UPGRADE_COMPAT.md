# Upgrade Compatibility

Scope: expanded compatibility guidance for `docs/PRODUCT_CANON.md` §7.2.

## Compatibility Surfaces

1. Workflow definitions (persisted JSON + activation semantics)
2. Engine/runtime behavior (execution semantics, durability guarantees)
3. Plugin SDK and binaries (`nebula-api` / plugin SDK linkage)

## Baseline Policy

- Patch/minor releases must preserve forward-compatible workflow JSON and documented SDK boundaries unless break is explicitly announced.
- Breaking behavior requires migration notes, tests, and upgrade guidance.
- Do not claim universal compatibility without a published matrix.

## Plugin Binary Reality

- Rust plugin binaries are compiled artifacts tied to SDK/engine versions.
- Upgrades may require recompilation against target SDK/engine.
- Binary-stable ABI is only promised on explicit FFI paths, not by default for native Rust plugin binaries.

## Compatibility Matrix Template


| Engine Version | Workflow JSON | Plugin SDK Source Compat | Native Plugin Binary Compat | Notes           |
| -------------- | ------------- | ------------------------ | --------------------------- | --------------- |
| x.y.z          | yes/no        | yes/no                   | yes/no                      | migration links |


Populate this matrix per release train.

## Upgrade Checklist

- Run workflow validation on stored definitions before activation.
- Verify control queue and cancel path behavior after migration.
- Rebuild/retest plugins against target SDK where required.
- Confirm observability paths (journal/errors/metrics) still satisfy operator diagnostics.

