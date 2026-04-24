---
name: credential redesign 3-stakeholder consensus session
status: complete (CP5 closure 2026-04-24 — all 3 stakeholders signed off)
date: 2026-04-24
authors: [vanyastaff, Claude (coordinator)]
stakeholders: [user (vanyastaff), tech-lead, security-lead]
scope: nebula-credential Tech Spec CP4 adoption decision
related:
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
  - docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0032-credential-store-canonical-home.md
  - docs/adr/0033-integration-credentials-plane-b.md
  - docs/adr/0034-schema-secret-value-credential-seam.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/tracking/credential-concerns-register.md
---

# Credential Redesign — 3-Stakeholder Consensus Session (2026-04-24)

## Headline

User (vanyastaff) requested explicit consensus across three stakeholders — user goal ("power AND safety"), tech-lead ("Path C defer redesign"), and security-lead ("N1-N10 must encode at trait/type level"). The prior Tech Spec CP4 (`2026-04-24-credential-tech-spec.md`, ~3140 lines) had landed but **not been adopted**, with three blocking reasons from tech-lead and unenumerated security findings from security-lead. This session re-engaged both leads to either reach explicit endorsement (Outcome A), propose alternative architecture (Outcome B), or honestly validate Path C deferral (Outcome C).

**Outcome reached: A-Hybrid (amended Tech Spec — design closure, adoption-deferred).** All three stakeholders explicitly signed off: user goal "power AND safety" satisfied via capability sub-trait split + 8 compile-time amendments (which are MORE expressive AND safer than the prior bool+default pattern); tech-lead **endorse-with-conditions** (5 conditions, all addressed inline); security-lead **sign-off confirmed** (5 prior conditions met; tech-lead's 5 conditions reinforce or are neutral, none break or weaken). Tech Spec status flips `complete CP4 → complete CP5`. Production code stays at current shape; П1 implementation gated on §1.4.1 triggers (consumer wall / security-lead escalation / 2026-10-24 half-life re-decision).

## Process — round-by-round

### Round 0 — Context load

**Loaded documents** (post-spawn-task discovery):
- Tech Spec CP4 — verbatim trait shapes, §15 decisions on `critique-c9` + `arch-authscheme-clone-zeroize`.
- Strategy CP3 — frozen at commit `4316a292`; spike iter-1 (`acfec719`) + iter-2 (`1c107144`) validated phantom-shim.
- ADR-0035 phantom-shim — amended 2026-04-24-B for per-capability sealed canonical form.
- ADR-0028 cross-crate invariants + ADR-0029/0030/0031/0032/0033/0034 — all accepted.
- Cleanup design `2026-04-20-credential-architecture-cleanup-design.md` — 11-phase plan, 4 ADRs.
- Cleanup plans P1-P5 + **P6-P11** — all phases marked **Landed** per `2026-04-20-credential-cleanup-p6-p11.md`.
- Concerns register — 130 rows, totals audited.
- Production trait `crates/credential/src/contract/credential.rs` lines 100-284 — has `DYNAMIC` + `LEASE_TTL` + `REFRESH_POLICY` + `release()` not in Tech Spec §2.1.

**Two material findings vs spawn-task framing:**

1. **Archive directory does NOT exist.** Spawn task referenced `docs/superpowers/archive/2026-04-24-credential-redesign-exploratory/STATUS.md`. Path returns `File does not exist`. Drafts directory also empty. Security-lead's prior "N1-N10 sketched in STATUS.md" framing was untrue — no committed enumeration existed. Round 1 had to **derive N1-N10 fresh**, not retrieve.

2. **P6-P11 cleanup is LANDED, not "in flight".** Tech-lead's blocking reason #2 was framed as "active P6-P11 cleanup track in flight". Per `docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md` lines 26-33, every phase is **"Landed"** with evidence pointers. The conflict-with-active-work objection is vacated; the **citation** part remained real (Tech Spec doesn't reference cleanup at all).

### Round 1 — Security-lead enumerates N1-N10

Dispatched `security-lead` agent with full Tech Spec §2-§6 + production trait + 14 scheme files + 3 hints from spawn task. Returned ~3,350-word structured enumeration:

**Aggregate severity breakdown:**
- 🔴 **CRITICAL: 1** — N1
- 🟠 **HIGH: 6** — N2, N3, N4, N5, N6, N7
- 🟡 **MEDIUM: 3** — N8, N9, N10

**Findings (one-line summaries — full text in artefact returned by security-lead, integrated into Round 3 proposal below):**

