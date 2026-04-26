# 08a — CP1 Tech Spec audit (structural)

**Auditor:** spec-auditor (sub-agent)
**Date:** 2026-04-24
**Document audited:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` lines 1-572 (CP1 § 0-§3)
**Scope:** structural integrity only (cross-section consistency, cross-doc reference resolution, signature compile-check vs `final_shape_v2.rs`, forward-ref bookkeeping, open items, glossary coherence). Content critique is rust-senior + security-lead + dx-tester domain.
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Verdict

**REVISE.** Three 🔴 BLOCKERS, three 🟠 HIGH, four 🟡 MEDIUM. The Tech Spec freezes signatures (§ 2 is "the signature-locking section") that diverge from `final_shape_v2.rs` and from the credential Tech Spec on three load-bearing shapes. Not freeze-ready until § 2.1, § 2.2.x, § 3.1 are reconciled with the spike artefact AND with credential Tech Spec § 3.4 / § 9.4.

Iterate-yes. Most findings are mechanical (one line each); none invalidates the CP1 design direction.

**Top 3 issues:**

1. 🔴 `ActionSlots::credential_slots()` signature is a self-contradiction across the doc set (§ 2.1 has no `&self`; final_shape_v2.rs L278 has `&self`; credential Tech Spec § 3.4 L851 has `&self` AND drops `'static`).
2. 🔴 `SlotType::ServiceCapability` variant payload silently drops `service: ServiceKey` from the credential Tech Spec authoritative shape (§ 9.4 L2452 enumerates 3 variants; § 3.1 enumerates 2 with degraded payloads).
3. 🔴 `Input: HasSchema` bound (§ 2.2.1/.2/.4) is not in `final_shape_v2.rs` (lines 210/221/239). Tech Spec § 2.0 line 101 claims signatures are "compile-checked against final_shape_v2.rs"; this claim is false for `Input`.

---

## Cross-section consistency

### 🔴 BLOCKER — § 2.1 / § 2.6 supertrait chain vs § 2.6 sealed-trait pattern

§ 2.1 line 112 says: `pub trait Action: ActionSlots + Send + Sync + 'static { fn metadata(&self) -> &ActionMetadata; }`. § 2.6 line 318 says `impl<T: StatelessAction> sealed_dx::ControlActionSealed for T {}` — note this seals `ControlAction` against ALL `StatelessAction` impls.

Inconsistency: § 2.1 `Action` requires `ActionSlots`, but § 2.6's blanket impl `impl<T: StatelessAction> sealed_dx::ControlActionSealed for T {}` does NOT require `T: ActionSlots`. A `StatelessAction` impl without `ActionSlots` impl will gain `ControlActionSealed` but cannot be `Action`. Either:

- The blanket should be `impl<T: StatelessAction + ActionSlots> ...` (mirroring final_shape_v2.rs L282), OR
- `Action` should drop the `ActionSlots` supertrait (unlikely — § 2.1 line 117 calls this load-bearing).

`final_shape_v2.rs:282`: `impl<T: StatelessAction + ActionSlots> Action for T {}` — this is the spike's chosen blanket and constrains `Action` membership exactly. The Tech Spec § 2.6 blanket erases the `ActionSlots` constraint silently.

Impact: implementer cannot tell whether the seal is on `StatelessAction` alone or on `StatelessAction + ActionSlots`. This is a **load-bearing seal-discipline question** because § 2.6 line 282 ("adding a sealed DX trait does NOT require canon revision") depends on the seal being structural.

Suggested fix: align § 2.6 blanket impl with `final_shape_v2.rs:282` shape — `impl<T: StatelessAction + ActionSlots> sealed_dx::ControlActionSealed for T {}`. Architect to redraft.

### 🔴 BLOCKER — `ActionSlots::credential_slots()` signature contradicts the spike + credential Tech Spec

Three doc/code claims, three different signatures:

| Source | Signature |
|---|---|
| Tech Spec § 2.1 line 117 (prose): `credential_slots() -> &'static [SlotBinding]` | no `&self` |
| Tech Spec § 3.1 line 422+446 (prose): `ActionSlots::credential_slots()` static slice | implicit no `&self` |
| ADR-0039 § 1 line 49: `fn credential_slots() -> &'static [SlotBinding]` | no `&self` |
| `final_shape_v2.rs:278`: `fn credential_slots(&self) -> &'static [SlotBinding]` | **HAS `&self`** |
| Credential Tech Spec § 3.4 line 851: `fn credential_slots(&self) -> &[SlotBinding]` | **HAS `&self`, drops `'static`** |

