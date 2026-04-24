# 03 — Scope Options (draft for co-decision)

**Phase:** 2 — Scope narrowing
**Date:** 2026-04-24
**Author:** architect (draft for co-decision)
**Decision body:** architect (propose) + tech-lead (priority-call) + security-lead (security gate)
**Status:** DRAFT — awaiting co-decision review
**Commit audited:** `d6cee19f814ff2c955a656afe16c9aeafca16244`

---

## Decision frame

Phase 1 surfaced 6 🔴 and 9 🟠 findings (see `02-pain-enumeration.md` §4). One finding is the primary driver: the credential×resource seam (`02-pain-enumeration.md:42-50`). Five others block public API usability, cost runtime safety on shutdown, or violate canon §4.5 "no false capabilities" (`02-pain-enumeration.md:161-172`). The remaining 🟠 findings are refactor debt that reasonable bodies could defer. This document does not adjudicate any of them — it frames three coherent cut-points so tech-lead and security-lead can pick.

The three options differ on **how much of the `Resource` trait shape is touched in a single cascade**. Option A is "doc + mechanical fixes, defer the seam"; Option B is "reshape the seam + fix Daemon, leave the rest"; Option C is "full reshape including `Runtime`/`Lease` collapse and `AcquireOptions` resolution." Each option is explicitly mutually-exclusive — callers do not mix surfaces.

Two couplings constrain the cut:

1. **`Auth` drop ↔ `on_credential_refresh` placement are a single decision.** Tech Spec §3.6 (`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996`) defines `type Credential` and `on_credential_refresh` on the `Resource` trait itself (blue-green per-resource swap). Today's code keeps `type Auth` on `Resource` and dispatches rotation via `Manager::on_credential_refreshed` against a `credential_resources` reverse-index (`crates/resource/src/manager.rs:262, 1360-1401`). These are **incompatible**: if we keep `Auth` on `Resource`, the rotation hook must stay on `Manager`; if we adopt §3.6 `on_credential_refresh` on `Resource`, `Auth` becomes redundant with `Credential` and dispatch changes shape. An option that says "drop `Auth` but keep `Manager`-orchestrated rotation" or "keep `Auth` and add `on_credential_refresh` on `Resource`" is internally incoherent and is not offered below.

2. **Atomicity of the rotation redesign is hard-required by security-lead** (`02-pain-enumeration.md:229-231`). The reverse-index write + dispatcher + per-resource hook must land in a single PR — cannot be split. This means Option A cannot "partially" do the seam: either the seam redesign is in scope entirely, or it is deferred entirely. There is no middle.

Out-of-scope for the cascade itself, independent of option chosen:

- **SF-1** (`deny.toml` wrappers for `nebula-resource`) ships as a standalone PR to devops. Noted by security-lead (`02-pain-enumeration.md:149-154, 229-232`).
- **SF-2** (wire `ManagedResource::set_failed()` in drain-abort) — architect recommendation: see option-level treatment below. The naive case (Option A) ships SF-2 standalone; Option B/C absorb it because the fix touches `Manager::graceful_shutdown` which is already in their surface.

---

## Option A — Minimal (doc surface + mechanical fixes, defer seam)

### A.1 Summary

Only ship doc rewrite + SF-1 + SF-2. Do not touch `Resource` trait shape or `Manager` API. Credential×resource seam deferred to follow-up project (not this cascade).

### A.2 In scope

| # | Finding | Treatment |
|---|---|---|
| 🔴-3 | `docs/api-reference.md` fabrication + `adapters.md` compile-fail (`02-pain-enumeration.md:59-67`) | Rewrite `api-reference.md`, `adapters.md`, `README.md`, `dx-eval-real-world.rs` against current `Auth`-shaped trait. Compile-test `dx-eval-real-world.rs` as part of CI. |
| 🔴-4 | Drain-abort phase corruption (`02-pain-enumeration.md:69-74`) | Ship SF-2: wire `ManagedResource::set_failed()` (`crates/resource/src/runtime/managed.rs:95`) in `graceful_shutdown::Abort` path (`crates/resource/src/manager.rs:1493-1510`). One-function change. |
| 🟠-10 | No `deny.toml` wrappers rule (`02-pain-enumeration.md:171`) | Ship SF-1 via devops as standalone PR (parallel to cascade). |
| 🟠-15 | `Resource::Credential` vs `Resource::Auth` 3-way doc contradiction (`02-pain-enumeration.md:176`) | Fixed implicitly by doc rewrite (all surfaces align on `Auth`). |

