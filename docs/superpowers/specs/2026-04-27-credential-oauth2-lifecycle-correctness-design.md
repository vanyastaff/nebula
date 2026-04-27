---
name: credential OAuth2 lifecycle correctness — fix terminal-error mapping, capability honesty, owner identity
status: draft (writing-skills output 2026-04-27)
date: 2026-04-27
authors: [vanyastaff, Claude]
phase: prod-track parallel (does not block П3 kickoff; pairs with concurrency-correctness spec)
scope: cross-cutting — nebula-engine (resolver, dispatchers, rotation), nebula-credential (oauth2, registry, context)
related:
  - docs/tracking/credential-audit-2026-04-27.md §XII Errata (review round 1)
  - ChatGPT 5.5 review round 2 (2026-04-27) — findings #4, #7, #9
  - docs/superpowers/specs/2026-04-27-credential-concurrency-correctness-design.md (sibling track; CC-02 cross-cuts CC-04)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.4 (capability sub-trait split) + §15.8 (capability-from-type authority)
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §6 (refresh outcomes + sentinel)
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
defers-to: none
---

# Credential OAuth2 Lifecycle Correctness — fix 3 lifecycle bugs

## §0 Meta

**Scope.** Fix 3 OAuth2 / credential-lifecycle correctness bugs surfaced by ChatGPT 5.5 round-2 review (2026-04-27). All 3 are operational-correctness issues whose impact spans interactive + automated flows; each is reproducible by a focused integration test.

**Bugs in scope:**
- **CL-04 — invalid_grant → ReauthRequired mapping.** OAuth2 terminal refresh errors (`invalid_grant`, `invalid_client`, etc.) are mapped to generic `CredentialError::Provider`, not `RefreshOutcome::ReauthRequired`. Result: doomed refreshes are retried indefinitely; reauth flag never set; UI never prompts the operator.
- **CL-07 — False capability reporting on OAuth2.** OAuth2 implements `Refreshable + Revocable + Testable` traits (capability-from-type authority per Tech Spec §15.8 advertises them), but several method bodies return «HTTP transport has moved» — meaning the registered capabilities are structurally available but operationally disabled. UI/CLI shows lifecycle actions that fail.
- **CL-09 — owner_id() fail-open to "system".** `CredentialContext::owner_id()` silently falls back to `"system"` when no owner is set. PendingStore binding claims 4 dimensions (credential / owner / session / token), but one dimension can collapse for production callers that forget owner wiring; isolation reduces to session+token only.

**Non-goals.**
- Concurrency / race-condition fixes (CC-01, CC-02, CC-03, CC-06) — sibling spec `2026-04-27-credential-concurrency-correctness-design.md`. Note: CL-04 cross-cuts with CC-02 (`reauth_required` propagation must hold once CL-04 sets the flag).
- StoredCredential type ambiguity (CC-08), built-in crate split (CC-10) — П3 scope.
- Generic redaction CI gate work — covered by security-hardening spec Stage 0.5 (SEC-13).

**Phase.** **Prod-track parallel.** Does not block П3 kickoff but should land before П3 integration tests would otherwise expose these bugs as flakes. Pairs naturally with the concurrency-correctness spec — both are «review round 2 fallout».

**Reading order.** §0 (this) → §1 (per-bug catalog) → §2-§4 (per-bug stage design) → §5 (doc sync) → §6 (tests) → §7 (acceptance) → §8 (deferred) → §9 (migration).

## §1 Bug catalog

### §1.1 CL-04 — invalid_grant → ReauthRequired mapping