Evidence:
- `Grep "fn credential_slots"` returns these four results across the doc + spike (verified above).
- Tech Spec § 0 line 44 invariant 4 says: "Spike-shape divergence. `final_shape_v2.rs` (the shapes Tech Spec § 2 freezes verbatim) is re-validated and a different shape is required." → freezes the spike. The spike has `&self`.

Impact: the macro emission shape (ADR-0039 § 1 example) generates code that won't satisfy the trait if the trait says `&self`. Implementer hits a contradiction between Tech Spec § 2.1 and ADR-0039 example on day 1.

Suggested fix: pick `&self` (matches spike + credential Tech Spec). Update § 2.1 line 117, § 3.1 line 422+446, AND ADR-0039 § 1 example. Also reconcile `&[SlotBinding]` vs `&'static [SlotBinding]` with credential Tech Spec § 3.4.

### 🔴 BLOCKER — `SlotType::ServiceCapability` variant payload drops `service` field

Tech Spec § 3.1 line 437:
```rust
ServiceCapability { capability: Capability },// Pattern 2/3: dyn projection
```

Credential Tech Spec § 9.4 line 2452 (load-bearing — defines the matching pipeline):
> three `SlotType` variants — `Concrete { type_id }` (Pattern 1), `ServiceCapability { capability, service }` (Pattern 2), or `CapabilityOnly { capability }` (Pattern 3).

Credential Tech Spec § 3.4 lines 855-857 (verbatim authoritative shape):
```rust
slot_type: SlotType::ServiceCapability {
    capability: Capability::Bearer,
    service: ServiceKey::Bitbucket,
},
```

Three discrepancies:
1. **`service: ServiceKey` field is silently dropped** from the variant. The matching pipeline at credential Tech Spec § 9.4 line 2469 USES `cred.metadata().service_key == Some(*service)` — without `service` the Pattern 2 dispatch cannot match.
2. **Pattern 1 variant `Concrete { type_id }` is missing entirely.** Tech Spec § 3.1 has only `DirectType` (no payload) and `ServiceCapability { capability }`. Final_shape_v2.rs line 64-67 also has `DirectType` (no payload), so this matches the spike — but the spike is a placeholder; the credential Tech Spec is authoritative for the runtime registry pipeline.
3. **Pattern 3 variant `CapabilityOnly { capability }` is missing.**

Tech Spec § 3.1 line 422 cites credential Tech Spec § 3.4 line 851-863 + spike final_shape_v2.rs:43-55 as the source. Both sources disagree with each other AND with what § 3.1 emitted.

Impact: § 9 codemod (CP3) will emit the wrong variant payloads; engine-side `iter_compatible` (credential Tech Spec § 9.4) cannot match Pattern 2 slots. § 3.1 freezes a degraded shape into "implementation-normative" Tech Spec.

Suggested fix: align § 3.1 enum verbatim with credential Tech Spec § 9.4 line 2452 (3 variants with full payloads). The spike's degraded shape was a stand-in (per spike NOTES § 4 question 5: "ResolvedSlot enum vs SchemeGuard direct return"); credential Tech Spec is canonical.

### 🟠 HIGH — `Input: HasSchema` bound divergence — § 2.0 compile-check claim is false

Tech Spec § 2.0 line 101: "Each shape below is freeze-grade Rust, **compile-checked against [`final_shape_v2.rs`](...)**" (emphasis mine).

§ 2.2.1 line 127: `type Input: HasSchema + Send + 'static;`
§ 2.2.2 line 145: `type Input: HasSchema + Send + 'static;`
§ 2.2.4 line 195: `type Input: HasSchema + Send + 'static;`

`final_shape_v2.rs:210`: `type Input: Send + 'static;` (no `HasSchema`)
`final_shape_v2.rs:221`: `type Input: Send + 'static;` (no `HasSchema`)
`final_shape_v2.rs:239`: `type Input: Send + 'static;` (no `HasSchema`)