### A.3 Out of scope

| # | Finding | Where it goes |
|---|---|---|
| 🔴-1 | Credential×resource seam (silent revocation drop + spec↔code mismatch) | **Follow-up project** (post-cascade). Must be atomic per security-lead. Today's behaviour remains: revocations silently dropped, `todo!()` reachable if anyone adds a reverse-index write path. |
| 🔴-2 | Daemon no public start path | Deferred. No treatment — ship caveat in doc rewrite that Daemon/EventSource are not usable via `Manager` public API today. |
| 🔴-5 | `Resource::Auth` dead weight | Deferred with 🔴-1. |
| 🔴-6 | EventSource orphan surface | Deferred with 🔴-2. |
| 🟠-7 | `_with` builder anti-pattern + 2101-line file | Deferred. |
| 🟠-8 | Reserved-but-unused API (`AcquireOptions::intent/.tags`, `ErrorScope::Target`, `AcquireIntent::Critical`) | Deferred — **but document as `#[doc(hidden)]` for this release** to remove false-capability signal from the public catalog per canon §4.5 (`02-pain-enumeration.md:91-95`). |
| 🟠-9 | Daemon + EventSource out-of-canon | Deferred. |
| 🟠-11 | `Runtime == Lease` 9/9 | Deferred. |
| 🟠-12 | `register_pooled` silently requires `Auth = ()` | Deferred — document the escape hatch (use generic `Manager::register()`) with a worked example in `adapters.md`. |
| 🟠-13 | Transport: 0 Manager-level tests | Accepted as test-debt; tracked separately. |
| 🟠-14 | Missing observability on rotation path | Deferred with 🔴-1. |

### A.4 Trait surface impact

**None.** `Resource` trait, `Manager` API, associated types unchanged.

### A.5 Migration impact (per consumer)

| Consumer | Change |
|---|---|
| `nebula-action` | None (source). |
| `nebula-sdk` | None (source). |
| `nebula-engine` | None (source). |
| `nebula-plugin` | None (source). |
| `nebula-sandbox` | None (source). |

Doc-only PR — no source changes beyond SF-2.

### A.6 Breaking changes

**None at source level.** SF-2 is an internal bug-fix. SF-1 is a CI-only enforcement (may surface latent dependency violations, but those would be bugs). `#[doc(hidden)]` on `AcquireIntent::Critical` / `AcquireOptions::{intent,tags}` is not a breaking change in the Rust API sense but is a user-visible surface reduction in rustdoc.

### A.7 Risk profile

**Low-risk, but the real risk is what Option A leaves on the table:**

- 🔴-1 (silent revocation drop) remains in production code. Security-lead's position (`02-pain-enumeration.md:229-231`) is that rotation dispatcher + reverse-index write must land atomically — Option A leaves them absent atomically, which is functionally equivalent to "no rotation" today. **Security-lead acceptability of A is the open question this round.**
- 🔴-2 / 🔴-6 documented-as-broken. DX-tester's Phase 2 input (`02-pain-enumeration.md:239-242`) accepts this only if paired with an explicit "future work" ticket.
- Docs written against `Auth` will need a second pass if the follow-up project reshapes the trait. **Per `feedback_incomplete_work.md`: do not write docs twice.** Option A implies a confidence bet that `Auth` stays.

**Validation required:** none — all fixes are mechanical. No spike needed.

### A.8 Spike scope (if any)

Not applicable.

### A.9 Artefact count

- Strategy Document: small (1 doc, covers doc-rewrite scope + SF list).
- ADR: 0 (no architectural decision — cascading out).
- Tech Spec: not required (no trait change).
- Sub-specs: 0.
- Total: ~1-2 documents.

### A.10 Budget estimate

**~1 day** agent-effort (Phase 3 Strategy 2h, Phase 4 skipped, Phase 5 doc rewrite 4-6h, Phase 6 CP review 2h, Phase 7 ratification 1h, Phase 8 post-mortem 1h).

---

## Option B — Targeted (credential seam + Daemon extraction + file split)

