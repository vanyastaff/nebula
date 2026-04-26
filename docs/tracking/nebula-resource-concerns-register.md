# nebula-resource Concerns Register

**Opened:** 2026-04-24 (cascade Phase 7)
**Status:** Active — design phase complete (2026-04-25); awaits implementation + soak
**Pattern:** modeled on `docs/tracking/credential-concerns-register.md`
**Owner during cascade:** orchestrator (main session)
**Owner post-cascade:** to transfer on implementation PR wave start
**Close condition:** MATURITY.md transition `frontier` → `core` per Strategy §6.4

**Tech Spec FROZEN 2026-04-25** — all 22 `tech-spec-material` rows have `decided` status with section pointers via Tech Spec §15.6. Lifecycle Rule 2 satisfied. See `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md` for resolution mapping.

---

## Label taxonomy

| Label | Definition |
|---|---|
| **strategy-blocking** | Would block Strategy §4 decision; must resolve before Phase 3 CP3 freeze |
| **tech-spec-material** | Phase 6 Tech Spec must encode / address |
| **sub-spec** | Warrants its own spec doc (cross-crate coordination, large subsurface) |
| **standalone-fix** | Can land independently as separate PR outside cascade |
| **post-cascade** | Defer to implementation PR wave or post-merge follow-up |
| **future-cascade** | Defer to a future redesign cycle with defined trigger condition |

---

## Active concerns

### Credential×Resource boundary

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-001 | `Resource::Auth` dead bound; `Resource::Credential` adoption per Tech Spec §3.6 | 🔴 | tech-spec-material | Phase 1 §1.1 convergent | Strategy §4.1 + ADR-0036 | Phase 6 Tech Spec §3 |
| R-002 | `credential_resources` reverse-index never populated → silent revocation drop today; latent `todo!()` panic | 🔴 | tech-spec-material | Phase 1 §1.1 (security-lead correction of Phase 0); `manager.rs:262, 370, 1378, 1400` | Strategy §4.1/§4.2 — atomic landing required | Phase 6 Tech Spec §5 |
| R-003 | `on_credential_revoked` semantics (default body, mechanism) | 🟠 | tech-spec-material | Strategy §4.2 (tech-lead TL-E2) | Invariant: "no further authenticated traffic" post-invocation; mechanism deferred to Tech Spec §5 | Phase 6 Tech Spec §5 |
| R-004 | Rotation dispatch mechanics — parallel `join_all` with per-resource timeout isolation | 🟠 | tech-spec-material | Strategy §4.3 (tech-lead TL-E1) | Invariant: per-resource timeout, NOT global | Phase 6 Tech Spec §5 |
| R-005 | `warmup_pool` must not call `Scheme::default()` under new shape | 🟡 | tech-spec-material | Phase 1 security-lead B-3 amendment | Strategy §4.9 (observability + invariant) | Phase 6 Tech Spec §5 |
| R-006 | `AuthScheme: Clone` bound forces secret cloneability — each clone is another zeroize obligation | 🟡 | future-cascade | Phase 1 §2.2 security-unique | Deferred — requires cross-crate reshape | Coordinate with credential side cascade |
| R-007 | `CredentialId` split import (`nebula_core` vs `nebula-credential`) | 🟡 | post-cascade | Phase 1 §2.2 security-unique | Cosmetic — drive-by fix | Any future PR touching the imports |

### Topology surface

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-010 | Daemon topology has no public start path (pub(crate) barrier) | 🔴 | tech-spec-material | Phase 1 §1.2 dx-tester `runtime/managed.rs:35` | Strategy §4.4: engine-fold extraction | Phase 6 Tech Spec §4 (extraction) |
| R-011 | EventSource same orphan-surface pattern — 0 Manager-level tests | 🔴 | tech-spec-material | Phase 1 §1.6 convergent | Strategy §4.4: extract with Daemon | Phase 6 Tech Spec §4 |
| R-012 | Daemon + EventSource out-of-canon §3.5 ("resource = pool/SDK client") | 🟠 | tech-spec-material | Phase 1 §1.6 tech-lead | Resolved by §4.4 engine-fold | Phase 6 Tech Spec §4 |
| R-013 | Transport topology has 0 Manager-level integration tests | 🟠 | post-cascade | Phase 1 §2.4 tech-lead | Test debt, not structural | Follow-up task after cascade |
| R-014 | Service vs Transport differentiation thin | 🟡 | future-cascade | Phase 1 §2.4 tech-lead | Defer — separation is defensible but low-value | Future cascade trigger: evidence for simplification |

