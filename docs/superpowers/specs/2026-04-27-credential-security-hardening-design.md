---
name: credential security hardening (post-audit 2026-04-27 SEC cluster)
status: draft (writing-skills output 2026-04-27)
date: 2026-04-27
authors: [vanyastaff, Claude]
phase: parallel-track (does not block П3 kickoff)
scope: cross-cutting — nebula-credential + nebula-engine (rotation path)
related:
  - docs/tracking/credential-audit-2026-04-27.md §XII Errata
  - docs/adr/0028-cross-crate-credential-invariants.md (N10 plaintext invariant)
  - docs/adr/0030-engine-owns-credential-orchestration.md §4 redaction gate
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.5 sensitivity dichotomy
defers-to:
  - 2026-04-27-credential-aad-key-id-redesign-design.md (SEC-03 — separate ADR amendment track)
  - 2026-04-27-credential-id-copy-migration-design.md (PERF-01/02/IDIOM-10 — architect bundle)
---

# Credential Security Hardening (post-audit 2026-04-27)

## §0 Meta

**Scope.** Implementation-ready design for the SEC cluster of `docs/tracking/credential-audit-2026-04-27.md` after §XII Errata reclassifications. Closes 7 production-track findings: SEC-01 (bounded reader), SEC-02 (URL-validate `error_uri`), SEC-05 + SEC-06 (plaintext lifecycle invariant N10), SEC-08 + SEC-11 (API surface tightening), SEC-09 + SEC-10 (Zeroizing wrapper discipline), and conditionally SEC-13 (refresh err redaction).

**Non-goals.**
- SEC-03 (AAD/`key_id` redesign) — separate spec with ADR-0028 amendment, different review depth, different rollback shape per Errata §XII.G.
- PERF-01 / PERF-02 / IDIOM-10 (`CredentialId: Copy` bundle) — architect-level breaking change, separate spec.
- SEC-04 — false alarm per Errata §XII.C; doc edit can be folded into Stage 4 §6.
- SEC-15 / SEC-17 (RUSTSEC sweep / pinning) — independent CI hygiene task, not gated by this spec.

**Phase note.** This spec is a **parallel-track** to Strategy §6.5 frozen sub-spec queue (ProviderRegistry / multi-step / schema migration / trigger↔credential / WS events). It does NOT block П3 kickoff and does NOT supersede §6.5 ordering. The day-numbered execution timeline is informational, not roadmap-of-record.

**Reading order.** §0 (this) → §1 (threat model) → §2-§6 (per-stage design) → §7 (test strategy) → §8 (acceptance) → §9 (deferred) → §10 (migration).

**Freeze policy.** This spec freezes after first commit; supersede via ADR amendment or new spec. Stage-execution may produce minor amendment notes inline (date-stamped) when invariants surface during work — but only if they do not change the SEC item set in scope.

## §1 Threat model recap

| Stage | Threat | Source / invariant |
|---|---|---|
| Stage 0 (SEC-13) | IdP echoes `refresh_token` in `error_description` → propagated to operator logs/SIEM via `TokenRefreshError::TokenEndpoint{summary}` | OAuth2 RFC 6749 §5.2 ambiguity; ADR-0030 §4 redaction gate boundary |
| Stage 1 (SEC-08, SEC-11) | Plugin / downstream caller produces envelopes outside AAD-mandatory contract (SEC-11) or serializes `SecretString` to arbitrary sink bypassing `[REDACTED]` (SEC-08) | API surface area larger than necessary |
| Stage 2 (SEC-05, SEC-06, SEC-09, SEC-10) | Plaintext credential material crosses `tokio::spawn` / blocking-pool boundaries via `CredentialGuard::clone()` or `SchemeGuard: Send` violation; or `format!("Bearer {}")` produces non-Zeroizing intermediate; or `.expose_secret().to_owned()` creates unwrapped `String` before `Zeroizing::new` wrap | PRODUCT_CANON §4.2 invariant N10: «plaintext does not cross spawn boundary» |
| Stage 3 (SEC-01, SEC-02) | Compromised / MITM IdP returns oversized response body (DoS/OOM despite 30s timeout) or injects control-chars / phishing URL into `error_uri` (log injection / operator phishing) | OAuth2 RFC 6749 §5.2 `error_uri` field semantics |

## §2 Stage 0 — SEC-13 verify-first (Day 1)

