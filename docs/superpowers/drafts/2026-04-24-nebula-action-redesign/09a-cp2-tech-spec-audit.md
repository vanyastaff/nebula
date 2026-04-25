# 09a — CP2 Tech Spec audit (structural)

**Auditor:** spec-auditor (sub-agent)
**Date:** 2026-04-24
**Document audited:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` lines 722-1483 (CP2 §4–§8; CP2 open items + CHANGELOG + Handoffs)
**Scope:** structural integrity only — cross-section consistency within CP2, cross-CP refs to §0–§3, cross-doc citation accuracy (Strategy / credential Tech Spec / ADRs / spike NOTES / security 03c+08c), §6 co-decision routing, §6 must-have floor wording vs scope decision §3, §5.4.1 + §6.4 cross-crate amendment flagging discipline. Content critique is rust-senior + security-lead + dx-tester + tech-lead domain.
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Verdict

**PASS-WITH-NITS.** Zero 🔴 BLOCKERS, three 🟠 HIGH, six 🟡 MEDIUM, three ✅ GOOD. CP2 is structurally tight: open-items list is one-to-one with body markers (13 entries match), forward-references all marked CP3 / CP4 / cross-crate, status header `DRAFT CP2` correct, ADR / Strategy / credential Tech Spec / security 03c+08c citations resolve where checked. The 🟠 findings are reference-precision issues (false "matches existing cap" claim, misattributed "co-decision per Strategy §6.3", internal contradiction on "only one soft amendment surfaced"); the 🟡 findings are line-number drift and a code-claim that doesn't survive `grep`.

Iterate-yes. All findings are mechanical (one-line edits each); none invalidates the CP2 design direction.

**Top 3 issues:**

1. 🟠 §6.1 line 1062: "matches existing `webhook.rs:1378-1413` `check_json_depth` cap" — `check_json_depth` does NOT have a hard-coded cap; it accepts `max_depth: usize` parameter from caller. Webhook doc at `webhook.rs:334` recommends 64, not 128. Spec re-frames "matches existing cap" but no such cap exists at the cited line range.
2. 🟠 §6 line 1056 misattributes "co-decision tech-lead + security-lead **per Strategy §6.3 (line 386-394)**." Strategy §6.3 line 386-394 is the reviewer-matrix table (CP2a = security-lead + spec-auditor parallel; "co-decision" terminology in Strategy §6.8 is reserved for B'+ activation, architect + tech-lead). Real authority for §6 co-decision is Strategy §4.4 + §2.12 + 03c §1 VETO; cite is wrong section.
3. 🟠 §7.2 line 1298: "The §5.4.1 amendment к credential Tech Spec §16.1.1 probe #7 (qualified-syntax probe) is the **only soft-amendment** surfaced by this Tech Spec." Internal contradiction with §6.4.2 line 1243 which explicitly flags a **second** soft amendment (`engine_construct_with_probe` test-only constructor variant к credential Tech Spec §15.7). §15 open-items list at lines 1448 + 1453 enumerates BOTH amendments; §7.2's "only one" claim is false.

---

## §4-§8 cross-section consistency

### 🟢 PASS — Macro emission ↔ test harness shape alignment

§4.3 emits `SlotBinding` literal with `field_name: "slack"`, `slot_type: SlotType::Concrete { type_id: TypeId::of::<SlackToken>() }`, `resolve_fn: ::nebula_engine::resolve_as_bearer::<SlackToken> as ::nebula_action::ResolveFn`. §5.3 Probe 6 verifies wrong-Scheme `resolve_as_bearer::<BasicCred>` (`BasicCred::Scheme = BasicScheme`) fails E0277. **Coherent.** Probe 6 exercises exactly the §4.3 macro-selection invariant.

§4.6.1 introduces Probe 7 (`parameters = Type` no-`HasSchema` rejection); §5.3 table row 7 references §4.6.1 verbatim. ✓

### 🟢 PASS — §6 security floor ↔ §4 enforcement points

§6.1 (depth cap) cites apply sites at `stateless.rs:370` + `stateful.rs:561, 573` — these are the dispatch-path JSON deserialization sites that §4 macro emission flows into via `*Handler::execute` (§7.1 step 2). §6.2 (hard removal) cites `context.rs:635-668` — the body method that §4 macro emission **replaces** with macro-emitted `ctx.resolved_scheme(&self.<slot>)` per §6.2.2. Coherent: §6 enforcement points are exactly what §4 emission relies on.

### 🟢 PASS — §7.1 SlotBinding flow ↔ §3.1+§3.2 CP1 contract

§7.1 step 3: "For each `SlotBinding` in `A::credential_slots()` (per §3.1 + §4.3), the adapter invokes `(binding.resolve_fn)(&ctx.creds, &slot_key)`." Verified against CP1 §3.1 (line 580+) + §3.2 HRTB type alias (line 636-640). The 6-step dispatch flow at §7.1 mirrors CP1 §3.2 dispatch path 1-7. Engine-side wrap point is correctly cited as inherited from §3.2-1 open item per §7.1 step 3 inline note.

### 🟠 HIGH — §7.2 line 1298 contradicts §6.4.2 line 1243 on amendment count

§7.2 line 1298:
> "The §5.4.1 amendment к credential Tech Spec §16.1.1 probe #7 (qualified-syntax probe) is the **only soft-amendment** surfaced by this Tech Spec."

§6.4.2 line 1243:
> "**Cross-crate amendment.** This is a **soft amendment к credential Tech Spec §15.7** — adds the `engine_construct_with_probe` test-only constructor variant. Same precedent as §5.4.1 — flagged here, NOT enacted by this Tech Spec."

Both amendments are listed in §15 open items at lines 1448 + 1453. §7.2 silently overlooks the §6.4 amendment.

Impact: cross-cascade coordinator at CP4 reading §7.2 sees "only one amendment to surface" and may miss the §15.7 `engine_construct_with_probe` amendment when coordinating with credential Tech Spec author.

Suggested fix: §7.2 line 1298 → "The §5.4.1 amendment к credential Tech Spec §16.1.1 probe #7 (qualified-syntax probe) **AND the §6.4.2 amendment к §15.7 (`engine_construct_with_probe` test-only constructor variant)** are the two soft-amendments surfaced by this Tech Spec; both flagged, not enacted." Architect to redraft.

### 🟡 MEDIUM — §6.2.5 cites `credential_typed` line range "563-632" but method actually starts at 603

§6.2.5 line 1145: "`credential_typed<S>(key: &str)` (`crates/action/src/context.rs:563-632` from earlier read; verified at this commit)."

`grep "fn credential_typed"` returns line 603. `credential_typed` body ends at line 631. Cited range 563-632 includes the trait `CredentialContextExt` block start at line 573 + end of method at 631 — but begins 40 lines BEFORE the method. Loose pinning.

Suggested fix: `crates/action/src/context.rs:603-631`. Architect to re-pin.

### 🟡 MEDIUM — §6.2.1 cites `credential<S>()` line range "635-668" but method ends at 669

Confirmed by `grep "fn credential\b"`: method starts at line 635, body closes at line 669. Spec citation `635-668` is off by 1.

Suggested fix: `crates/action/src/context.rs:635-669` (or `635-668` if the closing `}` line is intentionally omitted, but the convention elsewhere in this Tech Spec includes the closing `}` line — see §6.2.5 cite of `563-632` which includes its closing). Architect: pick convention and apply uniformly.

### 🟡 MEDIUM — §6.1.3 line 1097 cites `error.rs:58-71` for `#[non_exhaustive]` enum; actual is `error.rs:57-71`

