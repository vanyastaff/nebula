# Phase 2 — Security-Gate Review

**Date:** 2026-04-24
**Reviewer:** security-lead (subagent dispatch)
**Reviewing:** `03-scope-options.md` (architect draft)
**Mode:** co-decider (parallel authority with tech-lead; architect frames)
**Scope:** security-only adjudication. Non-security axes (DX, idiom, budget) intentionally untouched — tech-lead owns priority-call.

---

## TL;DR

| Option | Verdict |
|---|---|
| **A — Minimal** | **BLOCK** — leaves 🔴-1 silent-revocation-drop in production with no atomic landing path. "Follow-up project" framing is not acceptable. |
| **B — Targeted** | **ENDORSE WITH AMENDMENTS** (3 amendments) — materially satisfies the atomic-rotation-dispatcher ask from Phase 1 (`02-pain-enumeration.md:229-231`). |
| **C — Comprehensive** | **ENDORSE WITH AMENDMENTS** (same 3 amendments as B; no incremental security concerns from the C-only additions) — security-equivalent to B. |

**Preferred option:** **B + amendments.** The C-only additions (Runtime/Lease collapse, `AcquireOptions` resolution, possible Service/Transport merge) are net-neutral from a security standpoint; I defer to tech-lead on whether the scope expansion is worth the spike cost. I would not block C.

---

## Per-option verdicts

### Option A — BLOCK

**Verdict: BLOCK.** Cannot endorse. This directly answers architect's open question 1 (`03-scope-options.md:376`): "defer credential rotation redesign to a follow-up project" **does not satisfy** the Phase 1 security-lead position.

**Evidence:**

1. **My Phase 1 position was "atomic landing, standalone PR not viable"** — recorded verbatim in `02-pain-enumeration.md:229-231`:
   > Rotation dispatcher + reverse-index write must land atomically. Standalone PR not viable.

   Option A does not honor this — it explicitly defers rotation dispatcher to a separate project (`03-scope-options.md:50`: "Today's behaviour remains: revocations silently dropped, `todo!()` reachable if anyone adds a reverse-index write path"). That is the exact latent-panic shape I flagged. Calling the follow-up "atomic" does not satisfy the atomicity invariant of *today's* trunk — it just moves the problem off this cascade's critical path while leaving the production-observable gap open.

2. **The corrected threat characterization in `01-current-state.md §3.1` (lines 100-117)** makes the severity concrete:
   > **Today:** 🟠 HIGH — credential revocations silently dropped. Outstanding guards holding revoked credentials continue to serve traffic until natural refresh (minutes to hours in pooled configs). Effective TTL of a revoked credential extends to pool idle-timeout, not the revocation API call.

   Translating to the threat-actor taxonomy I keep in `MEMORY.md`:
   - **Compromised-credential incident response**: revocation is the tenant's only tool after a leak. Silent-drop means the tenant believes they revoked; the resource continues issuing authenticated queries for minutes-to-hours. That is a *safety-critical* capability we advertise but do not deliver.
   - **Supply-chain-actor-with-single-PR-write-access**: a reverse-index-write PR without the dispatcher transitions this from silent-drop to `todo!()` panic — liveness regression. Option A explicitly preserves both failure modes and makes the second trivially reachable.

3. **DX-tester's Phase 2 input agrees Daemon/EventSource can ship documented-broken** (`02-pain-enumeration.md:241`); no Phase 1 agent said the same about 🔴-1. Security-lead is the veto on the credential seam.

**What would change my verdict on A:** only one thing — if the team commits to **rotation dispatch deprecation today**, i.e., removing `Manager::on_credential_refreshed` / `on_credential_revoked` from the public surface *and* removing the reverse-index field *atomically in Option A's doc PR*, so that neither silent-drop nor latent-panic can be reached, pending full redesign. That is itself a substantive API change, not a doc-only surface, and effectively forces Option B. So in practice: **A is unfixable without becoming B.**

### Option B — ENDORSE WITH AMENDMENTS

**Verdict: ENDORSE WITH AMENDMENTS.** Option B directly addresses the security ask. The atomic landing, Tech Spec §3.6 blue-green pattern, and observability-as-DoD are all already baked into B.2 🔴-1 treatment (`03-scope-options.md:120`). Three amendments below to tighten the contract.

**Positive security observations already in the draft:**

