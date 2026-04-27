---
name: credential security hardening — implementation plan
status: draft (writing-plans output 2026-04-27 — awaiting execution-mode choice)
date: 2026-04-27
authors: [vanyastaff, Claude]
phase: parallel-track (does not block П3 kickoff)
scope: cross-cutting — nebula-credential, nebula-engine
related:
  - docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md
  - docs/tracking/credential-audit-2026-04-27.md §XII Errata
  - docs/adr/0028-cross-crate-credential-invariants.md (N10 plaintext invariant)
  - docs/adr/0030-engine-owns-credential-orchestration.md §4 redaction gate
new-adrs:
  - none (no architectural shifts; spec amends behavior under existing ADRs)
---

# Credential Security Hardening — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 7 production-track SEC findings from audit Errata §XII (and 1 conditional SEC-13) in 5 sequential stages with mandatory landing gates per stage.

**Architecture:** This plan executes the spec at `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md` verbatim. Stage order is strict: Stage 0 → 1 → 2 → 3 → 4. Each stage produces one merge commit (squash); PRs may be 1+ per stage at execution choice. Test discipline: 4 mandatory compile-fail probes + 3 mandatory runtime tests + 1 SEC-10 structural grep check + 1 conditional Stage 0.5 test, mapped 1:1 to SEC IDs for traceability.

**Tech Stack:** Rust 1.95.0 (pinned), tokio 1.51, `zeroize` 1.8, `secrecy` workspace, `url` 2.x (existing dep), `wiremock` 0.6 (test-only), `trybuild` 1.0 (compile-fail), `cargo-nextest` (test runner per CI matrix). No new dependencies introduced.

**Pre-execution requirement:** Create dedicated worktree per `superpowers:using-git-worktrees`. Plan execution agent runs inside the worktree; main branch sees only the merge commits at landing.

**Reading order for the engineer:** Spec at `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md` (full design — §0-§10). Audit Errata §XII for severity reclassifications context. ADR-0028 (cross-crate invariants — particularly N10) + ADR-0030 §4 (redaction gate). Then this plan.

**Total estimate:** 5 stages, ~9 days focused work per spec §0 timeline.

---

## File Map

### Created files

| Path | Purpose | Stage |
|------|---------|-------|
| `crates/credential/tests/compile_fail_serde_secret_pub.rs` | trybuild driver for SEC-08 | 1 |
| `crates/credential/tests/probes/serde_secret_pub.rs` | SEC-08 probe input | 1 |
| `crates/credential/tests/probes/serde_secret_pub.stderr` | SEC-08 expected error | 1 |
| `crates/credential/tests/compile_fail_encrypt_no_aad_removed.rs` | trybuild driver for SEC-11 | 1 |
| `crates/credential/tests/probes/encrypt_no_aad_removed.rs` | SEC-11 probe input | 1 |
| `crates/credential/tests/probes/encrypt_no_aad_removed.stderr` | SEC-11 expected error | 1 |
| `crates/credential/tests/compile_fail_credential_guard_clone.rs` | trybuild driver for SEC-05 | 2 |
| `crates/credential/tests/probes/credential_guard_clone.rs` | SEC-05 probe input | 2 |
| `crates/credential/tests/probes/credential_guard_clone.stderr` | SEC-05 expected error | 2 |
| `crates/credential/tests/compile_fail_scheme_guard_send.rs` | trybuild driver for SEC-06 | 2 |
| `crates/credential/tests/probes/scheme_guard_send.rs` | SEC-06 probe input | 2 |
| `crates/credential/tests/probes/scheme_guard_send.stderr` | SEC-06 expected error | 2 |
| `crates/credential/tests/zeroize_drop_oauth2_bearer.rs` | runtime drop verification SEC-09 | 2 |
| `crates/engine/tests/oauth_idp_oversized_body_bounded.rs` | wiremock SEC-01 | 3 |
| `crates/engine/tests/oauth_idp_error_uri_validation.rs` | wiremock SEC-02 (table-driven) | 3 |
| `crates/engine/tests/refresh_err_redaction_token_in_description.rs` | (conditional) Stage 0.5 wiremock | 0.5 |