### Manager surface

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-020 | `manager.rs` 2101 L grab-bag | 🟠 | tech-spec-material | Phase 1 §1.4 convergent | Strategy §4.5: file-split into 5 submodules (mod/options/gate/execute/rotation) | Phase 6 Tech Spec §5 |
| R-021 | `register_*_with` builder anti-pattern; inconsistent `with_*` conventions | 🟠 | tech-spec-material | Phase 1 §2.3 rust-senior | Strategy §4.5: resolved by file-split + redesign | Phase 6 Tech Spec §5 |
| R-022 | `register_pooled` silently requires `Auth = ()`; no documented escape for authed adapters | 🟠 | tech-spec-material | Phase 1 §1.5 convergent | Resolved by §4.1 `type Credential: Credential` + `NoCredential` opt-out | Phase 6 Tech Spec §5 |
| R-023 | Drain-abort phase corruption — `graceful_shutdown::Abort` flips phase to Ready without recording failure | 🔴 | tech-spec-material | Phase 1 §1.5 rust-senior `manager.rs:1493-1510`, `runtime/managed.rs:93-102` | Strategy §4.6: absorbed into file-split PR; `ManagedResource::set_failed()` wired | Phase 6 Tech Spec §5 |

### Documentation

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-030 | `docs/api-reference.md` ~50% fabrication rate (`ResourceContext::with_scope/.with_cancel_token`, `AcquireCircuitBreakerPreset`, 4-field `ResourceMetadata`) | 🔴 | tech-spec-material | Phase 1 §1.3 dx-tester | Strategy §4.7: full rewrite after trait shape locks | Phase 6 Tech Spec §6 (docs subsection) |
| R-031 | `docs/adapters.md` compile-fails on 4/7 code blocks; hidden `HasSchema` super-trait requirement | 🔴 | tech-spec-material | Phase 1 §1.3 dx-tester | Strategy §4.7: ground-up rewrite | Phase 6 Tech Spec §6 |
| R-032 | `docs/Architecture.md` describes vanished v1 module map | 🟠 | tech-spec-material | Phase 0 | Strategy §4.7: rewrite OR delete (redundant with README) | Phase 6 Tech Spec §6 |
| R-033 | `docs/README.md` case-drift broken intra-doc links | 🟠 | tech-spec-material | Phase 0 | Strategy §4.7: fix in rewrite | Phase 6 Tech Spec §6 |
| R-034 | `docs/dx-eval-real-world.rs` references nonexistent `nebula_resource::Credential` | 🟠 | tech-spec-material | Phase 1 §1.3 dx-tester | Strategy §4.7: fix, delete, or gate in CI | Phase 6 Tech Spec §6 |
| R-035 | `docs/events.md` variant count 7 vs actual 10 | 🟡 | tech-spec-material | Phase 0 | Strategy §4.7: resolved in doc rewrite | Phase 6 Tech Spec §6 |

### Infrastructure

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-040 | No `deny.toml` wrappers rule for `nebula-resource` despite 5 consumers spanning tiers | 🟠 | standalone-fix | Phase 1 §2.2 security-lead | **SF-1** — separate PR, dispatch to devops before cascade completion | devops (standalone PR) |
| R-041 | No `benches/` directory, not in CodSpeed shard (runtime-critical crate) | 🟡 | post-cascade | Phase 0 devops audit | Follow-up task | Post-cascade bench harness |
| R-042 | Zero feature flags — no slim mode despite heavy deps (telemetry/metrics/resilience) | 🟡 | future-cascade | Phase 0 devops audit | Future cascade trigger: embedded / constrained context requirement | Future cascade |
| R-043 | Macros emit `DeclaresDependencies` for trait not defined in runtime crate | 🟡 | tech-spec-material | Phase 0 §3.6 | Trace wiring in Phase 6 Tech Spec CP1 | Phase 6 Tech Spec §1 |