`grep ValidationReason` confirms: `#[non_exhaustive]` is at line 57 (not 58); enum body closes at line 71. Off by 1 line on enum-start citation.

Suggested fix: `crates/action/src/error.rs:57-71`.

### 🟡 MEDIUM — §4.6.1 line 884 line-citation drift on `for_stateless::<A>()` builder sites

§4.6.1 line 893: "`ActionMetadata::for_stateless::<A>()` at `crates/action/src/metadata.rs:176, 191, 206, 221`."

`grep "pub fn for_stateless"` + parallels confirms actual lines: 167 (for_stateless), 182 (for_stateful), 197 (for_paginated), 212 (for_batch). Cited range is **off by 9 lines** — likely from prior commit before code shifts.

Suggested fix: `crates/action/src/metadata.rs:167, 182, 197, 212`.

---

## Cross-CP / cross-doc reference resolution

### 🟢 PASS — Forward references all marked CP3 / CP4 / cross-crate

Sample-checked CP2 → CP3 / CP4:
- "CP3 §9 picks" / "CP3 §9 wires" / "CP3 §9 confirms" / "CP3 §9 details runbook" — all mechanical-deferral framing, none dangling.
- "CP4 cross-section pass surfaces" cited at §5.4.1 line 1038 + §6.4.2 line 1243 (cross-crate amendments) — both flagged not enacted, deferral home identified.
- "CP3 §7" cited at §4.1.2 line 760 (resource-slot emission shape), §7.4 line 1334 (engine scheduler-integration), §8.1.2 line 1357 (PollAction trait shape lock) — all marked CP3 scope.

