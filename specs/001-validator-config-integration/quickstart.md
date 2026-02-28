# Quickstart: Validator Integration in Config Crate

## 1. Validate feature baseline

Run config-focused tests:

```bash
cargo test -p nebula-config
```

Run validator-focused tests:

```bash
cargo test -p nebula-validator
```

## 2. Verify activation gate behavior

- Execute scenarios where candidate config is valid and confirm activation succeeds.
- Execute scenarios where candidate config is invalid and confirm activation is rejected.
- Confirm active config remains last-known-good after invalid reload attempt.

## 3. Verify compatibility contract

- Run contract fixtures for config-validator interaction categories.
- Confirm category semantics remain stable in repeated runs.
- Confirm behavior-significant changes require migration mapping updates.

Reference contract tests:

```bash
cargo test -p nebula-config validator_activation_contract_test
cargo test -p nebula-config validator_reload_rejection_contract_test
cargo test -p nebula-config validator_last_known_good_test
cargo test -p nebula-config validator_category_compatibility_test
```

## 4. Verify diagnostics safety

- Trigger validation failures with sensitive-like keys.
- Confirm diagnostics include source/path context.
- Confirm sensitive values are redacted.

Reference diagnostics tests:

```bash
cargo test -p nebula-config validator_redaction_contract_test
cargo test -p nebula-config validator_diagnostics_context_test
cargo test -p nebula-config validator_runbook_requirements_test
```

## 5. Run quality gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo doc --no-deps --workspace
```