Tech Spec § 2.2.1 line 139 cites this as "documented per ADR-0039 § Context (Goal G2 — closes CR9 undocumented bound)." ADR-0039 itself does not have a `Context` section using this header verbatim, but CR9 is real (pain enum line 112: `Input: HasSchema bound undocumented`).

Two acceptable resolutions:
1. Tech Spec correctly adds `HasSchema` bound (matches Goal G2 / CR9 closure intent), and § 2.0 line 101 needs to say "compile-checked against the spike at final_shape_v2.rs WITH the addition of `HasSchema` per CR9 documentation requirement." Right now the claim is unconditional.
2. Drop `HasSchema` to match the spike. (Unlikely — defeats CR9 closure.)

Impact: § 2.0's compile-check claim is the document's freeze warrant. False compile-check claim = freeze cannot proceed at face value.

Suggested fix: pick (1). Add language to § 2.0 paragraph noting that `Input: HasSchema` is a deliberate addition over the spike, with rationale "closes CR9 (pain enum line 112)." OR add to § 0.2 a fifth invariant covering deliberate-divergence rationale.

### 🟠 HIGH — § 2.7.1 misattributes "Phase 0 finding S3"

§ 2.7.1 line 334:
> **Phase 0 finding S3** ([`02-pain-enumeration.md`](...) § 4 row "S3"): `crates/action/src/result.rs:217` documents `Terminate` as "Phase 3 of the ControlAction plan and is not yet wired."

Verifications:
- `02-pain-enumeration.md` § 4 has TWO subsections — 🔴 CRITICAL (rows CR1–CR11) and 🟠 MAJOR (categorized list). **No row labeled "S3" exists** under § 4. Grep `S3 |row \"S3\"|^S3` returns one match: line 120 `ActionResult::Terminate not gated despite "Phase 3 not wired" (Phase 0 S3)` — and that line is inside the 🟠 MAJOR list as a parenthetical, not a row identifier. The "S3" prefix is a Phase 0 (`01-current-state.md` etc.) carry-over.
- Source-of-truth at `crates/action/src/result.rs:217`: the file does have the verbatim text "Phase 3 of the ControlAction plan and is **not yet wired**" (verified). The file claim is real.
- The "row 'S3'" framing is fabricated. There is no row.

Impact: implementer/reviewer chasing the citation hits dead end. § 2.7.1 is the load-bearing CP1 decision — its evidence chain must cite a real row, not an imagined one.

Suggested fix: replace "Phase 0 finding S3 (§ 4 row 'S3')" with "pain enumeration § 4 🟠 architectural-coherence list, line 120: `ActionResult::Terminate not gated despite 'Phase 3 not wired' (Phase 0 S3)`." Phase 0 attribution is fine — the parenthetical `(Phase 0 S3)` traces to the original Phase 0 audit in `01-current-state.md` / `01a-code-audit.md`. Architect to verify by running `grep "S3 " 01a-code-audit.md` and pinning the actual originating section.

### 🟠 HIGH — § 3.3 `resolve_as_bearer` signature drifts from credential Tech Spec § 3.4 step 3

Tech Spec § 3.3 lines 486-500:
```rust
pub fn resolve_as_bearer<C>(
    ctx: &CredentialContext<'_>,
    key: &SlotKey,
) -> BoxFuture<'_, Result<ResolvedSlot, ResolveError>>
where
    C: Credential<Scheme = BearerScheme>,
{
    Box::pin(async move {
        let cred: &C = ctx.registry.resolve::<C>(&key.credential_key)
            .ok_or(ResolveError::NotFound { key: key.credential_key.clone() })?;
        ...
    })
}
```

Credential Tech Spec § 3.4 line 878-890 (cited as authoritative):
```rust
fn resolve_as_bearer<C>(
    ctx: &CredentialContext<'_>,
    key: &str,                                           // <-- &str, not &SlotKey
) -> Result<BearerScheme, ResolveError>                  // <-- not BoxFuture
where C: Credential<Scheme = BearerScheme>,
{
    let cred: &C = ctx.registry.resolve::<C>(key)        // <-- key directly, not key.credential_key
        .ok_or(ResolveError::NotFound { key: key.into() })?;
    let state: &C::State = ctx.load_state::<C>(key)?;    // <-- not async
    let scheme: BearerScheme = C::project(state);
    Ok(scheme)
}
```