### 🟢 PASS — §5.3 spike NOTES line/§ citations

Verified `spike NOTES §1.2 / §1.3 / §1.4 / §1.5 / §3 finding #1 / §3 finding #2 / §4 question 5` against actual NOTES headers — all resolve. §5.4 + §6.4 cite "spike finding #1" and "spike finding #2" at NOTES §3 lines 177 + 189 respectively. ✓

### 🟢 PASS — Strategy §2.12 + §4.4 four-item floor verbatim mapped to §6.1-§6.4

Strategy §4.4 (line 245-254) lists 4 invariants: (1) JSON depth bomb; (2) S-C2 hard removal; (3) Display sanitization; (4) cancellation-zeroize test. CP2 §6.1 / §6.2 / §6.3 / §6.4 enumerate the same four items in the same order, citing Strategy §2.12 item N + 03c §N item N for each. ✓

### 🟠 HIGH — §6 line 1056 misattributes "co-decision" authority to Strategy §6.3 (line 386-394)

§6 line 1056:
> "This section is **co-decision tech-lead + security-lead** per Strategy §6.3 (line 386-394)."

Strategy §6.3 line 386-394 is the **Tech Spec checkpoint roadmap reviewer-matrix table** (verified). It does NOT use the term "co-decision." The reviewer matrix at §6.3 splits CP2 into CP2a (security-lead + spec-auditor parallel) + CP2b (rust-senior + dx-tester + spec-auditor parallel). Neither row uses "co-decision tech-lead + security-lead" framing.

The "co-decision" term in Strategy is reserved for §6.8 B'+ contingency activation (architect + tech-lead — `grep "co-decision" Strategy.md` returns 5 matches, all at §6.8 / §4.4 / §6.5 referring to the architect + tech-lead pair, NOT tech-lead + security-lead).

Real authority chain for §6 floor:
- Strategy §4.4 (line 245-254) = "non-negotiable invariant; Tech Spec must cite each item as not-deferrable"
- Strategy §2.12 (line 99-106) = scope decision §3 reference
- 03c §1 = security-lead VETO authority on shim-form drift (line 78 = "VETO if conditions aren't met")

The substantive routing — tech-lead + security-lead jointly review §6 — is **valid** but its provenance is NOT Strategy §6.3 line 386-394. The actual basis is §1 G3 freeze invariants + 03c §1 VETO + §0.2 item 3 freeze-invalidation.

Impact: a reviewer who clicks "Strategy §6.3 line 386-394" expecting to find the co-decision rule lands on the reviewer-matrix table that doesn't mention security-lead in CP2b's row. Authority chain on a security-VETO surface should not be ambiguous.

Suggested fix: §6 line 1056 → "This section is **co-decision tech-lead + security-lead** per Strategy §4.4 (security must-have floor invariant) + 03c §1 (VETO authority) + §1 G3 freeze invariants. Strategy §6.3 reviewer matrix splits CP2a/CP2b (security-lead reviews CP2a, rust-senior + dx-tester review CP2b — Tech Spec collapses CP2a+CP2b into CP2 per §0.1 status table)." Architect to redraft.

Bonus: this also surfaces a sub-issue — Strategy §6.3 has CP1/CP2a/CP2b/CP3/CP4; Tech Spec §0.1 has CP1/CP2/CP3/CP4. The collapse is intentional (per CHANGELOG line 1468 status header note) but Tech Spec doesn't anywhere call out the divergence from Strategy's matrix. Optional CP3 § note worth.

### 🟠 HIGH — §6.1 line 1062 false "matches existing cap" claim

§6.1 line 1062:
> "Depth cap **128** at every adapter JSON boundary (matches existing `webhook.rs:1378-1413` `check_json_depth` cap)."

`check_json_depth` does NOT have a hard-coded cap. Verified at `crates/action/src/webhook.rs:1378`:

