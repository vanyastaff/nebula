# Implementation Plan: nebula-locale

**Crate**: `nebula-locale` | **Path**: `crates/locale` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The locale crate provides locale negotiation, translation bundle lookup with fallback chains, and localized error rendering. It targets consistent internationalization across API/runtime/action/validator consumers. The crate is planned for a future phase.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (optional, for catalog reload)
**Key Dependencies**: `nebula-core`
**Testing**: `cargo test -p nebula-locale`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ⬜ Planned | Create crate, negotiation/translation MVP, fallback chain |
| Phase 2: Runtime Hardening | ⬜ Planned | Catalog validation, missing-key telemetry, context propagation |
| Phase 3: Scale and Performance | ⬜ Planned | Translation bundle cache, benchmark, memory footprint |
| Phase 4: Ecosystem and DX | ⬜ Planned | Key linting, catalog completeness, dynamic reload |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Create crate MVP; define key namespace and fallback chain spec; localized error rendering adapters.

**Deliverables**:
- `crates/locale` MVP for negotiation and translation
- Key namespace and fallback chain specification
- Localized error rendering adapters for API/runtime/action/validator

**Exit Criteria**:
- Contract tests pass for API/runtime/action/validator consumers

**Risks**:
- Inconsistent legacy key usage across crates

### Phase 2: Runtime Hardening

**Goal**: Catalog validation at startup; missing-key telemetry; standardized context propagation.

**Deliverables**:
- Robust catalog validation and startup checks
- Missing-key telemetry and alerting hooks
- Standardized locale context propagation

**Exit Criteria**:
- Deterministic fallback behavior; actionable observability

### Phase 3: Scale and Performance

**Goal**: Translation bundle cache and lookup optimizations; benchmark; memory bounds.

**Deliverables**:
- Translation bundle cache and lookup optimizations
- Benchmark locale negotiation/render paths
- Tuned memory footprint for multi-locale deployments

**Exit Criteria**:
- Stable latency and memory bounds under target load

### Phase 4: Ecosystem and DX

**Goal**: Key linting; catalog completeness tooling; dynamic catalog reload.

**Deliverables**:
- Key linting and catalog completeness checks
- Staged support for dynamic catalog reload
- Contributor guidelines for localization workflows

**Exit Criteria**:
- Safe and maintainable localization lifecycle in production

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`
- **Depended by**: `nebula-api` (localized error responses), `nebula-validator` (localized validation messages)

## Verification

- [ ] `cargo check -p nebula-locale`
- [ ] `cargo test -p nebula-locale`
- [ ] `cargo clippy -p nebula-locale -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-locale`