### Public API surface (idiomatic)

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-050 | 5 associated types × combinatorial `where` bounds; 9/9 tests prove `Runtime == Lease` unused | 🟠 | future-cascade | Phase 1 §2.3 rust-senior | Strategy §5.3: future cascade, trigger when second consumer needs distinct shape | Future cascade |
| R-051 | Reserved-but-unused public API (`AcquireOptions::intent/.tags`, `ErrorScope::Target`, `AcquireIntent::Critical`) | 🟠 | tech-spec-material | Phase 1 §2.3 rust-senior | Strategy §5.2: interim treatment picked in Phase 6 Tech Spec §5 (deprecate? remove? retain?) | Phase 6 Tech Spec §5 |
| R-052 | `Resource::destroy` default no-op encourages leaks | 🟡 | post-cascade | Phase 1 §2.3 rust-senior | Revisit after Phase 4 spike surfaces impact | Phase 4 spike surface OR post-cascade |
| R-053 | `integration/` module name collides with adapter-integration sense | 🟡 | tech-spec-material | Phase 1 §2.4 tech-lead | Strategy §4.5: rename as part of file-split | Phase 6 Tech Spec §5 |
| R-054 | `ResourceMetadata` `#[non_exhaustive]` with one field | 🟡 | post-cascade | Phase 1 §2.3 rust-senior | Cosmetic — leave as-is; `#[non_exhaustive]` preserves future-add safety | Accepted as-is |

### Observability

| ID | Concern | Severity | Labels | Source | Status | Owner |
|----|---------|----------|--------|--------|--------|-------|
| R-060 | Rotation path ships without trace span / counter / `ResourceEvent::CredentialRefreshed` | 🟠 | tech-spec-material | Phase 1 §2.2 security-lead | Strategy §4.9 + Phase 6 CP-review gate (DoD) | Phase 6 Tech Spec §4 (observability section) |

### Positive findings (preserved as invariants)

| ID | Finding | Labels | Source | Status |
|----|---------|--------|--------|--------|
| R-100 | `#[forbid(unsafe_code)]` at crate root | invariant-preservation | Phase 0 | Maintain in Phase 6 Tech Spec §6 |
| R-101 | Zero CVEs in dependency tree | invariant-preservation | Phase 1 §2.2 security-lead | Maintain via cargo audit in CI |
| R-102 | Secrets never hit Debug/Display/log output from resource code | invariant-preservation | Phase 1 §2.2 security-lead | Maintain — verify in Phase 6 Tech Spec §6 |

---

## Summary by label

| Label | Count |
|---|---|
| strategy-blocking | 0 (all resolved in Phase 3 Strategy) |
| tech-spec-material | 22 |
| sub-spec | 0 |
| standalone-fix | 1 (R-040 SF-1 deny.toml) |
| post-cascade | 5 |
| future-cascade | 4 |
| invariant-preservation | 3 |
| **Total** | **35** |

*Row count (35) vs Phase 1 finding count (28) reconciliation:* register adds items from Strategy decisions (e.g., R-003 `on_credential_revoked` default body) and Phase 0 infrastructure findings not represented as Phase 1 severity tags (e.g., R-041/R-042/R-043). Phase 1 28-finding split is in `02-pain-enumeration.md §4`; register reorganizes per ownership rather than origin.

---

## Lifecycle rules

1. **strategy-blocking items** must be 0 before CP3 freeze. At 2026-04-24 Phase 3 lock, count was 0. ✓
2. **tech-spec-material items** must be addressed (status not "open") before Phase 6 Tech Spec CP4 freeze.
3. **standalone-fix items** must ship a PR before Phase 6 Tech Spec CP4 freeze OR be explicitly deferred with a new trigger condition.
4. **post-cascade + future-cascade items** migrate to an issue tracker after Phase 8 summary; this register retires once MATURITY.md transitions `frontier` → `core`.
5. **invariant-preservation items** transition to `PRODUCT_CANON.md §4.x` or the crate README after Phase 6 Tech Spec ratification.

---

## Register updates

- 2026-04-24 T+~135min — Opened by orchestrator (Phase 7 dispatch)
- 2026-04-25 — Cascade continuation: Phase 4 spike PASSED (commit `262665f8`); Phase 6 CP1 ratified (commit `1e416b91`); ADR-0036 + ADR-0037 flipped `proposed` → `accepted`; Phase 6 CP2 ratified (commit `e0f49536`); Phase 6 CP3 + CP4 ratified, Tech Spec FROZEN
- 2026-04-25 — All 22 `tech-spec-material` rows status `decided` per Tech Spec §15.6 mapping. Lifecycle Rule 2 satisfied.
- (Future updates append-only as implementation PR wave + soak progress)