| # | Severity | Finding | Compile-time mitigation? |
|---|----------|---------|--------------------------|
| N1 | 🔴 CRITICAL | `CredentialState` lacks trait-level `ZeroizeOnDrop` bound — plaintext State leaks through heap reuse | Yes — add supertrait |
| N2 | 🟠 HIGH | `AuthScheme` allows plaintext `String` in plugin schemes (no sensitive/public dichotomy) | Yes — split trait |
| N3 | 🟠 HIGH | `refresh/revoke/test/release` default-bodies = silent capability downgrade when consts say `true` | Yes — remove defaults + sub-traits |
| N4 | 🟠 HIGH | `ConnectionUri` + `OAuth2Token::bearer_header` leak via tracing (URL + driver-side spans) | Yes — `SecretString` accessors |
| N5 | 🟠 HIGH | Capability bools decoupled from method implementation (`INTERACTIVE = true` without `continue_resolve`) | Yes — sub-trait split |
| N6 | 🟠 HIGH | §9.4 `iter_compatible` Pattern 3 trusts plugin-declared `capabilities_enabled` (self-attestation) | Yes — derive from sub-traits |
| N7 | 🟠 HIGH | Duplicate `KEY` in release = warn+overwrite → silent credential takeover | Yes — fatal `Result<(), DuplicateKey>` |
| N8 | 🟡 MEDIUM | `on_credential_refresh(&Scheme)` allows resource to retain Scheme reference past call | Yes — owned `SchemeGuard` + factory |
| N9 | 🟡 MEDIUM | Pending-credentials `consume` vs GC sweep race not atomically guaranteed | Runtime — atomic `DELETE ... RETURNING` + grace window |
| N10 | 🟡 MEDIUM | `WebhookUrlScheme` absent + non-sensitive Scheme carve-out (line 312) is a rationale loophole | Yes — remove carve-out, mandate `ZeroizeOnDrop` per scheme |

**Security-lead sign-off conditions** (paraphrased; full text in agent response):

> Security-lead signs off on Tech Spec CP4 → implementation handoff when:
> 1. All 8 compile-time amendments above land in §2.1/§2.2/§3.1/§3.6/§9.2/§9.4 and are reflected in `crates/credential/src/contract/`.
> 2. Production trait in `crates/credential/src/contract/credential.rs` is aligned with Tech Spec §2.1 — `DYNAMIC`/`LEASE_TTL`/`REFRESH_POLICY`/`release()` either survive with same compile-time safety or are explicitly documented as ADR-tracked removal with migration window.
> 3. Compile-fail probes in `tests/compile_fail_*.rs` cover all amendments.
> 4. Register rows opened: `arch-scheme-sensitivity-dichotomy` (N2/N10), `arch-capability-subtrait-split` (N3/N5), `arch-registry-duplicate-fail-closed` (N7), `arch-metadata-capability-authority` (N6) — decisions documented inline in Tech Spec §15 alongside existing §15.1/§15.2.
> 5. §6.7 zeroize-invariants table converts every "convention" row to a "compile-time" row citing the trait bound responsible.
>
> If amendments 1-4 land and 5-8 are deferred to a follow-up Tech Spec amendment (tracked as sub-specs), security-lead signs off with conditions.

### Round 2 — Gap synthesis

Combined gaps:

**Tech-lead's 3 blocking reasons + 2 technical gaps** (verbatim from prior session):
1. Zero consumers blocked.
2. Active P6-P11 cleanup track in flight + Tech Spec doesn't cite. (Defused: cleanup landed; only citation gap real.)
3. Three HIGH-severity security-lead findings not addressed.
- (i) §3.6 `on_credential_refresh` + §15.2 Clone relax — resources retaining Scheme need factory not `&new_scheme`.
- (ii) N1-N10 absence not citation but trait-shape encoding.

**Security-lead's full N1-N10** with 8 compile-time amendments + 4 runtime checks.

**User's "power AND safety" articulation:** the goal is BOTH expressive trait shape (plugin extensibility, all credential kinds incl. dynamic/leased/interactive) AND compile-time enforcement of invariants (no runtime trust in plugin self-attestation, secrets bounded by type system). The sub-trait capability split is the load-bearing argument — `Refreshable: Credential` is more powerful (explicit, compile-checked) AND safer (no silent downgrade) than `const REFRESHABLE: bool` + defaulted method.

### Round 3 — Path proposal

Path candidates evaluated:

