# Phase 0 Research: Validator Contract Hardening

## Decision 1: Keep typed-first validation as the primary contract

- Decision: Retain `Validate<T>`/`ValidateExt<T>` as stable public surface and keep `validate_any` as a bridge.
- Rationale: Type-bound APIs preserve compile-time guarantees and prevent semantic drift across crates.
- Alternatives considered:
  - Dynamic schema-only validation as primary API: rejected due to weaker compile-time safety.

## Decision 2: Formalize error-envelope compatibility as a first-class contract

- Decision: Treat `ValidationError` code, message shape, field path, and nested details as compatibility fixtures.
- Rationale: API/workflow/plugin consumers depend on deterministic mapping and machine-readable diagnostics.
- Alternatives considered:
  - String-only errors: rejected because consumers lose stable parsing and migration safety.
  - Unversioned ad-hoc fields: rejected due to high regression risk in minor releases.

## Decision 3: Enforce additive-only minor evolution with explicit migration policy

- Decision: Minor versions allow additive validators/combinators only; behavior-significant semantic changes require major release and migration map.
- Rationale: Matches current docs and protects downstream contracts.
- Alternatives considered:
  - Silent semantic changes in minor releases: rejected due to cross-crate breakage risk.

## Decision 4: Use contract tests plus property/integration suites for regression prevention

- Decision: Define compatibility fixtures for error code + field-path stability and combine with existing integration/property/bench tests.
- Rationale: This targets both correctness and deterministic behavior guarantees.
- Alternatives considered:
  - Unit tests only: rejected because cross-crate compatibility failures may be missed.

## Decision 5: Keep synchronous side-effect-free core; no async or retry semantics in validator

- Decision: Validation remains deterministic, synchronous, and side-effect free; retries and backoff stay outside this crate.
- Rationale: Existing architecture and reliability/security docs classify validation failures as deterministic contract failures.
- Alternatives considered:
  - Internal retry/fallback logic inside validator: rejected because it blurs crate boundaries and complicates semantics.

## Decision 6: Bound adversarial behavior through policy contracts and safe diagnostics

- Decision: Explicitly document protections for regex-heavy and nested payloads via caller policies and bounded error aggregation.
- Rationale: Security and reliability docs identify CPU spikes and oversized error trees as primary abuse vectors.
- Alternatives considered:
  - Unbounded error accumulation and free-form messages: rejected for reliability and leakage risk.
