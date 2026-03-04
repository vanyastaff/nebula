# Test Strategy

## Test Pyramid

- **Unit:** Per-derive tests: parse valid attribute, expand, compile expanded code in test (compile_test or similar). Invalid attribute yields compile error in test. Test each derive (Action, Resource, Plugin, Credential, Parameters, Validator, Config) with minimal and full attributes.
- **Integration:** Generated type used in dummy crate that depends on action (or resource/plugin/credential); compile and run trait methods (e.g. metadata()). Contract test: macro output + action crate = compiles and runs.
- **Contract:** Generated impl satisfies trait; engine/runtime/plugin/credential accept the type. Formal contract test: expand in test, compile with trait crate, assert trait methods return expected shape.
- **E2E:** Out of scope for macro crate (engine/API own E2E; they may use macro-generated types).

## Critical Invariants

- For valid input, expansion compiles when combined with trait crate (action/resource/plugin/credential). No panic in macro; invalid input yields compile_error, not panic.
- Generated code implements the trait (compiler enforces); metadata() or equivalent returns values derived from attributes.
- No unsafe in macro crate (audit or CI).

## Scenario Matrix

- **Happy path:** Valid attributes → expand → compile in test with trait crate → call trait method → assert.
- **Invalid path:** Missing required attribute → compile error in test (try_compile or similar). Wrong type in attribute → error.
- **Compatibility path:** After trait crate change, contract test passes or fails explicitly; update macro or document compatibility.

## Tooling

- **trybuild or compiletest:** For testing compile errors (invalid macro input). Optional.
- **CI:** cargo test; optional contract test with action/plugin/credential (dev-deps or separate crate).
- **cargo expand:** Document for author debugging; optional CI check that expand produces expected structure.

## Exit Criteria

- All derives have at least one test that expands and compiles (with trait crate). Invalid attribute tests where feasible. No unsafe. Contract test (when added) in CI.