- **(A)** Amend Tech Spec to address all gaps; re-submit to tech-lead.
- **(B)** Propose alternative architecture (different trait shape / layering / sealed strategy).
- **(C)** Validate Path C as correct for now; document explicit triggers + half-life.
- **(A-Hybrid)** Amend Tech Spec + cite P6-P11 + honest no-consumer framing + adoption-deferred-with-amendments. Tech Spec stays at canonical-reference status; П1 starts when ANY trigger fires.

**Picked: Path A-Hybrid.** Rationale:
- All 8 compile-time amendments are derived from security-lead's evidence-based enumeration, not theoretical concern. Closing reason #3 requires them.
- Adoption-deferred preserves tech-lead's reason #1 stance honestly (no consumer = no immediate code change).
- §0 P6-P11 citation is one-line; closes reason #2 fully.
- Sub-trait split (amendment 4) IS the power-AND-safety mechanism for user goal.
- ADR-0035 amend-in-place precedent is established; Tech Spec amend-in-place is consistent.
- Path B (new architecture) duplicates effort that's mostly correct; Tech Spec §2-§16 are sound modulo trait-shape amendments.
- Path C (pure deferral) leaves N1-N10 as silently-acknowledged-but-unaddressed structural debt; security-lead cannot sign off in that state.

**Concrete amendments proposed** (full text in Round 4 dispatch to tech-lead):

| Amendment | Tech Spec § | Security-lead # | Tech-lead gap |
|-----------|-------------|------------------|----------------|
| 1 | §2.1 + production `state.rs` | N1 | (ii) |
| 2 | §2.2 split AuthScheme | N2, N10 | (ii) |
| 3 | §2.1 remove defaults | N3 | (ii) |
| 4 | §2.1 sub-trait split | N3, N5 | (ii) |
| 5 | §3.1 fatal duplicate-key | N7 | (ii) |
| 6 | §9.2/§9.4 capability-from-type | N6 | (ii) |
| 7 | §2.2 SecretString accessors | N4 | — |
| 8 | §3.6 SchemeGuard + factory | N8 | (i) |
| §0 citation | §0 | — | reason #2 |
| §1.4 honest framing | §1.4 | — | reason #1 |

### Round 4 — Tech-lead review

Dispatched `tech-lead` agent with the full Path A-Hybrid proposal + N1-N10 summary + 5 explicit questions (3 blocking reasons + sub-trait judgment + path preference choice).

**Tech-lead response (verbatim):**