### Modified files

| Path | Change | Stage |
|------|--------|-------|
| `crates/engine/src/credential/rotation/token_refresh.rs` | (Stage 0.5 conditional) add `redact_error_description` helper | 0.5 |
| `crates/credential/src/secrets/serde_secret.rs` | `pub` → `pub(crate)` on `serialize` | 1 |
| `crates/credential/src/secrets/crypto.rs` | delete `pub fn encrypt`; migrate internal callers to `encrypt_with_key_id` | 1 |
| `crates/credential/src/secrets/guard.rs` | drop `Clone` impl on `CredentialGuard` | 2 |
| `crates/credential/src/secrets/scheme_guard.rs` | add `_marker: PhantomData<*const ()>` field | 2 |
| `crates/credential/src/credentials/oauth2.rs` | `bearer_header()` returns `Zeroizing<String>` (was `String`); allocate inside `Zeroizing::new(String::with_capacity(...))` then `push_str` | 2 |
| `crates/engine/src/credential/rotation/token_refresh.rs` | restructure `refresh_oauth2_state`: secret borrows live inside an inner block returning the built `RequestBuilder`; eliminate the 3 `Zeroizing<String>` intermediates; add `read_token_response_limited` (error path) + `sanitize_error_uri` helpers | 2, 3 |
| `crates/credential/Cargo.toml` | dev-dep additions for trybuild (verify presence; add if missing) | 1, 2 |
| `crates/engine/Cargo.toml` | dev-dep additions for wiremock (verify presence; add if missing) | 3 |
| `docs/MATURITY.md` | append «Security hardening 2026-04-27 SEC-cluster (PR <sha>)» under Audited column for `nebula-credential` | 4 |
| `docs/OBSERVABILITY.md` | add `credential.refresh.err_uri_rejected_total` metric + `credential.refresh.body_truncated` span attr | 4 |
| `docs/UPGRADE_COMPAT.md` | add row for breaking changes (Clone/Send/visibility) | 4 |
| `docs/tracking/credential-concerns-register.md` | flip rows for SEC-01/02/05/06/08/09/10/11 to `decided` with PR SHA | 4 |
| `docs/tracking/credential-audit-2026-04-27.md` | append §XII.E «Implementation status» footer | 4 |
| `docs/GLOSSARY.md` | add 5 missing terms (Plane B, Pending rotation FSM, Dynamic provider class, sentinel/N=3-in-1h, herd-сценарий) | 4 |
| `crates/credential/src/secrets/crypto.rs` | (optional SEC-04 doc fix) doc comment correction | 4 |
| `CHANGELOG.md` | one-line summary entry under unreleased section | 4 |

### Deleted files

None at file level. Function deletions are line-level only — `pub fn encrypt` body removed within `crypto.rs`.

---

## Stage 0 — SEC-13 verify-first (Day 1)

**Goal:** Determine if ADR-0030 §4 redaction CI gate fires on `token_refresh.rs` error path. Conditional Stage 0.5 fix only if gate misses.

### Task 0.1 — Worktree creation

- [ ] Create worktree per `superpowers:using-git-worktrees`
  - Verify: `git worktree list` shows new worktree
  - Branch name: `credential-sec-hardening` (or similar)

### Task 0.2 — Read ADR-0030 §4 + locate gate

- [ ] Read `docs/adr/0030-engine-owns-credential-orchestration.md` §4
  - Identify canonical redaction CI gate name and assertion target
  - Record gate name in PR description draft
- [ ] Search workspace for gate test:
  - `cargo nextest run --workspace 2>&1 | grep -i redact` — record matching test names
  - `grep -rn "redact\|REDACTED" crates/credential crates/engine --include='*.rs'` — record
  - `grep -rn "TokenRefreshError::TokenEndpoint" crates/` — record call sites

### Task 0.3 — Manual trace verification

- [ ] Construct test fixture: IdP returns `{"error":"invalid_grant","error_description":"refresh_token=abc123 expired"}`
- [ ] Render `Display` of resulting `TokenRefreshError`
- [ ] Assert: rendered output does NOT contain `abc123`
- [ ] Output verdict in PR description, exactly one of:
  - `«GATE FIRES — SEC-13 dropped from scope»`
  - `«GATE DOES NOT FIRE — proceed to Stage 0.5»`

