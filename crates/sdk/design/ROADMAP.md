# Roadmap

Phased path to stable, production-ready proc-macros for the Nebula workflow platform. Macros reduce boilerplate for Action, Resource, Plugin, Credential, Parameters, Validator, Config; output must remain compatible with nebula-action and related crates.

## Phase 1: Contract and Output Stability

- **Deliverables:**
  - Document generated code and stability: what each derive produces; which attributes are stable; compatibility with action/plugin/credential/resource traits.
  - Contract tests: derive output compiles and satisfies trait bounds; roundtrip with SDK and engine (e.g. nebula-sdk + engine integration test using derived types).
  - Error messages and attribute validation: invalid attributes produce clear compile errors.
- **Risks:**
  - Trait or attribute changes in action/plugin/credential breaking macro output without macro release.
- **Exit criteria:**
  - All public derives (Action, Resource, Plugin, Credential, Parameters, Validator, Config) documented; generated code passes contract tests.
  - No undocumented breaking change to generated code.

## Phase 2: Attribute and Compatibility Hardening

- **Deliverables:**
  - Attribute set frozen or versioned for patch/minor: additive attributes in minor; removal or behavior change = major.
  - Compatibility matrix: macro version X works with action/plugin/credential version Y; document in README or MIGRATION.
  - Edge cases: optional fields, generics, nested types; no panics in macro expansion.
- **Risks:**
  - Complex type shapes (generics, lifetimes) causing obscure macro errors or wrong output.
- **Exit criteria:**
  - Attribute policy documented; CI tests macro against current action/plugin/credential; MIGRATION for breaking attribute changes.

## Phase 3: Diagnostics and DX

- **Deliverables:**
  - Improved compile errors: suggest correct attribute syntax; point to doc or example when attribute is invalid.
  - Optional: expansion debugging (e.g. cargo expand) documented for authors.
  - No new domain logic in macros; only code generation for existing traits and types.
- **Risks:**
  - Over-engineering diagnostics; proc-macro compile time growth.
- **Exit criteria:**
  - Authors get actionable errors when derive or attributes are wrong; docs point to examples.

## Phase 4: Ecosystem and Versioning

- **Deliverables:**
  - Macro crate version aligned with platform: when action/plugin/credential have breaking changes, macro major bump and MIGRATION for attribute or output changes.
  - Re-export and usage from nebula-sdk stable; SDK prelude documents macro path.
  - No duplicate or conflicting macros (single source of truth for Action, Parameters, etc.).
- **Risks:**
  - Workspace crates upgrading at different times; macro used with older action crate.
- **Exit criteria:**
  - Version compatibility documented; authors using SDK prelude get compatible macro output by default.

## Metrics of Readiness

- **Correctness:** Generated code implements traits correctly; engine and runtime accept macro-generated actions/plugins/credentials.
- **Stability:** Attribute set and output shape stable in patch/minor; breaking = major + MIGRATION.
- **Operability:** Clear errors and docs; no unsafe in macro crate.