Three drift points:
1. **`key: &SlotKey` vs `key: &str`** — § 3.3 follows the spike (final_shape_v2.rs § 2: `SlotKey { credential_key: String, field_name: &'static str }`). Credential Tech Spec uses `&str` directly. Tech Spec § 3.3 needs to either cite this as a deliberate divergence or align.
2. **Return type `BoxFuture<'_, Result<ResolvedSlot, ResolveError>>` vs `Result<BearerScheme, ResolveError>`** — credential Tech Spec resolves to `BearerScheme` directly (sync); Tech Spec § 3.3 returns `BoxFuture<ResolvedSlot>`. This is open item § 3.2-1 ("ResolvedSlot wrap point — engine-side wrapper vs inside `resolve_fn`") — Tech Spec § 3.2 line 474 acknowledges the ambiguity but § 3.3 commits to the spike interpretation without flagging the divergence inline.
3. **`load_state` async (line 496) vs sync (credential Tech Spec line 887)** — `state = ctx.load_state::<C>(...).await?` vs `let state = ctx.load_state::<C>(key)?`.

Impact: § 3.3 is "freeze-grade Rust" per § 2.0 line 101, but freezes a divergence from credential Tech Spec § 3.4 — which it explicitly cites as load-bearing (§ 3 narrative line 418, § 3.2 line 450, § 3.2 line 472).

Suggested fix: hoist the open item § 3.2-1 from § 3.2 into § 3.3 inline ("§ 3.3 freezes the spike's BoxFuture+ResolvedSlot interpretation per open item § 3.2-1; deliberate-divergence vs credential Tech Spec § 3.4 step 3 narrative — CP3 § 9 ratifies wrap point"). Strategy § 5.1.1 deadline ("before CP3 § 7 drafting") still binds, but the divergence needs to be explicit, not silent.

---

## Cross-doc reference resolution

### 🟢 PASS — Strategy citations

Verified line ranges (sample, all green):
- Tech Spec § 0.1 cites Strategy § 6.3 line 386-394 → confirmed (Tech Spec checkpoint roadmap table).
- Tech Spec § 1 G6 cites Strategy § 4.3.2 → confirmed (lines 222-229).
- Tech Spec § 1.2 N5 cites Strategy § 4.2 line 198-206 → confirmed.
- Tech Spec § 1.2 N5 cites Strategy § 6.5 line 408-413 → confirmed.
- Tech Spec § 1.2 N4 cites Strategy § 6.6 line 421 → confirmed (line 421 says "It gates path (c) availability only").
- Tech Spec § 1.2 N7 cites Strategy line 432-440 sunset table → confirmed.
- Tech Spec § 1 G5 cites rust-senior 02c § 6 / § 8 line numbers → present in 02c (not re-verified line-by-line; cross-checked via Strategy § 4.3.1 which cites the same rows).

### 🟡 MEDIUM — § 2.7.1 cites "Strategy § 4.3.2 (line 224-229)" — off by 2

§ 4.3.2 begins at Strategy line 222 (`#### § 4.3.2 \`unstable-retry-scheduler\``), not 224. Tech Spec § 2.7.1 line 336 cites "Strategy § 4.3.2 line 226-228" for "no parallel retry surface" — actual quote at line 229. Off-by-3.

Impact: minor, but § 0 line 39 freeze-policy says "if a cited line range moves due to upstream document edits, this Tech Spec must be re-pinned (CHANGELOG entry + reviewer pass)." This pinning is already wrong on commit-zero.

Suggested fix: re-pin all Strategy § 4.3.2 line citations after audit.

### 🟡 MEDIUM — § 2.3 PRODUCT_CANON line citation propagates Strategy error

Tech Spec § 2.3 cites Strategy § 2.3 line 70 which itself says "PRODUCT_CANON § 4.5 false-capability rule (line 131)." Actual canon line: 133 (verified `Grep "false-capability" docs/PRODUCT_CANON.md` returns line 133). Tech Spec doesn't cite line 131 directly — it cites Strategy — so this is Strategy's error inherited. Flagged for awareness; correction belongs in Strategy revision, not Tech Spec.

### 🟡 MEDIUM — § 2.6 cites "ADR-0035 § 3" but uses ADR-0035-amendment-2026-04-24-B form without flagging which iteration

