# Tasks: nebula-locale

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `LOC`

---

## Phase 1: Contract and Safety Baseline ⬜

**Goal**: Create crate; locale negotiation/translation MVP; fallback chain; localized error adapters.

- [ ] LOC-T001 Create `crates/locale` crate with `Cargo.toml` and workspace registration
- [ ] LOC-T002 Define key namespace specification in `src/namespace.rs` — naming conventions, separator rules
- [ ] LOC-T003 [P] Implement fallback chain in `src/fallback.rs` — e.g., en-US → en → default
- [ ] LOC-T004 [P] Implement `LocaleNegotiator::negotiate(requested: &[&str], supported: &[&str]) -> Locale`
- [ ] LOC-T005 Implement `TranslationBundle::lookup(key, locale) -> Option<&str>` with fallback chain
- [ ] LOC-T006 Implement localized error rendering adapter for `nebula-validator` error messages
- [ ] LOC-T007 [P] Implement localized error rendering adapter for `nebula-api` HTTP responses
- [ ] LOC-T008 Write contract tests for API/runtime/action/validator consumers in `tests/consumers.rs`

**Checkpoint**: Contract tests pass for all consumer crates; fallback chain deterministic.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Catalog validation at startup; missing-key telemetry; locale context propagation.

- [ ] LOC-T009 Implement catalog validation at startup — report unknown keys, duplicates, missing fallback coverage
- [ ] LOC-T010 [P] Emit structured missing-key telemetry event when key not found in any fallback
- [ ] LOC-T011 [P] Implement standardized locale context propagation — extract from HTTP headers/request context
- [ ] LOC-T012 Write test: deterministic fallback behavior for all missing-key scenarios
- [ ] LOC-T013 Verify no silent fallback masking content gaps — all fallbacks observable

**Checkpoint**: Deterministic fallback; missing-key observability; startup validation catches gaps.

---

## Phase 3: Scale and Performance ⬜

**Goal**: Bundle cache; benchmark negotiation/render paths; memory footprint control.

- [ ] LOC-T014 Implement translation bundle cache in `src/cache.rs` — LRU or arena-based
- [ ] LOC-T015 [P] Add criterion benchmarks for locale negotiation in `benches/negotiation.rs`
- [ ] LOC-T016 [P] Add benchmarks for translation lookup with cache hit/miss
- [ ] LOC-T017 Tune memory footprint for multi-locale deployments — measure and document memory usage

**Checkpoint**: Negotiation/render within UX latency budget; memory bounded.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Key linting; catalog completeness; dynamic reload; contributor guidelines.

- [ ] LOC-T018 Implement key linting tool — detect unused keys, key collisions, missing translations
- [ ] LOC-T019 [P] Implement catalog completeness check — verify all supported locales have all required keys
- [ ] LOC-T020 [P] Implement staged support for dynamic catalog reload (hot-reload without restart)
- [ ] LOC-T021 Write contributor guidelines for adding new locales and keys

**Checkpoint**: Key linting and completeness checks run in CI; dynamic reload safe and documented.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-locale`
- [ ] `cargo test -p nebula-locale`
- [ ] `cargo clippy -p nebula-locale -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-locale`
- [ ] No unresolved key collisions or fallback ambiguity in CI
