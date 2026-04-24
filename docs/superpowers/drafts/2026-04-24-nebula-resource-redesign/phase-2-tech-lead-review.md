# Phase 2 — Tech-Lead Priority-Call Review

**Date:** 2026-04-24
**Reviewer:** tech-lead (subagent dispatch)
**Reviewing:** `03-scope-options.md` (architect draft)
**Commit audited:** `d6cee19f814ff2c955a656afe16c9aeafca16244`

---

## Priority call

**Option B — Targeted,** with two bounded amendments (see §"Amendments" below).

**Rationale (4 sentences).** B is the smallest scope that atomically closes the credential×resource seam (`02-pain-enumeration.md:42-50`) — the single primary driver every agent converged on — while satisfying security-lead's hard "atomic rotation dispatcher + reverse-index + observability" requirement (`02-pain-enumeration.md:229-231`). A is off the table: leaving 🔴-1 (silent revocation drop at `crates/resource/src/manager.rs:1360-1401`) in production while writing `Auth`-shaped docs that will need a second pass directly violates `feedback_incomplete_work.md` + "don't write docs twice" (`02-pain-enumeration.md:240`); the security gap alone likely trips security-lead's gate per their stated position (`02-pain-enumeration.md:229-231`). C adds Runtime/Lease collapse + `AcquireOptions` resolution, neither of which the evidence *forces* right now — Runtime/Lease friction is real (9/9 tests, `02-pain-enumeration.md:127`) but orthogonal to the credential seam, and `AcquireOptions::intent/.tags` needs an engine-side design (ticket #391) that doesn't exist yet, so resolving it inside this cascade would be guessing. B matches my Phase 1 preview (`02-pain-enumeration.md:223-227`) at the 4-point level; C adds two sub-decisions that should land standalone in a later cascade.

**Endorsement level:** high confidence. B is the expert call per `feedback_hard_breaking_changes.md` + `feedback_bold_refactor_pace.md`: hard breaking changes, one-PR migration, 5 in-tree consumers, MATURITY = `frontier`. No shims (`feedback_no_shims.md`) — `type Auth` is removed, not bridged.

---

## Architect's questions — my answers

### Q1 — Is Option A acceptable to security-lead? (`03-scope-options.md` OQ-1)

Not my call to gate on, but my priority-call position: **A is unacceptable independent of security-lead.** Even if security-lead accepted "atomically, in a follow-up project," A forces us to write the doc rewrite (`api-reference.md`, `adapters.md`, `dx-eval-real-world.rs`, README) against the `Auth`-shaped trait that is known-to-be-superseded by Tech Spec §3.6 (`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996`). That's the textbook "incomplete work" pattern. Defer A.

### Q2 — Credential reshape shape: §3.6 on `Resource` vs `AuthenticatedResource: Resource` sub-trait

**Adopt Tech Spec §3.6 verbatim. `type Credential: Credential` on `Resource` directly, with `type Credential = NoCredential;` as the idiomatic opt-out for credential-less resources.** Not a sub-trait.

**Why I changed from the Phase 1 preview wording.** My preview said "add `AuthenticatedResource: Resource` sub-trait" (`02-pain-enumeration.md:224-225`). That was a reflex for "how do I make the credential-less case ergonomic without forcing every resource to acknowledge credentials?" Having now re-read §3.6 (Credential Tech Spec lines 928-996) — which designs `on_credential_refresh` as a **default-no-op method on `Resource`** — and having verified zero in-tree `impl Resource for ...` in non-test production code (`02-pain-enumeration.md:196`), the sub-trait adds no value:

1. **§3.6's default no-op is exactly the "credential-less" escape hatch.** The `type Credential = NoCredential;` pattern + default `async fn on_credential_refresh(...) { Ok(()) }` is zero-cost for resources that don't care, and one `type` line + one method override for resources that do. A sub-trait would require *two* traits to learn, two register paths (`register` vs `register_authenticated`), and complicates `Manager` dispatch (which trait bound does the reverse-index dispatcher require?).

2. **§3.6 is already the downstream contract.** The credential tech spec is checkpoint-4-ratified and committed (`de497c1a`, `08014e48`, `affea2aa`). Diverging the resource side would force a second round of reconciliation later. Per `feedback_adr_revisable.md` / spec alignment: when the spec has a concrete shape, adopt it — don't draft a parallel shape.

3. **Sub-trait is the fallback, not the default.** The spike probe 1 (`03-scope-options.md` B.8, probe 1) exists precisely to validate §3.6 ergonomics. If `type Credential = NoCredential;` doesn't compose cleanly across all 5 topologies, fall back to sub-trait. But don't pre-commit to the fallback.