Tech Spec § 2.6 line 288: "Sealing follows the per-capability inner-sealed-trait pattern from [ADR-0035 § 3](...)#3-sealed-module-placement-convention) (the post-amendment-2026-04-24-B canonical form)."

The amendment date "2026-04-24-B" reads as plausible but I cannot verify it from the inputs given. ADR-0035 was not in the audit input set. If amendment 2026-04-24-B is the chosen iteration, fine; if not, flag.

Suggested fix: architect to cross-check ADR-0035 amendment history; if 2026-04-24-B does not exist or has been superseded by a later amendment (e.g., 2026-04-24-C), update.

### ✅ GOOD — ADR cross-references all resolve

ADR-0038, ADR-0039, ADR-0040 are all in `docs/adr/` with `status: proposed` matching Tech Spec § 0.1 line 35 claim. ADR cross-citations from § 1 G1-G6 / § 2.x / § 4 are consistent.

---

## § 2 signature compile-check findings

Beyond the three 🔴 / 🟠 above, the rest of § 2 mostly aligns with `final_shape_v2.rs`. Specific spot-checks:

| Tech Spec | final_shape_v2.rs | Match |
|---|---|---|
| § 2.2.2 `StatefulAction::State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static` (line 147) | Line 223 `type State: Send + Sync + 'static;` (no `Serialize`/`DeserializeOwned`/`Clone`) | **🟠 HIGH — DIVERGENCE.** Architect already flagged this in CP1 review per the dispatch instructions. Tech Spec adds `Serialize + DeserializeOwned + Clone` over spike — deliberate (per § 2.2.2 line 159 rationale: "engine's contract — `Serialize` + `DeserializeOwned` for persisted iteration state"). Same problem as `Input: HasSchema` — § 2.0's "compile-checked against final_shape_v2.rs" claim is unconditionally false. Needs deliberate-divergence note. |
| § 2.2.3 `TriggerSource: Send + Sync + 'static`, `type Event: Send + 'static` (line 166-168) | Line 250-252 same | ✓ |
| § 2.2.3 `TriggerAction::Source: TriggerSource` (line 171) | Line 255 same | ✓ |
| § 2.2.4 `Resource: Send + Sync + 'static`, `type Credential: Credential` (line 189-191) | Line 233-235 same | ✓ |
| § 2.2.4 `ResourceAction::Resource: Resource` (line 194) | Line 238 same | ✓ |
| § 2.3 `BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>` (line 215) | Line 38 `BoxFuture<'a, T> = Pin<Box<dyn ... + Send + 'a>>` | **🟡 MEDIUM — NAME DRIFT.** Tech Spec uses `BoxFut`; spike uses `BoxFuture`. Both reference shapes are correct; the alias name differs. Credential Tech Spec § 3.4 line 869 also says `BoxFuture`. Recommend Tech Spec align to `BoxFuture` for consistency, OR document the rename + reason. |
| § 2.4 `*Handler` traits use `BoxFut<'a, ...>` return | spike doesn't have `*Handler` traits — they're production-only | n/a (no spike to compile-check against) |
| § 2.5 `ActionHandler` enum 4 variants | spike doesn't have this enum — production-only | n/a |
| § 2.6 sealed_dx module — see 🔴 above for the blanket-impl bound | spike final_shape_v2.rs:282 different blanket bound | **🔴 BLOCKER above** |

### 🟡 MEDIUM — § 2.4 dyn-safety claim is unverified

§ 2.4 line 266 says "Each handler trait is dyn-safe (per rust-senior 02c § 6 line 358) — `Arc<dyn StatelessHandler>` continues to compile post-modernization." The 02c § 6 citation is consistent with Strategy § 4.3.1 line 216 (rust-senior 02c § 6 line 358 confirms cut at ~8 lines, dyn-safety preserved). But Tech Spec doesn't verify dyn-safety against current `crates/action/src/handler.rs` shape. Spike `final_shape_v2.rs` does NOT model the `*Handler` family (intentionally; spike scope was credential resolution, not handler erasure).

Impact: not a structural error — citation chain holds. But the implementer-facing claim "Arc<dyn StatelessHandler> continues to compile post-modernization" has zero direct compile evidence in the audit input set.