**Goal.** Determine whether ADR-0030 §4 redaction CI gate fires on the refresh error path (`token_refresh.rs` → `TokenRefreshError::TokenEndpoint{summary}`). Conditional fix only if gate does not fire.

**Verification protocol:**

1. Read `docs/adr/0030-engine-owns-credential-orchestration.md` §4 to identify the canonical redaction CI gate name and assertion target.
2. Search workspace for the test/probe implementing that gate:
   - `cargo nextest run --workspace 2>&1 | grep -i redact`
   - `grep -r "redact\|REDACTED" crates/credential crates/engine --include='*.rs'`
   - `grep -r "TokenRefreshError::TokenEndpoint" crates/`.
3. Manual trace: construct a test fixture where IdP returns `{"error":"invalid_grant","error_description":"refresh_token=abc123 expired"}`. Verify the rendered `Display` of the resulting `TokenRefreshError` does not contain `abc123`.
4. Output: written verdict in Stage 0 PR description — either «GATE FIRES, SEC-13 dropped from scope, register row flipped to non-finding» OR «GATE DOES NOT FIRE on this path, fix required (proceed to Stage 0.5)».

**Stage 0.5 (only if gate does not fire).**

- Add explicit redaction in `crates/engine/src/credential/rotation/token_refresh.rs`: filter `error_description` content through a redaction predicate. Initial heuristic: token-shaped substrings of length ≥20 OR substrings containing `=` followed by ≥16 chars OR substring matching `(?i)(refresh|access|bearer)_?(token|tok)\\s*[=:]\\s*\\S+`.
- Test: `crates/engine/tests/refresh_err_redaction_token_in_description.rs` — wiremock IdP returning crafted `error_description`, assert `Display`-rendered error contains `[REDACTED]` not `abc123`.

**Landing gate (Stage 0):**
- Verification verdict written in PR description.
- If fix added: 1 new test passing under `cargo nextest run -p nebula-engine credential::refresh::redaction`.
- `docs/tracking/credential-audit-2026-04-27.md` §XII.E updated to reflect SEC-13 disposition.

**Files touched:**

Created (conditional):
- `crates/engine/tests/refresh_err_redaction_token_in_description.rs`

Modified (conditional):
- `crates/engine/src/credential/rotation/token_refresh.rs` — add `redact_error_description` helper

## §3 Stage 1 — Visibility tightening (Day 2)

**Goal.** Eliminate two API leaks where `pub` surface allows arbitrary downstream callers to bypass invariants.

**Sub-issues:**

| ID | Site | Fix |
|---|---|---|
| SEC-08 | `crates/credential/src/secrets/serde_secret.rs:12-14` — `pub fn serialize(&SecretString, S)` module-public | `pub` → `pub(crate)`; existing internal callers unaffected |
| SEC-11 | `crates/credential/src/secrets/crypto.rs:158-177` — `pub fn encrypt` (without AAD/`key_id`) | **Delete** the function; force callers to `encrypt_with_key_id` |

**Design — SEC-11 deletion rationale.** Keeping bare `encrypt` even as `pub(crate)` invites future engine-internal callers to bypass AAD. The function is small enough that any internal call site can switch to `encrypt_with_key_id` with a known-empty `key_id` mapping (same effect, but goes through the AAD-mandatory contract). Removal forces every encryption path through one chokepoint.

**Compile-fail probes (mandatory landing gate):**

- `crates/credential/tests/compile_fail_serde_secret_pub.rs` — external attempt to call `nebula_credential::secrets::serde_secret::serialize` from a probe fixture fails with `E0603 module private`.
- `crates/credential/tests/compile_fail_encrypt_no_aad_removed.rs` — call to `nebula_credential::secrets::crypto::encrypt` fails with `E0425 cannot find function`.

**Files touched:**

Created:
- `crates/credential/tests/compile_fail_serde_secret_pub.rs`
- `crates/credential/tests/probes/serde_secret_pub.rs` + `serde_secret_pub.stderr`
- `crates/credential/tests/compile_fail_encrypt_no_aad_removed.rs`
- `crates/credential/tests/probes/encrypt_no_aad_removed.rs` + `encrypt_no_aad_removed.stderr`

Modified:
- `crates/credential/src/secrets/serde_secret.rs` — `pub` → `pub(crate)` on `serialize`
- `crates/credential/src/secrets/crypto.rs` — delete `pub fn encrypt`; audit + migrate internal callers to `encrypt_with_key_id`

