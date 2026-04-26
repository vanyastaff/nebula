# Tech Spec CP3 §11 — DX-Tester Review

**Reviewer:** dx-tester
**Date:** 2026-04-24
**Scope:** CP3 §11 (adapter authoring contract) + §10.2 dual-helper DX impact
**Inputs:** Tech Spec [§10.2 lines 1788-1817](../../specs/2026-04-24-nebula-resource-tech-spec.md), [§11 lines 1930-2222](../../specs/2026-04-24-nebula-resource-tech-spec.md); Phase 1 [§1.3 lines 59-67](02-pain-enumeration.md), [§2.1 lines 103-110](02-pain-enumeration.md); spike [`resource-shape-test/src/lib.rs`](spike/crates/resource-shape-test/src/lib.rs).

---

## Verdict: **ENDORSE_WITH_AMENDMENTS**

§11 is a substantial improvement over the current `crates/resource/docs/adapters.md` and would, on net, give a newcomer a working adapter — closing the 🔴-3 fabrication gap. **Three bounded amendments** required before this counts as the binding rewrite content (compile-claim integrity, missing imports, and one §11.7 example mismatch).

§10.2 dual-helper decision: **endorsed**. 10 methods is tolerable cognitive load given the documented compile-time enforcement and migration parity, *if* §11 is the doc surface that introduces the dual to newcomers (which it is). The unified-`RegisterOptions` alternative would be more discoverable in isolation but loses the key 🟠-12 fix below.

---

## §11 walkthrough findings (per subsection)

### §11.1 — Required imports (line 1934-1962)

**Finding:** Import list is **complete enough to compile** for the §11.2 walkthrough but has one inconsistency with §11.2.

- **OK:** `NoCredential` location pinned to `nebula_credential` (CP1 Q1 honoured at line 1960). Resolves Phase 1 🟠 finding "three contradictory stories about Resource::Credential vs Auth" ([Phase 1 §2.1](02-pain-enumeration.md)) — only one home now.
- **OK:** Comments explain *why* each import is needed — far better than current `adapters.md`.
- **Gap (Amendment 1):** Line 1948 imports `NoScheme` from `nebula_credential`, but the §11.2 example body at lines 1969-1974 does NOT actually `use NoScheme` anywhere — it uses the `<Self::Credential as Credential>::Scheme` projection instead. A newcomer copy-pasting the §11.1 import block plus §11.2 walkthrough will get an `unused_imports` warning on `NoScheme`. Either drop `NoScheme` from §11.1 (relying on the projection alone) or use it explicitly in §11.2's `_scheme: &NoScheme` parameter type as §11.4 does at line 2068. **Pick one form and apply consistently across §11.2/§11.4/§11.5.**

### §11.2 — Minimum Resource impl shape (lines 1964-2034)

**Critical finding:** Line 1966 claims **"Every line compiles against trunk after the redesign — no `ignore` blocks"**. That claim is **false** as written:

- **Amendment 2 (compile-claim integrity).** Lines 1982-1983 show:
  ```rust
  impl HasSchema for MockPostgresConfig { /* ... */ }
  impl ResourceConfig for MockPostgresConfig { /* ... */ }
  ```
  These `/* ... */` placeholders are **not valid Rust** — `HasSchema::schema()` is required and has no default body. A newcomer doc-testing this block sees `error[E0046]: not all trait items implemented`. The same pattern recurs at line 1986 (`MockPostgresConnection { /* ... */ }`) and line 2018 (`Ok(MockPostgresConnection { /* ... */ })`).

  This is **the same fabrication shape Phase 1 🔴-3 indicted** ([§1.3 line 61](02-pain-enumeration.md): "`ResourceConfig: HasSchema` super-trait hidden, never mentioned in adapters.md"). §11.2 *does* mention it implicitly via the import, but the placeholder bodies hide that the newcomer needs to write a real schema.

  **Required:** either (a) make §11.2's `MockPostgresPool` block end-to-end real (provide a `HasSchema` impl with `nebula_schema::Schema::builder()...` derivation; provide `MockPostgresConnection` field bodies), or (b) **drop the "every line compiles" claim** and explicitly mark §11.2 as `# Pseudocode — see [`spike/.../lib.rs:133-156`](spike/crates/resource-shape-test/src/lib.rs#L133) for the literal compiling form`. Option (a) preferred — it's exactly what the spike's `MockKvStore` ([`lib.rs:125-156`](spike/crates/resource-shape-test/src/lib.rs)) already does.