### B.1 Summary

Reshape the `Resource` trait to match credential Tech Spec §3.6 (`type Credential`, `on_credential_refresh` hook). Extract Daemon + EventSource from the crate (canon §3.5 alignment). Split `manager.rs` file (keep `Manager` type monolithic). Rewrite docs against the new shape. Wire SF-2 atomically inside the Manager refactor. Do NOT touch `Runtime`/`Lease` collapse or `AcquireOptions::intent` wiring.

### B.2 In scope

| # | Finding | Treatment |
|---|---|---|
| 🔴-1 | Credential×resource seam | **Adopt Tech Spec §3.6 verbatim.** `Resource` trait gains `type Credential: Credential`, `on_credential_refresh(&self, new_scheme) -> Result<(), Self::Error>` default-noop method. `type Auth` is removed. `Manager::on_credential_refreshed` / `on_credential_revoked` replaced by per-resource dispatch to `on_credential_refresh`; reverse-index is populated at register time from `type Credential` metadata. Ships atomically: trait reshape + reverse-index write + dispatcher + rotation observability (trace span + counter + event per `feedback_observability_as_completion.md`). |
| 🔴-2 | Daemon no public start path | **Extract Daemon from `nebula-resource` crate.** Daemon becomes part of engine/scheduler layer (tech-lead preview `02-pain-enumeration.md:224-227` + rust-senior / canon §3.5 concurrence `02-pain-enumeration.md:85-88`). `TopologyTag::Daemon` variant removed; `DaemonRuntime`, `DaemonConfig`, `topology/daemon.rs`, `runtime/daemon.rs` deleted. Migration path: provide minimal "daemon-like" primitive in engine for the one in-tree consumer, or delete if no real consumer exists. |
| 🔴-3 | Docs fabrication | Rewrite `api-reference.md`, `adapters.md`, `README.md`, `dx-eval-real-world.rs` against new `Resource` (with `type Credential`). Include worked example of `on_credential_refresh` blue-green swap. Compile-test `dx-eval-real-world.rs` in CI. |
| 🔴-4 | Drain-abort phase corruption | Fold SF-2 into Manager refactor PR; wire `ManagedResource::set_failed()` in `Abort` path. |
| 🔴-5 | `Resource::Auth` dead weight | Resolved by 🔴-1 treatment (`type Auth` removed). |
| 🔴-6 | EventSource orphan surface | **Extract EventSource same as Daemon.** Same justification. |
| 🟠-7 | `_with` builder anti-pattern + 2101 L file | **File-split only** per tech-lead preview (`02-pain-enumeration.md:82`). Extract `options` / `shutdown` / `register` submodules from `manager.rs`. `_with` methods remain on `Manager`. Type stays monolithic. |
| 🟠-9 | Daemon + EventSource out-of-canon | Resolved by 🔴-2 + 🔴-6 extraction. |
| 🟠-10 | No `deny.toml` wrappers rule | Ship SF-1 separately (devops). |
| 🟠-12 | `register_pooled` requires `Auth = ()` | Resolved implicitly: after Auth removal, `register_pooled` takes `R: Pooled` (and thus `R: Resource` with `type Credential`). For credential-less resources, `type Credential = NoCredential;` is the pattern. |
| 🟠-14 | Missing observability on rotation path | Included in 🔴-1 (atomic: trait + dispatcher + trace/counter/event). |
| 🟠-15 | `Credential`/`Auth` doc contradiction | Resolved by adopting §3.6 naming uniformly. |

### B.3 Out of scope

| # | Finding | Where it goes |
|---|---|---|
| 🟠-8 | Reserved-but-unused public API (`AcquireOptions::intent/.tags`, etc.) | **Sub-spec — Phase 3 Strategy to propose deferred decision.** Interim: mark `#[doc(hidden)]` or `#[deprecated]` to stop the false-capability signal without committing to a semantics. |
| 🟠-11 | `Runtime == Lease` 9/9 | Deferred. Accepted as friction; a future cascade may collapse. |
| 🟠-13 | Transport: 0 Manager-level tests | Accepted as test-debt. Phase 5 may add minimum coverage if it surfaces naturally in refactor. |
| 🟡-16..22 | Secondary Rust/security/tech-lead findings | Tracked in a follow-up issue list; Phase 3 Strategy acknowledges without committing. |