**Landing gate (Stage 1):**
- Both compile-fail probes pass under `cargo nextest run -p nebula-credential --test 'compile_fail_*'`.
- `cargo clippy --workspace -- -D warnings` green.
- No internal call site of `crypto::encrypt` remains (`grep -r "crypto::encrypt[^_]" crates/` returns empty).

## §4 Stage 2 — Plaintext lifecycle (Day 3-6) — N10 invariant closure

**Goal.** Close PRODUCT_CANON §4.2 invariant N10 violation cluster: «plaintext does not cross spawn boundary».

**Sub-issues:**

| ID | Site | Fix |
|---|---|---|
| SEC-05 | `crates/credential/src/secrets/guard.rs:64-71` — `CredentialGuard: Clone` derived | Remove `Clone` impl; `CredentialGuard` becomes `!Clone` |
| SEC-06 | `crates/credential/src/secrets/scheme_guard.rs:64` — `SchemeGuard` is implicitly `Send` if `Scheme: Send` | Add explicit `!Send` marker via `_marker: PhantomData<*const ()>` field |
| SEC-09 | `crates/credential/src/credentials/oauth2.rs:125-128` — `format!("Bearer {}", token.expose_secret())` produces non-Zeroizing `String` intermediate | `bearer_header()` constructs `Zeroizing<String>` directly; `format!` macro replaced with `let mut s = Zeroizing::<String>::new(String::with_capacity(...)); write!(&mut s, ...)?;` |
| SEC-10 | `crates/engine/src/credential/rotation/token_refresh.rs:62-72` — `expose_secret().to_owned()` creates unwrapped `String` before `Zeroizing::new` wrap; ×3 sites (refresh_tok / client_id / client_secret) | Single-expression form: `Zeroizing::new(secret.expose_secret().to_owned())` does NOT close gap (compiler may produce temp `String` before move-into-Zeroizing). Introduce `secret.to_zeroizing_string()` helper on `&SecretString` that internally allocates inside `Zeroizing::with_capacity` and copies via `extend_from_slice`. Use at all 3 sites. |

**Critical: order of operations within Stage 2.** SEC-05 + SEC-06 are TYPE-level changes (compile-fail probes fire). SEC-09 + SEC-10 are PATTERN-level (no compile-fail; runtime drop test or visual review). Land SEC-05 + SEC-06 first to lock the type invariant; then SEC-09 + SEC-10 within the same stage so runtime patterns are closed inside the locked type frame.

**Compile-fail probes (mandatory landing gate):**

- `crates/credential/tests/compile_fail_credential_guard_clone.rs` — `let g2 = guard.clone()` fails with `E0599 no method clone`.
- `crates/credential/tests/compile_fail_scheme_guard_send.rs` — `tokio::spawn(async move { let _ = guard; })` fails with `E0277 SchemeGuard cannot be sent between threads safely`.

**Runtime tests (zeroization verification):**

- `crates/credential/tests/zeroize_drop_oauth2_bearer.rs` — construct `OAuth2Token`, capture pointer of `Zeroizing<String>` returned by `.bearer_header()`, drop, assert memory at the captured address is zeroed (use `std::ptr::read_volatile` for safety; document `#[ignore]` if MIRI-incompatible — fall back to deterministic-drop verification via a counter).
- `crates/engine/tests/zeroize_drop_token_refresh_intermediates.rs` — refresh path with `tokio::test`; introduces a `#[derive(Zeroize, Drop)]` instrumented wrapper to count drops; asserts all 3 sites (refresh_tok / client_id / client_secret) drop their owned-string exactly once with zeroize.

**Files touched:**

Created:
- `crates/credential/tests/compile_fail_credential_guard_clone.rs`
- `crates/credential/tests/probes/credential_guard_clone.rs` + `credential_guard_clone.stderr`
- `crates/credential/tests/compile_fail_scheme_guard_send.rs`
- `crates/credential/tests/probes/scheme_guard_send.rs` + `scheme_guard_send.stderr`
- `crates/credential/tests/zeroize_drop_oauth2_bearer.rs`
- `crates/engine/tests/zeroize_drop_token_refresh_intermediates.rs`