- ✅ **Atomic landing is explicit.** `03-scope-options.md:120` says "Ships atomically: trait reshape + reverse-index write + dispatcher + rotation observability." This is exactly my Phase 1 ask.
- ✅ **Tech Spec §3.6 blue-green adoption is explicit.** `03-scope-options.md:157-178` shows the trait shape matches Tech Spec §3.6 (lines 928-996) verbatim: `type Credential: Credential`, `on_credential_refresh(&self, new_scheme) -> Result<(), Self::Error>` default-noop. This is my Phase 1 ask: per-resource swap, not manager-orchestrated recreation.
- ✅ **Per-resource blue-green isolation.** Tech Spec §3.6 lines 981-992 (the `PostgresPool` example) demonstrates the pattern: `Arc<RwLock<Pool>>` swap, old RAII guards drain naturally, new queries use new pool. This is what I argued for in Phase 1 because it **never observes plaintext outside the resource's own impl**: the new scheme is passed to the resource, which builds the new pool locally and swaps under its own `RwLock`. Manager never holds the scheme longer than the dispatch call. Compare with manager-orchestrated recreation where the manager would have to hold the scheme across a recreate-destroy cycle — more surface, more lifetime coupling, higher chance of leaked plaintext in an error path.
- ✅ **Observability included in 🔴-1, not bolted on.** `03-scope-options.md:130` folds 🟠-14 (missing observability) into 🔴-1 treatment. Aligns with my `feedback_observability_as_completion.md` — trace span + counter + event ship with the state change, not after.

**Amendments required:**

**Amendment B-1 (hard-required): Isolation invariant for concurrent `on_credential_refresh` dispatch.**

- `03-scope-options.md:221` flags concurrency as unresolved: "per-resource `on_credential_refresh` calls may run in parallel across many resources sharing one credential, or serial? Tech Spec §3.6 doesn't specify."
- **Security constraint regardless of serial/parallel choice:** one resource's failing `on_credential_refresh` MUST NOT prevent other resources from rotating. A failing hook must be isolated (error recorded, observability emitted, `HealthChanged { healthy: false }` event fired for that resource), not propagate to sibling dispatches.
- Threat: if hook-failure propagates, a malicious/buggy resource's `on_credential_refresh` panic could block credential rotation for *all* resources sharing the credential → extends the effective revocation window across tenants.
- Phase 3 Strategy must encode this as an invariant + Phase 4 spike probe 3 (`03-scope-options.md:236`) must validate it. Spike probe 3 already names this ("one failing hook does not block the other") — good, but Strategy must commit to it before Phase 4.

**Amendment B-2 (hard-required): Revocation path (`on_credential_revoked`) receives same treatment as refresh.**

- Tech Spec §3.6 (lines 928-996) only defines `on_credential_refresh`. It does not define `on_credential_revoke` or equivalent.
- Current `Manager::on_credential_revoked` (`manager.rs:1386`, `todo!()` body) is the more safety-critical of the two — **revocation is the incident-response lever**. Silent-drop on revoke is the bigger risk than silent-drop on refresh.
- Option B as drafted (`03-scope-options.md:120`) mentions "dispatcher replaced by per-resource dispatch" but doesn't split refresh vs revoke treatment.
- **Required:** Phase 3 Strategy must explicitly answer: does `on_credential_refresh` carry both semantics (resource decides how to react to the scheme being revoked — typically tear down pool), or is there a separate `on_credential_revoke`? Spec §3.6 is silent; the Strategy must extend §3.6 and loop spec-auditor if the extension changes the Tech Spec's semantic surface.
- Architect flag: this is potentially a spec-auditor handoff in Phase 3. If Strategy extends §3.6, Tech Spec must be updated or a follow-up ADR written.
- **Concrete ask:** whatever the answer, the revocation dispatch must emit a `HealthChanged { healthy: false }` event per-resource (already in B.7 risk point 3, `03-scope-options.md:219`). Keep that item; don't let it fall out.

**Amendment B-3 (hard-required): `warmup_pool` footgun (🟡-17, `02-pain-enumeration.md:178`) must be resolved by the Auth→Credential migration.**