| Field | Value |
|---|---|
| Severity | HIGH |
| Location | `crates/engine/src/credential/resolver.rs::perform_refresh` (mapping site) + `crates/engine/src/credential/rotation/token_refresh.rs::TokenRefreshError` (error source) |
| Pattern | `TokenRefreshError::TokenEndpoint { status, summary }` is unconditionally mapped to `CredentialError::Provider`; never produces `RefreshOutcome::ReauthRequired`. `TokenRefreshError::MissingRefreshToken` likewise. |
| Concrete scenario | OAuth provider returns `{"error":"invalid_grant","error_description":"refresh_token expired"}`. Engine records this as a refresh failure. Other replicas under the herd-сценарий retry the same doomed refresh. Reauth flag never set; operator never prompted. Refresh storms continue until sentinel threshold (N=3 in 1h) escalates. Even then, the escalation path may not flip `reauth_required` — verify in Stage 0. |
| Why structural enforcement | Tech Spec §6 (refresh coordination) defines `RefreshOutcome::ReauthRequired` as the canonical signal; current code never produces it for terminal IdP rejections. |

### §1.2 CL-07 — False capability reporting on OAuth2

| Field | Value |
|---|---|
| Severity | MEDIUM |
| Location | `crates/credential/src/credentials/oauth2.rs` (capability impls) + `crates/credential/src/contract/registry.rs` (capability-from-type fold) + `crates/engine/src/credential/dispatchers.rs` (dispatch entry points) |
| Pattern | OAuth2 type `impl Refreshable, Revocable, Testable`; per Tech Spec §15.8 these become reported `Capabilities` flags via `compute_capabilities::<OAuth2Credential>()`. But the Refreshable / Revocable / Testable method bodies in OAuth2 contain placeholder paths returning «HTTP transport has moved» errors. |
| Concrete scenario | Admin UI calls `iter_compatible(Capabilities::TESTABLE)` to list testable credential types; OAuth2 appears. Admin clicks «test credential» → engine dispatches to OAuth2's `test()` impl → returns transport-disabled error. UX surface mismatched with operational reality. |
| Why structural enforcement | Tech Spec §15.8 mandates capability-from-type honesty — the type system must not advertise capabilities the runtime cannot deliver. Either wire the runtime path end-to-end OR split «trait implemented» from «capability operational in this build» at the registry level. |

### §1.3 CL-09 — owner_id() fail-open to "system"

| Field | Value |
|---|---|
| Severity | MEDIUM |
| Location | `crates/credential/src/context.rs::CredentialContext::owner_id` |
| Pattern | `owner_id() -> &str` returns `self.owner_id.as_deref().unwrap_or("system")` (or equivalent); session_id has fail-closed semantics (returns `Result`); owner asymmetric. |
| Concrete scenario | Two real users (Alice, Bob) initiate OAuth flows simultaneously through a code path that did not set `owner_id` on `CredentialContext`. Both flows record `owner = "system"` in PendingStore binding. Cross-user replay window opens: Alice's pending token + Bob's session/credential triplet falsely match if Alice's session_id collides (low but nonzero). |
| Why structural enforcement | Tech Spec §3 (PendingStore binding) requires 4 dimensions for isolation. Silent owner collapse weakens isolation to 3. `owner_id` should be `Result<&str, ContextError::MissingOwner>` symmetric with `session_id`. |

## §2 Stage 0 — Investigation + reproducible tests

**Goal.** Reproduce each bug + verify cross-cut interactions before fixes commit.

### Tasks

- [ ] **CL-04 reproducer:** `crates/engine/tests/cl04_invalid_grant_to_reauth.rs`
  - wiremock IdP returning `{"error":"invalid_grant","error_description":"refresh_token expired"}`.
  - Call `resolve_with_refresh`; assert returns `Err(ResolveError::ReauthRequired { reason: ReauthReason::InvalidGrant, .. })` (post-fix). Currently fails — returns `CredentialError::Provider`.
  - Cross-cut: assert `reauth_required = true` is persisted on the row (consumed by CC-02 fix).

- [ ] **CL-07 audit:** `crates/credential/tests/cl07_capability_honesty.rs`
  - For each declared capability of `OAuth2Credential`, dispatch through engine; if transport-disabled → fail the test (capability is dishonest).
  - Output: list of capabilities that fail. Decides Stage 2 strategy: wire end-to-end OR mark «not in this build».