- **OK:** "Five things to note" sidebar (lines 2027-2034) is the right shape — it surfaces the `Auth::default()`-removal, the borrow-not-clone invariant, and `async fn` impl-side preference. These are the actual newcomer traps.

### §11.3 — Topology selection guide (lines 2035-2054)

**Finding:** Solid. Five topologies × selection criteria + three "common selection mistakes" (lines 2049-2053). The mistakes are real (`Resident` for stateless HTTP, `Pooled` for `reqwest::Client`, `Service` for connection pools) — these are exactly the wrong choices a newcomer would make from name-matching.

**Minor:** No mention that **registering a topology requires implementing both `Resource` AND the topology sub-trait** (e.g., `impl Pooled for MockPostgresPool {}`). The §11.2 walkthrough doesn't show the `impl Pooled for MockPostgresPool {}` line; the spike does ([`lib.rs:290`](spike/crates/resource-shape-test/src/lib.rs)). A newcomer reading §11.2 alone would not register their type as `Pooled`. **Suggested:** add the `impl Pooled for MockPostgresPool {}` line at the end of §11.2's code block.

### §11.4 — `NoCredential` opt-out walkthrough (lines 2055-2082)

**Finding:** Strong. The "common mistake" callout at line 2082 (`type Credential = ();` doesn't compile) is exactly the trap a newcomer migrating from the old `Auth = ()` would hit. The compile-fail probe `_no_credential_scheme_is_inert_must_fail` is cited correctly.

The "three guarantees" sidebar (lines 2076-2080) — Manager skips reverse-index write, compile-time enforcement, zero overhead — is the right surface for explaining why the opt-out is preferable to a runtime `Option<CredentialId>` shape.

### §11.5 — Credential-bearing walkthrough (lines 2084-2131)

**Finding:** Framing is honest ("a hypothetical `nebula-credential-postgres` crate" at line 2086) — that mitigates the fabrication risk Phase 1 🔴-3 flagged. **But:**

- **Amendment 3 (annotation):** Line 2089 imports `nebula_credential_postgres::{PostgresCredential, PostgresConnectionScheme}`. These types do not exist in-tree (verified: only archived plan docs and a macro doc-comment example reference them). Combined with `scheme.username()` and `scheme.password_redacted_str()` at line 2118 — also methods that don't exist on any in-tree scheme — this is a `///` doc-test that would compile-fail under the §11 acceptance gate ("`cargo test --doc` green for any non-`ignore` blocks", line 1962). **Required:** annotate §11.5 explicitly as `\`\`\`rust,ignore` (or `\`\`\`rust,no_run` — but better `ignore` since it can't compile at all). Without this, §11.5 *re-creates the §11.2 problem* but at higher risk because the API surface is hypothetical.

  Phase 1 🔴-3 specifically cited "`adapters.md` references nonexistent adapter crates `nebula-resource-postgres` / `nebula-resource-redis`" — §11.5 with `nebula-credential-postgres` repeats this exact failure mode unless the `ignore` annotation lands.

- **OK:** "Four invariants surfaced by this walkthrough" (lines 2126-2131) — borrow-not-clone, no `Scheme::default()` in `create`, Manager doesn't hold the scheme, pool swap on rotation — these are the load-bearing security/hot-path invariants from CP1+CP2. Good repetition.

### §11.6 — `on_credential_refresh` / `on_credential_revoked` overrides (lines 2133-2178)

**Finding:** Excellent. The blue-green swap example is the canonical pattern from credential Tech Spec §3.6, reproduced concretely. The "three things to note" sidebar at lines 2174-2178 — override augments not replaces, idempotency, budget — capture the operational invariants that Phase 1 didn't have any documentation for at all.

**Minor:** Line 2150 `build_pool_from_scheme` is a free function that doesn't exist anywhere; same `ignore` annotation requirement as §11.5. Lower stakes because the framing is "this is what your override looks like" rather than "this compiles".

### §11.7 — Testing your adapter (lines 2180-2213)

**Finding:** Three test layers (compile-fail probes, integration tests, rotation tests) covers the right surface. Compile-fail probes correctly punted to crate-side per §7.5.

