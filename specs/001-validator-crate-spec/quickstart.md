# Quickstart: Validator Contract Hardening

## 1. Baseline checks

Run validator-focused tests first:

```bash
cargo test -p nebula-validator
```

Run workspace quality gates required by constitution:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
```

Task tracker alignment:

- `Phase 1/2`: complete setup + foundational contract artifacts before story work.
- `US1`: validate compatibility fixtures and typed/dynamic equivalence.
- `US2`: validate combinator semantics and adversarial/perf scenarios.
- `US3`: validate error schema, secret redaction, and nested bounds.
- `US4`: validate governance/migration policy checks.

## 2. Verify public API contract

- Confirm stable exports listed in `contracts/validator-public-api.md`.
- Ensure no behavior/signature regressions for existing validator/combinator APIs.
- If behavior changes are required, classify as major and update migration docs.

## 3. Verify error-envelope contract

- Validate fixture payloads against `contracts/validation-error-envelope.schema.json`.
- Confirm code and field-path stability across compatibility fixtures.
- Confirm sensitive inputs are not exposed in `message`, `help`, or `params`.

## 4. Verify integration contracts

- Run or update consumer fixtures for `api`, `workflow`, `plugin`, and `runtime` mappings.
- Ensure deterministic outcomes for equivalent typed and dynamic validation paths.

## 5. Prepare release governance artifacts

- Update `docs/crates/validator/MIGRATION.md` when deprecations or break candidates appear.
- Record compatibility impact and mapping table for changed semantics.
- Keep additive-only minor release policy for validator rules and combinators.