### Task 0.4 — Stage 0.5 (conditional, only if gate misses)

- [ ] Add `redact_error_description` helper in `crates/engine/src/credential/rotation/token_refresh.rs`
  - Heuristic: regex match for token-shaped substrings (length ≥20) OR `=` followed by ≥16 chars OR case-insensitive `(refresh|access|bearer)_?(token|tok)\s*[=:]\s*\S+`
  - Replace matches with `[REDACTED]`
- [ ] Apply helper at the point where `error_description` enters `TokenRefreshError::TokenEndpoint{summary}`
- [ ] Create test `crates/engine/tests/refresh_err_redaction_token_in_description.rs`:
  - wiremock IdP returns crafted `error_description` containing `refresh_token=abc123`
  - Assert `Display`-rendered error contains `[REDACTED]`, NOT `abc123`

### Task 0.5 — Audit Errata update

- [ ] Update `docs/tracking/credential-audit-2026-04-27.md` §XII.E:
  - If gate fires: SEC-13 entry → «non-finding (gate-firing-confirmed)»
  - If fix added: SEC-13 entry → «decided (PR <sha> Stage 0.5)»

### Stage 0 — Landing gate

- [ ] Verification verdict written in PR description
- [ ] If Stage 0.5 ran: 1 new test green under `cargo nextest run -p nebula-engine credential::refresh::redaction`
- [ ] Audit Errata §XII.E updated
- [ ] Commit (squash):
  - if fix added: `fix(engine): SEC-13 — refresh err redaction (Stage 0.5)`
  - if no fix: `docs(credential): SEC-13 — verify-first verdict (Stage 0)`

---

## Stage 1 — Visibility tightening (Day 2)

**Goal:** Tighten `serde_secret::serialize` to `pub(crate)`; delete bare `crypto::encrypt`.

### Task 1.1 — SEC-08: serde_secret::serialize visibility

- [ ] Edit `crates/credential/src/secrets/serde_secret.rs`:
  - Change `pub fn serialize` to `pub(crate) fn serialize`
- [ ] Search workspace for callers: `grep -rn "serde_secret::serialize\|use.*serde_secret" crates/`
  - Verify all callers are within `nebula-credential` crate
  - If any external caller exists: STOP, escalate to user before proceeding

### Task 1.2 — SEC-11: delete bare crypto::encrypt

- [ ] Edit `crates/credential/src/secrets/crypto.rs`:
  - Locate `pub fn encrypt` (line 158-177 per audit)
  - Audit internal callers: `grep -rn "crypto::encrypt[^_]\|::encrypt(" crates/`
  - For each caller: switch to `encrypt_with_key_id` with appropriate `key_id` argument
  - Delete the function body and its `pub` declaration

### Task 1.3 — Compile-fail probe SEC-08

- [ ] Create `crates/credential/tests/compile_fail_serde_secret_pub.rs`:
  ```rust
  #[test]
  fn compile_fail_serde_secret_pub() {
      let t = trybuild::TestCases::new();
      t.compile_fail("tests/probes/serde_secret_pub.rs");
  }
  ```
- [ ] Create `crates/credential/tests/probes/serde_secret_pub.rs` calling `nebula_credential::secrets::serde_secret::serialize` from outside-crate context
- [ ] Run `TRYBUILD=overwrite cargo test -p nebula-credential --test compile_fail_serde_secret_pub` once to capture `.stderr`
- [ ] Verify `.stderr` contains `error[E0603]` referencing module privacy

### Task 1.4 — Compile-fail probe SEC-11

- [ ] Create `crates/credential/tests/compile_fail_encrypt_no_aad_removed.rs` (same structure as 1.3)
- [ ] Create `crates/credential/tests/probes/encrypt_no_aad_removed.rs` calling `nebula_credential::secrets::crypto::encrypt`
- [ ] Run with `TRYBUILD=overwrite` to capture `.stderr`
- [ ] Verify `.stderr` contains `error[E0425]` cannot find function