### B.4 Trait surface impact

**`Resource` trait before → after:**

```rust
// Before (crates/resource/src/resource.rs:220-234)
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: ...;
    type Auth: AuthScheme;  // <-- dead weight
    // ...
}

// After (per Tech Spec §3.6 + Option B)
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: ...;
    type Credential: Credential;  // <-- replaces Auth

    // create() signature changes: now takes &Self::Credential::Scheme
    // instead of &Self::Auth
    fn create(&self, config: &Self::Config,
              scheme: &<Self::Credential as Credential>::Scheme,
              ctx: &ResourceContext) -> impl Future<...>;

    /// Default no-op. Connection-bound resources override for blue-green swap.
    fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme)
        -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
    // ... (other methods unchanged)
}
```

**`Manager` API:**
- `Manager::on_credential_refreshed` / `on_credential_revoked` bodies replaced by real dispatch (no more `todo!()`).
- `credential_resources` reverse-index **populated** at register (`manager.rs:370` `credential_id: None` → `credential_id: Some(R::Credential::id())` or equivalent metadata extraction).
- `register_pooled_with` etc. lose `R: Resource<Auth = ()>` bound; gain `R: Resource<Credential = NoCredential>` or similar marker (or remove the _with surface entirely — see B.7 risk).
- `register_daemon` / `register_event_source` helpers not added (extraction removes the need).

### B.5 Migration impact (per consumer)

| Consumer | Change |
|---|---|
| `nebula-action` | Re-export update only (prelude includes `Resource`, now with `type Credential`). No impl sites. |
| `nebula-sdk` | `pub use nebula_resource::Resource;` unchanged path; shape of trait re-exported is new. SDK prelude consumers see new trait if they write custom `impl Resource for ...` — no in-tree SDK impls found. |
| `nebula-engine` | `Manager` usage in `resource_accessor.rs:39, 104` and test `resource_integration.rs` unchanged at API level. Engine gains/loses whatever Daemon/EventSource extraction produces (TBD in Strategy). |
| `nebula-plugin` | Uses `AnyResource` (object-safe), not `Resource`. No breaking change from trait reshape. Extraction of Daemon/EventSource may drop items from plugin's resource iterator (`registry.rs:115, 149`) — needs verification. |
| `nebula-sandbox` | TBD — grep shows `nebula-resource` dep declared but no `impl Resource` site located in phase scan. Strategy to verify. |

**Zero in-tree `impl Resource for ...` in non-test code** per the grep evidence (`Grep impl Resource for` returned only test files + runtime submodules). This means the `Auth → Credential` type-surface change hits **tests + docs + plugin object-safety** but no production impl sites in consumers. Migration cost is lower than it appears.

### B.6 Breaking changes

