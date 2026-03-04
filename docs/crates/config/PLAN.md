# Implementation Plan: nebula-config

**Crate**: `nebula-config` | **Path**: `crates/config` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-config provides multi-source configuration loading with hot-reload, typed getters, and precedence semantics for the Nebula workflow engine. Phases 1-2 are complete with contract baseline and validation hardening. Phase 3 (reliability/reload) is mostly done; Phase 4 (source ecosystem expansion) is next.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (watcher, reload)
**Key Dependencies**: async-trait, tokio, tokio-util, futures, serde, serde_json, thiserror, notify (file watcher), toml (optional), yaml-rust2 (optional), chrono, url, dashmap, nebula-log, nebula-validator
**Testing**: `cargo test -p nebula-config`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract Baseline and Documentation | ✅ Done | SPEC-template docs, precedence/path semantics, governance codified |
| Phase 2: Compatibility and Validation Hardening | ✅ Done | Compatibility fixtures, validator trait bridge, contract tests |
| Phase 3: Reliability and Reload Semantics | 🔄 Mostly Done | Atomic reload verified; watcher lifecycle/backoff guidance remaining |
| Phase 4: Source Ecosystem Expansion | ⬜ Planned | Remote/database/kv source adapters, security model |

## Phase Details

### Phase 1: Contract Baseline and Documentation (Completed)

**Goal**: Align docs and fixtures as single source of truth for config API.

**Deliverables**:
- SPEC-template docs and interaction contracts aligned
- Precedence/path semantics documented in API + fixtures
- Governance/migration requirements codified in contract tests

**Exit Criteria**:
- Docs and fixtures treated as source of truth in CI

**Risks**:
- Downstream consumers may still rely on undocumented local conventions

### Phase 2: Compatibility and Validation Hardening (Completed)

**Goal**: Harden validation integration and ensure backward compatibility.

**Deliverables**:
- Compatibility fixtures for precedence/path/type conversion
- Direct validator trait bridge integrated into ConfigValidator
- Validator compatibility + governance contract tests

**Exit Criteria**:
- Contract suite remains green across crate changes and releases

**Risks**:
- Stricter validation may expose latent config debt in late-adopting consumers

### Phase 3: Reliability and Reload Semantics (Mostly Done)

**Goal**: Ensure reliable reload behavior under all failure modes.

**Deliverables**:
- Atomic reload behavior verification
- Reload failure preservation of last-known-good state
- Failure-mode guidance documented
- Stronger watcher lifecycle/backoff guidance for high-frequency reload workloads (remaining)

**Exit Criteria**:
- Explicit watcher/backoff guidance and targeted stress tests

**Risks**:
- Race conditions in high-frequency reload scenarios

### Phase 4: Source Ecosystem Expansion

**Goal**: Expand config sources beyond local files.

**Deliverables**:
- Production-ready remote/database/kv source adapters
- Security model for remote source auth and trust

**Exit Criteria**:
- Source adapter contracts and security tests pass

**Risks**:
- Increased attack surface and operational complexity

## Dependencies

| Depends On | Why |
|-----------|-----|
| nebula-log | Logging for reload events and config operations |
| nebula-validator | ConfigValidator bridge for typed validation |

| Depended By | Why |
|------------|-----|
| nebula-resilience | Reads resilience policies from config |

## Verification

- [ ] `cargo check -p nebula-config`
- [ ] `cargo test -p nebula-config`
- [ ] `cargo clippy -p nebula-config -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-config`