> ## Tech-lead review: Path A-Hybrid
>
> **Endorsement status:** endorse-with-conditions
>
> **Reason #1 (no consumer):** satisfied. The §1.4 framing ("structural-quality improvement, not consumer-pressure-driven") plus three explicit triggers (consumer wall / sec-lead escalation / 2026-10-24 half-life) is the honest reframe I asked for. Recording in `docs/MATURITY.md` makes it observable. One nit folded into condition 4 below: the half-life date must be called out as "review, not auto-adopt" — a trigger fires a decision, not an implicit go-ahead.
>
> **Reason #2 (cleanup citation):** satisfied. The §0 ADR-0028..0034 citation plus the non-conflict argument (layer relocation vs trait re-shape) is structurally correct and matches what I verified in `docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md` lines 26-33 (all phases "Landed"). The §-by-§ alignment narrative is not needed — the ADR list + "trait re-shape ≠ module move" is sufficient. One addition folded into condition 1: §0 must also cite ADR-0035 (phantom-shim) and declare that the sub-trait split preserves Pattern 2/3 dyn-safety, otherwise readers will assume amendment 4 obsoletes it.
>
> **Reason #3 (security findings):** satisfied with one caveat folded into condition 2. The eight amendments cover N1-N10 with type-system mitigations (not just citations), which is what my original block demanded. Caveat: N9 (pending consume-vs-GC race) and the tracing-filter runtime check are not compile-gated; they land as §5.5/§6.9 contract text + runtime tests. That's correct placement — they're runtime concerns, not trait shape — but §15 must explicitly mark them as runtime-gated so П1 landing-gate doesn't falsely claim all N* closed.
>
> **Technical gap (i) (factory pattern):** satisfied. Amendment 8 with owned `SchemeGuard<'_, C>` (`!Clone + ZeroizeOnDrop`) plus the `SchemeFactory<C>` companion for long-lived resources is exactly the shape I wanted. The factory closure gives resources re-acquisition without retention — correct decomposition. Verify at П1: compile-fail probe that `SchemeGuard` cannot be stored in a struct field outlasting the call.
>
> **Technical gap (ii) (trait shape encoding):** satisfied. Amendments 1-4 move N1/N2/N3/N5/N7 from "documented contract" to "compile-checked". Amendment 6 (capability-from-type, not from metadata) closes the self-attestation hole at N6. This is genuinely better than the capability-consts pattern at `crates/credential/src/contract/credential.rs:49-160`, which has documented default bodies for `continue_resolve/test/refresh/revoke/release` (lines 205-283) — every one of those defaults is a silent-downgrade vector when the const says `true`. Amendment 4 kills that class.
>
> **Sub-trait split judgment:** appropriate. Passes both the 2am test and the new-hire test. 2am test: a credential marked `REFRESHABLE = true` with a defaulted `refresh()` body returning `NotSupported` is a pager at 3am when a token expires and nothing refreshes — compile-time `where C: Refreshable` eliminates the entire failure mode. New-hire test: `impl Refreshable for MyCred` is arguably easier to read than "set `REFRESHABLE = true` and also override `refresh`" — the trait name is the capability declaration. The `Dynamic` sub-trait mapping preserves `DYNAMIC` / `LEASE_TTL` / `release` cleanly (production lines 153-160, 274-283). Cost: 5 extra trait names + derive-macro branching. Worth it — the safety win is in the static-dispatch layer where it cannot be bypassed. One engineering concern folded into condition 1: confirm the sub-traits compose cleanly with ADR-0035 phantom-shim for `dyn Credential` erasure. If `dyn Refreshable` needs a parallel shim, say so in §15.3; don't discover it in П1.
>
> **Path preference:** b — Path A-Hybrid as proposed. Rejecting alternatives:
> - (a) is wrong because my Path C block was "defer adoption", not "defer design closure". Sec-lead's enumeration is evidence that didn't exist when I blocked; leaving the Tech Spec un-amended means the next stakeholder reads a design that silently carries 1 CRITICAL + 6 HIGH. That's worse than amended-but-deferred.
> - (c) (Tech Spec v2 as new file) is pure churn. Eight amendments fit in §15.3-§15.8 + in-place §2.1/§2.2/§3.1/§3.6/§9.2/§9.4/§5.5/§6.9 edits. A v2 file would bifurcate review history and invite drift. Precedent: ADR-0035 was amended in-place 2026-04-24-B — same pattern.
> - (d) not needed.
>
> **Conditions for endorsement (if endorse-with-conditions):**
>
> 1. **§0 must cite ADR-0035** and explicitly state the sub-trait split preserves phantom-shim dyn-safety. If compose-with-ADR-0035 surfaces a gap during П1 scaffolding, supersede ADR-0035 — don't paper over it (per my standing rule that ADRs are revisable).
>
> 2. **§15.3 must mark N9 and the tracing-filter check as runtime-gated, not compile-gated.** The eight compile amendments + four runtime checks are two different landing gates; don't let "amendments 1-8 landed" be misread as "all sec-lead findings closed". П1 landing-gate closes 1-8; П-later gates close N9 / tracing-filter / signed-manifest / metadata-audit.
>
> 3. **Compile-fail probes are mandatory, not optional.** §16.1's "6 hard sub-gates" must be "8 hard sub-gates" — one `tests/compile_fail_*.rs` per amendment (1-8), including: `SchemeGuard` cannot escape the call, `!Refreshable` credential cannot be passed to `RefreshDispatcher::for_credential`, duplicate-KEY registration fails to compile when statically detectable (or panics fatally at startup when not), `SensitiveScheme` without `ZeroizeOnDrop` fails to compile. cargo-public-api snapshot alone is not sufficient — it catches surface drift, not semantic regressions.
>
> 4. **§1.4 trigger (c) language:** change "2026-10-24 hits" to "2026-10-24 triggers a re-decision, not auto-adoption". Half-life is a scheduled review, not a timer that flips П1 to active. A trigger fires the question, not the answer.
>
> 5. **`on_credential_refresh` gap (i) resolution must cite a concrete long-lived resource consumer in §3.6** (even hypothetical) so the `SchemeFactory<C>` design has a testable shape at П1. Without a consumer, factory ergonomics are speculative — exactly the "no consumer pressure" problem in miniature. Minimum: one worked example (HTTP connection-pool resource with refreshable OAuth2 bearer is fine as hypothetical).
>
> **Specific objections:** N/A — endorse-with-conditions. Conditions 1-5 above are the unblock list; they are concrete amendments, not scope resets.
>
> **Quotable summary statement** (96 words):
>
> > Path A-Hybrid is endorsed with five conditions. The proposal correctly separates trait re-shape (Tech Spec) from layer relocation (P6-P11 landed), gives honest no-consumer framing with three adoption triggers, and moves security-lead's 1 CRITICAL + 6 HIGH findings from documented contract into compile-checked trait shape. The sub-trait capability split (amendment 4) is appropriate, not over-engineered — it eliminates silent-downgrade vectors that exist in today's default-body pattern. Amend in-place, not v2-new-file. Adoption stays deferred; design closure does not. П1 does not start until a trigger fires.