- **Amendment 1 follow-up:** Line 2195-2199 shows `manager.register_pooled(MockPostgresPool, MockPostgresConfig::default(), PoolConfig::default())`. Verified against current `manager.rs:404-429`: this signature matches (`resource, config, pool_config`). **OK.**
- **Inconsistency:** Line 2196 imports `PoolConfig` directly but the §11.1 imports list at line 1956 only imports `Pooled`, not `PoolConfig`. A newcomer copying §11.1 + §11.7 hits `error[E0412]: cannot find type \`PoolConfig\``. Add `pub use topology::pooled::config::Config as PoolConfig` (already in §10.1 at line 1774) to the §11.1 imports list, OR rewrite §11.7 to use the fully-qualified path.

### §11.8 — Common pitfalls (lines 2215-2222)

**Finding:** Six pitfalls, all real. Resolves multiple Phase 1 findings:

| §11.8 pitfall | Phase 1 finding it closes |
|---|---|
| `Scheme::default()` inside `create` | 🟡-17 (silent-empty-credential bug) |
| Cloning scheme onto `self` | Security-lead constraint #2 |
| Sharing pool across credentials | §10.5 SL-1 deferral surfaced |
| Implementing `Daemon` / `EventSource` | ADR-0037 redirect — clear migration signpost |
| Forgetting `#[derive(Clone)]` | 🟠 finding (silent compile error) |
| Wrong `ErrorKind` mapping | Tangential but useful |

**One missing pitfall:** **`type Credential` vs `type Auth` confusion**. Phase 1 §2.1 line 107 surfaced "three contradictory stories" about whether the associated type is `Credential` or `Auth`. The migration from `type Auth = ()` to `type Credential = NoCredential` is exactly the rename a newcomer might miss. §11.4 line 2082 covers `Credential = ()` (the wrong unit-type substitution); §11.8 should also cover **`type Auth = NoCredential` (the wrong-name-right-type pitfall)** — i.e., a newcomer who reads outdated docs writes `type Auth = NoCredential;` and gets `error[E0220]: associated type \`Auth\` not found for \`Resource\``.

This is the highest-confusion path during the migration window per Phase 1 finding 🟠. **Suggested addition** to §11.8.

---

## §10.2 dual-helper DX impact

**Verdict:** ENDORSE.

### Cognitive-load assessment

10 methods on `Manager` is **tolerable** for an adapter author *because* the dual pattern is regular: every topology has `register_X` (no-cred) + `register_X_with` (any-cred). After learning one pair, the other four pairs are zero-effort to recognise. This is the same delta as `register_pooled` vs `register` today — newcomers already navigate that distinction.

### Does it match Phase 1 🟠-12?

**Yes, and this is the key win.** Phase 1 [§2.1 line 108](02-pain-enumeration.md) recorded:

> 🟠 `register_pooled` silently requires `Auth = ()` — no documented escape for real auth — `manager.rs:411,446,476,507,538`; adapters.md:354-355 says "use Manager::register directly" with zero example.

§10.2 fixes this **structurally**:

- **Bound is explicit at compile time.** `register_pooled<R: Pooled<Credential = NoCredential>>` (Tech Spec line 1806) — the bound is now part of the function signature, not a hidden trait constraint. A newcomer using a credential-bearing R gets an immediate compile error pointing at `register_pooled` requiring `Credential = NoCredential`, with the fix (`register_pooled_with`) named in the error message context. Compare to current shape at `manager.rs:411`'s `where R: Resource<Auth = ()>`, which is technically the same enforcement mechanism but with a deprecated bound name and zero discoverability.
- **The "real auth" escape is named, not punted.** Current `adapters.md` says "use `Manager::register` directly" with no example. §10.2's `register_pooled_with(R, config, pool_config, opts)` is *the* documented escape. Phase 1 🟠-12 closes.
- **Type-bound enforcement.** §10.2 line 1798 explicitly chose compile-time over runtime enforcement (`RegisterOptions::credential_id == None`-as-runtime-check rejected). For a newcomer, compile errors >> runtime errors at registration time.

### Unified-`RegisterOptions`-only alternative: rejected, correctly

The unified path would force every `register_pooled(MockKv, config, pool_config, RegisterOptions::default())` even for the trivial no-credential case. Three downsides for newcomers:

1. **Boilerplate floor rises** from 3 args to 4 args for the 60% of unauthenticated registrations (Tech Spec line 1796 cites this proportion of in-tree consumers).
2. **The `RegisterOptions::default()` is meaningless ceremony** for a no-cred case — a newcomer reading the line cannot tell if the `Default` is actually doing something.
3. **The `credential_id == None`-on-credential-bearing-R check moves runtime**, which is exactly what Phase 1 was complaining about (silent-runtime-failures at registration).