**One binding constraint.** The `NoCredential` marker type (and its `Scheme = ()` or equivalent) must live where both sides can see it without cycles. Probable home: `nebula-credential::NoCredential` (where the `Credential` trait already lives), re-exported through `nebula-resource` prelude. Phase 3 Strategy to confirm. Don't invent a resource-side `NoCredential` that shadows a credential-side equivalent.

### Q3 — Rotation dispatch concurrency (parallel vs serial across N resources sharing one credential)

**Parallel, with per-resource isolation.** Specifically:

- `Manager::on_credential_refreshed` collects the `Vec<ResourceKey>` from the reverse-index, then dispatches `on_credential_refresh` to each in parallel via `futures::future::join_all` (or a bounded `FuturesUnordered` if we're worried about unbounded fan-out at scale).
- **Per-resource failure isolation is a hard invariant:** one resource's hook returning `Err` must not cancel or block other resources' hooks. Failed resources transition to `Failed` phase + emit `HealthChanged { healthy: false }`; others continue.
- **No shared lock held across hook calls.** The reverse-index read completes before dispatch; `DashMap` releases its entry guard before we `await` anything. (Today's code at `manager.rs:1364-1368` already clones the `Vec<ResourceKey>` — keep that pattern.)

**Why parallel (not serial).**

1. **Serial is a latency multiplier.** A single slow `on_credential_refresh` (e.g., a Postgres pool rebuilding 32 connections) would block every other resource's rotation. For a credential shared by a Postgres pool + Redis client + S3 client, serial dispatch means users see cascade latency equal to the sum of all rotation costs.
2. **The §3.6 blue-green swap is designed as an atomic, independent operation per resource** (spec lines 981-993 show `Arc<RwLock<Pool>>` + write-lock swap — no cross-resource coordination). The spec author assumed independence; parallel reflects that.
3. **Failure isolation is easier in parallel.** In serial, the failure model forces "do I keep going after one fails?" into every implementation. In parallel, isolation is structural: each future runs in its own task, each failure is collected into its own result slot.

**Concurrency cap.** Unbounded `join_all` is fine for reasonable resource counts (dozens per credential). If operators register hundreds of resources against one credential (unlikely but possible), a `FuturesUnordered` with a soft cap of ~32 concurrent hooks is a conservative default. Phase 4 spike (B.8 probe 3) should stress-test this.

**Observability requirement per `feedback_observability_as_completion.md`.** The dispatcher must emit:
- One parent trace span `resource.credential_refresh` tagged with `credential.id` + `affected_count`.
- One child span per dispatch, tagged with `resource.key` + `outcome` (Ok / Err kind).
- Counter `resource_credential_refresh_total{outcome}` with labels for `ok` / `err`.
- `ResourceEvent::CredentialRefreshed { resource_key, outcome }` per resource. (New variant; confirm via `events.rs` audit in Phase 3.)

This is **in scope** for the Phase 5 rotation dispatcher PR, not follow-up.

### Q4 — Open Q4 (`AcquireOptions::intent/.tags`: C.8a remove vs C.8b wire)

Not applicable — Option B defers to a future cascade. But my position if asked: **C.8a (remove).** Per canon §4.5 "no false capabilities" and `feedback_incomplete_work.md`, fields with zero readers should not be in the public surface. Ticket #391 is engine-side; when engine actually implements deadline scaling / queue priority / trace tagging, it can re-add the fields with real semantics. Shipping empty fields "in case we need them" is the false-capability anti-pattern the canon explicitly forbids. — But this is not Option B scope. In Option B, mark `#[doc(hidden)]` interim (as architect proposed in B.3) and don't remove yet.

### Q5 — Open Q5 (Service/Transport merge in Option C)

Not applicable — Option B defers. My position if asked: **defer. Evidence ("defensible but thin," `02-pain-enumeration.md:139`) does not force a merge; Transport has 0 Manager-level tests (`02-pain-enumeration.md:138`) but that's a test-debt finding, not a merge rationale.** Per `feedback_boundary_erosion.md`: merging without strong evidence trades one unclear boundary for a different unclear boundary.

### Q6 — Engine consumption of Daemon/EventSource (`03-scope-options.md` OQ-2)

**Verified: no live engine dependency.** Grep of `crates/` (this session) for `DaemonRuntime|EventSourceRuntime|TopologyTag::Daemon|TopologyTag::EventSource` returns 6 files, all inside `crates/resource/` + its docs. Zero hits in `crates/engine/`. **Extraction is safe**; no replacement primitive needed in engine. Phase 3 Strategy should re-verify with a broader grep across the workspace (including examples/), but the signal is strong.

---

## Amendments

Two bounded amendments to Option B before I endorse.

### Amendment 1 — Lock Q2 to §3.6 shape; remove sub-trait fallback from live scope

**What.** B.8 probe 1 in architect's draft treats `AuthenticatedResource: Resource` sub-trait as a "fallback if §3.6 ergonomics don't work out." Remove that escape valve from Phase 4 spike exit criteria. If `type Credential = NoCredential;` doesn't compose cleanly, that's a **spike failure that escalates back to Phase 2**, not a mid-flight shape change.

**Why.** Sub-trait fallback mid-flight means the Strategy doc, ADR text, and Tech Spec adoption narrative all need to be written in a shape-agnostic way — that's architecture-debt from the first document. Phase 4 is supposed to de-risk a specific shape; allowing two shapes doubles review surface and invites bike-shedding mid-cascade. Per `feedback_context_hygiene.md`: don't pile shape-optionality into Phase 3-5 when Phase 2 should decide.

**Effect on architect's draft.** B.7 risk #1, B.8 exit criteria, and OQ-3 all tighten: §3.6 is *the* shape; spike either validates it or escalates.

### Amendment 2 — Explicit rotation observability commitment to DoD

**What.** Add to B.2 row 🔴-1 treatment: "Rotation dispatcher PR ships with trace spans + counter + per-resource `ResourceEvent::CredentialRefreshed` variant. Not a follow-up."

**Why.** Per `feedback_observability_as_completion.md`, new hot path = typed error + trace span + invariant check are DoD, not TODO. Architect's draft implies this in B.2 ("rotation observability (trace span + counter + event)") but doesn't pin it as a gate. I want Phase 5 review (CP review, Phase 6) to hard-check this, not treat it as "nice-to-have."

**Effect on architect's draft.** B.2 🔴-1 row gains an explicit observability sub-list; B.10 budget stays unchanged (already implicitly included in the 8-12h Phase 5).

---

**Note: these are 2 amendments, within the bounded-amendments envelope.** Not proposing a new option.

---

## Security-lead handoff

Security-lead: three specific questions I want you to gate on.

1. **Does the parallel-dispatch model in Q3 open any attack surface?** Specifically: if an attacker can force one resource's `on_credential_refresh` to hang (e.g., slow network to a compromised Postgres replica), does parallel dispatch give them a way to extend the effective TTL of a rotated credential on *other* resources? My position: no — each resource's blue-green swap is local; hung resources just don't update, but the new credential is already active for refresh-initiators. But I want your sign-off.

2. **Per-resource failure isolation: is `HealthChanged { healthy: false }` + phase transition to `Failed` sufficient for a rotation-hook failure, or do you require harsher containment?** E.g., do you want outstanding guards on a failed resource to be *tainted* immediately (new acquires blocked, existing guards forced to re-resolve before their next operation), or is phase=Failed adequate?

3. **Amendment 1 (lock §3.6 shape, no sub-trait fallback): does removing the fallback path open a security regression?** My read: no — the shape doesn't change threat model, only ergonomics. But worth your explicit confirmation since spec↔code alignment was on your Phase 1 priority list (`02-pain-enumeration.md:231`).

Also: confirm SF-1 (`deny.toml` wrappers) is still tracked as standalone PR parallel to cascade (`03-scope-options.md` §"Out-of-scope for the cascade"). I'm not blocking B on SF-1 landing first, but devops needs a pointer.

---

## Convergence estimate

**Lock in round 1 — high probability (~80%).**

Rationale:
- Option B aligns 1:1 with my Phase 1 priority-call preview (`02-pain-enumeration.md:223-227`).
- Security-lead's preview position (atomic dispatcher, §3.6 blue-green support) is baked into B.
- Rust-senior's Runtime/Lease proposal is explicitly deferred (not rejected) — that's a clear "not now, not no."
- DX-tester's Daemon-gap concern is resolved by extraction.
- My Q2 answer commits to the §3.6 shape that architect already drafted as primary; no shape conflict between us.
- My amendments are both tightening-of-scope, not direction changes.

**Only plausible round-2 trigger:** security-lead disagrees on Q3 concurrency model (parallel vs serial) or flags my Amendment 1 (no sub-trait fallback) as security-relevant. Both are narrow disagreements; even if round 2 happens, it's a single-axis re-sync, not a scope rewrite.

---

## Artefact references

| Artefact | Path |
|---|---|
| This review | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md` |
| Architect's options draft | `.../03-scope-options.md` |
| Phase 1 pain enumeration | `.../02-pain-enumeration.md` |
| Phase 0 current state | `.../01-current-state.md` |
| Credential Tech Spec §3.6 | `docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996` |
| Manager rotation todo!() | `crates/resource/src/manager.rs:1360-1401` |
| Reverse-index declaration | `crates/resource/src/manager.rs:262` |
| Register hardcoded-None | `crates/resource/src/manager.rs:370` |

*End of review. Awaiting orchestrator synthesis with security-lead gate.*