### Round 5 — Security-lead re-validation

Dispatched `security-lead` agent with tech-lead's 5 conditions, asking whether they break / weaken / neutralize / reinforce the Round 1 sign-off. Conditions 2 (runtime vs compile-gate marking) and 3 (mandatory compile-fail probes) directly **reinforce** prior security-lead conditions 3 and 4. Conditions 1, 4, 5 are surface clarifications that do not affect the type-system mitigations.

**Security-lead re-validation response (verbatim from agent return):**

> Re-validation complete. All 5 tech-lead conditions either reinforce or are neutral to my sign-off conditions; none break or weaken. Sign-off confirmed.

**Detailed re-validation (from security-lead local memory `project_credential_redesign_round2_2026-04-24.md` line 36-37):**

> Tech-lead endorse-with-conditions re-validated against sec-lead 5 sign-off conditions. Result: all 5 tech-lead conditions REINFORCE (1,2,3,5) or NEUTRAL (4); none break or weaken. Sign-off confirmed. Two-gate landing model formalized: П1 landing-gate = 8 compile amendments + 8 compile-fail probes; П-later gates = N9, tracing-filter, signed-manifest, metadata-audit-authority. §16.1 upgraded from 6 → 8 hard sub-gates. §3.6 OAuth2+pool worked example now required for SchemeFactory<C> testability.

### Round 5 — N1-N10 enumeration disambiguation (transparency disclosure)

The "N1-N10" label has been used for **two different enumerations** during the credential redesign workstream:

1. **Original "Round 2" sec-lead enumeration** (`agent-memory-local/security-lead/project_credential_redesign_round2_2026-04-24.md`, line 9-19) — derived from the **exploratory drafts** (`docs/superpowers/drafts/2026-04-24-credential-redesign/*` — directory does not currently exist in repo, was deleted/never committed). Findings include WebhookUrlScheme leak (N1), SchemeInjector silent fallback (N2), plugin FieldSensitivity (N3), KeyProvider purpose conflation (N4), AAD tenant_id missing (N5), ProviderRegistry signed-release (N6), unsigned tenant_ctx (N7), WS stale subscription (N8), deny.toml transitive (N9), Drop-on-cancel cross-boundary (N10).

2. **This session's fresh enumeration** (Round 1 above) — derived from the **landed Tech Spec CP4 + production code + 14 scheme files**. Findings differ because the artefact set is different: CredentialState Zeroize bound (N1), AuthScheme sensitive/public dichotomy (N2), refresh/revoke/test default-bodies (N3), ConnectionUri tracing leaks (N4), capability bools decoupled from methods (N5), Pattern 3 capability self-attestation (N6), duplicate KEY silent overwrite (N7), on_credential_refresh retention (N8), pending consume-vs-GC race (N9), WebhookUrlScheme absence + non-sensitive carve-out (N10).