- Current code at `manager.rs:1268` calls `R::Auth::default()` for warmup. Phase 1 flagged this as a plugin footgun (if a plugin's `AuthScheme` impls `Default` with zero bytes, warmup uses empty credential).
- Under Option B, `type Auth` is removed and replaced by `type Credential`. `warmup_pool` must not gain a `R::Credential::Scheme::default()` call — that would reproduce the footgun with a fresh coat of paint.
- **Required:** Phase 3 Strategy must specify warmup semantics for credential-bearing resources. Likely answer: warmup requires a real scheme (fetched from credential store at warmup time), not a `Default`. If no scheme is available at warmup, warmup is skipped for credential-bearing pools.
- This is a small point but it's the kind of thing that slips through a trait-reshape PR if not called out. Phase 5 review will flag it anyway; calling it out in Strategy saves a round.

### Option C — ENDORSE WITH AMENDMENTS

**Verdict: ENDORSE WITH AMENDMENTS (same B-1, B-2, B-3 as above).**

From a pure security standpoint, C is **equivalent to B** on the security-critical axes:
- Rotation seam treatment: identical (§3.6 adoption).
- Atomic landing: identical.
- Observability: identical.
- Revocation handling: same amendment (B-2) applies.
- Warmup footgun: same amendment (B-3) applies.

The C-only additions (`Lease = Runtime` default, `AcquireOptions::intent/.tags` resolution, possible Service/Transport merge) are **orthogonal to credential security**. They touch trait-shape and topology-count surfaces, not secret-handling paths. I have no security objection to any of them.

**One note on C.8a (remove `AcquireOptions::intent/.tags`) vs C.8b (wire semantics):** security-lead has a mild preference for **C.8a (remove)** on canon §4.5 grounds (consistent with `feedback_incomplete_work.md` that I endorsed in Phase 1). Advertised-but-unimplemented options that might later be read by trust-sensitive code (e.g. `AcquireIntent::Critical` could plausibly gate a capability check someday) are dead-code attack surface — an attacker who finds a way to set them and discovers they're inert learns nothing, but an attacker who finds a way to set them and discovers they *are* read in a half-finished state learns a lot. Simpler to remove until needed. **Not a BLOCK — defer to tech-lead on the sub-decision.**

**No additional security amendments beyond B-1/B-2/B-3.**

---

## Security-critical constraints (option-agnostic)

Regardless of which option wins (B or C — A blocked), the Strategy (Phase 3) and Tech Spec (Phase 5, if one is drafted) **MUST** encode these constraints:

1. **Atomic landing of rotation redesign.** Trait reshape + reverse-index write at register + dispatcher at refresh/revoke + observability ship in **one PR**. No intermediate commit may leave the reverse-index populated without the dispatcher, or vice versa. Evidence: `02-pain-enumeration.md:229-231`, `01-current-state.md §3.1:114-117`.

2. **Tech Spec §3.6 blue-green pattern is mandatory for connection-bound resources.** The swap happens *inside* the resource impl, not inside the manager. The manager's dispatcher passes the new scheme to the resource; the resource handles its own `Arc<RwLock<Pool>>`-style swap. Manager never holds the new scheme longer than the dispatch call. Evidence: credential Tech Spec §3.6 lines 959-993 (the `PostgresPool` example with `Arc<RwLock<deadpool_postgres::Pool>>` and `on_credential_refresh` body swapping `*guard = new_pool`).

3. **Observability on rotation path is DoD, not follow-up.** Every `on_credential_refresh` and revocation dispatch emits: tracing span (scoped to the credential id + resource key), counter increment, and `ResourceEvent` on the broadcast bus. Silent-success is as bad as silent-drop for incident response. Consistent with my saved feedback `feedback_observability_as_completion.md`. Evidence: `02-pain-enumeration.md:175` (🟠-14).

4. **Isolation invariant for concurrent dispatch.** One resource's failing `on_credential_refresh` does not block sibling dispatches. Failure is recorded, `HealthChanged { healthy: false }` emitted for the failing resource, other resources proceed. Encoded as Strategy invariant + Phase 4 spike probe. (Expansion of B-1 amendment.)

5. **Revocation treatment is distinguished from refresh in the Strategy.** Either `on_credential_refresh` carries both semantics (documented), or a separate `on_credential_revoke` is added. If extending Tech Spec §3.6, loop spec-auditor. Revocation is the incident-response lever — it is more security-critical than refresh. (Expansion of B-2 amendment.)

6. **`deny.toml` containment wrapper for `nebula-resource` (SF-1) ships before or with the cascade, not after.** `deny.toml` entries for the other crates at `deny.toml:41-81` show the pattern; resource is a gap. This locks the 5 consumers today and prevents a future PR from adding a 6th consumer in an unexpected tier without explicit review. **Standalone PR via devops — see §Standalone-fix PR recommendations below.**

7. **No `clone()` on secret schemes in the dispatcher hot path.** Phase 1 noted `AuthScheme: Clone` as 🟡-16 (`02-pain-enumeration.md:177`). Under Option B, `Credential::Scheme` replaces `AuthScheme`. The dispatcher passes `&<Self::Credential as Credential>::Scheme` to `on_credential_refresh` per Tech Spec §3.6 line 951 — it's a borrow, good. Strategy must preserve this: do not introduce a `clone()` on the scheme at dispatch time even if it's "more convenient." Each clone is another zeroize obligation. (credential-security-review skill §4.2 safety invariants.)

8. **Warmup must not use `Scheme::default()`.** Warmup of a credential-bearing pool must fetch a real scheme or skip warmup. `Default` schemes have been a footgun historically (`02-pain-enumeration.md:178`). (Expansion of B-3 amendment.)

9. **Input validation at the dispatcher boundary is preserved.** The manager's dispatcher receives `credential_id: &CredentialId` from external event sources (credential refresh events). Current read path at `manager.rs:1365` already uses the id only as a dashmap key — safe. Strategy must not introduce any path where the id is used to construct a log message, a filesystem path, or an error body without sanitization. Low risk but worth codifying.

---

## My preferred option + amendments

**Preferred: Option B + amendments B-1, B-2, B-3.**

Reasoning:
- B is the minimum scope that satisfies the security ask (atomic rotation landing, §3.6 adoption, observability DoD).
- C adds security-neutral scope. If tech-lead wants C for idiomatic / DX reasons, I have no security objection, but I don't have a positive security argument for C over B either.
- A is blocked as explained.

If tech-lead priority-calls C, my amendments transfer unchanged — B-1, B-2, B-3 still apply. No additional security constraints for C.

---

## Coordination notes (tech-lead, architect)

**To tech-lead:**

1. **Your priority-call preview** (`02-pain-enumeration.md:224-227`) to drop `Auth` entirely in favor of `Credential` per §3.6 has **no adverse security impact** as long as the replacement satisfies the same zeroize/no-Debug/no-log guarantees that `AuthScheme` (via `SecretString`) currently enforces. Per Tech Spec §3.6 lines 935-957, `Credential::Scheme` is an associated type bounded by the `Credential` trait — secrets in schemes must continue to use the current secret-type wrapper (verify against `crates/credential/src/lib.rs` re-exports in Phase 3 — Strategy must cite the exact trait bound). I do not believe this is at risk, but calling it out so we don't regress in the reshape.

2. **Your preview picked "add `AuthenticatedResource: Resource` sub-trait with `type Credential`"** (`02-pain-enumeration.md:224`) but architect's Option B draft picked "drop `Auth`, add `type Credential` on `Resource` directly with `NoCredential` default." These are **different** shapes (open question 3 in `03-scope-options.md:380`). From a security standpoint, **both are acceptable** — neither weakens the seam; both enforce per-resource hook semantics. I have no security preference between them. Tech-lead decides on idiom grounds. My only ask: whichever shape is picked, the Strategy names the shape precisely and the Phase 4 spike validates it end-to-end (B.8 probe 1-3 already covers this for the §3.6-on-`Resource` shape; would need a parallel probe set for the sub-trait shape).

3. **Co-decider tie-break position** (per protocol, if you and I split): on Options A/B/C I am firm — A blocks, B or C endorses with B-1/B-2/B-3. On the B-vs-C sub-choice I defer to you unless C adds scope that materially threatens the atomic-landing invariant (I don't see how it would, but flagging as caveat).

**To architect:**

1. **Your draft is clean.** Open question 1 (`03-scope-options.md:376`) is answered: A is blocked, B/C acceptable with amendments. Open question 2 (Daemon/EventSource extraction, `03-scope-options.md:378`) is not a security concern — I have no blocking input there. Open question 6 (rotation dispatch concurrency) is answered by my amendment B-1: isolation invariant required regardless of serial/parallel choice.

2. **If B is picked and Strategy extends Tech Spec §3.6** (e.g., to add `on_credential_revoke` per my B-2 amendment), please loop **spec-auditor** to verify the extension doesn't conflict with §3.6's intent. This might also warrant a Tech Spec amendment or follow-up ADR — architect frames, spec-auditor verifies, I gate.

3. **Minor:** your recommendation section (`03-scope-options.md:392`) correctly flags Option A as dismissed on security-lead grounds. Confirmed.

---

## Standalone-fix PR recommendations

**SF-1 (`deny.toml` wrapper rule for `nebula-resource`) — confirmed standalone PR.**

- **Still standalone?** **Yes.** Ship as a separate PR. Does not need to land with the cascade. It's a CI-only surface change, zero source-code impact, no design dependency on the redesign.
- **Dispatch to:** **devops.** Confirmed from Phase 1 position (`02-pain-enumeration.md:232`).
- **Urgency:** should land *before* the cascade merges or in parallel. Reason: the cascade will touch `nebula-resource` public surface; landing SF-1 first means the cascade's migration PR is validated against the locked consumer set. If SF-1 lands *after* the cascade, we miss a chance to catch any inadvertent new consumer introduced by the migration.
- **Size:** trivial (5-10 lines TOML at `deny.toml:41-81`) per `02-pain-enumeration.md:150`.

**SF-2 (drain-abort phase corruption) — security-neutral, consumed into B/C per architect's plan.**

- Option A would ship SF-2 standalone. But since A is blocked, SF-2 is absorbed into Option B (or C) per `03-scope-options.md:27` — it touches `Manager::graceful_shutdown` which is already in B's surface. **Acceptable.** No security-specific reason to split it out separately.
- **Security-impact statement:** SF-2 fixes an observability corruption (phase flipped to `Ready` instead of `Failed` on drain-abort). This is a liveness/audit-trail issue, not a confidentiality issue. No secrets leak through this bug. Severity is 🔴 because of operational-impact (operators cannot detect the failed-drain state), not credential-impact. Security-lead defers to rust-senior / tech-lead on implementation.

**SF-3 (doc rewrite) — not shipping standalone.**

- Doc rewrite waits for trait shape to lock (DX-tester's Phase 2 input, `02-pain-enumeration.md:240`). No security input here.

**No new standalone-fix candidates** surfaced in this review.

---

## Convergence estimate

**Lock in round 1. High confidence.**

Reasoning:
- All three agents (architect, tech-lead, me) converge on rejecting A.
- Tech-lead preview picked a shape very close to Option B (`02-pain-enumeration.md:224-227`). Architect drafted Option B to match that preview 1:1 (`03-scope-options.md:362`: "≈ matches preview 1:1"). I endorse B with bounded amendments (3, all mechanical).
- The only contested sub-question is **B vs C** and the Auth-shape (drop vs sub-trait). These are idiom / DX decisions, not security decisions — tech-lead priority-calls and I do not block.
- My amendments B-1/B-2/B-3 are not surprising or scope-expanding; they tighten invariants that Phase 3 Strategy would encode anyway. No agent should push back on them.

**What would force round 2:**
- Tech-lead rejects my BLOCK on Option A and insists A is acceptable (would need to make a specific argument about why "follow-up project" addresses the atomic-landing invariant — I don't see the path).
- Tech-lead or architect adds a C-only element that materially expands the credential-seam surface (unlikely — C's additions are trait-shape / topology-count).
- Spec-auditor (if looped per B-2) raises a §3.6-extension conflict.

None of these seem likely in round 1 based on the convergent Phase 1 evidence. **Estimate: ~85-90% chance of round-1 lock on Option B + amendments.**

---

## Skill + memory alignment check

- **`credential-security-review` skill §4.2 safety invariants:** honored by constraint #7 (no `clone()` on secret schemes in dispatcher).
- **`credential-security-review` skill §12.5 encryption rules:** not directly touched by this cascade (credential persistence is not reshaped). Preserved by reference.
- **`MEMORY.md` `feedback_observability_as_completion.md`:** honored by constraint #3 (observability is DoD, not follow-up).
- **`MEMORY.md` `feedback_incomplete_work.md`:** honored by rejecting Option A's "documented-broken" framing for 🔴-1 (but accepting it for 🔴-2/🔴-6 where DX-tester and tech-lead both endorse documented-broken).
- **`MEMORY.md` `feedback_hard_breaking_changes.md`:** aligned — B/C both ship as breaking PRs to 5 in-tree consumers, not deprecation-window machinery.

---

*End of security-gate review. Returning to orchestrator with under-250-word summary.*