### Stage 1 — Landing gate

- [ ] 2 compile-fail probes green: `cargo nextest run -p nebula-credential --test 'compile_fail_serde_secret_pub' --test 'compile_fail_encrypt_no_aad_removed'`
- [ ] `cargo clippy --workspace -- -D warnings` green
- [ ] `grep -r "crypto::encrypt[^_]" crates/` returns zero (post-migration)
- [ ] `cargo nextest run -p nebula-credential` green
- [ ] Commit (squash): `feat(credential)!: SEC-08+SEC-11 — visibility tightening (Stage 1)`

---

## Stage 2 — Plaintext lifecycle (Day 3-6) — N10 invariant closure

**Goal:** Close PRODUCT_CANON §4.2 N10: «plaintext does not cross spawn boundary».

### Task 2.1 — SEC-05: CredentialGuard !Clone

- [ ] Edit `crates/credential/src/secrets/guard.rs`:
  - Drop `Clone` impl on `CredentialGuard` (line 64-71 per audit)
  - Update doc comments to note `!Clone` invariant + N10 reference
- [ ] Workspace audit: `grep -r "CredentialGuard.*\.clone()" crates/`
  - Expected: zero call sites
  - If non-zero: STOP, escalate before proceeding

### Task 2.2 — SEC-06: SchemeGuard !Send marker

- [ ] Edit `crates/credential/src/secrets/scheme_guard.rs`:
  - Add field `_marker: PhantomData<*const ()>` to `SchemeGuard<'a, C>` struct
  - Update `pub(crate) fn new` constructor to initialize `_marker: PhantomData`
- [ ] Workspace audit: `grep -B 5 -A 5 "tokio::spawn" crates/credential crates/engine crates/resource`
  - Verify no `SchemeGuard` captured across spawn boundary
  - If any boundary crossing exists: refactor to use `SchemeFactory::acquire` per Tech Spec §15.7

### Task 2.3 — Compile-fail probes (SEC-05 + SEC-06)

- [ ] Create `crates/credential/tests/compile_fail_credential_guard_clone.rs` calling `.clone()` on `CredentialGuard`
- [ ] Create `crates/credential/tests/compile_fail_scheme_guard_send.rs` doing `tokio::spawn(async move { let _ = guard; })` on `SchemeGuard`
- [ ] Run probes with `TRYBUILD=overwrite`
- [ ] Verify `.stderr`:
  - probe 1: `error[E0599]` no method named `clone`
  - probe 2: `error[E0277]` SchemeGuard cannot be sent between threads safely

### Task 2.4 — SEC-09: bearer_header → Zeroizing<String>

- [ ] Edit `crates/credential/src/credentials/oauth2.rs`:
  - Locate `bearer_header()` (line 125-128 per audit)
  - Replace with explicit `Zeroizing<String>` construction:
    ```rust
    let token_plain = self.access_token.expose_secret();
    let mut header = Zeroizing::new(String::with_capacity(7 + token_plain.len()));
    header.push_str("Bearer ");
    header.push_str(token_plain);
    header
    ```
  - Update return type from `String` to `Zeroizing<String>`
- [ ] Workspace audit of callers: `grep -rn "bearer_header()" crates/`
  - Update call sites to handle new return type (`Zeroizing<String>` vs `String`)
  - `nebula-action` consumers (if any): update in same PR

### Task 2.5 — SEC-10: scope-tighten secret borrows in `refresh_oauth2_state`