```rust
fn check_json_depth(bytes: &[u8], max_depth: usize) -> Result<(), serde_json::Error> {
    // depth.saturating_add(1); if depth > max_depth { return Err(...) }
}
```

`max_depth` is a parameter passed by the caller. The function has no fixed cap. The webhook recommended depth at `webhook.rs:334` doc comment says **"Recommended `max_depth`: 64"** — not 128.

The cap "128" is the value chosen by **Strategy §2.12 item 1 + scope decision §3 item 1** (mandated by "v2 design spec post-conference amendment B3" per Strategy §2.12 line 101). It does NOT match an "existing cap" — there is no such cap.

Impact: implementer reading "matches existing cap" believes 128 is grounded in current code. Re-verification fails (`grep '128' webhook.rs` — only line 100 in a doc comment about prior cap; not the current primitive). Subtle but the spec's framing is factually wrong.

Suggested fix: §6.1 line 1062 → "Depth cap **128** at every adapter JSON boundary (per Strategy §2.12 item 1 + v2 design spec post-conference amendment B3; the existing `webhook.rs:1378-1413` `check_json_depth` primitive accepts `max_depth: usize` parameter and is reused at the new cap value 128)." Architect to redraft.

Note: the recommendation at `webhook.rs:334` of 64 is for **webhook body** parsing; 128 is the chosen cap for **adapter input/state** parsing. Different boundaries, different caps. Spec doesn't acknowledge the asymmetry.

### 🟡 MEDIUM — §6.1.1 line 1072 webhook deserialization claim doesn't survive `grep`

§6.1.1 line 1072:
> "Webhook body deserialization at `crates/api/src/services/webhook/transport.rs` already pre-bounds via `body_json_bounded` (uses `check_json_depth` per `crates/action/src/webhook.rs:1378-1413`); CP2 §6.1 verifies this site is unchanged."

Verification:
- `grep "body_json_bounded\|check_json_depth\|from_slice\|json::from" crates/api/src/services/webhook/transport.rs` returns **no matches**.
- `transport.rs` receives axum `Bytes` (verified at line 27 + lines 33-37 imports from `nebula_action::WebhookRequest`); the body parsing is delegated downstream to `WebhookRequest`-handling code.
- `body_json_bounded` IS defined at `crates/action/src/webhook.rs:343` (a method on `WebhookRequest`) but its callers in `crates/api/src/services/webhook/` use a different path.

The claim "already pre-bounds via `body_json_bounded`" is unsupported at the cited file. The actual bounding likely happens at the `WebhookRequest` consumer (either action's webhook trigger handlers or the dispatcher inside transport.rs's `handler.handle_event` call) — but **not at `transport.rs` directly**. CP2 §6.1's "verified unchanged" promise cannot be discharged at the cited file.

Impact: security-lead reviewing CP2 §6.1 may sign off on "webhook bounding already in place" without confirming where; reviewer trace from `crates/api/src/services/webhook/transport.rs` to actual depth-cap enforcement is broken.

Suggested fix: architect to trace the actual depth-cap site in webhook flow (likely inside `WebhookRequest` consumer) and re-pin the citation. OR add an open item: "§6.1-2 — verify webhook body depth-cap enforcement site at CP3 §9; CP2 cites `transport.rs` but body parsing happens downstream." 03c §2 item 1 already requires the verification ("verify `body_json_bounded` is used, not raw `from_slice`") so it's a discharge-promise rather than new work.

### 🟢 PASS — ADR-0035 / ADR-0036 / ADR-0037 / ADR-0038 cross-citations

§4 cites ADR-0036 §Decision item 1 (line 730), ADR-0036 §Decision item 3 + §Negative item 2 (line 824, 830), ADR-0037 §1 / §2 / §3 / §4 / §5 (lines 724, 824, 1004, 948, 860), ADR-0035 §1 / §3 / §4.3 (lines 754, 856, 753), ADR-0038 §Implementation notes (line 1462). ADR-0035 amendment "2026-04-24-B" still cited at this Tech Spec via CP1 inheritance — flagged but not re-verified (CP1 audit medium 🟡 still applies; not new in CP2).

---

## §6 co-decision item bookkeeping

CP2 §6 surfaces 4 explicit co-decision items + 1 forward-track:

