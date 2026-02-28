# Implementation Plan: Validator Integration in Config Crate

**Branch**: `001-validator-config-integration` | **Date**: 2026-02-28 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-validator-config-integration/spec.md`

## Summary

Integrate `nebula-validator` into `nebula-config` as an explicit activation gate contract for load/reload, enforce last-known-good retention on validation failure, and formalize cross-crate compatibility/governance rules in `docs/crates` with repeatable contract fixtures.

## Technical Context

**Language/Version**: Rust 2024, MSRV 1.92  
**Primary Dependencies**: `tokio`, `serde`, `serde_json`, `thiserror`, `futures`, `nebula-config`, `nebula-validator`, `nebula-log`  
**Storage**: In-memory merged config snapshot (JSON tree), file/env source inputs, docs in `docs/crates/*`  
**Testing**: `cargo test -p nebula-config`, contract/integration fixtures under `crates/config/tests`  
**Target Platform**: Rust workspace services on Linux/macOS/Windows  
**Project Type**: Rust workspace library integration (`crates/config` + `crates/validator`)  
**Performance Goals**: No regression in typed read path; reload validation remains bounded for representative config sizes  
**Constraints**: Invalid candidate must never activate; reload fallback must preserve last-known-good; diagnostics must redact sensitive values  
**Scale/Scope**: Single feature slice focused on config-validator interaction contract and supporting docs/tests in `docs/crates`

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- I. Type Safety First: PASS. Keep typed access and validator contracts explicit.
- II. Isolated Error Handling: PASS. No shared global error type introduced; boundary mapping only.
- III. Test-Driven Development: PASS. Contract tests/fixtures required before behavior changes.
- IV. Async Discipline: PASS. Load/reload validation remains async with cancellation-safe behavior.
- V. Modular Workspace Architecture: PASS. Scope limited to `config` <-> `validator` integration and docs.
- VI. Observability by Design: PASS. Diagnostics contract includes actionable context and redaction.
- VII. Simplicity and YAGNI: PASS. No new abstraction layer beyond required contract surface.
- VIII. Rust API Guidelines and Documentation: PASS. Public behavior documented in crate docs/contracts.

Post-design re-check: PASS. No constitution violations introduced by planning artifacts.

## Project Structure

### Documentation (this feature)

```text
specs/001-validator-config-integration/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── config-validator-contract.md
└── tasks.md
```

### Source Code (repository root)

```text
crates/config/
├── src/
│   ├── core/
│   ├── loaders/
│   ├── validators/
│   └── lib.rs
└── tests/
    ├── contract/
    └── fixtures/

crates/validator/
├── src/
└── tests/

docs/crates/
├── config/
└── validator/
```

**Structure Decision**: Use existing workspace crate boundaries. Implement behavior and contract checks in `crates/config`, keep `crates/validator` as provider contract, and align governance/interaction docs in `docs/crates/config` and `docs/crates/validator`.

## Complexity Tracking

No constitution violations requiring exception tracking.