Suggested fix: at CP2 § 7-8 testing scope, add a CP1-forward-promise saying "dyn-safety of § 2.4 handler shapes verified in CP3 implementation gate" OR cite a specific 02c probe.

---

## Forward-ref bookkeeping

### ✅ GOOD — Forward references all marked deferred, not dangling

Sample-checked:
- "§ 4 (CP2)" cited in § 1 G3 line 71, § 2.4 line 266, § 2.8 line 412, § 3.4 line 525, § 3.4 line 540 — all consistent ("CP2 scope" annotation).
- "§ 7 / § 9 (CP3)" cited in § 1 N4, § 2.2.3 line 184, § 2.6 line 322, § 2.7-1 line 396, § 2.7-2 line 398, § 3.1 line 446, § 3.2 line 466, § 3.2-1 line 474, § 3.4 line 538 — all marked CP3.
- "§ 16 (CP4)" cited in § 1.2 N5 line 92 — marked CP4.

### ✅ GOOD — Open items raised this checkpoint enumeration

§ 0.2-derived open items list at line 544-557 enumerates 11 items; cross-checked against body section flags:
- § 2.7-1, § 2.7-2 (in body) ✓
- § 2.6 DX-blanket trait-by-trait audit (in body line 314-317) ✓
- § 3.2-1 ResolvedSlot wrap point (in body line 474) ✓
- § 3.4 cancellation-zeroize test instrumentation (in body line 540) ✓
- § 1.2 N5 paths a/b/c (in body) ✓
- § 2.2.3 TriggerAction cluster-mode hooks (in body line 184) ✓
- § 2.2.4 resource-side scope (in body via N1) ✓
- § 2.8 redacted_display() helper crate location (in body line 412) ✓
- § 3.1 ActionRegistry::register* call-site exact line range (in body line 446) ✓
- § 3.2 ActionContext API location (in body line 466 + § 3.2 § 5.1.1) ✓

Eleven body-flags ↔ eleven list entries. **One-to-one match.** 

### 🟡 MEDIUM — § 3.3 inline open item from § 3.2-1 not surfaced in open-items list

§ 3.2-1 line 474 says "CP1 inherits the spike's interpretation pending CP3 ratification." § 3.3 then commits to that interpretation in `resolve_as_bearer` signature (with `BoxFuture` return + `&SlotKey` key) but does not flag the inheritance inline. The open-items list captures § 3.2-1 once; § 3.3 silently rides on it.

Suggested fix: either inline-flag § 3.3 ("freezes spike interpretation per § 3.2-1") or split § 3.2-1 into two list entries — one for the wrap-point question, one for the cascading § 3.3 signature divergence vs credential Tech Spec § 3.4.

---

## Open items + glossary coherence

### ✅ GOOD — Strategy § 5 carries-forward fully reflected

Strategy § 5.1.1–§ 5.1.5 (5 items) are all surfaced in Tech Spec open-items list:
- § 5.1.1 ActionContext API location → Tech Spec § 3.2 entry ✓
- § 5.1.2 redacted_display() helper crate → Tech Spec § 2.8 entry ✓
- § 5.1.3 Credential Tech Spec § 7.1 line numbers → not surfaced as Tech Spec open item, but Tech Spec doesn't cite § 7.1 line numbers directly (only the § 3.4 line 869 HRTB shape) — this is consistent.
- § 5.1.4 B'+ contingency activation criteria → not surfaced as Tech Spec open item, but Strategy § 6.8 already discharged it; not Tech Spec scope.
- § 5.1.5 cluster-mode hooks final shape → Tech Spec § 2.2.3 entry ✓

### 🟡 MEDIUM — Glossary check: `BoxFut<'a, T>` not in `docs/GLOSSARY.md`

Tech Spec § 2.3 introduces `BoxFut<'a, T>` as a type alias. No glossary check was run; if `docs/GLOSSARY.md` does not have `BoxFut` (highly likely — recent introduction), Tech Spec must either add a glossary entry or flag the term as proposed-for-glossary at CP4 § 14.

Suggested fix: at CP4 § 14, audit § 2 vocabulary against `docs/GLOSSARY.md`; add entries for `BoxFut`, `SlotBinding`, `SchemeGuard`, `ActionSlots`, `sealed_dx`, etc., as needed.

### ✅ GOOD — `sealed_dx::*` module path consistent