| Item | Tech Spec section | Routed to | Cited authority |
|---|---|---|---|
| §6.1.2 — depth-cap mechanism (pre-scan vs `serde_stacker`) | §6.1.2 line 1076 | tech-lead + rust-senior (security-neutral; rust-senior call) | 03c §2 item 1 |
| §6.2 — hard-removal mechanism (Option a delete) | §6.2.2 line 1111 | tech-lead + security-lead VETO retained | 03c §1 + 08c §Gap 1 Option (a) |
| §6.3.2 — `redacted_display()` crate location (`nebula-redact` NEW vs `nebula-log` co-resident) | §6.3.2 line 1162 | tech-lead + security-lead | 08c §Gap 3 |
| §6.4.2 — `ZeroizeProbe` instrumentation (per-test vs `serial_test::serial`) | §6.4.2 line 1218 | tech-lead + security-lead | 08c §Gap 4 |
| §6.5 — cross-tenant Terminate boundary | §6.5 line 1254 | CP3 §9 (tech-lead + security-lead jointly per 08c §Gap 5) | 08c §Gap 5 |

### 🟢 PASS — Handoffs section (line 1476-1483) routes each co-decision item

Verified each item appears in the Handoffs section with explicit reviewer attribution:
- §6.1.2 → tech-lead handoff (1) at line 1478 + rust-senior handoff (3) at line 1480 ✓
- §6.2 → tech-lead (2) + security-lead (2) ✓
- §6.3.2 → tech-lead (3) + security-lead (3) + rust-senior (4) ✓
- §6.4.2 → tech-lead (4) + security-lead (4) ✓
- §6.5 → security-lead (5) ✓

Each item is in **at least two** reviewer queues (one for the decision-maker, one for VETO-holding security-lead). Discipline is correct.

### 🟢 PASS — §6 floor wording matches scope decision §3 / Strategy §2.12 verbatim-or-near-verbatim

| Item | Scope decision §3 (line 81-84) | Strategy §2.12 (line 101-104) | CP2 §6 |
|---|---|---|---|
| 1 | "JSON depth bomb — depth cap (128) at every adapter JSON boundary" | "depth cap (128) at every adapter JSON boundary" | §6.1 line 1062 "Depth cap 128 at every adapter JSON boundary" ✓ |
| 2 | "replace type-name heuristic with explicit keyed dispatch...CR3 fix MUST be hard removal, not `#[deprecated]` shim" | "Hard removal, not `#[deprecated]` shim" | §6.2 line 1105 "Hard removal, NOT `#[deprecated]`" ✓ |
| 3 | "ActionError Display sanitization — route through `redacted_display()` helper" | "route through `redacted_display()` helper in `tracing::error!` call sites" | §6.3 line 1149 "ActionError Display sanitization via `redacted_display()` helper" ✓ |
| 4 | "Cancellation-zeroize test — closes S-C5; pure test addition" | "closes S-C5; pure test addition, no architectural cost" | §6.4 line 1204 "Cancellation-zeroize test (closes S-C5)" ✓ |

All four items are verbatim-or-near-verbatim — no softening, no wording drift toward "deprecated" / "deferred" / "best-effort". `feedback_no_shims.md` posture preserved.

### 🟢 PASS — VETO trigger language verbatim from 03c §1

§6.2.3 line 1124-1130 quotes 03c §1.B + 03c §4 handoff verbatim. Cross-checked against `03c-security-lead-veto-check.md` line 63 + line 78 — matches word-for-word.

---

## §6 must-have floor wording check

(Already covered above under "Cross-CP / cross-doc reference resolution" / "PASS — §6 floor wording matches scope decision §3.")

No softening, no shim regression, all four items in-scope with implementation forms locked. ✅

---

## §5.4 + §6.4 cross-crate amendment flag check

### 🟢 PASS — §5.4.1 flag-not-enact discipline

§5.4.1 line 1029-1041 explicitly states:
> "**This Tech Spec FLAGS the amendment** but does NOT enact it. Per ADR-0035 amended-in-place precedent, cross-crate amendments to credential Tech Spec are coordinated via the credential Tech Spec author (architect)."

