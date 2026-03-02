# Roadmap

`nebula-parameter` roadmap targets stronger contracts between schema, UI, and runtime execution.

## Phase 1: Contract Cleanup

- align all docs/examples with actual API and kind/capability semantics
- publish stable naming conventions for keys/paths and display dependencies
- document required vs nullable behavior per kind

## Phase 2: Validation Depth and Performance

- benchmark deep nested `object/list` validation
- optimize recursive path building and error allocation in large schemas
- add stress tests for large collections and high error counts

## Phase 3: Stronger Type Bridges

- improve typed extraction helpers from `ParameterValues`
- define clearer conversion contracts for numbers/integers/decimals
- reduce ambiguity in `"any"`-typed parameter flows

## Phase 4: Display/Dependency Engine Improvements

- formalize dependency graph extraction from display rules
- detect cycles/contradictory visibility rules at schema build time
- add diagnostics for unreachable parameters

## Phase 5: Toolchain and Stability

- workspace baseline today: Rust `1.93`
- prepare migration path to Rust `1.93+`
- lock machine-readable error/code compatibility expectations