- [ ] Edit `crates/engine/src/credential/rotation/token_refresh.rs:62-72`:
  - **Delete** the 3 `Zeroizing<String>` declarations (`refresh_tok`, `client_id`, `client_secret`)
  - Restructure into inner block that owns the secret-borrows and returns the built `RequestBuilder`:
    ```rust
    let scope_joined: Option<String> = (!state.scopes.is_empty())
        .then(|| state.scopes.join(" "));
    let req = {
        let refresh_tok = state
            .refresh_token
            .as_ref()
            .ok_or(TokenRefreshError::MissingRefreshToken)?
            .expose_secret();
        let client_id = state.client_id.expose_secret();
        let client_secret = state.client_secret.expose_secret();

        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_tok),
        ];
        if let Some(ref scope) = scope_joined {
            form.push(("scope", scope.as_str()));
        }

        let client = oauth_token_http_client();
        let mut req = client.post(&state.token_url);
        match state.auth_style {
            AuthStyle::Header => {
                req = req.basic_auth(client_id, Some(client_secret));
                req = req.form(&form);
            }
            AuthStyle::PostBody => {
                form.push(("client_id", client_id));
                form.push(("client_secret", client_secret));
                req = req.form(&form);
            }
        }
        req // moved out; secret borrows drop here
    };

    let resp = req.send().await
        .map_err(|e| TokenRefreshError::Request(e.to_string()))?;
    ```
  - **No** `Zeroizing<String>` intermediates in our code; reqwest copies the `&str` refs into its internal request body for the HTTP round-trip duration (unavoidable; documented as best-effort defense).
  - **No** new helper method introduced. Type-enforced by Rust borrow scoping — incorrect usage would be a borrow-checker error, not a developer-discipline issue.

### Task 2.6 — SEC-09: bearer_header runtime drop test

- [ ] Create `crates/credential/tests/zeroize_drop_oauth2_bearer.rs`:
  - Construct `OAuth2Token` with known plaintext
  - Call `.bearer_header()`, capture pointer to `Zeroizing<String>`'s buffer
  - Drop, verify zeroed via `std::ptr::read_volatile` byte-by-byte
  - If MIRI flagged: mark `#[cfg(not(miri))]` + add doc comment with rationale; alternative deterministic-drop verification via custom Zeroizing wrapper that increments static AtomicUsize on drop

> **Note:** SEC-10 has no dedicated runtime test — the «no `Zeroizing<String>` intermediate» property is enforced structurally by the absence of owned String declarations in `refresh_oauth2_state`. Stage 2 landing gate verifies via grep: `! grep -n "Zeroizing::<String>::new\|Zeroizing::new.*expose_secret" crates/engine/src/credential/rotation/token_refresh.rs`. If grep returns hits, Stage 2 is incomplete.

### Stage 2 — Landing gate

- [ ] 2 compile-fail probes green (SEC-05, SEC-06)
- [ ] 1 runtime test green (SEC-09 zeroize_drop_oauth2_bearer)
- [ ] SEC-10 structural check passes: `! grep -nE "Zeroizing::<String>::new|Zeroizing::new\(.*expose_secret" crates/engine/src/credential/rotation/token_refresh.rs` returns no hits
- [ ] `cargo clippy --workspace -- -D warnings` green
- [ ] `cargo nextest run -p nebula-credential -p nebula-engine` green
- [ ] **Manual review by security-lead recommended** (load-bearing per §4.2 PRODUCT_CANON N10)
- [ ] Update `docs/tracking/credential-concerns-register.md` rows for SEC-05/06/09/10 to `decided` with stage commit SHA
- [ ] Commit (squash): `feat(credential)!: SEC-05+SEC-06+SEC-09+SEC-10 — N10 plaintext lifecycle (Stage 2)`

---

## Stage 3 — IdP boundary (Day 7-8)

**Goal:** Harden OAuth2 IdP request/response boundary against compromised / MITM IdP.

### Task 3.1 — SEC-01: bounded reader on IdP body

- [ ] Edit `crates/engine/src/credential/rotation/token_refresh.rs`:
  - Locate `resp.text().await` at line 109 (error path)
  - Add helper at module top:
    ```rust
    async fn read_token_response_limited(
        resp: reqwest::Response,
        limit: usize,
    ) -> Result<String, BodyTooLarge>
    ```
  - Implementation: use `resp.bytes_stream()` + `try_for_each` accumulating into `Vec<u8>`; if `len > limit` return `BodyTooLarge`; final `String::from_utf8` (or lossy if needed)
  - Define `BodyTooLarge` error variant if not present
  - Replace error-path `resp.text().await` with `read_token_response_limited(resp, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES).await`

### Task 3.2 — SEC-02: sanitize_error_uri

