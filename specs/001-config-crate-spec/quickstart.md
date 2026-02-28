# Quickstart: Config Contract Hardening

## 1. Baseline quality checks

Run config crate tests:

```bash
cargo test -p nebula-config
```

Run required workspace quality gates:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
```

## 2. Verify precedence and merge contract

- Validate layered source precedence fixtures.
- Confirm identical source sets produce identical merged outputs.
- Confirm conflicting source values resolve by documented priority.

## 3. Verify activation and reload safety

- Confirm invalid candidate config never becomes active.
- Confirm last-known-good snapshot remains active on failed reload.
- Confirm successful candidate activation is atomic.

## 4. Verify typed path access contract

- Run path retrieval compatibility fixtures for representative consumer keys.
- Confirm deterministic error categories for missing paths and type mismatches.
- Confirm minor-version compatibility for existing path contracts.

## 5. Verify governance and migration readiness

- Update `docs/crates/config/MIGRATION.md` when behavior-significant changes are proposed.
- Require explicit old->new mapping for precedence/path/validation semantic changes.
- Keep minor releases additive for source/validator/watcher contract surface.

## 6. Verify diagnostics safety

- Confirm sensitive values are redacted in operational diagnostics.
- Validate diagnostics include actionable source/path context for operators.

## Task-Phase References

- Phase 1-2:
  - scaffold tests and baseline docs before story work.
- Phase 3 (US1):
  - run precedence contracts:
    - `cargo test -p nebula-config precedence_matrix`
    - `cargo test -p nebula-config merge_determinism`
    - `cargo test -p nebula-config env_precedence`
- Phase 4 (US2):
  - run reload safety contracts:
    - `cargo test -p nebula-config reload_rejection`
    - `cargo test -p nebula-config last_known_good`
    - `cargo test -p nebula-config activation_atomicity`
- Phase 5 (US3):
  - run path/typed access contracts:
    - `cargo test -p nebula-config typed_access_compatibility`
    - `cargo test -p nebula-config path_error_categories`
- Phase 6 (US4):
  - run governance/migration contracts:
    - `cargo test -p nebula-config governance_policy`
    - `cargo test -p nebula-config migration_requirements`