Modified:
- `crates/credential/src/secrets/guard.rs` — drop `Clone` impl on `CredentialGuard`
- `crates/credential/src/secrets/scheme_guard.rs` — add `_marker: PhantomData<*const ()>`
- `crates/credential/src/credentials/oauth2.rs` — `bearer_header()` returns `Zeroizing<String>` constructed via `write!` into pre-allocated `Zeroizing<String>`
- `crates/credential/src/secrets/secret_string.rs` (or equivalent) — add `to_zeroizing_string(&self) -> Zeroizing<String>` helper method on `SecretString`
- `crates/engine/src/credential/rotation/token_refresh.rs` — replace `.expose_secret().to_owned()` with `.to_zeroizing_string()` at 3 sites (refresh_tok / client_id / client_secret)

**Landing gate (Stage 2):**
- 2 compile-fail probes + 2 runtime tests green.
- N10 invariant cluster closure noted in `docs/tracking/credential-concerns-register.md` (SEC-05/06/09/10 rows: `proposed` → `decided` with stage commit SHA).
- Manual review by security-lead recommended (this stage carries the most weight per §1 threat model).

## §5 Stage 3 — IdP boundary (Day 7-8)

**Goal.** Harden OAuth2 IdP request/response boundary against compromised / MITM IdP.

**Sub-issues:**

| ID | Site | Fix |
|---|---|---|
| SEC-01 | `crates/engine/src/credential/rotation/token_refresh.rs:109` — `resp.text().await` unbounded on error path (timeout-only mitigation per Errata) | Replace with bounded reader using existing `OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES` (256 KiB). Helper: `read_token_response_limited(resp).await -> Result<String, BodyTooLarge>` |
| SEC-02 | `crates/engine/src/credential/rotation/token_refresh.rs:170-173` — `error_uri` concatenated raw into operator-facing summary | `sanitize_error_uri` helper: `Url::parse` → reject if scheme not in `["https"]` → length cap 256 chars → strip control chars (`\x00-\x1F` + `\x7F`) → enclose in visible delimiters |

**Design — SEC-02 sanitizer:**

```rust
fn sanitize_error_uri(raw: &str) -> Cow<'_, str> {
    use std::borrow::Cow;
    let parsed = match url::Url::parse(raw) {
        Ok(u) if u.scheme() == "https" => u,
        _ => return Cow::Borrowed("[invalid_error_uri_redacted]"),
    };
    let s = parsed.to_string();
    if s.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Cow::Borrowed("[control_chars_in_error_uri_redacted]");
    }
    if s.len() > 256 {
        return Cow::Owned(format!("{}…[truncated]", &s[..256]));
    }
    Cow::Owned(s)
}
```

Sanitizer return value is enclosed in summary as `error_uri=<sanitized>` so log readers see it as a deliberate field, not a free-form sentence injection.

**Tests (wiremock-driven):**

- `crates/engine/tests/oauth_idp_oversized_body_bounded.rs` — wiremock IdP returns 1 MiB error body; assert reader returns `BodyTooLarge` error (not OOM, not 30s wait — bounded fail-fast).
- `crates/engine/tests/oauth_idp_error_uri_validation.rs` — table-driven cases:
  - `http://attacker.example` → rejected (scheme allowlist)
  - `https://valid.example/ok` → passes through unchanged
  - `https://x.example/[control char \x01]` → rejected (control char strip)
  - 300-char `https://x.example/aaaa…` → truncated at 256 chars + `…[truncated]` suffix
  - `javascript:alert(1)` → rejected (parse fails or scheme rejected)
  - empty string → rejected (parse fails)

**Files touched:**

Created:
- `crates/engine/tests/oauth_idp_oversized_body_bounded.rs`
- `crates/engine/tests/oauth_idp_error_uri_validation.rs`

Modified:
- `crates/engine/src/credential/rotation/token_refresh.rs` — both fixes inline; `read_token_response_limited` + `sanitize_error_uri` helpers added at module top.

**Landing gate (Stage 3):**
- 2 wiremock tests pass under `cargo nextest run -p nebula-engine credential::rotation`.
- No regression in existing OAuth2 happy-path integration tests.

## §6 Stage 4 — Doc sync (Day 9)

**Goal.** Per `feedback_observability_as_completion.md`, all invariant changes flow into observability/maturity docs in the same stage-batch (not as a follow-up). DoD requirement.

**Updates:**