- [ ] Edit `crates/engine/src/credential/rotation/token_refresh.rs`:
  - Add helper at module top:
    ```rust
    fn sanitize_error_uri(raw: &str) -> std::borrow::Cow<'_, str> {
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
  - Apply at line 170-173 — replace raw `error_uri` concatenation with `let safe_uri = sanitize_error_uri(&uri);` then format as `error_uri=<safe_uri>` in the summary string

### Task 3.3 — Wiremock test: oversized body bounded

- [ ] Create `crates/engine/tests/oauth_idp_oversized_body_bounded.rs`:
  - wiremock IdP that returns 1 MiB error body (use `.set_body_bytes(vec![b'a'; 1024 * 1024])`)
  - Call refresh path against this IdP
  - Assert returns `Err` containing `BodyTooLarge` error variant
  - Assert response time < 5s (not 30s timeout — bounded fail-fast)

### Task 3.4 — Wiremock test: error_uri validation table-driven

- [ ] Create `crates/engine/tests/oauth_idp_error_uri_validation.rs`:
  - Table-driven cases (each is one wiremock setup):

  | Input `error_uri` | Expected sanitized output |
  |---|---|
  | `http://attacker.example` | `[invalid_error_uri_redacted]` |
  | `https://valid.example/ok` | `https://valid.example/ok` (passthrough) |
  | `https://x.example/\u{0001}path` | `[control_chars_in_error_uri_redacted]` |
  | 300-char `https://x.example/aaaa...` | truncated at 256 + `…[truncated]` |
  | `javascript:alert(1)` | `[invalid_error_uri_redacted]` |
  | `""` (empty) | `[invalid_error_uri_redacted]` |

  - For each case: assert rendered `Display` of `TokenRefreshError` contains the expected sanitized string

### Stage 3 — Landing gate

- [ ] 2 wiremock tests green
- [ ] No regression in `cargo nextest run -p nebula-engine credential::rotation`
- [ ] `cargo clippy --workspace -- -D warnings` green
- [ ] Update register rows for SEC-01/02 to `decided`
- [ ] Commit (squash): `feat(engine): SEC-01+SEC-02 — IdP boundary hardening (Stage 3)`

---

## Stage 4 — Doc sync (Day 9)

**Goal:** Per `feedback_observability_as_completion.md` — invariant changes flow into observability/maturity docs in the same stage-batch (DoD requirement).

### Task 4.1 — MATURITY.md row update

- [ ] Edit `docs/MATURITY.md`:
  - Locate row for `nebula-credential`
  - Append «Security hardening 2026-04-27 SEC-cluster (PR `<sha>`)» under «Audited» column

### Task 4.2 — OBSERVABILITY.md additions

- [ ] Edit `docs/OBSERVABILITY.md` §credential-events:
  - Add metric: `credential.refresh.err_uri_rejected_total` (counter; labels: rejection_reason ∈ {scheme, controlchars, parse_failed})
  - Add span attr: `credential.refresh.body_truncated` (boolean; emitted on `read_token_response_limited` truncation path)
- [ ] Implement metric emission in `sanitize_error_uri` rejection sites (Stage 3 helper):
  - Use `metrics::counter!("credential.refresh.err_uri_rejected_total", "reason" => "scheme").increment(1);` at each rejection branch
- [ ] Implement span attr emission in `read_token_response_limited` truncation site:
  - `tracing::Span::current().record("credential.refresh.body_truncated", true);` at `BodyTooLarge` return
- [ ] Verify metric/span emission via existing observability test pattern in `crates/engine/tests/`

### Task 4.3 — UPGRADE_COMPAT.md row

- [ ] Edit `docs/UPGRADE_COMPAT.md`:
  - Add row dated 2026-04-27 for SEC hardening:
    > «`CredentialGuard: !Clone`, `SchemeGuard: !Send`, `crypto::encrypt` removed, `serde_secret::serialize` → `pub(crate)`, `bearer_header()` returns `Zeroizing<String>` (was `String`). Breaking, same-major (active dev mode).»

### Task 4.4 — Register row flips

