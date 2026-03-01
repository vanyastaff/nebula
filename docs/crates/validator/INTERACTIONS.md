# Interactions

## Ecosystem Map (Current + Planned)

### Existing crates (validator has no nebula-* dependencies)

- **Upstream (stdlib/vendor only):** `regex`, `serde`, `serde_json`, `smallvec`, `moka`, `thiserror`. No nebula-* crates — validator is a leaf for the platform.
- **Downstream (depend on nebula-validator):**
  - `nebula-config` — config load/reload validation via validator trait bridge; category naming compatibility pinned by `crates/config/tests/fixtures/compat/validator_contract_v1.json`.
  - `nebula-parameter` — parameter schema and runtime value validation; uses Validate + validate_any / AsValidatable for JSON.
  - `nebula-macros` — may use validator for derive or attribute-based validation.
  - `nebula-sdk` — re-exports or uses validator for authoring and testing.

### Planned / indirect

- `api`: will validate request payloads; expects stable error codes and field paths.
- `workflow` / `engine` / `runtime`: may validate workflow definitions or execution input; use ValidationError shape for diagnostics.

## Downstream Consumers

- **nebula-config:** Uses validator for config validation; category names and error shape must match `validator_contract_v1.json`. Load and reload gate return `ValidationError`/`ValidationErrors`.
- **nebula-parameter:** Converts parameter rules into validator chains; validates `serde_json::Value` via `validate_any` and `AsValidatable`. Same error codes and field-path conventions.
- **nebula-macros / nebula-sdk:** Authoring and macro-generated validation; depend on stable `Validate<T>` and error structure.
- **API (when implemented):** Will map `ValidationError` (code, field, message) to HTTP 400 response body; error code stability is a contract.

## Upstream Dependencies

- **regex:** pattern and content validators (MatchesRegex, Email, Url).
- **serde / serde_json:** AsValidatable for `serde_json::Value`; error serialization (to_json_value).
- **smallvec:** ValidationError params (SmallVec for 0–2 params inline).
- **moka:** optional caching in Cached combinator.
- **thiserror:** not used for ValidationError (custom Display/Error impl); may be used elsewhere.
- **Fallback:** Validation works without moka if Cached is not used; no optional nebula feature flags.

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| validator ↔ config | out | ConfigValidator bridge; error shape + category names match `crates/config/tests/fixtures/compat/validator_contract_v1.json` | sync | ValidationError(s) on load/reload | config does not duplicate rules |
| validator ↔ parameter | out | ValidationRule → validator chains; validate_any / AsValidatable for JSON | sync | ValidationError(s) on invalid param value | parameter owns rule descriptors |
| validator ↔ macros | out | Macros may generate validator! / compose! / any_of! usage | sync | same error shape | macros crate may depend on validator |
| validator ↔ sdk | out | Re-exports or authoring API; stable Validate&lt;T&gt; and error codes | sync | same error shape | sdk depends on validator |
| validator ↔ api (planned) | out | stable error codes + field paths → HTTP 400 body | sync | fail request with structured error | no retries |
| validator ↔ workflow/engine (planned) | out | workflow/node config validation | sync | reject invalid definitions | preflight checks |

## Runtime Sequence

1. Consumer crate builds typed validator chain.
2. Inputs validated at boundary (API/request/config/load).
3. On failure, `ValidationError(s)` mapped to consumer-specific error envelope.
4. On success, downstream execution proceeds.

## Cross-Crate Ownership

- `validator` owns rule semantics and error code meaning.
- `api` owns HTTP representation of validation failures.
- `engine/runtime` own orchestration and retry policies (not validator).
- `sandbox` owns capability policy enforcement.

## Failure Propagation

- failures bubble up as deterministic validation failures.
- retries are generally forbidden for pure validation failures.
- only caller-level transport retries are allowed (outside validator semantics).

## Versioning and Compatibility

- error code stability is a consumer contract.
- breaking change protocol:
  - declare in `MIGRATION.md`
  - major version bump
  - provide code mapping table old -> new.

Field-path compatibility:

- dot-path and JSON pointer contracts are consumer-visible.
- format changes must follow major-version migration protocol.

## Contract Tests Needed

- cross-crate fixture tests for API error mapping.
- compatibility tests for workflow/plugin configs across versions.
- contract suite in this crate:
  - `tests/contract/compatibility_fixtures_test.rs`
  - `tests/contract/typed_dynamic_equivalence_test.rs`
  - `tests/contract/governance_policy_test.rs`
  - `tests/contract/migration_requirements_test.rs`

Downstream consumer requirements:

- config integration must pin shared category mapping fixtures.
- consumer CI should fail if category names drift without migration mapping.