1. `Resource::Auth` associated type removed → `Resource::Credential` added. All `impl Resource for ...` sites updated.
2. `Resource::create` signature changes: `auth: &Self::Auth` → `scheme: &<Self::Credential as Credential>::Scheme`.
3. `Manager::on_credential_refreshed` / `on_credential_revoked` return types may change (still `Result<_, Error>` but semantics meaningful now).
4. `TopologyTag::Daemon`, `TopologyTag::EventSource` variants removed.
5. `DaemonRuntime`, `EventSourceRuntime`, related config/topology types deleted from `nebula-resource`.
6. `register_daemon_*`, `register_event_source_*` helpers never added (anti-breaking: they didn't exist, so no removal).
7. `ManagedResource::topology` visibility unchanged (`pub(crate)`) — no externally breaking.
8. Doc re-writes (not Rust-breaking but user-visible).

Per `feedback_hard_breaking_changes.md`: breaking is acceptable given `frontier` MATURITY and 5 in-tree consumers. All listed above are captured in a single migration PR (tech-lead preview `02-pain-enumeration.md:227`).

### B.7 Risk profile

**Primary risks:**

1. **`Resource::Credential = NoCredential` pattern may not cleanly replace today's `Auth = ()`.** Needs spike (see B.8). If the credential-less case can't be expressed idiomatically, fallback is to introduce an `AuthenticatedResource: Resource` sub-trait (non-cascade consulting Question #1 in `02-pain-enumeration.md:245`) — this changes Option B's shape mid-flight.

2. **Daemon/EventSource extraction may surface "but engine was relying on ..."** — Phase 3 Strategy must do the actual grep across `crates/engine/` to confirm no engine integration depends on `DaemonRuntime`. If engine does depend on them, extraction expands scope (need a replacement primitive in engine).

3. **Manager rotation dispatcher must handle: (a) write reverse-index at register, (b) read + dispatch at refresh/revoke, (c) observe (trace span + counter + event), (d) emit `HealthChanged { healthy: false }` on revocation.** This is mechanically more than SF-2 but less than a new type — budget-wise tractable inside Phase 5.

4. **Rotation dispatch concurrency: per-resource `on_credential_refresh` calls may run in parallel across many resources sharing one credential**, or serial? Tech Spec §3.6 doesn't specify (`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996`). Phase 3 Strategy must decide; Phase 4 spike should probe.

**What requires spike validation:** the `type Credential` ergonomics for credential-less resources (the 9/9 of today's test resources that set `type Auth = ();`). Spike should probe: does `type Credential = NoCredential;` compile cleanly in all topology runtimes? Does the blue-green swap pattern compose with `PooledRuntime` without new adapter layers?

### B.8 Spike scope

**Single focused spike (Phase 4):**

**Goal:** validate that `Resource::Credential` (adopted from Tech Spec §3.6) cleanly supports the full Option B topology surface (Pool, Resident, Service, Transport, Exclusive) with both credential-bearing and credential-less resources.

**Probe:**

1. Construct a minimal `Resource` impl with `type Credential = NoCredential` for each of the 5 retained topologies. Confirm `register_pooled`, etc., accept it without `Auth = ()` bound.
2. Construct a `Resource` impl with `type Credential = PostgresConnectionCredential` (per Tech Spec §3.6 example). Override `on_credential_refresh` with a blue-green swap. Drive a rotation through `Manager::on_credential_refreshed` end-to-end. Confirm: reverse-index populated at register, dispatcher fires the hook, observability emits (span + counter + event).
3. Concurrency: when two resources share a credential, confirm Manager dispatches both hooks (parallel or serial — whichever Strategy picks). Confirm one failing hook does not block the other (isolation invariant).

**Exit criteria:** all three probes pass with no new adapter types. If any probe requires adding an adapter, scope expands — escalate to tech-lead for amendment.

**Iteration budget:** max 2 spike iterations per cascade protocol.

### B.9 Artefact count

- Strategy Document (Phase 3): 1 — includes trait reshape rationale, Daemon/EventSource extraction plan, observability spec for rotation.
- ADR: **2** — one for "adopt Tech Spec §3.6 on `Resource` trait (supersede previous `Auth`-shape)"; one for "extract Daemon + EventSource from `nebula-resource`."
- Tech Spec (Phase 5): may be skipped (rotation design already in credential Tech Spec §3.6; resource side is an adoption, not a new spec). Phase 3 Strategy is sufficient.
- Sub-specs: 1 optional — "Deferred: `AcquireOptions::intent/.tags` semantics (future)."
- Total: ~3-4 documents.

### B.10 Budget estimate

**~3 days** agent-effort. Phase 3 Strategy (4-6h) → Phase 4 Spike (4-8h with 1 iteration; 8-16h worst case with 2 iterations) → Phase 5 Tech Spec adoption + doc rewrite (8-12h) → Phase 6 CP review (4h) → Phase 7 ratification (2h) → Phase 8 post-mortem (1h).

---

## Option C — Comprehensive (full trait reshape + Runtime/Lease collapse + AcquireOptions resolution)

### C.1 Summary

Everything in Option B, plus: collapse `Runtime`/`Lease` distinction (rust-senior proposal `02-pain-enumeration.md:235-237`), wire `AcquireOptions::intent/.tags` to documented semantics or remove the fields entirely, reconsider Service/Transport merge if spike evidence supports it. Commits to migrating all 5 in-tree consumers in a single PR wave.

### C.2 In scope

Everything in B.2, plus:

| # | Finding | Treatment |
|---|---|---|
| 🟠-8 | Reserved-but-unused public API | **Resolved one way or the other, not deferred.** Two sub-options within C: C.8a **remove** `AcquireIntent::Critical`, `AcquireOptions::intent`, `AcquireOptions::tags` entirely (argues: engine can re-add when it actually reads them; today's surface is false-capability); C.8b **wire minimal semantics** — `intent: AcquireIntent` drives deadline scaling or queue priority, `tags` propagate to trace spans. Strategy picks. |
| 🟠-11 | `Runtime == Lease` 9/9 | **Collapse via default: `type Lease = Self::Runtime` as associated type default**, effectively making `Lease` an override point for resources that genuinely distinguish. (Rust stable supports associated-type defaults in traits.) Tests continue to work; 9/9 `Lease = Runtime` lines become removable. |
| 🟠-22 | Service vs Transport thin differentiation | **Not merged unconditionally.** Strategy examines: does Transport = Service + keepalive + max_sessions warrant its own topology, or fold? Spike evidence from B.8 may reveal. If merged, `TopologyTag::Service`/`Transport` collapse into one; `register_transport` either becomes alias or is removed. |
| 🟡-24 | `ResourceMetadata` `#[non_exhaustive]` with one field | Accepted as-is; fold into Strategy narrative. |

### C.3 Out of scope

| # | Finding | Where it goes |
|---|---|---|
| 🟠-13 | Transport: 0 Manager-level tests | Accepted test-debt — but if Service/Transport merge happens, Transport tests are subsumed into Service test surface (the debt evaporates). |
| 🟡-17..21 | Secondary findings | Tracked. |

### C.4 Trait surface impact

Everything in B.4, plus:

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static = Self::Runtime;  // <-- default
    type Error: ...;
    type Credential: Credential;
    // ...
}
```

And possibly:

- `AcquireOptions::intent` + `.tags` removed entirely (C.8a), or wired (C.8b — Strategy picks).
- `TopologyTag::Service` + `TopologyTag::Transport` possibly merged into `TopologyTag::Service` with an optional `keepalive` config (if evidence supports).

### C.5 Migration impact (per consumer)

Same consumer set as B.5. Additional change surface:

- If 🟠-8 → C.8a (remove): any consumer setting `intent` / `tags` has the field removed. Grep confirms **zero in-tree reads of `intent` or `tags`** (`02-pain-enumeration.md:91` — the point of the finding). Breaking change but trivial.
- If `Lease = Runtime` default: all `type Lease = Self::Runtime;` lines in tests and any in-tree impls become redundant and can be removed.
- If Service/Transport merge: `register_transport` becomes an alias or is removed. Today Transport has 0 Manager-level tests (`02-pain-enumeration.md:175`) — one consumer affected is sandbox (TBD, as in B.5).

### C.6 Breaking changes

Everything in B.6, plus:

9. `Resource::Lease` gains default — not source-breaking (adding default is additive), but semantically couples `Lease == Runtime` as the expected case.
10. `AcquireOptions::intent` + `.tags` either removed (C.8a) or gain enforced semantics (C.8b).
11. Possibly `TopologyTag::Transport` variant removed (if merged).

### C.7 Risk profile

**Higher risk than B. Expanded surface = more spike validation and more chance for scope creep.**

1. **Associated-type default for `Lease`:** Rust's associated-type defaults have known interaction edges with `where` clauses. Spike must confirm no topology's `where R::Lease: ...` bound is broken.
2. **`AcquireOptions::intent` C.8a vs C.8b is a contested decision in miniature.** If tech-lead + security-lead split on whether to remove or wire, this becomes its own co-decision sub-protocol inside Phase 3.
3. **Service/Transport merge has compound risk** — tech-lead flagged the differentiation as "defensible but thin" (`02-pain-enumeration.md:139`), not "definitely remove." Merging without strong evidence trades one refactor for another.
4. **All-5-consumer PR wave is bigger**; mistakes more likely. Per `feedback_bold_refactor_pace.md` this is acceptable if executed in one pass with compiler trust at milestones — but the budget assumes no re-review rounds on the migration PR itself.

**What requires spike validation:** (a) `type Lease = Self::Runtime` default clean through all 5 topologies; (b) `AcquireOptions` surface reconciliation evidence; (c) Service/Transport merge feasibility (if pursued).

### C.8 Spike scope

Everything in B.8, plus:

- Probe 4: `type Lease = Self::Runtime` default across all 5 topologies in minimal impls. Does `where R::Lease: ...` on `acquire_*` still compose, or need explicit override?
- Probe 5 (if C.8b): model `AcquireIntent` wiring to trace spans; confirm it composes with existing resilience policy.
- Probe 6 (optional): prototype Service+Transport merge; compare code reduction vs behaviour preservation.

**Iteration budget:** max 2 spike iterations. If Probe 6 is contentious, fall back to Option B' (B + Runtime/Lease only) mid-flight — this is a permitted descope but must be flagged.

### C.9 Artefact count

- Strategy Document: 1 (larger than B — covers wider scope).
- ADR: **3** — B's 2 plus one for "Runtime/Lease collapse via default" (or "Service/Transport merge" if that's chosen).
- Tech Spec: optional — if trait shape plus `AcquireOptions` semantics warrant it, a short resource-side Tech Spec; otherwise Strategy + ADRs are sufficient.
- Sub-specs: 0-1.
- Total: ~4-6 documents.

### C.10 Budget estimate

**~4-5 days** agent-effort. Phase 3 Strategy (8h, broader scope) → Phase 4 Spike (8-16h with 2 iterations) → Phase 5 Tech Spec + all-5-consumer migration (12-20h) → Phase 6 CP review (4-6h) → Phase 7 ratification (2h) → Phase 8 post-mortem (1h). **Bumps up against the 5-day envelope.** If ratification forces a second iteration of any Phase, escalates.

---

## Comparison matrix

| Axis | A — Minimal | B — Targeted | C — Comprehensive |
|---|---|---|---|
| 🔴 findings addressed | 2 / 6 (🔴-3 docs, 🔴-4 drain) | 6 / 6 | 6 / 6 |
| 🟠 findings addressed | 2 / 9 (🟠-10 via SF-1, 🟠-15 implicit) | 7 / 9 | 9 / 9 |
| 🟡/🟢 findings addressed | 0 | 0-1 | 1-3 |
| Trait shape change | None | `Auth → Credential` + `on_credential_refresh` | + `Lease = Runtime` default |
| Manager API change | None | Rotation dispatcher real + file-split | + possibly `_with` removal / topology merge |
| Breaking PR count | 0 (doc-only + SF-1/SF-2 standalone) | 1 (one bundled migration) | 1 (larger bundled migration) |
| Daemon/EventSource fate | Stay as-is, documented broken | Extracted from crate | Extracted from crate |
| Spike required | No | Yes — 3 probes | Yes — 6 probes, 2 iterations |
| Tech-lead preview endorsement | partial (aligns with "migration = one PR" only) | ≈ matches preview 1:1 | beyond preview |
| Security-lead acceptability | **open question — leaves 🔴-1 in prod** | likely YES (atomic seam fix = security-lead ask) | likely YES (same as B plus wider surface) |
| Rust-senior preview endorsement | none (doesn't address `_with` or `Runtime`/`Lease`) | partial (addresses seam + file split; doesn't touch collapses) | ≈ matches preview |
| DX-tester preview endorsement | partial (docs fixed; trait still asymmetric for Daemon/ES) | full (Daemon gap removed via extraction, docs align) | full |
| Budget (agent-days) | ~1 | ~3 | ~4-5 |
| Risk of scope creep mid-flight | Very low | Medium (one spike boundary) | High (multiple spike decisions compose) |
| Documents produced | 1-2 | 3-4 | 4-6 |

---

## Open questions for co-decision review

These require tech-lead + security-lead adjudication before scope can lock. Raised per phase-2 protocol.

1. **Is Option A acceptable to security-lead?** 🔴-1 (silent revocation drop) stays in production code under Option A. Security-lead preview said "rotation dispatcher + reverse-index write must land atomically" — does "atomically, eventually in a follow-up project" satisfy this, or does security-lead block A on the grounds that the production-observable gap is unacceptable? **This is the primary gate question for this round.**

2. **Daemon/EventSource extraction — does engine actually consume them?** Tech-lead preview recommends extraction. Phase 3 Strategy needs a grep across `crates/engine/` confirming no live dependency on `DaemonRuntime` / `EventSourceRuntime`. If engine does consume, extraction expands Option B scope (need replacement primitive in engine). **Blocks Option B unless confirmed now.** (Architect flag: partial evidence in phase-1 shows engine consumers use `Manager` directly; no grep hits on `DaemonRuntime` in engine in the phase scan. But this was not exhaustive.)

3. **In Option B, if `type Credential = NoCredential` is not ergonomic for credential-less resources, do we fall back to `AuthenticatedResource: Resource` sub-trait?** This is Phase 1 open question #1 (`02-pain-enumeration.md:245`). Tech-lead preview picks "drop `Auth`, add `AuthenticatedResource` sub-trait with `type Credential`". Architect drafted B as "drop `Auth`, add `type Credential` on `Resource` directly with `NoCredential` default for auth-less" per §3.6. **Is the §3.6 shape preferred, or the sub-trait shape?** Spike probe 1 validates the former; sub-trait is fallback.

4. **In Option C, `AcquireOptions::intent/.tags` — C.8a (remove) or C.8b (wire semantics)?** If Option C is picked, this becomes a sub-decision inside Strategy. Engine owner's position matters (#391 referenced in `crates/resource/src/options.rs:18-22` is an engine-side ticket). Architect leans C.8a (remove; re-add with real semantics when engine needs them) per `feedback_incomplete_work.md` + canon §4.5. Would defer to tech-lead if Option C is selected.

5. **In Option C, Service/Transport merge — do we pursue or defer?** Tech-lead flagged differentiation as "defensible but thin" — not "definitely merge." Adding merge investigation expands spike scope (Probe 6) and carries risk if evidence is inconclusive. **Recommendation: defer merge to a future cascade**; C includes Runtime/Lease collapse + AcquireOptions resolution but leaves Service/Transport as-is. Co-decision should confirm or amend.

6. **Rotation dispatch concurrency model (affects B and C equally).** When a credential is shared by N resources, does `Manager::on_credential_refreshed` dispatch N `on_credential_refresh` hooks in parallel or serial? Tech Spec §3.6 doesn't specify. Security-lead's stance? Isolation invariant (one failing hook doesn't block others) should hold either way. **Phase 3 Strategy to decide; flagging here as an architectural commitment tech-lead + security-lead should be aware of.**

---

## Recommendation (architect, not decision)

**Option B** — targeted scope. It is the smallest option that addresses the primary driver (credential×resource seam) atomically, satisfies security-lead's "atomic rotation dispatcher" requirement, aligns 1:1 with tech-lead's priority-call preview, fits in the 5-day envelope with margin, and leaves a clean surface for a future cascade to collapse `Runtime`/`Lease` and resolve `AcquireOptions` without forcing those decisions now.

Option A is dismissed on security-lead grounds (open question 1) unless they accept the follow-up-project framing. Option C is a good option but adds complexity that the evidence does not yet force — Runtime/Lease collapse can wait a quarter without harm, and AcquireOptions resolution is better done when the engine integration ticket (#391) actually has a design behind it.

**Final decision rests with tech-lead (priority-call) + security-lead (gate). This document frames.**

---

## Artefact references

| Artefact | Path |
|---|---|
| This doc | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md` |
| Phase 0 ground truth | `.../01-current-state.md` |
| Phase 1 pain enumeration (canonical) | `.../02-pain-enumeration.md` |
| Cascade log | `.../CASCADE_LOG.md` |
| Credential Tech Spec §3.6 | `docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996` |
| `Resource` trait (current) | `crates/resource/src/resource.rs:220-299` |
| `Manager` rotation methods (todo!() today) | `crates/resource/src/manager.rs:1360-1401` |
| Reverse-index field (declared, never written) | `crates/resource/src/manager.rs:262`, init `:293`, register hardcodes None `:370` |
| `ManagedResource::set_failed` (dead-coded SF-2 fix) | `crates/resource/src/runtime/managed.rs:95` |
| `AcquireOptions::intent/.tags` (reserved) | `crates/resource/src/options.rs:17-64` |
| `ManagedResource.topology` pub(crate) barrier | `crates/resource/src/runtime/managed.rs:35` |
| `_with` surface (5 variants) | `crates/resource/src/manager.rs:561, 597, 627, 659, 691` |

---

*End of draft. Awaiting tech-lead priority-call + security-lead gate. Max 3 rounds per protocol. If co-decision converges this round, Phase 3 Strategy drafting begins against the locked option.*