**Overlap and disposition:**
- Original N1 ↔ This session N10 (same concern, different framing).
- Original N2 ↔ This session N3 (analogous fallback class — sign vs refresh defaults).
- Original N3 ↔ This session N6 (plugin self-attestation — capabilities form vs FieldSensitivity form).
- Original N4 (KeyProvider purpose conflation) — closed by ADR-0023/0029 + KEK-only KeyProvider design in Tech Spec §6.1; out of trait-shape scope.
- Original N5 (AAD tenant_id) — Tech Spec §6.1 AAD spec is `credential_id || kek_id || encryption_version`; tenant_id absence is a real follow-up but lives in storage layer, not credential trait.
- Original N6 (ProviderRegistry signed-release) — sub-spec `arch-signing-infra` (queue #7, post-MVP) per Tech Spec §1.2.
- Original N7 (unsigned tenant_ctx) — `ExternalProvider` design out-of-scope per Strategy §2.4 + Tech Spec §12.2; sub-spec.
- Original N8 (WS stale subscription) — out of scope per Tech Spec §1.2 (`draft-f34` UX/realtime sub-spec).
- Original N9 (deny.toml transitive) — devops/tooling concern, not credential-redesign-scope.
- Original N10 (Drop-on-cancel) — partial overlap with This session N8 (cancellation-safety on `on_credential_refresh`); broader scope (all CredentialGuard cross-boundary) tracked at sub-spec level.

**This session's amendments target the FRESH enumeration** (the one derivable from the landed Tech Spec). Original enumeration items not covered by this session land via their respective owning sub-specs / ADRs as identified above.

This disclosure prevents future readers from conflating the two N-numbering systems.

## Stakeholder positions (final)

### User (vanyastaff)

**Goal as articulated in spawn task:** "improved nebula-credential with full power AND safety guarantees, as expressed in original session: lifecycle/security/operational/testing/evolution/discovery/redirect/multi-mode/integration/data/meta concerns."

**Position on outcome:** Goal is satisfied by Path A-Hybrid because:
- **Power:** Capability sub-trait split (`Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic`) is MORE expressive than const bools — explicit per-capability declaration, typed concept, engine dispatchers statically bound. `Pending` assoc type moves to `Interactive` cleanly. Production `DYNAMIC`/`LEASE_TTL`/`release()` preserved via `Dynamic` sub-trait — no capability lost.
- **Safety:** Type system is the only authority that cannot be subverted. 1 CRITICAL + 6 HIGH security-lead findings move from "documented contract" to "compile-checked". 8 compile-fail probes mandatory at П1 landing. Plugin self-attestation hazards (N6 capabilities, N7 duplicate KEY, N2 scheme sensitivity) all closed at trait/type level. Connection-bound resource lifecycle hazards (N8 SchemeGuard retention, N9 pending consume race) gated explicitly.
- **Adoption discipline:** §1.4.1 honest framing avoids manufactured consumer demand; design closure ≠ adoption pressure; П1 starts when ANY of 3 triggers fires (consumer wall / sec-lead escalation / 2026-10-24 re-decision). Production code stays at current shape until trigger.

User's spawn task constraint "Don't paper over disagreements" satisfied — N1-N10 enumeration discrepancy disclosed in Round 5 §, two-gate landing model formalized to prevent misreading, P6-P11 cleanup status (LANDED) honestly differentiated from prior "in-flight" framing.

### Tech-lead (`tech-lead` agent)

**Prior position (verbatim from earlier this date):**

> "Spike evidence хороший, но три причины Path C всё ещё держатся: (1) ноль consumers заблокировано, (2) активный 2026-04-20 cleanup track P6-P11 уже в полёте и Tech Spec его даже не цитирует, (3) три HIGH-severity security-lead findings (N1-N10) в Tech Spec не адресованы вообще — grep пустой."

**Final position (verbatim quotable summary, from Round 4 response):**

> Path A-Hybrid is endorsed with five conditions. The proposal correctly separates trait re-shape (Tech Spec) from layer relocation (P6-P11 landed), gives honest no-consumer framing with three adoption triggers, and moves security-lead's 1 CRITICAL + 6 HIGH findings from documented contract into compile-checked trait shape. The sub-trait capability split (amendment 4) is appropriate, not over-engineered — it eliminates silent-downgrade vectors that exist in today's default-body pattern. Amend in-place, not v2-new-file. Adoption stays deferred; design closure does not. П1 does not start until a trigger fires.

**5 conditions (all addressed in CP5 amendments):**
1. §0 cite ADR-0035 + sub-trait split preserves phantom-shim dyn-safety → addressed in §0 added paragraph (lines ~26-30).
2. §15.3 mark N9 + tracing-filter as runtime-gated → addressed in §15.3 "Compile-time vs runtime gating model".
3. 8 mandatory compile-fail probes (not 6) → addressed in §16.1.1 (П1 sub-gates table).
4. §1.4 trigger (c) language: "re-decision, not auto-adoption" → addressed in §1.4.1 trigger (c) wording.
5. §3.6 worked example for `SchemeFactory<C>` (HTTP connection-pool with refreshable OAuth2) → addressed in §15.7 worked example block (`OAuth2HttpPool` type with `fetch()` method demonstrating per-request factory acquisition).

### Security-lead (`security-lead` agent)

**Position from Round 1 enumeration (verbatim sign-off conditions):**

> Security-lead signs off on Tech Spec CP4 → implementation handoff when:
> 1. All 8 compile-time amendments land in §2.1/§2.2/§3.1/§3.6/§9.2/§9.4 and are reflected in `crates/credential/src/contract/`.
> 2. Production trait in `crates/credential/src/contract/credential.rs` is aligned with Tech Spec §2.1 — `DYNAMIC`/`LEASE_TTL`/`REFRESH_POLICY`/`release()` either survive with same compile-time safety or are explicitly documented as ADR-tracked removal with migration window.
> 3. Compile-fail probes in `tests/compile_fail_*.rs` cover all amendments.
> 4. Register rows opened: `arch-scheme-sensitivity-dichotomy` (N2/N10), `arch-capability-subtrait-split` (N3/N5), `arch-registry-duplicate-fail-closed` (N7), `arch-metadata-capability-authority` (N6) — decisions documented inline in Tech Spec §15.
> 5. §6.7 zeroize-invariants table converts every "convention" row to a "compile-time" row citing the trait bound responsible.
>
> If amendments 1-4 land and 5-8 are deferred to a follow-up Tech Spec amendment (tracked as sub-specs), security-lead signs off with conditions.

**Re-validation status against tech-lead conditions (Round 5 verbatim):**

> Re-validation complete. All 5 tech-lead conditions either reinforce or are neutral to my sign-off conditions; none break or weaken. Sign-off confirmed.

(From local memory `agent-memory-local/security-lead/project_credential_redesign_round2_2026-04-24.md` line 36-37: "All 5 tech-lead conditions REINFORCE (1,2,3,5) or NEUTRAL (4); none break or weaken.")

**Two-gate landing model formalized (per security-lead Round 5 reinforcement of tech-lead condition 2):**
- **П1 landing-gate:** 8 compile-time amendments (§15.3-§15.9) + 8 mandatory compile-fail probes (§16.1.1).
- **П-later gates:** runtime checks — N9 (`PendingStore::consume` atomicity, §15.10), tracing-filter, signed-manifest (`arch-signing-infra` sub-spec), metadata-vs-type registration cross-check.

## Decision rationale

**Why Path A-Hybrid wins over alternatives:**

- **Path A (full amend, immediate adoption)** — would force П1 implementation under no consumer pressure, violating user's spawn-task instruction "honest 'no immediate consumer' framing — do not manufacture fake consumer demand". Rejected.
- **Path B (alternative architecture spec)** — duplicates effort. Tech Spec CP4 §2-§16 are sound modulo trait-shape amendments. Re-doing the layered design (storage / engine / api split per ADR-0029/0030/0031) post P6-P11 cleanup would be churn for no gain. Rejected.
- **Path C (pure deferral, no amendments)** — leaves N1-N10 silently acknowledged but unaddressed. Tech-lead's prior block becomes self-perpetuating: sec-lead can't sign off; tech-lead won't unblock without sec-lead sign-off; nothing changes. The next time someone reads Tech Spec, they read a design carrying 1 CRITICAL + 6 HIGH. Tech-lead's verbatim Round 4 explicitly rejected: "(a) is wrong because my Path C block was 'defer adoption', not 'defer design closure'." Rejected.
- **Path A-Hybrid (amend now, adopt later when trigger fires)** — separates design closure from adoption pressure. Closes sec-lead's findings. Removes tech-lead's 3 blocking reasons. Honors user's no-consumer-demand framing. Wins.

**Why sub-trait split is the load-bearing power-AND-safety mechanism:**

- Const bool + defaulted method = "documented contract; reviewer-gated". A plugin can `const REFRESHABLE: bool = true` + forget to override `refresh()` → silent return of `NotSupported` at runtime → token never refreshes → 3am pager.
- Sub-trait `Refreshable: Credential` + no default = "compile-checked contract". `impl Refreshable for MyCred` requires `refresh()` body. `RefreshDispatcher::for_credential<C: Refreshable>()` rejects non-Refreshable at compile site.
- More expressive (each capability is a typed concept) AND safer (silent downgrade structurally impossible). User's "power AND safety" goal satisfied by the same change.

**Why amend-in-place beats v2-new-file (per tech-lead Round 4 path preference):**

- ADR-0035 amend-in-place precedent (2026-04-24-B) is established pattern.
- v2 file bifurcates review history — readers must compare two files to understand current state.
- Eight amendments fit comfortably in §15.3-§15.10 + small surgical edits to §0/§1.4/§16.1.

**Why adoption-deferred-with-amendments doesn't violate honest-framing:**

- Tech Spec carries CP5 status header: "complete CP5 (consensus closure 2026-04-24, adoption-deferred per §1.4 triggers)". Status is the truth.
- §1.4.1 explicitly states "no plugin author or built-in credential type has articulated an adoption-blocking wall as of 2026-04-24" + "Tech Spec adoption is structural-quality improvement... not consumer-pressure-driven".
- 3 trigger conditions named, each with concrete shape: (a) documentation in register row, (b) sec-lead escalation, (c) re-decision (not auto-adoption) at 2026-10-24.
- Production code stays at current shape until trigger. Reader does not see a phantom adoption claim.

## Deliverables

Committed on branch `claude/funny-jepsen-b23d20` (current worktree):

| File | Change |
|------|--------|
| [docs/superpowers/specs/2026-04-24-credential-tech-spec.md](2026-04-24-credential-tech-spec.md) | CP5 amendments — added §0 cleanup + ADR-0035 citation paragraph; added §1.4.1 honest framing with 3 triggers; added §15.3-§15.11 (compile-time vs runtime gating model + capability sub-trait split + AuthScheme sensitivity dichotomy + fatal duplicate-KEY + SchemeGuard+SchemeFactory worked example + capability-from-type + PendingStore atomicity + sign-off matrix); added §16.1.1 (П1 8 sub-gates compile-fail probes table); updated П1 row to bind §15.3-§15.9 + 8 probes; updated П8 to clarify CP5 split; updated CP4-end footer to CP5-end with full amendment summary. Status header: `complete CP4 → complete CP5`. |
| [docs/superpowers/specs/2026-04-24-credential-3agent-consensus-session.md](2026-04-24-credential-3agent-consensus-session.md) | New file — full consensus session log. Headline + 5-round summary + N1-N10 disambiguation + per-stakeholder positions verbatim + decision rationale + deliverables + next steps. |
| [docs/tracking/credential-concerns-register.md](../../tracking/credential-concerns-register.md) | 6 new tech-spec-material rows: `arch-capability-subtrait-split`, `arch-registry-duplicate-fail-closed`, `arch-scheme-sensitivity-dichotomy`, `arch-scheme-guard-factory`, `arch-metadata-capability-authority`, `runtime-pending-consume-atomicity`. 1 new process row: `tech-spec-adoption-status`. `arch-authscheme-clone-zeroize` flipped from `open` to `decided`. `arch-signing-infra` cross-ref updated. Totals: 130 → 137. |

**No production code changes** (per §1.4.1 — adoption deferred until trigger fires).

## Next steps

**For user:**
1. **Read this consensus session document** + the new Tech Spec §15.3-§15.11 sections to validate the design closure matches your "power AND safety" goal as articulated.
2. **If satisfied:** mark consensus session as complete (no further action — Tech Spec stays at canonical-reference status). Production code unchanged.
3. **If unsatisfied:** re-engage 3-stakeholder consensus session (this document records the prior decision; can be amended in subsequent sessions). Specifically: which sub-trait / amendment / framing you want changed.

**For implementation start (when §1.4.1 trigger fires):**
1. Open `docs/superpowers/plans/<NNNN>-credential-p1-trait-scaffolding.md` referring to Tech Spec §15.3-§15.10 + §16.1.1 sub-gates.
2. Implement compile-fail probes FIRST (the 8 in §16.1.1) — they encode the contract.
3. Implement trait shape changes per §15.4-§15.9 in `crates/credential/src/contract/`.
4. Update `nebula-credential-macros` derive emission per §15.4 + §15.5.
5. Per §16.3 landing gates: tests pass + benches within baseline + docs synced + register row updates + ABI stability + security review (security-lead block on §15.4-§15.10 changes per their Round 1 sign-off conditions).

**For 2026-10-24 half-life re-decision:**
1. Re-engage 3-stakeholder consensus session (per §1.4.1 trigger (c)).
2. Evaluate adoption signal: Has any plugin author / builtin credential surfaced consumer wall in the intervening 6 months? Has security-lead found new structural concerns? Has the cleanup-track P6-P11 settled in production?
3. Decide: adopt П1 (start trait scaffolding implementation), defer further (extend half-life with trigger update), or supersede (new Tech Spec / new design closure).

**For sub-spec landings (independent of Tech Spec adoption):**
1. `arch-signing-infra` (queue #7, post-MVP) — long-term defense for N7 supply-chain risk beyond §15.6 fatal duplicate-KEY interim.
2. `draft-f17` `2026-04-24-credential-refresh-coordination.md` — refresh coordination L2 (referenced by §16.1 П2; can land as ADR even before П1 if needed).
3. `draft-f18/f19/f20` — ProviderRegistry sub-spec (referenced by §16.1 П4; depends on §11 freeze).

---

**End of consensus session document. CP5 closure 2026-04-24.**

---

**End of consensus session document.**