| Path | Change |
|---|---|
| `docs/MATURITY.md` row `nebula-credential` | Append «Security hardening 2026-04-27 SEC-cluster (PR `<sha>`)» under «Audited» column |
| `docs/OBSERVABILITY.md` §credential-events | Add metric `credential.refresh.err_uri_rejected_total` (counter for SEC-02 sanitizer rejections); add span attr `credential.refresh.body_truncated` (boolean for SEC-01 truncation) |
| `docs/UPGRADE_COMPAT.md` | Add row «`CredentialGuard: !Clone`, `SchemeGuard: !Send`, `crypto::encrypt` removed, `serde_secret::serialize` → `pub(crate)`» — breaking, same-major version (active dev mode) |
| `docs/tracking/credential-concerns-register.md` | Flip rows for SEC-01/02/05/06/08/09/10/11 (and SEC-13 if Stage 0.5 ran) from `proposed` → `decided` with PR SHA |
| `docs/tracking/credential-audit-2026-04-27.md` §XII.E | Append footer «Implementation status: Stages 0-3 landed at PR `<sha>`, Stage 4 at `<sha>`» |
| `docs/GLOSSARY.md` | Add 5 missing terms identified by spec-auditor: **Plane B**, **Pending** (rotation FSM state — distinct from `PendingDrain`), **Dynamic** (provider class), **sentinel** / **N=3-in-1h** (refresh coordinator escalation threshold), **herd-сценарий** (refresh stampede). Definitions pulled from existing inline references in adjacent specs |
| `crates/credential/src/secrets/crypto.rs:136-142` | (optional SEC-04 doc fix) doc comment «OS CSPRNG» → «CSPRNG seeded from OS via `getrandom`» |
| `CHANGELOG.md` | One-line summary entry under unreleased section |

**Landing gate (Stage 4):**
- All updated docs render cleanly (no broken markdown links via `mdbook test` or local viewer).
- Register totals table audited per `credential-concerns-register.md` §Maintenance contract (counts mutually consistent).

## §7 Test strategy

**Compile-fail probes — 4 mandatory:**

| Probe | Stage | Expected error |
|---|---|---|
| `compile_fail_serde_secret_pub.rs` | 1 | `E0603 module private` |
| `compile_fail_encrypt_no_aad_removed.rs` | 1 | `E0425 cannot find function` |
| `compile_fail_credential_guard_clone.rs` | 2 | `E0599 no method clone` |
| `compile_fail_scheme_guard_send.rs` | 2 | `E0277 SchemeGuard cannot be sent between threads` |

**Runtime tests — 4 mandatory + 1 conditional:**

| Test | Stage | Driver |
|---|---|---|
| `zeroize_drop_oauth2_bearer.rs` | 2 | unit (drop count + ptr::read_volatile) |
| `zeroize_drop_token_refresh_intermediates.rs` | 2 | tokio::test (instrumented wrappers) |
| `oauth_idp_oversized_body_bounded.rs` | 3 | wiremock |
| `oauth_idp_error_uri_validation.rs` | 3 | wiremock (table-driven 6 cases) |
| `refresh_err_redaction_token_in_description.rs` | 0.5 (conditional) | wiremock |

**Test discipline:**
- Each test ID maps to one SEC ID for traceability.
- All under `cargo nextest run -p nebula-credential -p nebula-engine`.
- CI matrix already includes both crates.

**Out-of-scope for this spec (per audit §V Low-priority):**
- Property tests / fuzz on encryption round-trip (TEST-06).
- `cargo fuzz` on deserialization boundaries (TEST-07).
- insta-snapshot tests on events (TEST-08).

These belong to a separate test-coverage cleanup spec.

## §8 Acceptance criteria per stage

**Stage 0 — verdict-driven:**
- ✅ Verification verdict (gate fires / does not fire) committed inline in PR description.
- ✅ If fix needed: Stage 0.5 test passes; SEC-13 register row flipped to `decided`.
- ✅ If no fix: SEC-13 register row flipped to `non-finding (gate-firing-confirmed)`.

**Stage 1:**
- ✅ 2 compile-fail probes green.
- ✅ `cargo clippy --workspace -- -D warnings` green.
- ✅ Internal call sites of `crypto::encrypt` audited and migrated to `encrypt_with_key_id`.

**Stage 2:**
- ✅ 2 compile-fail probes + 2 runtime tests green.
- ✅ N10 invariant cluster closure noted in register.
- ✅ Manual review by security-lead recommended (load-bearing per §4.2 PRODUCT_CANON).

**Stage 3:**
- ✅ 2 wiremock tests green.
- ✅ No regression in `cargo nextest run -p nebula-engine credential::rotation`.