§5.4.1 lists target (credential Tech Spec §16.1.1 probe #7), current shape (line 3756 verbatim quote), proposed amendment (qualified-syntax form replacing naive `guard.clone()`), enactment path ("CP4 cross-section pass surfaces; credential Tech Spec author lands inline edit"). Discipline matches ADR-0035 amended-in-place precedent (verified — amendments land via inline `*Amended by ADR-NNNN, YYYY-MM-DD*` prefix, not via new ADR).

### 🟢 PASS — §6.4 cross-crate amendment flag-not-enact discipline

§6.4.2 line 1243:
> "**Cross-crate amendment.** This is a **soft amendment к credential Tech Spec §15.7** — adds the `engine_construct_with_probe` test-only constructor variant. Same precedent as §5.4.1 — flagged here, NOT enacted by this Tech Spec. CP4 cross-section pass surfaces; credential Tech Spec author lands the inline edit."

Same flag-not-enact discipline. Both amendments listed in §15 open items section (line 1448 + 1453) for tracking.

### 🟠 HIGH — §7.2 line 1298 false claim "only one soft-amendment surfaced" (already covered)

(See top section.) §7.2 says only §5.4.1 amendment exists; §6.4.2 + §15 open items list contradict. Architect needs to update §7.2 to reflect both.

---

## Bookkeeping

### 🟢 PASS — §15 open items list (CP2) ↔ body markers one-to-one

13 list entries at lines 1444-1456 vs body markers:

| List entry | Body marker location | Status |
|---|---|---|
| §4.4-1 | line 856 | ✓ |
| §4.7-1 | line 928 | ✓ |
| §5.1-1 | line 948 | ✓ |
| §5.3-1 | line 1000 | ✓ |
| §5.4.1 (cross-crate) | lines 1028-1040 | ✓ (flagged-not-enacted) |
| §6.1.2 (byte-pre-scan vs Value-walking) | line 1091 | ✓ (untagged inline note; list cites "§6.1.2" generally) |
| §6.2-1 | line 1147 | ✓ |
| §6.3-1 | line 1194 | ✓ |
| §6.4-1 | line 1252 | ✓ |
| §6.4 cross-crate amendment | line 1243 | ✓ (flagged-not-enacted) |
| §6.5 | line 1262 | ✓ (body says "Open item §6.5-1 tracks") |
| §7.3-1 | line 1316 | ✓ |
| §7.1 step 3 (ResolvedSlot wrap point) | line 1276 | ✓ (inherited from §3.2-1 CP1 open item) |

13 ↔ 13. **Complete bidirectional match.** Open-items bookkeeping is clean.

### 🟢 PASS — Forward-track for CP3 (5 items at line 1458-1463)

5 forward-track items each name a CP3 sub-section (§7 or §9), an owner-ish role, and a payload. None dangling.

### 🟡 MEDIUM — §6.5 body says "Open item §6.5-1 tracks" but `§6.5-1` marker is absent from §15 list

§6.5 body line 1262: "CP2 §6.5 commits to CP3 §9 lock; this Tech Spec section flags the requirement, the engine-side enforcement form is CP3 scope. Open item §6.5-1 tracks."

§15 list line 1454 calls the same item "**§6.5** — Cross-tenant Terminate boundary..." — drops the `-1` suffix. Either the body or the list is right; the suffix should be consistent (every other item uses §X.Y-N format).

Suggested fix: pick `§6.5-1` (matches body phrasing) and add to list, OR drop `-1` from body to match list. Architect call.

### 🟡 MEDIUM — Status header table line 1468 says "DRAFT CP1 (iterated 2026-04-24)" → "DRAFT CP2"

CHANGELOG entry line 1468 references the prior status header text; current §0.1 status table at lines 28-33 shows:

```
| **DRAFT CP1** | §0–§3 | ... | locked CP1 |
| **DRAFT CP2** (this revision) | §4–§8 | ... | active |
```

The transition was correctly applied (CP1 row marked `locked CP1`; CP2 row marked `active`). The CHANGELOG entry is bookkeeping noise — confirms intent matches the actual change. No fix; flagged only because §0.2 freeze policy says "if a cited line range moves due to upstream document edits, this Tech Spec must be re-pinned" — this DID move (the table revision), and the CHANGELOG records it. Minor positive observation.

---

## Terminology / glossary

### 🟡 MEDIUM — `nebula-redact` crate is referenced but not in `docs/GLOSSARY.md`

`grep "nebula-redact\|redact" docs/GLOSSARY.md` returns no matches for `nebula-redact` (only `SecretToken` / `SecretValue` for redaction-adjacent terms). §6.3.2 commits CP2 to creating a NEW `nebula-redact` crate; per CP1 audit 🟡 carry-forward + Strategy §5.1.2, glossary entries land at CP4 §14 cross-section pass. Open item §6.3-1 names "full redaction rule set CP3 §9 design scope" but does NOT name glossary as a follow-up.

Suggested fix: add to CP4 carry-forward: "glossary entries needed for `nebula-redact` + `redacted_display()` + `ZeroizeProbe`." Architect to add at CP4 §14 audit.

### 🟢 PASS — `co-decision` term used consistently within §6

§6 uses "co-decision" (with hyphen) at line 1054, 1056, 1058, 1162. Strategy uses "co-decision" at §6.5, §6.8 (B'+ activation), §4.3.x. Same term, different referents, but the Tech Spec uses it for the security-lead + tech-lead pair in §6 — different from Strategy's architect + tech-lead pair. Term reuse with different referent is a minor reader friction but not drift.

---

## Coverage summary

- Structural: 1 finding (§7.2 line 1298 internal contradiction on amendment count — 🟠)
- Cross-section consistency: 0 findings
- External verification: 5 findings (§6.1 line 1062 false "matches existing cap" claim — 🟠; §6 line 1056 misattributed §6.3 cite — 🟠; §6.1.1 line 1072 transport.rs claim doesn't survive grep — 🟡; §6.2.5 line 1145 line range loose — 🟡; §6.2.1 line 1109 + §6.1.3 line 1097 + §4.6.1 line 893 line-citation drift — 🟡 ×3)
- Bookkeeping: 1 finding (§6.5 / §6.5-1 suffix inconsistency — 🟡)
- Terminology: 1 finding (`nebula-redact` glossary missing — 🟡)
- Definition-of-done (§17): out of CP2 scope (CP4 spec-auditor full audit per Strategy §6.3 line 392)

Total: 0 🔴 + 3 🟠 + 6 🟡 + 3 ✅

---

## Summary for orchestrator

**Verdict: PASS-WITH-NITS.** CP2 is structurally tight. Open-items list is one-to-one with body markers (13 ↔ 13). Forward-references all marked CP3 / CP4 / cross-crate. Status header `DRAFT CP2` correct. Cross-doc citations to Strategy §2.12 / §4.4 / scope decision §3 / 03c §1 + §2 / 08c §Gap 1-5 / spike NOTES §1.x + §3 + §4 / credential Tech Spec §15.7 line 3394-3516 all resolve. §6 must-have floor wording is verbatim-or-near-verbatim against scope decision §3 + Strategy §2.12. §5.4.1 + §6.4.2 cross-crate amendments correctly flagged-not-enacted per ADR-0035 precedent. Co-decision routing through Handoffs section is disciplined (each item in ≥2 reviewer queues).

**Iterate-yes.** All 9 findings are mechanical (one-line edits each); no design rework.

**Top 3 must-fix before CP2 ratify:**
1. §7.2 line 1298 — change "only soft-amendment" to acknowledge BOTH §5.4.1 (§16.1.1 probe #7) AND §6.4.2 (§15.7 `engine_construct_with_probe`) amendments.
2. §6 line 1056 — replace "per Strategy §6.3 (line 386-394)" with "per Strategy §4.4 (security must-have floor invariant) + 03c §1 (VETO authority) + §1 G3 freeze invariants"; optionally note the CP2a+CP2b → CP2 collapse.
3. §6.1 line 1062 — replace "matches existing webhook.rs:1378-1413 check_json_depth cap" with "per Strategy §2.12 item 1 + v2 design spec post-conference amendment B3; the existing check_json_depth primitive accepts max_depth: usize parameter and is reused at the new cap value 128."

**Handoff: architect** for all 🟠 / 🟡 findings (none require tech-lead decision; all are content corrections). Architect to redraft §7.2 line 1298, §6 line 1056, §6.1 line 1062, line-citation re-pin sweep on §6.1.1 / §6.1.3 / §6.2.1 / §6.2.5 / §4.6.1, §6.5 suffix consistency.

**Handoff: tech-lead** advisory only — §6 co-decision routing (4 items + 1 forward-track) is structurally complete; no decisions defer because routing is unclear.

**Handoff: security-lead** advisory only — VETO trigger language at §6.2.3 cited verbatim from 03c §1.B; §6.4.2 per-test ZeroizeProbe choice closes 08c §Gap 4; §6.5 cross-tenant boundary forward-tracked to CP3 §9 per 08c §Gap 5. No security-substantive drift surfaced.