- [ ] Edit `docs/tracking/credential-concerns-register.md`:
  - Find rows for SEC-01, SEC-02, SEC-05, SEC-06, SEC-08, SEC-09, SEC-10, SEC-11
  - If Stage 0.5 ran: also SEC-13 (otherwise flip to `non-finding`)
  - Flip status: `proposed` → `decided`
  - Add Resolution pointer: this spec + landing PR SHA
- [ ] Audit register totals table per §Maintenance contract — verify counts mutually consistent

### Task 4.5 — Audit Errata footer

- [ ] Edit `docs/tracking/credential-audit-2026-04-27.md` §XII.E:
  - Append at end: «Implementation status: Stages 0-3 landed at PR `<sha>`, Stage 4 at `<sha>`»

### Task 4.6 — GLOSSARY.md additions

- [ ] Edit `docs/GLOSSARY.md` to add 5 terms (per spec-auditor finding §XII.B err-8):
  - **Plane B** — pull definition from ADR-0033 § integration-credentials-plane-b
  - **Pending (rotation FSM)** — pull from `crates/credential/src/rotation/state.rs` (distinct from `PendingDrain` in resource lifecycle)
  - **Dynamic (provider class)** — pull from sub-trait split Tech Spec §15.4
  - **sentinel** / **N=3-in-1h** — pull from refresh-coordination spec §6 (n8n #13088 escalation threshold)
  - **herd-сценарий** — pull from refresh stampede pattern (TTL synchronization on multiple replicas; reference П2 plan)

### Task 4.7 — Optional SEC-04 doc fix

- [ ] (optional) Edit `crates/credential/src/secrets/crypto.rs:136-142`:
  - Change doc comment «OS CSPRNG» → «CSPRNG seeded from OS via `getrandom`» (per Errata §XII.C clarification)

### Task 4.8 — CHANGELOG entry

- [ ] Edit `CHANGELOG.md` under unreleased section:
  ```markdown
  ### Security
  - Credential security hardening: 7 audit findings closed (SEC-01/02/05/06/08/09/10/11)
    plus SEC-13 conditionally. Breaking changes: `CredentialGuard !Clone`, `SchemeGuard !Send`,
    `crypto::encrypt` removed, `bearer_header` returns `Zeroizing<String>`. See
    `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md`.
  ```

### Stage 4 — Landing gate

- [ ] All 6 docs updated (MATURITY, OBSERVABILITY, UPGRADE_COMPAT, register, audit Errata, GLOSSARY)
- [ ] CHANGELOG entry committed
- [ ] Register totals table internally consistent (audit per §Maintenance)
- [ ] All updated docs render cleanly (no broken markdown links — `cargo doc` if applicable, or `mdbook test` if mdbook-driven)
- [ ] Commit (squash): `docs(credential): SEC hardening doc sync (Stage 4)`

---

## Spec-level DoD (final acceptance)

- [ ] All 4 mandatory compile-fail probes green
- [ ] All 3 mandatory runtime tests green
- [ ] SEC-10 structural grep check passes (no `Zeroizing<String>` intermediates in `refresh_oauth2_state`)
- [ ] Conditional Stage 0.5 test green (if executed)
- [ ] All 6 docs synced
- [ ] CHANGELOG entry added
- [ ] All commits land on main via squash-merge
- [ ] Audit Errata §XII.E footer reflects landing SHAs
- [ ] No regression in `cargo nextest run --workspace`

---

## Open items to resolve during execution

| Item | Resolved at |
|---|---|
| SEC-13 verdict (gate fires or not) | Stage 0 Task 0.3 |
| External callers of `bearer_header()` outside `nebula-credential` | Stage 2 Task 2.4 (audit before commit) |
| External callers of `serde_secret::serialize` | Stage 1 Task 1.1 (audit before commit) |
| trybuild MIRI compatibility for `ptr::read_volatile` drop tests | Stage 2 Task 2.7 (`#[cfg(not(miri))]` if needed) |
| Existing OBSERVABILITY emission test pattern | Stage 4 Task 4.2 (locate before adding) |
| Existing `OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES` constant location | Stage 3 Task 3.1 (verify in `token_http.rs` per audit) |

---

**Plan complete.** Execute via `superpowers:subagent-driven-development` (recommended) OR `superpowers:executing-plans`.