**Stage 4:**
- ✅ 6 docs updated (MATURITY, OBSERVABILITY, UPGRADE_COMPAT, register, audit Errata footer, GLOSSARY).
- ✅ Register row flips committed in same PR/stage.
- ✅ Audit Errata §XII.E status footer added.
- ✅ CHANGELOG entry added.

**Spec-level DoD:**
- All 4 compile-fail probes + 4 runtime tests + 1 conditional runtime test green under workspace nextest.
- All 6 docs synced.
- 1-line summary added to `CHANGELOG.md`.

## §9 Deferred / parallel tracks

This spec **does not** cover the following audit findings; each has a forward-pointer to where it lands:

| Item | Forward-pointer | Reason for split |
|---|---|---|
| SEC-03 (AAD + `key_id` redesign) | `2026-04-27-credential-aad-key-id-redesign-design.md` (TBD) | Different threat model (audit-trail integrity, not theft); ADR-0028 amendment required; storage reverse-deps audit needed |
| PERF-01 + PERF-02 + IDIOM-10 (`CredentialId: Copy` migration) | `2026-04-27-credential-id-copy-migration-design.md` (TBD) | Architect-level breaking change; touches signatures across `nebula-credential` + `nebula-engine` + downstream `nebula-resource`; rust-senior architect-handoff signal |
| PERF-05 (Refreshable trait-hook) | П3 capability sub-trait scope (Tech Spec §15.4) | Correctness erosion fix; requires `Refreshable::refresh_via_engine_http` AFIT signature design |
| IDIOM-01 (`provider.rs` AFIT migration) | П3 capability sub-trait scope | Clean win (zero `dyn` consumers in workspace); slot under П3 |
| IDIOM-03 (`Box<dyn Error + Send>` → `+ Sync`) | Folded into П3 error-module split (per audit §VII.C overlap with ARCH-06) | Tied to error-taxonomy refactor |
| TEST-01 / TEST-02 (e2e + per-resource swap test) | П3 planning | locked-post-spike per register `user-test-integration` / `user-test-concurrency` |
| GAP-01 (`manager.rs:1378` `todo!()` fan-out) | П3+ deferred cascade per Tech Spec §15.7:3522-3523 | Intentional П1 state via `OnCredentialRefresh<C>` parallel trait |
| ARCH-02 / ARCH-03 (test-shim duplication) | non-finding per Errata §XII.C | Intentional ADR-0032 §3 design |
| SEC-15 / SEC-17 (key_id deadline / RUSTSEC sweep) | Independent CI hygiene task | Not gated by this spec |

## §10 Migration / rollout

**Breaking changes (active dev mode, semver-major bump on next release):**

| Change | Workspace audit | Risk |
|---|---|---|
| `CredentialGuard: !Clone` | `grep -r "CredentialGuard.*\.clone()" crates/` confirms zero call sites at spec time. External plugins not yet exist (П1 just landed). | Low |
| `SchemeGuard: !Send` | `grep` for `tokio::spawn` patterns in resource impls — existing usage is single-task-local, no spawn boundary crossed. | Low |
| `crypto::encrypt` removed | No external callers; internal callers migrate to `encrypt_with_key_id`. Stage 1 audits before deletion. | Low |
| `serde_secret::serialize` → `pub(crate)` | No external `pub use` re-export. Internal usage unaffected. | Low |
| `bearer_header()` return type → `Zeroizing<String>` (was `String`) | Single internal caller (rotation path). External `nebula-action` consumers TBD; check during stage execution. | Medium |

**Rollout discipline:**
- One stage = one merge commit (squash). Multi-PR-per-stage allowed by execution choice; spec does not mandate.
- Stages execute in strict order (Stage 0 → Stage 1 → Stage 2 → Stage 3 → Stage 4); skipping requires spec amendment.
- Stage 4 (doc sync) is part of spec, not follow-up; spec is NOT considered DoD until Stage 4 lands.

**Rollback contract:**
- Each stage's commit is independently revertable.
- Stage 2 revert restores `Clone`/`Send` impls — losing N10 enforcement; regression must be noted in revert PR description.
- Stage 0.5 (if executed) revert leaves redaction patch removed — but no test breakage downstream, since Stage 0.5 test is added by the same commit.

---

**Spec complete.** Implementation plan to follow per writing-plans skill (next-up artefact: `docs/superpowers/plans/2026-04-27-credential-security-hardening.md`).