- [ ] **CL-09 reproducer:** `crates/engine/tests/cl09_owner_fail_open.rs`
  - Construct `CredentialContext` without `owner_id`; call `execute_continue` for an Interactive credential.
  - Assert: returns `Err(ExecutorError::MissingOwnerId)` (post-fix). Currently the call succeeds with `owner = "system"`.

**Stage 0 landing gate:** 3 reproducers compile and fail (assert against current buggy behavior). They become regression tests.

## §3 Stage 1 — CL-04 fix: terminal error mapping

**Goal.** Map OAuth2 terminal IdP errors to `RefreshOutcome::ReauthRequired` with a typed `ReauthReason`. Persist the reason on the row (cross-cuts with CC-02).

**Design.**

Extend `TokenRefreshError` with a discriminator on terminal errors:
```rust
pub enum TokenRefreshError {
    TokenEndpoint { status: StatusCode, summary: String, terminal: TerminalKind },
    MissingRefreshToken, // implicitly terminal — caller must reauth
    Request(String),     // transport — retryable
    Parse(String),       // body parsing — retryable (transport may have truncated)
    MissingAccessToken,  // protocol violation — terminal
}

pub enum TerminalKind {
    InvalidGrant,
    InvalidClient,
    InvalidRequest,
    UnauthorizedClient,
    UnsupportedGrantType,
    Other(String), // unknown but token-endpoint-class — treat as terminal
}
```

In `parse_token_response` (or wherever JSON error decoding lives), parse `"error"` field and map to `TerminalKind`. Per OAuth2 RFC 6749 §5.2, the `error` field has 5 standardized values that are all terminal w.r.t. the current refresh attempt: `invalid_grant`, `invalid_client`, `invalid_request`, `unauthorized_client`, `unsupported_grant_type`.

Mapping in `perform_refresh`:
```rust
match run_refresh(state).await {
    Err(TokenRefreshError::TokenEndpoint { terminal, .. }) => {
        let reason = match terminal {
            TerminalKind::InvalidGrant => ReauthReason::InvalidGrant,
            TerminalKind::InvalidClient => ReauthReason::InvalidClient,
            // ... etc
        };
        // mark row reauth_required + persist reason
        let mut updated = stored.clone();
        updated.reauth_required = true;
        updated.reauth_reason = Some(reason.clone());
        storage.put(id, updated, expected_version).await?;
        return Ok(RefreshOutcome::ReauthRequired { reason });
    }
    Err(TokenRefreshError::MissingRefreshToken) => {
        // mark + return ReauthRequired { reason: NoRefreshToken }
        ...
    }
    Err(TokenRefreshError::Request(_)) | Err(TokenRefreshError::Parse(_)) => {
        // transport / parse — retryable; existing flow
        return Err(...);
    }
    Err(TokenRefreshError::MissingAccessToken) => {
        // protocol violation — terminal
        ...
    }
    Ok(state) => { /* existing success flow */ }
}
```

`RefreshOutcome` enum needs a `ReauthRequired { reason: ReauthReason }` variant. `ReauthReason` enum: `InvalidGrant`, `InvalidClient`, `NoRefreshToken`, `InvalidRequest`, `UnauthorizedClient`, `UnsupportedGrantType`, `MissingAccessToken`, `SentinelRepeatedThreshold`, `Unknown`.

**Cross-cut with CC-02:** persisting `reauth_required = true` requires the row to have these fields. Verify in Stage 0 that `StoredCredential` already has `reauth_required` + `reauth_reason` columns; if not, schema migration in this stage.

**Landing gate (Stage 1):**
- CL-04 reproducer flips green.
- New unit tests for each `TerminalKind` mapping.
- Cross-cut test with CC-02: invalid_grant → mark + subsequent resolve returns ReauthRequired.
- Commit (squash): `feat(credential)!: CL-04 — OAuth2 terminal error → ReauthRequired (Stage 1)`