The dual-helper path keeps the shortcut visually obvious (`register_pooled`) while the `_with` suffix is a clear "I'm adding configuration" signal. **DX budget honoured.**

### One concern (not blocking)

**The §11.1 imports list does not import `RegisterOptions`** but §11.7 line 2213 mentions `Manager::register_pooled_with(R, config, opts.with_credential_id(cred_id))` — a newcomer trying to copy that line needs `use nebula_resource::RegisterOptions;`. Add to §11.1 imports OR add a "for credential-bearing adapters, also import `RegisterOptions`" note.

---

## Required amendments (bounded — three items, all small)

1. **§11.1 + §11.2 + §11.7: import-completeness pass.**
   - Drop `NoScheme` from §11.1 OR use it consistently in §11.2 (`_scheme: &NoScheme` parameter). Pick one form across §11.2/§11.4/§11.5.
   - Add `PoolConfig` (or fully-qualified `topology::pooled::config::Config`) to §11.1 imports — currently used by §11.7 but not imported.
   - Add `RegisterOptions` to §11.1 imports for the credential-bearing path used by §11.7 line 2213.

2. **§11.2: compile-claim integrity.** Either:
   - **(preferred)** Make the `MockPostgresPool` block compile end-to-end — provide real `HasSchema` impl, real field bodies. The spike's `MockKvStore` ([`lib.rs:125-156`](spike/crates/resource-shape-test/src/lib.rs#L125)) already does this; lift it. Add the missing `impl Pooled for MockPostgresPool {}` line so the example registers cleanly via `register_pooled` in §11.7.
   - **(fallback)** Drop the line-1966 "Every line compiles against trunk" claim, mark §11.2 as `# Pseudocode — see [spike `lib.rs:133`] for the compiling form`, and link to the spike. **This is strictly worse** — Phase 1 🔴-3 cited compile-fail examples as the headline gap; §11.2 should not knowingly ship pseudocode while claiming it compiles.

3. **§11.5 + §11.6: explicit `ignore` annotation.** Both subsections use hypothetical types (`PostgresCredential`, `build_pool_from_scheme`, etc.). Mark the code blocks with `\`\`\`rust,ignore` and add a one-line preamble: "the following is pseudocode — adapt against your real `Credential` impl". Mirrors the spec's own line-1962 acceptance gate ("`cargo test --doc` green for any non-`ignore` blocks").

**Optional (non-blocking):** §11.8 — add a 7th pitfall covering `type Auth = NoCredential` (the wrong-name-right-type confusion), since the migration window is the highest-risk DX moment.

---

## Summary against §11 success criteria

- **Closes Phase 1 🔴-3 (50% fabrication rate)?** Yes, *if* the three amendments land. Without amendment 2, §11.2 reproduces the failure shape at smaller scale (~15% fabrication: ~3 placeholder bodies). With amendments, fabrication rate falls to 0 in §11.2 / §11.4 / §11.7 (the literal-compile sections), with §11.5 / §11.6 honestly framed as `ignore`.
- **Closes Phase 1 §2.1 🟠-12 (`register_pooled` silently requires `Auth = ()`)?** Yes, via §10.2 dual-helper + §11.4 callout + §11.5 walkthrough naming `_with` as the canonical escape.
- **Topology selection guide complete?** Yes for the five remaining topologies. Daemon/EventSource redirect at line 2047 is correct.
- **Newcomer can write a working adapter end-to-end?** With amendments: yes, by composition of §11.1 (imports) + §11.2 (skeleton) + §11.4 (no-cred) or §11.5 (cred-bearing) + §11.7 (test). Without amendments: hits compile errors at the §11.2 placeholder bodies and §11.5 hypothetical types.

**Verdict re-stated:** ENDORSE_WITH_AMENDMENTS — three bounded, mechanical edits land §11 as binding rewrite content for `crates/resource/docs/adapters.md`.

---

## Handoff

Handoff: architect for amendment cycle (CP3 §0.3 freeze policy permits Tech Spec checkpoint amendments — these three are bounded, mechanical, and do not re-litigate any Strategy decision or CP1/CP2 lock).

If architect declines amendments 2 + 3 (the compile-claim ones), handoff escalates to: tech-lead for ratification call on whether §11 ships with documented pseudocode or full compile-clean walkthroughs. Phase 1 evidence supports the latter.