Tech Spec § 2.6 lines 291-300 declare `mod sealed_dx`. Lines 295-299 enumerate 5 inner sealed traits; lines 303-311 reference them as supertraits on the public DX traits. Lines 313-319 reference the blanket impls. Module path `sealed_dx::*` is used consistently throughout § 2.6. ADR-0040 § 1 lines 56-66 use the same `sealed_dx::*` form. ✓

### ✅ GOOD — Status header `DRAFT CP1`

Frontmatter line 3: `status: DRAFT CP1`. Matches expected per audit checklist.

### 🟡 MEDIUM — `BoxFut<'a, T>` vs `BoxFuture<'a, T>` synonym proliferation

Same alias used under two names in Tech Spec § 2.3 (`BoxFut`) and § 3.2 (`BoxFuture` in the `ResolveFn` HRTB definition line 461). § 3.2 line 461 uses `BoxFuture<'ctx, ...>` directly inside the `ResolveFn` type (not via the § 2.3 `BoxFut` alias). Reader has to mentally unify the two names.

Suggested fix: inline note in § 3.2 line 461 — "(`BoxFuture` here is the credential Tech Spec § 3.4 verbatim form; equivalent to `BoxFut<'ctx, T>` from § 2.3 — consider unifying)." OR rename § 2.3 alias to `BoxFuture`.

---

## Coverage summary

- Structural: 1 finding (§ 2.6 sealed-trait blanket bound mismatch with § 2.1 supertrait — 🔴)
- Cross-section consistency: 2 findings (HasSchema bound divergence — 🟠; State trait bound divergence — 🟠)
- External verification: 4 findings (`credential_slots` signature — 🔴; `SlotType` payload drop — 🔴; `resolve_as_bearer` signature drift — 🟠; § 2.7.1 fabricated row "S3" — 🟠)
- Bookkeeping: 1 finding (§ 3.3 not flagged in open-items list — 🟡)
- Terminology: 2 findings (`BoxFut` vs `BoxFuture` synonyms — 🟡; glossary entries missing — 🟡)
- Definition-of-done (§ 17): out of CP1 scope (CP4 spec-auditor full audit per Strategy § 6.3 line 392)
- Strategy citation line numbers: 1 finding (off-by-3 on § 4.3.2 line range — 🟡)

Total: 3 🔴 + 3 🟠 + 4 🟡 + 2 ✅

---

## Summary for orchestrator

**Verdict: REVISE.** CP1 is ~95% structurally coherent — the open-items list is complete, forward-references all resolve, ADR/Strategy cross-citations land, status header correct. But § 2 freezes three signatures (`ActionSlots::credential_slots()`, `SlotType::ServiceCapability`, `Input: HasSchema`) that diverge from `final_shape_v2.rs` and credential Tech Spec § 3.4 / § 9.4. § 2.0 line 101 ("compile-checked against final_shape_v2.rs") is the document's freeze warrant, and that claim is unconditionally false on these three points.

**Iterate-yes.** All findings are mechanical (one-line edits) except the `SlotType` enum payload drop, which is one-table edit. None invalidates the CP1 design direction. Architect can resolve in one revision pass.

**Top 3 must-fix before CP1 ratify:**
1. § 2.1 / § 3.1 / ADR-0039 align `credential_slots()` to `&self` form per spike + credential Tech Spec.
2. § 3.1 `SlotType` enum gain `Concrete { type_id }` + `service: ServiceKey` field on `ServiceCapability` + `CapabilityOnly { capability }` per credential Tech Spec § 9.4 line 2452.
3. § 2.0 line 101 add deliberate-divergence rationale for `Input: HasSchema` and `State: Serialize + DeserializeOwned + Clone` over spike.

**Handoff: architect** for all 🔴 / 🟠 / 🟡 findings (none require tech-lead decision; all are content corrections). Architect to redraft § 2.1, § 2.6 blanket impl, § 3.1 enum, § 3.3 deliberate-divergence note, § 2.7.1 row attribution, line-pin re-sweep on Strategy § 4.3.2.

**Handoff: tech-lead** advisory only — § 2.7.1 wire-end-to-end choice is sound (Phase 1 solo-decision per pain enum line 184-185 + 227 confirmed); evidence chain just needs the row attribution corrected.