## §4 Stage 2 — CL-07 fix: capability honesty

**Goal.** Eliminate the gap between declared capability traits and operational reality.

**Decision tree (pick in Stage 0 audit):**

**Option A — Wire end-to-end (preferred if scope allows).**
- Implement OAuth2 `Refreshable::refresh()`, `Revocable::revoke()`, `Testable::test()` properly via the engine HTTP path.
- Tech Spec §15.4 + §15.7 plus the «refresh-via-engine-http» trait-hook (rust-senior architect signal in audit Errata) wire this canonically.
- Removes «transport has moved» placeholder paths.

**Option B — Split capability declaration from operability.**
- Add `OperationalCapabilities` enum to registry: `CompiledIn`, `Disabled(reason: &'static str)`.
- Type-level capability declaration unchanged (impl Refreshable still claims it). Registry-level reporting splits: `iter_compatible(caps, OperationalFilter::Operational)` filters to compiled-in only.
- UI/CLI uses Operational filter; capability-from-type authority preserved for type-correctness checks.

Recommendation: **Option A**, because Option B preserves the dishonest declaration at the type level — exactly the «discipline-not-structural» pattern flagged by `feedback_type_enforce_not_discipline.md`. Option A removes the dishonesty structurally.

If Option A scope is too large (e.g., revoke endpoint requires per-IdP customization not yet designed) — fall back to Option B for the affected capability only, and mark in Stage 0 audit.

**Landing gate (Stage 2):**
- CL-07 audit re-runs and finds zero dishonest capabilities.
- For each capability fixed under Option A: integration test against wiremock IdP exercising the operational path.
- For each capability under Option B: unit test verifying registry filter behavior.
- Commit (squash): `feat(credential)!: CL-07 — capability honesty (Stage 2)`

## §5 Stage 3 — CL-09 fix: owner_id() fail-closed

**Goal.** Symmetric fail-closed semantics on `CredentialContext::owner_id` (matching `session_id`).

**Design.**

```rust
pub enum ContextError {
    MissingOwner,
    MissingSession,
    // ... existing variants
}

impl CredentialContext {
    pub fn owner_id(&self) -> Result<&str, ContextError> {
        self.owner_id.as_deref()
            .ok_or(ContextError::MissingOwner)
    }
}
```

`execute_continue` and any other site requiring binding fields:
```rust
let owner = ctx.owner_id().map_err(|_| ExecutorError::MissingOwnerId)?;
let session = ctx.session_id().map_err(|_| ExecutorError::MissingSessionId)?;
let binding = PendingBinding { credential_id, owner, session, token };
```

**Migration audit (workspace-wide):**
- `grep -rn "owner_id()" crates/` — every call site must handle the new `Result`.
- Identify call sites that previously relied on `"system"` fallback. For each:
  - If the caller is internal/system code (e.g., automated rotation scheduler), explicitly construct context with `owner_id = "system"` (intentional, not silent).
  - If the caller is user-driven, add owner to context construction site or return `MissingOwner` upward.

**Landing gate (Stage 3):**
- CL-09 reproducer flips green.
- Workspace audit shows zero callers relying on silent fallback.
- For intentional «system» callers: explicit owner string set with audit-trail comment.
- Commit (squash): `feat(credential)!: CL-09 — owner_id Result-typed (Stage 3)`

## §6 Stage 4 — Doc sync

