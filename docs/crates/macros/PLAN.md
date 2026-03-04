# Implementation Plan: nebula-macros

**Crate**: `nebula-macros` | **Path**: `crates/macros` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-macros provides proc-macros (derive and attribute) for the Nebula workflow platform, reducing boilerplate for Action, Resource, Plugin, Credential, Parameters, Validator, and Config types. The focus is on output stability, compatibility with downstream trait crates, and clear diagnostics for macro authors.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: N/A (proc-macro crate; compile-time only)
**Key Dependencies**: syn (full, extra-traits), quote, proc-macro2
**Dev Dependencies**: trybuild, serde, serde_json, toml, yaml-rust2, async-trait, nebula-validator
**Testing**: `cargo test -p nebula-macros`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract and Output Stability | ⬜ Planned | Document generated code, contract tests, error messages |
| Phase 2: Attribute and Compatibility Hardening | ⬜ Planned | Attribute freeze/versioning, compatibility matrix, edge cases |
| Phase 3: Diagnostics and DX | ⬜ Planned | Improved compile errors, expansion debugging |
| Phase 4: Ecosystem and Versioning | ⬜ Planned | Version alignment, SDK re-export, single source of truth |

## Phase Details

### Phase 1: Contract and Output Stability

**Goal**: Establish documented, tested contracts for all generated code.

**Deliverables**:
- Document generated code and stability: what each derive produces; which attributes are stable; compatibility with action/plugin/credential/resource traits
- Contract tests: derive output compiles and satisfies trait bounds; roundtrip with SDK and engine
- Error messages and attribute validation: invalid attributes produce clear compile errors

**Exit Criteria**:
- All public derives (Action, Resource, Plugin, Credential, Parameters, Validator, Config) documented; generated code passes contract tests
- No undocumented breaking change to generated code

**Risks**:
- Trait or attribute changes in action/plugin/credential breaking macro output without macro release

### Phase 2: Attribute and Compatibility Hardening

**Goal**: Freeze or version attribute sets and ensure cross-crate compatibility.

**Deliverables**:
- Attribute set frozen or versioned for patch/minor: additive attributes in minor; removal or behavior change = major
- Compatibility matrix: macro version X works with action/plugin/credential version Y
- Edge cases: optional fields, generics, nested types; no panics in macro expansion

**Exit Criteria**:
- Attribute policy documented; CI tests macro against current action/plugin/credential; MIGRATION for breaking attribute changes

**Risks**:
- Complex type shapes (generics, lifetimes) causing obscure macro errors or wrong output

### Phase 3: Diagnostics and DX

**Goal**: Improve developer experience with better compile errors and debugging tools.

**Deliverables**:
- Improved compile errors: suggest correct attribute syntax; point to doc or example when attribute is invalid
- Optional: expansion debugging (cargo expand) documented for authors
- No new domain logic in macros; only code generation for existing traits and types

**Exit Criteria**:
- Authors get actionable errors when derive or attributes are wrong; docs point to examples

**Risks**:
- Over-engineering diagnostics; proc-macro compile time growth

### Phase 4: Ecosystem and Versioning

**Goal**: Align macro versioning with platform and ensure stable SDK re-export.

**Deliverables**:
- Macro crate version aligned with platform: breaking changes in action/plugin/credential trigger macro major bump and MIGRATION
- Re-export and usage from nebula-sdk stable; SDK prelude documents macro path
- No duplicate or conflicting macros (single source of truth for Action, Parameters, etc.)

**Exit Criteria**:
- Version compatibility documented; authors using SDK prelude get compatible macro output by default

**Risks**:
- Workspace crates upgrading at different times; macro used with older action crate

## Dependencies

| Depends On | Why |
|-----------|-----|
| (none at runtime) | Proc-macro crate has no runtime internal dependencies |
| nebula-validator (dev) | Used in trybuild tests for derive validation |

| Depended By | Why |
|------------|-----|
| nebula-action | Derives Action trait |
| nebula-sdk | Re-exports macros for plugin/action authors |

## Verification

- [ ] `cargo check -p nebula-macros`
- [ ] `cargo test -p nebula-macros`
- [ ] `cargo clippy -p nebula-macros -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-macros`