- `docs/MATURITY.md`: append «Lifecycle correctness 2026-04-27 (CL-cluster) (PR `<sha>`)» under `nebula-credential` and `nebula-engine` Audited columns.
- `docs/OBSERVABILITY.md`: add metric `credential.reauth_required_total` (counter; labels: `reason ∈ {InvalidGrant, InvalidClient, NoRefreshToken, ...}`); span attr `credential.context.missing_owner` (boolean for diagnostic).
- `docs/UPGRADE_COMPAT.md`: 3 breaking changes (TokenRefreshError variants, RefreshOutcome::ReauthRequired variant, owner_id Result type).
- `docs/tracking/credential-concerns-register.md`: 3 new rows (CL-04, CL-07, CL-09) with `decided` status.
- `docs/superpowers/specs/2026-04-24-credential-tech-spec.md`: amendment notes in §6 (terminal-error mapping discipline) and §15.4 / §15.8 (capability honesty interpretation).
- `CHANGELOG.md`: «Security/Correctness» entry — 3 OAuth2 lifecycle fixes.

## §7 Test strategy

**Reproducer regressions (mandatory landing gates per Stage 1-3):**
- `cl04_invalid_grant_to_reauth.rs`
- `cl07_capability_honesty.rs`
- `cl09_owner_fail_open.rs`

**New unit / integration tests:**
- Per-`TerminalKind` mapping tests (5 OAuth2 standard terminals + Other).
- Capability operational dispatch tests for each fixed capability (Option A) or registry filter tests (Option B).
- Owner Result-handling tests: `MissingOwner` vs `MissingSession` symmetry.

**Wiremock infrastructure:** reuses pattern from security-hardening Stage 3 — no new test infrastructure introduced.

## §8 Acceptance criteria per stage

| Stage | Acceptance |
|---|---|
| 0 | 3 reproducers committed and failing |
| 1 | CL-04 reproducer green; per-`TerminalKind` mapping tests; cross-cut with CC-02 verified |
| 2 | CL-07 audit returns zero dishonest capabilities; per-capability integration / filter tests |
| 3 | CL-09 reproducer green; workspace audit shows zero silent-fallback callers |
| 4 | All docs synced; register rows added |

**Spec-level DoD:**
- 3 reproducers green.
- All new tests green.
- `cargo nextest run --workspace` no regression.
- `cargo clippy --workspace -- -D warnings` green.
- Tech Spec §6 + §15.4 / §15.8 amendments applied.

## §9 Deferred / parallel tracks

| Item | Forward-pointer | Reason |
|---|---|---|
| CC-01, CC-02, CC-03, CC-06 (concurrency) | `2026-04-27-credential-concurrency-correctness-design.md` | Sibling spec; CC-02 cross-cuts CL-04 |
| `StoredCredential.data: Vec<u8>` ambiguity (CC-08) | П3 architectural scope (TBD) | Type design refactor |
| Built-in crate split (CC-10) | П3 architectural scope (TBD) | Crate organization |
| OAuth2 revoke endpoint per-IdP customization | If Option A in Stage 2 chosen but revoke is too expensive | May fall back to Option B for revoke only |

## §10 Migration / rollout

**Breaking changes (active dev mode):**
- `TokenRefreshError` enum variants restructured (new `terminal: TerminalKind` field on `TokenEndpoint`). Workspace impact: limited to `crates/engine/src/credential/rotation/`.
- `RefreshOutcome` adds `ReauthRequired { reason: ReauthReason }` variant. Existing matches break.
- `CredentialContext::owner_id()` returns `Result<&str, ContextError>` instead of `&str`. Workspace impact: every caller must handle the Result.
- (If Option A in Stage 2) OAuth2 capability methods now have working bodies — no longer return «HTTP transport has moved» error.

**Rollout discipline:**
- Stages 1-3 = 3 squash-merge commits. Stage 4 = 1 commit. Each stage independently revertable.
- Stage 1 (CL-04) cross-cuts with concurrency-correctness CC-02. Land CC-02 FIRST, then CL-04 — otherwise CL-04 sets `reauth_required = true` but CC-02 hasn't yet wired the resolve-time check; the row flag exists but is ignored.
- Sequencing dependency: **concurrency-correctness Stage 2 (CC-02) → this spec Stage 1 (CL-04)**.

---

**Spec complete.** Implementation plan to follow per writing-plans skill.
