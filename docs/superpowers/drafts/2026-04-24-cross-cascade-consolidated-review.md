---
name: Cross-cascade consolidated review (action × credential × resource)
status: review-complete
date: 2026-04-26
authors: [architect (consolidated review pass)]
scope: Pre-implementation cross-cascade integrity check across the three frozen Tech Specs and 6 ADRs landed across credential / resource / action redesign cascades
inputs:
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md (FROZEN CP3)
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (FROZEN CP4 + Q1+Q6+Q7+Q8 amendments)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md (CP6 frozen)
  - docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md
  - docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md (FROZEN CP4 2026-04-25)
  - docs/superpowers/specs/2026-04-24-nebula-resource-redesign-summary.md
  - docs/adr/0035-phantom-shim-capability-pattern.md (proposed; amended 2026-04-24-B / 2026-04-24-C)
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md (accepted 2026-04-24)
  - docs/adr/0037-daemon-eventsource-engine-fold.md (accepted 2026-04-25 amended-in-place)
  - docs/adr/0038-action-trait-shape.md (accepted 2026-04-25)
  - docs/adr/0039-action-macro-emission.md (accepted 2026-04-25 amended-in-place)
  - docs/adr/0040-controlaction-seal-canon-revision.md (proposed pending user)
  - docs/tracking/cascade-queue.md
posture: review-only — no Tech Spec or ADR modifications proposed; no amendment text drafted; no cascade-queue edits
---

# Cross-cascade consolidated review

## §0 Reading map

This review evaluates **5 composition seams** across 3 cascades (action, credential, resource) using 6 ADRs as the architectural binding. Severity tags per cascade prompt: 🔴 STRUCTURAL (blocks implementation), 🟠 INCOMPLETE (design works, doc gap), 🟡 MINOR (cross-ref improvement), 🟢 COMPOSE-CLEAN (verified end-to-end).

---

## §1 Seam 1 — action × credential (ADR-0035 phantom-shim composition)

### §1.1 Verdict — 🟢 COMPOSE-CLEAN with one 🟡 cross-ref drift

The phantom-shim composition is structurally sound across all three documents. Macro emission contract closes ADR-0035 §4.3 action-side rewrite obligation cleanly.

### §1.2 Findings

**§1.2.1 — 🟢 ADR-0035 §4.3 obligation discharged.** ADR-0038 §Decision item 4 + ADR-0039 §1 + action Tech Spec §4.1.1 + §4.3 + §4.5 jointly emit the `CredentialRef<dyn ServiceCapability>` → `CredentialRef<dyn ServiceCapabilityPhantom>` rewrite per ADR-0035 canonical form. Verified: action Tech Spec line 1455 (`CredentialRef<dyn ServiceCapability<GitHub, Bearer>>` rewrite to `CredentialRef<dyn ServiceCapabilityPhantom<GitHub, Bearer>>`) + line 1521 (post-amendment `SlotType::Concrete { type_id }` matching enum mirror to credential Tech Spec §9.4 line 2452 / §15.8 line 3522).

**§1.2.2 — 🟢 SlotBinding shape aligned across crates post-Q7/Q8 amendments.** Action Tech Spec §3.1 line 1097-1131 declares `SlotBinding { field_name: &'static str, slot_type: SlotType, resolve_fn: ResolveFn }` with `SlotType` three-variant matching pipeline (`Concrete { type_id }`, `ServiceCapability { capability, service }`, `CapabilityOnly { capability }`) — exact mirror of credential Tech Spec §9.4 line 2452 + §15.8 supersession (line 3522). ADR-0039 §1 amended-in-place 2026-04-25 to fold capability into `SlotType` variants matches the post-supersession credential authoritative shape; pre-amendment shape (separate `capability` field) was the divergence; amendment closed it.

**§1.2.3 — 🟢 HRTB resolve_fn shape verbatim across crates.** Action Tech Spec §3.2 line 1149-1153 `ResolveFn = for<'ctx> fn(ctx: &'ctx CredentialContext<'ctx>, key: &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>` matches credential Tech Spec §3.4 line 869 verbatim. Same shape as `RefreshDispatcher::refresh_fn` per credential Tech Spec §7.1 line 1834-1842. Compositional cross-ref is honest.

**§1.2.4 — 🟢 SchemeGuard<'a, C> RAII flow lifetime-pinned.** Action Tech Spec §7.2 line 2087-2091 cites credential Tech Spec §15.7 line 3394-3429 + §15.7 line 3503-3516 (iter-3 lifetime-pin refinement) verbatim. Action's adapter execute path per §3.2 step 5-7 wraps `SchemeGuard::engine_construct(scheme, &'a credential_ctx)` with shared `'a` between guard and credential context per credential Tech Spec §15.7 iter-3. Cancellation-zeroize invariant per action Tech Spec §3.4 + §6.4 mirrors credential Tech Spec §15.7 line 3412 Drop ordering.

**§1.2.5 — 🟢 ctx.resolved_scheme(&self.<slot>) call site through credential vocabulary.** Action Tech Spec §6.2.2 line 1900 + §11.2 + §9.3.1 commits `ctx.resolved_scheme(&CredentialRef<C>) -> Result<&SchemeGuard<'a, C>, ResolveError>` as the action-author surface. Credential Tech Spec §3.4 line 916-925 documents the action body sees `&Scheme` directly via `Deref` — no `&dyn Phantom` exposed at action body. End-to-end seam covered.

**§1.2.6 — 🟢 Q7 R6 sealed-DX peer trait restoration aligns with credential vocabulary.** Action Tech Spec §2.6 line 687-749 declares `WebhookAction` and `PollAction` as **peers of TriggerAction** (not subtraits) per production reality — both decorate `Action + Send + Sync + 'static` not `TriggerAction`, both consume CredentialRef-shape fields per zone-rewriting (§4.1.1). This composes with credential Tech Spec §3.4 dispatch narrative without modification — credential vocabulary is field-shape-level, not trait-shape-level.

**§1.2.7 — 🟢 Q8 F2 idempotency_key hook composes through engine cluster-mode placeholder.** Action Tech Spec §2.2.3 line 316-321 + §3.7 line 1357-1411 declares `IdempotencyKey` type adjacent to `TriggerAction::idempotency_key()` + four engine cluster-mode trait placeholders (`CursorPersistence`, `LeaderElection`, `ExternalSubscriptionLedger`, `ScheduleLedger`). Credential Tech Spec is unaffected (this is action-side hook, engine-side consumer). Composition is doc-only-contract through engine cascade slot 2.

**§1.2.8 — 🟡 Cross-crate flagged amendments to credential Tech Spec — 2 outstanding.** Action Tech Spec §15.3 + §15.4 flag two credential Tech Spec soft amendments:
- §15.3 — credential Tech Spec §16.1.1 probe #7 should adopt qualified-syntax form per ADR-0039 §3 (auto-deref Clone shadow per spike finding #1). Current credential Tech Spec line 3756 still uses unqualified `let g2 = guard.clone()` form (silent-pass risk).
- §15.4 — credential Tech Spec §15.7 should add `engine_construct_with_probe` test-only constructor variant per CP2 §6.4.2 ZeroizeProbe choice.

Both flagged as "FLAGGED, NOT ENACTED" pending credential Tech Spec author cross-section pass coordination. Until enacted, action-side probe (per action Tech Spec §5.4) catches the violation independently. **Implementation is unblocked**; the flagged amendments are documentation hygiene, not structural blockers.

### §1.3 Citations

- ADR-0035 §1 canonical form (line 67-108) + §2 Pattern 4 lifecycle erasure (line 130-164) + §4.3 action-side rewrite obligation (line 258-260)
- Credential Tech Spec §3.4 line 807-939 (Pattern 2 dispatch narrative); §9.4 line 2452 + §15.8 line 3522 (`SlotType` matching pipeline); §15.7 line 3394-3516 (`SchemeGuard`/`SchemeFactory` + iter-3 lifetime-pin); §16.1.1 line 3756 (probe #7 — flagged amendment target)
- Action Tech Spec §2.1.1 line 143-148 (`ActionSlots` companion); §3.1 line 1088-1135 (`SlotBinding` registry); §3.2 line 1141-1165 (HRTB dispatch); §3.4 line 1216-1232 (cancellation-zeroize); §4.1.1 line 1449-1457 (zone shapes); §4.3 line 1499-1521 (per-slot emission); §6.4 line 2004-2052 (cancellation-zeroize tests); §7.2 line 2083-2099 (SchemeGuard RAII); §15.3 + §15.4 (flagged amendments)
- ADR-0038 line 46-58 (decision narrative); ADR-0039 line 47-99 (post-amendment `SlotBinding`/`SlotType`)

---

## §2 Seam 2 — action × resource (lifecycle + DI composition)

### §2.1 Verdict — 🔴 STRUCTURAL — `Resource` trait shape divergence between cascades

The `Resource` trait surface visible in the action Tech Spec disagrees fundamentally with the `Resource` trait surface visible in the resource Tech Spec — two structurally-incompatible declarations exist. This is the most consequential cross-cascade gap surfaced by the review.

### §2.2 Findings

**§2.2.1 — 🔴 STRUCTURAL — Resource trait double-declaration.** Three independent `Resource` trait declarations exist across the three cascade documents:

| Source | Trait shape | Associated types | Lifecycle methods |
|---|---|---|---|
| **resource Tech Spec line 157-299** | `pub trait Resource: Send + Sync + 'static` | `Config`, `Runtime`, `Lease`, `Error`, `Credential` (5 types) | `key`, `create`, `check`, `on_credential_refresh`, `on_credential_revoke`, `shutdown`, `destroy`, `schema`, `metadata` (9 methods) |
| **action Tech Spec §2.2.4 line 449-451** | `pub trait Resource: Send + Sync + 'static` | `Credential: Credential` (1 type) | NONE |
| **credential Tech Spec §3.6 line 977-998** | `pub trait Resource` | `Credential: Credential`, `Error: Classify + Send + Sync + 'static` | `create`, `on_credential_refresh` (2 methods) |
| **credential Tech Spec §15.7 line 3418-3429** (CP5 supersession of §3.6) | `pub trait Resource: Send + Sync` | `Credential: Credential`, `Error: std::error::Error` | `on_credential_refresh` (with `SchemeGuard<'a, _>` shape) |

**Action Tech Spec line 449-451** declares a stub `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }` immediately preceding `pub trait ResourceAction { type Resource: Resource; ... }`. This is a redeclaration, not an import, and contradicts the resource Tech Spec's authoritative full-shape declaration.

The resource Tech Spec's `Resource` trait is the **canonical authority** post-FROZEN CP4 ratification — ADR-0036 §Decision (line 64-95) commits the 5-assoc-type + 9-method shape; resource Tech Spec §1.4 line 99 line-locks "Full Rust signature for the `Resource` trait in §2.1 — every method, every associated type, every default body" as CP1 ratification gate; CP4 freeze ratifies.

**Implementation collision.** When implementer reads action Tech Spec §2.2.4 + §3.5 (typification narrative) and tries to write `impl Resource for PostgresPool { type Credential = ...; }` per the **action Tech Spec's stub shape**, they will hit `error[E0046]: not all trait items implemented, missing: Config, Runtime, Lease, Error, key, create, check, on_credential_refresh, on_credential_revoke, shutdown, destroy` once they import the resource crate. The action Tech Spec's stub trait declaration **does not match the surface implementers will encounter at runtime**.

**N1 acknowledgement does not close the gap.** Action Tech Spec §1.2 N1 says "Resource integration deeper than ADR-0035 §4.3 rewrite obligation [is OUT]" and N1-extended (line 85) says `Resource: Resource` bound creates ordering dependency on credential cascade landing first. But **N1 doesn't actually opt out of the trait shape collision** — the trait `Resource` is *declared* in action Tech Spec §2.2.4 with a 1-assoc-type stub shape that contradicts the cross-cascade authority. N1 says the implementation is delegated; the stub trait declaration in §2.2.4 either needs to be (a) removed and replaced with `use nebula_resource::Resource;` import, or (b) explicitly marked as "minimal forward-declaration; full shape per resource Tech Spec §2.1," or (c) hard-deleted because action's `ResourceAction::Resource: Resource` bound only needs the trait identity, not its full shape.

**Implementation impact.** Pre-implementation, the stub trait will cause **immediate compile failure** when both crates land in the same workspace — `nebula-action::Resource` and `nebula-resource::Resource` are name-collisional. ResourceAction::Resource bound under action Tech Spec resolves `nebula-action::Resource`, while consumer impls (PostgresPool, etc.) implement `nebula-resource::Resource`. Bounds will not unify.

**§2.2.2 — 🟠 INCOMPLETE — `ResourceAction::configure(&self, ctx) -> Self::Resource` paradigm narrative gap.** Action Tech Spec §2.2.4 (post-Q7 R2 amendment) declares `ResourceAction::configure` returning `Result<Self::Resource, Self::Error>` and `cleanup(self.Resource, ctx)`. But where does `Self::Resource: Resource`'s actual `Resource::create(config, scheme, ctx)` get called? Action Tech Spec line 489 says "Consumer actions ALWAYS acquire `SchemeGuard<'a, C>` per request" but doesn't narrate how `ResourceAction::configure` obtains the constructed `Resource::Runtime`.

Resource Tech Spec §2.1 line 185-191 declares `Resource::create(&self, config, scheme, ctx) -> Self::Runtime`. The flow `ResourceAction::configure` → `Resource::create(&Scheme)` → `Self::Runtime` is implicit — engine apparently calls `ResourceAction::configure(...)` which **internally** calls `Self::Resource::create(...)` — but neither cascade documents this composition explicitly. Action Tech Spec §3 runtime model (line 1084+) does not narrate the configure → create → cleanup → destroy lifecycle bridge across the action / resource crate boundary.

**Resolution scope.** The composition is reasonable to infer (`ResourceAction::configure` builds the `Resource::Config`, calls `Self::Resource::create(config, scheme, ctx)`, returns the `Self::Runtime` as `Self::Resource`); but there is no doc-anchor in either Tech Spec that explicitly walks the hand-off. Implementer will derive or ad-hoc this seam.

**§2.2.3 — 🟠 INCOMPLETE — ResourceHandler `Box<dyn Any + Send + Sync>` ⇄ resource topology runtime mapping unspecified.** Action Tech Spec §2.4 line 581-595 (post-Q7 R4) declares `ResourceHandler::configure(.., ctx) -> Box<dyn Any + Send + Sync>` and `cleanup(resource: Box<dyn Any>, ctx) -> Result<(), ActionError>`. Resource Tech Spec §2.4 line 451-498 declares 5 topology sub-traits (`Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`) extending `Resource`. Each topology has its own runtime / lease shape (per resource Tech Spec §2.4 line 482 `Service::TOKEN_MODE: TokenMode = TokenMode::Cloned` etc.).

The mapping from `Box<dyn Any + Send + Sync>` (action handler erasure boundary) to `<R as Resource>::Runtime` (resource topology-specific lease shape) is not specified. Implementer must derive: which adapter downcasts the `Box<dyn Any>` to `<R as Resource>::Runtime`? Where does the topology-specific acquire path (resource Tech Spec §3 Manager-side acquire_pooled/acquire_resident/etc.) interact with action's `ResourceAction::configure` boundary?

**Implementation impact.** Pre-implementation, the `ResourceHandler` ⇄ `Resource` bridging adapter does not have a documented home — it lives on neither action's nor resource's side explicitly. Likely lands in `nebula-engine` per action Tech Spec §11 adapter section, but no engine-side trait surface specifies the bridge.

**§2.2.4 — 🟢 COMPOSE-CLEAN — N1-extended dependency note correctly identifies ordering.** Action Tech Spec §1.2 N1-extended (line 85) explicitly identifies the credential cascade landing first as a precondition for the `ResourceAction::Resource: Resource` bound (because `Resource: Resource` requires `type Credential: Credential` per resource Tech Spec §2.1 + ADR-0036). Cascade-queue.md slot 1 captures this. Path (a) single coordinated PR works implicitly; paths (b)/(c) MUST sequence credential cascade leaf-first per cascade-final precondition. This is documentary-correct.

### §2.3 Citations

- Resource Tech Spec line 157-299 (`Resource` trait full shape — 5 types + 9 methods); §2.4 line 446-498 (5 topology sub-traits)
- Action Tech Spec §2.2.4 line 449-490 (`Resource` stub + `ResourceAction` configure/cleanup); §1.2 N1 line 83-86 (resource integration N1 + N1-extended); §2.4 line 574-596 (`ResourceHandler` `Box<dyn Any>` boundary)
- Credential Tech Spec §3.6 line 977-998 (pre-CP5 `Resource` trait shape); §15.7 line 3418-3429 (CP5 supersession with `SchemeGuard<'a, _>`)
- ADR-0036 §Decision line 64-107 (5-assoc-type + 9-method canonical commitment)
- ADR-0035 §4.3 line 258-260 (action-side rewrite obligation)

---

## §3 Seam 3 — credential × resource (refresh + ownership)

### §3.1 Verdict — 🔴 STRUCTURAL — `on_credential_refresh` signature mismatch between credential CP5 and resource Tech Spec

Credential Tech Spec §15.7 (CP5 supersession of §3.6) commits `on_credential_refresh(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>)` — **owned `SchemeGuard`** with shared `'a` lifetime. Resource Tech Spec §2.1 line 224-230 commits `on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme)` — **borrowed `&Scheme`** with no `SchemeGuard`. These are incompatible.

### §3.2 Findings

**§3.2.1 — 🔴 STRUCTURAL — `on_credential_refresh` parameter shape divergence.** Two incompatible signatures:

| Source | Signature | `Scheme` carrier | Lifetime form |
|---|---|---|---|
| **credential Tech Spec §15.7 line 3422-3428 (CP5 SUPERSEDED §3.6)** | `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>) -> Result<(), Self::Error>` | `SchemeGuard<'a, Self::Credential>` (owned, `!Clone`, `ZeroizeOnDrop`, `Deref`) | Single `'a` — refined per iter-3 (line 3508-3513) to `(new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>)` shared `'a` |
| **resource Tech Spec §2.1 line 224-230** | `async fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> Result<(), Self::Error>` | `&Scheme` (borrowed reference) | No explicit lifetime — implicit elided `&'_ Scheme` |

The credential Tech Spec §3.6 line 970 explicitly states: "**Superseded by §15.7 (CP5 2026-04-24).** Signature below takes `&<Self::Credential as Credential>::Scheme` (borrowed reference). **Canonical CP5 form: `SchemeGuard<'_, Self::Credential>` — owned, `!Clone`, `ZeroizeOnDrop`, `Deref<Target = Scheme>`, lifetime-bound to call.**"

Resource Tech Spec adopted the **pre-supersession** §3.6 borrowed-`&Scheme` shape, NOT the post-supersession §15.7 `SchemeGuard<'a, _>` shape. Resource Tech Spec line 32 cites credential Tech Spec §3.6 lines 928-996 as the authoritative shape — but the cited range is the **superseded** shape per the credential Tech Spec's own §3.6 supersession header.

**ADR-0036 alignment.** ADR-0036 §Decision line 81-84 documents `on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme)` (borrowed `&Scheme` — **matches resource Tech Spec, NOT post-CP5 credential authoritative**). ADR-0036's conceptual signature pre-dates credential CP5 supersession; the ADR's "spec alignment" claim (ADR-0036 line 56-57: "credential Tech Spec §3.6 prescribes a structurally different shape") is correct *for the §3.6 shape it cites*, but credential Tech Spec §15.7 supersedes that shape during credential CP5.

**Implementation impact.** Resource Tech Spec ratifies a `Resource` trait that **violates the credential CP5 SchemeGuard contract**. When implementer writes `impl Resource for PostgresPool { async fn on_credential_refresh(&self, new_scheme: &PostgresConnectionScheme) ... }` per resource Tech Spec, they bypass the `SchemeGuard<'a, _>` zeroize / no-Clone / no-retention discipline that credential Tech Spec §15.7 mandates. The cross-crate compile-fail probes (per credential Tech Spec §16.1.1 probes #6, #7) test against the `SchemeGuard<'a, _>` shape — they may not fire on resource-side `&Scheme` impls.

**Compile-fail probe coverage gap.** Credential Tech Spec §15.7 probe (line 3499) `tests/compile_fail_scheme_guard_retention.rs` tests `Resource` impl that stores `SchemeGuard` in struct field outlasting call. But if Resource trait's `on_credential_refresh` takes `&Scheme` (resource Tech Spec shape) instead of `SchemeGuard<'a, _>` (credential Tech Spec shape), the probe fixture cannot be written — there's no `SchemeGuard` parameter to retain.

**§3.2.2 — 🟢 COMPOSE-CLEAN — `SchemeFactory<C>` ownership boundary correctly placed.** Credential Tech Spec §15.7 line 3438-3447 declares `SchemeFactory<C>` for long-lived resources. Resource Tech Spec doesn't explicitly cite `SchemeFactory`, but the worked example in credential Tech Spec line 3457-3491 (`OAuth2HttpPool` with `bearer_factory: SchemeFactory<MyOAuth2Credential>`) demonstrates the canonical ownership pattern — **resource holds the factory, not the SchemeGuard**. This is consistent with resource Tech Spec's intent (per §2.1.1 line 311-326 blue-green pool swap example), even though resource Tech Spec doesn't name `SchemeFactory` directly.

**§3.2.3 — 🟢 COMPOSE-CLEAN — Revocation hook `&CredentialId` parameter aligned.** Both credential Tech Spec §3.6 line 990-991 (legacy) and resource Tech Spec §2.1 line 255-261 + §2.3 line 436-441 use `on_credential_revoke(&self, credential_id: &CredentialId) -> Result<(), Self::Error>` — borrowed `&CredentialId`, no `SchemeGuard` (revocation has no new scheme to swap). Credential Tech Spec §4.3 line 1062-1068 revocation lifecycle modes consistent with resource-side post-invocation invariant per resource Tech Spec §2.3 line 437. Composition is verified.

**§3.2.4 — 🟢 COMPOSE-CLEAN — RefreshDispatcher / engine refresh source per credential cascade.** Credential Tech Spec §7.1 + §15.6 declares `RefreshDispatcher` engine-side. Resource Tech Spec §3.6 line 978-979 confirms `Manager::on_credential_refreshed` consumes refresh events from credential plane. The dispatch direction is correct (credential plane → resource plane); the parameter shape mismatch in §3.2.1 is the actual gap.

### §3.3 Citations

- Credential Tech Spec §3.6 line 968-998 (legacy shape with supersession header); §15.7 line 3383-3516 (`SchemeGuard`/`SchemeFactory` decision + iter-3 lifetime-pin refinement); §16.1.1 line 3755-3756 (probes #6, #7 — `SchemeGuard` retention + Clone)
- Resource Tech Spec §2.1 line 220-261 (`on_credential_refresh` + `on_credential_revoke` declarations); §2.1.1 line 311-329 (blue-green swap example); §2.3 line 426-444 (invariants); §3.6 line 968-980 (Manager ⇄ todo()-replacement)
- ADR-0036 §Decision line 81-95 (rotation hooks conceptual signatures); §Status line 32 ("credential Tech Spec §3.6 lines 928-996 as the ratified downstream contract")

---

## §4 Seam 4 — all three × ADR-0035

### §4.1 Verdict — 🟢 COMPOSE-CLEAN with one 🟡 sealed-mod naming convention drift

ADR-0035's per-capability inner sealed-trait pattern composes uniformly across credential / action / resource. One minor naming drift exists between cascades' sealed-mod naming.

### §4.2 Findings

**§4.2.1 — 🟢 §3 sealed convention applied uniformly.** Three sealed-mod hosts:
- Credential: `mod sealed_caps { pub trait BearerSealed {} pub trait BasicSealed {} ... }` per ADR-0035 §3 (line 178-185)
- Credential lifecycle: `mod sealed_lifecycle { pub trait RefreshableSealed {} ... }` per ADR-0035 §2 Pattern 4 (line 141-147)
- Action DX: `mod sealed_dx { pub trait ControlActionSealed {} pub trait WebhookActionSealed {} ... }` per ADR-0040 §1 line 56-63 + action Tech Spec §2.6 line 638-647

All three follow the canonical "crate-private outer module + pub-within-scope inner sealed traits" pattern per ADR-0035 §3 amendment 2026-04-24-B. Per-capability inner sealed traits avoid coherence collision when capabilities share supertrait. Resource Tech Spec does not declare its own sealed module (resource trait is sealed-extension via topology sub-traits per §2.4, not via blanket sealed-trait pattern); this is correct — resource doesn't need a sealed module because topology extension is via inheritance, not blanket impl.

**§4.2.2 — 🟢 §4.3 action-side rewrite obligation honored end-to-end.** Per §1.2.1 above. Action Tech Spec §4.1.1 + ADR-0038 §Decision item 4 + ADR-0039 §1 jointly close the obligation.

**§4.2.3 — 🟢 Per-capability sealed pattern composes structurally across all 4 ADR-0035 patterns.** ADR-0035 §2 (line 124-164) names 4 patterns:
- Pattern 1 — concrete `CredentialRef<ConcreteCredential>` (no phantom)
- Pattern 2 — `CredentialRef<dyn ServiceXBearerPhantom>` (phantom + sealed)
- Pattern 3 — `CredentialRef<dyn AcceptsBearerPhantom>` (phantom + sealed)
- Pattern 4 — `Box<dyn RefreshablePhantom>` lifecycle phantom (added 2026-04-24-C)

All four patterns flow through:
- Macro emission per ADR-0039 §1 (capability + service + concrete dispatch)
- Action Tech Spec §3.1 `SlotType` three-variant enum (Concrete / ServiceCapability / CapabilityOnly)
- Credential Tech Spec §9.4 / §15.8 engine-side `iter_compatible` filter
- Cancellation-zeroize per credential Tech Spec §15.7 + action Tech Spec §3.4

Pattern 4 (lifecycle phantom) is unique to engine-side runtime registries (RefreshDispatcher iteration over Refreshable credentials) — composes with action's idempotency_key hook surface per §1.2.7 above through engine cluster-mode placeholder.

**§4.2.4 — 🟡 sealed-mod naming convention drift.** Cascade-level naming:
- credential capabilities: `mod sealed_caps`
- credential lifecycle: `mod sealed_lifecycle`
- action DX: `mod sealed_dx`

Per ADR-0035 §3 line 178 uses `mod sealed_caps` for credential capabilities; the convention "one crate-private mod, named to indicate scope" is honored (each crate gets one module per logical scope). But **plugin authors writing their own capability traits** (per ADR-0035 §3 line 203 "Plugin authors declaring their own capability traits follow the same convention") will see three different naming patterns documented across the workspace. This is a low-priority naming-cohesion observation; ADR-0035 §3 line 203 explicitly says plugin authors maintain their own `mod sealed_caps` (singular), so the convention is "every crate has its own `mod sealed_caps` for its own capability scope." Action's `mod sealed_dx` deviates ("dx" not "caps"). Plugin author guidance could surface "name your sealed mod after the trait family scope" rather than mandating `sealed_caps` always — current docs don't address this consistency question.

**Implementation impact.** Negligible. Each crate's sealed mod is crate-private; cross-crate naming has no compile-time interaction. This is a doc-style observation, not a blocker.

### §4.3 Citations

- ADR-0035 §1 line 67-122 (canonical form, 4 patterns); §2 line 124-164 (scope + Pattern 4 lifecycle); §3 line 167-203 (sealed module placement convention with 2026-04-24-B amendment); §4 line 205-260 (macro emission contract); §4.3 line 258-260 (action-side rewrite obligation)
- Credential Tech Spec §3.4 line 851-870 (capability + sealed dispatch); §15.7 line 3394-3516 (`SchemeGuard` + lifetime-pin)
- Action Tech Spec §2.6 line 631-784 (sealed DX trait family with `mod sealed_dx`); §4.1.1 line 1449-1457 (zone shapes); §4.3 line 1499-1521 (per-slot emission)
- ADR-0038 §Decision item 4 line 56-58 (composition with ADR-0035); ADR-0039 §1 line 49-99 (post-amendment SlotBinding); ADR-0040 §1 line 49-70 (sealed DX via per-capability inner sealed pattern)

---

## §5 Seam 5 — cascade-queue.md slot consistency

### §5.1 Verdict — 🟠 INCOMPLETE — slot 1 trait-shape doesn't account for resource cascade landing

Cascade-queue.md slot 1 lists "Credential CP6 implementation" with architect-recommended shape `CredentialRef<C> / SlotBinding / SchemeGuard<'a, C> / SchemeFactory / RefreshDispatcher per credential Tech Spec CP6`. But the resource cascade has now landed (frozen CP4 2026-04-25) with its own `Resource` trait surface that consumes credential CP6 vocabulary; slot 1's shape doesn't reflect the additional surface obligations the resource cascade imposes.

### §5.2 Findings

**§5.2.1 — 🟠 INCOMPLETE — slot 1 doesn't accommodate resource Tech Spec's authoritative `Resource` trait shape.** Slot 1's recommended shape only enumerates credential-side primitives. But the resource cascade's frozen Tech Spec §2.1 declares the canonical `Resource` trait (5 assoc types + 9 lifecycle methods) that **consumes** credential CP6 primitives in `create(scheme: &<Self::Credential as Credential>::Scheme, ctx)`, `on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme)`, etc. The `Resource` trait surface is part of credential CP6 implementation transitively — when CP6 lands, the resource Tech Spec's `Resource` shape must be implementable against it.

Per Seam 3 finding §3.2.1, the `on_credential_refresh` signature mismatch (resource cascade adopts `&Scheme`; credential CP5/CP6 mandates `SchemeGuard<'a, _>`) means **slot 1 implementation cannot proceed without resolving the parameter-shape mismatch first**. Slot 1's "architect-recommended shape" column does not surface this dependency.

**§5.2.2 — 🟠 INCOMPLETE — slot 2 cluster-mode coordination references action's 4× engine trait placeholders + resource's daemon eventsource (ADR-0037) without reconciliation.** Cascade-queue.md slot 2 says "TriggerAction cluster-mode hooks (`IdempotencyKey`, `on_leader_acquire` / `on_leader_release`, `dedup_window` metadata) per action Tech Spec §2.2.3 cluster-mode trailing prose; engine-side coordination implementation."

Two surface obligations now cross slot 2:
- Action Tech Spec §3.7 line 1357-1411 — 4 doc-only engine trait placeholders (CursorPersistence, LeaderElection, ExternalSubscriptionLedger, ScheduleLedger)
- Resource cascade ADR-0037 + resource Tech Spec §12 — DaemonRegistry + EventSource→TriggerAction adapter

Slot 2 lists action's hooks but doesn't reference the resource cascade's daemon/eventsource extraction obligation. The implementer reading slot 2 sees "cluster-mode coordination" without the Daemon / EventSource engine landing site. Resource Tech Spec §12 commits the engine landing (`crates/engine/src/daemon/`); slot 2 should surface this as part of the same engine-side cascade.

Additionally, **action Tech Spec §3.7's `ExternalSubscriptionLedger` placeholder may overlap with resource cascade's EventSource→TriggerAction adapter** (both deal with external-subscription registration on workers per §2.2.3 webhook lifecycle + §12.3 EventSource adapter pattern). Reviewer cannot tell from slot 2 alone whether these are the same surface or two different ones.

**§5.2.3 — 🟢 COMPOSE-CLEAN — Slots 3-7 (ScheduleAction, EventAction, AgentAction+ActionTool, StreamAction+StreamStage, TransactionAction) correctly reference action Tech Spec §15.12 Q8 deferred-cascade-slot architecture.** Each slot's architect-recommended shape aligns with the trait family taxonomy locked at action Tech Spec §15.12 Q8 Phase 2. Slot 3's "Sealed-DX peer of TriggerAction" shape per ADR-0040 §2 Webhook/Poll precedent is consistent with action Tech Spec §2.6 peer trait design.

**§5.2.4 — 🟢 COMPOSE-CLEAN — Slot 8 (`nebula-auth` Tech Spec cascade) correctly identifies SSO/SAML/OIDC/LDAP/MFA scope as outside action / credential / resource.** Slot 8 captures Q8 Part D outside-scope auth findings; slot is properly architected as separate cascade, not bolted onto credential or action.

**§5.2.5 — 🟢 COMPOSE-CLEAN — Slot governance discipline correctly enforced.** Per cascade-queue.md slot governance (line 35-40), three-field commit (Owner/Date/Position) is required for "committed" status. All 8 slots currently TBD — correctly marked as intent placeholders until commitment. Path-gating decisions correctly cite slots as uncommitted (action Tech Spec §16.1 path (b)/(c) viability gate per Strategy §6.6).

### §5.3 Citations

- cascade-queue.md (entire file — 8-slot table + governance rules)
- Action Tech Spec §1.2 N4 line 91 + §2.2.3 line 290-321 (cluster-mode hook surface) + §3.7 line 1357-1411 (4× engine trait placeholders)
- Resource Tech Spec §12 line 2245-2366 (Daemon + EventSource engine landing)
- ADR-0037 line 23-31 (engine-fold decision); ADR-0036 line 30-34 (cross-cascade coordination)
- Credential Tech Spec §15.7 line 3383-3516 (`SchemeGuard` + factory contract — slot 1 obligation)

---

## §6 Cross-cascade gaps summary

### §6.1 Severity tally

| Severity | Count | Items |
|---|---|---|
| 🔴 STRUCTURAL | **2** | §2.2.1 (Resource trait double-declaration); §3.2.1 (`on_credential_refresh` parameter shape divergence credential CP5 vs resource Tech Spec) |
| 🟠 INCOMPLETE | **4** | §2.2.2 (`ResourceAction::configure` → `Resource::create` lifecycle bridge undocumented); §2.2.3 (`ResourceHandler` `Box<dyn Any>` ⇄ resource topology mapping); §5.2.1 (slot 1 doesn't surface resource cascade dependency); §5.2.2 (slot 2 cluster-mode + daemon/eventsource overlap unreconciled) |
| 🟡 MINOR | **2** | §1.2.8 (2 flagged credential Tech Spec amendments outstanding); §4.2.4 (sealed-mod naming convention drift) |
| 🟢 COMPOSE-CLEAN | **15** | §1.2.1 through §1.2.7; §2.2.4; §3.2.2 through §3.2.4; §4.2.1 through §4.2.3; §5.2.3 through §5.2.5 |

### §6.2 Critical-path summary

The two 🔴 STRUCTURAL gaps both center on the **Resource trait surface**:

1. **Double-declaration.** Action Tech Spec §2.2.4 declares a stub `Resource` trait (1 assoc type) parallel to resource Tech Spec §2.1's authoritative declaration (5 assoc types + 9 methods). Implementation collision is immediate at workspace compile time.

2. **`on_credential_refresh` signature.** Resource Tech Spec adopts the **superseded** §3.6 borrowed-`&Scheme` shape; credential CP5 §15.7 supersedes to owned `SchemeGuard<'a, _>` shape. Resource impls per resource Tech Spec violate the credential CP5 SchemeGuard zeroize/no-Clone/no-retention contract. Compile-fail probe coverage gap.

These two gaps are **inter-related** — both reflect that the resource cascade's frozen `Resource` trait shape was anchored against the **pre-supersession** credential Tech Spec §3.6 shape (per ADR-0036 §Status line 32: "credential Tech Spec §3.6 lines 928-996 as the ratified downstream contract being adopted verbatim"), and the credential CP5/CP6 supersession (§15.7) was not propagated to the resource cascade during co-cascade coordination.

### §6.3 Important pattern observations

**Pattern A — supersession propagation gap.** When credential Tech Spec super-amended §3.6 → §15.7 during CP5 (2026-04-24), resource cascade was either (a) not yet commenced, or (b) commenced but not re-pinned to the post-CP5 shape. ADR-0036 cites the §3.6 line range 928-996 as authority but the post-CP5 §15.7 line 3383-3516 is the real authority. ADR-0036 was accepted 2026-04-24, the same date credential CP5 supersession landed; it is plausible the resource cascade authoring froze the §3.6 shape from a snapshot taken before CP5 supersession.

**Pattern B — stub trait declarations are fragile.** Action Tech Spec §2.2.4's `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }` is an **anti-pattern** — it shadows the canonical authority with a partial shape that compiles but doesn't unify with cross-crate impls. The correct shape would be `use nebula_resource::Resource;` import-only, OR explicit forward-declaration comment naming resource Tech Spec §2.1 as the canonical site.

**Pattern C — composition narrative gaps multiply at boundaries.** §2.2.2 + §2.2.3 + §5.2.1 + §5.2.2 are all "boundary documentation gaps" — the individual cascade documents lock their internal contracts rigorously, but the **inter-cascade composition** (configure→create→cleanup; Box<dyn Any>↔topology Runtime; cascade-queue ⇄ resource cascade dependency; cluster-mode ⇄ daemon overlap) is left to the implementer's inference. Pre-implementation boundary-narrative passes per Tech Spec freeze do not currently include cross-cascade narrative passes.

---

## §7 Implementation readiness verdict

**AMENDMENT-NEEDED** — Two 🔴 STRUCTURAL gaps must close before implementation can begin, and four 🟠 INCOMPLETE gaps SHOULD close before implementation begins to reduce implementer ambiguity.

Path forward (architect's framing — tech-lead decides amendment routing):

### §7.1 Required amendments (🔴 STRUCTURAL — block implementation)

**Amendment R1 — Resource trait single-source-of-truth.** Either:
- **(a)** Action Tech Spec §2.2.4 hard-deletes the stub `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }` declaration; replaces with `use nebula_resource::Resource;` import-only. Preserves `ResourceAction::Resource: Resource` bound; removes parallel-shape declaration.
- **(b)** Action Tech Spec §2.2.4 hard-deletes the stub, OR explicitly marks "minimal forward-declaration; full shape per resource Tech Spec §2.1 (5 assoc types + 9 lifecycle methods); this stub is **NOT** the implementer surface."

(a) is the cleaner option per `feedback_no_shims.md` (no parallel surface). Either lands as Tech Spec amendment-in-place per ADR-0035 amended-in-place precedent.

**Amendment R2 — `on_credential_refresh` signature reconciliation.** Either:
- **(a)** Resource Tech Spec §2.1 + §2.1.1 + ADR-0036 §Decision re-pin to credential Tech Spec §15.7 CP5 SchemeGuard shape: `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> Result<(), Self::Error>`. Updates 5 in-tree consumer-resource impls. Closes credential CP5 SchemeGuard contract violation.
- **(b)** Credential Tech Spec §15.7 CP5 supersession is *itself* re-evaluated — restore the borrowed-`&Scheme` shape OR document why resource-side may use `&Scheme` while action-side uses `SchemeGuard<'a, _>`. (NOT recommended — credential CP5 supersession had iter-3 spike validation per credential Tech Spec §15.7 line 3503-3516; reversal would invalidate the spike.)

(a) is the principled option — credential CP5 has spike-validated `SchemeGuard` shape; resource cascade should re-pin. Resource Tech Spec acknowledges its own gate-pass record (§0.2 line 38) and amendment cycle (§0.3 line 51-57 — "ADR-0036 / ADR-0037 amendment via amended-in-place pattern"); the amendment routing exists.

### §7.2 Recommended amendments (🟠 INCOMPLETE — reduce implementer ambiguity)

**Amendment I1 — `ResourceAction::configure` → `Resource::create` lifecycle bridge narrative.** Action Tech Spec §3 (or new §3.5.x) walks the engine-side composition: `ResourceAction::configure(&self, ctx)` body internally calls `<Self::Resource as Resource>::create(config, scheme, ctx)`, returns the `Self::Resource` (which carries `Self::Resource::Runtime`). Cleanup analog: `cleanup(self.Resource, ctx)` body calls `<Self::Resource as Resource>::destroy(self.Resource.into_runtime(), ctx)`. Doc-only narrative; no signature changes.

**Amendment I2 — `ResourceHandler` `Box<dyn Any + Send + Sync>` ⇄ topology runtime mapping.** Action Tech Spec §2.4 (or new §2.4.x) names the engine-side adapter that bridges `Box<dyn Any>` (action handler erasure) ↔ `<R as Resource>::Runtime` (topology-specific runtime per resource Tech Spec §2.4 sub-traits). Adapter likely lives in `nebula-engine` per action Tech Spec §11; doc-only narrative.

**Amendment I3 — cascade-queue.md slot 1 surface obligation expansion.** Slot 1 architect-recommended shape column updates to include `Resource` trait surface from resource Tech Spec §2.1 (since CP6 implementation lands the trait; resource cascade is a downstream consumer). Suggested wording: "Includes `Resource` trait surface per resource Tech Spec §2.1 (Resource trait + 5 topology sub-traits) as cross-cascade dependency." Tracks the resource × credential trait reshaping landing site.

**Amendment I4 — cascade-queue.md slot 2 reconcile cluster-mode + daemon/eventsource.** Slot 2 architect-recommended shape column updates to surface daemon/eventsource extraction (resource Tech Spec §12 + ADR-0037) as part of engine-side cluster-mode cascade. Suggested wording: "Includes engine-side `crates/engine/src/daemon/` extraction per ADR-0037 + resource Tech Spec §12 — Daemon + EventSource→TriggerAction adapter." Resolves overlap concern in §5.2.2.

### §7.3 Acceptable as-is (🟡 MINOR — improvement, not blocker)

- §1.2.8 — 2 flagged credential Tech Spec amendments are documented as "FLAGGED, NOT ENACTED" with cross-section coordination plan. Acceptable; lands during cross-section pass.
- §4.2.4 — sealed-mod naming convention drift is doc-style observation; doesn't affect implementation.

---

## §8 Recommendation

**Hand-off to tech-lead** for amendment routing decision. The two 🔴 STRUCTURAL gaps are spec-spec disagreements (action vs resource Tech Spec on `Resource` trait declaration; resource vs credential Tech Spec on `on_credential_refresh` shape) that must be resolved by re-pinning one side to the other side's authority. Per architect's reading:

- **R1** — Action Tech Spec defers to resource Tech Spec on `Resource` trait shape (action's stub is the wrong-direction shadow; remove it).
- **R2** — Resource Tech Spec defers to credential Tech Spec §15.7 CP5 supersession on `on_credential_refresh` parameter shape (resource adopted superseded §3.6 shape; re-pin to §15.7 SchemeGuard shape).

Both amendments fit ADR-0035 amended-in-place precedent — neither requires new ADR; both are signature reconciliations between cascade Tech Specs that ratified at adjacent dates without cross-pin verification. The resource Tech Spec frontmatter (line 11-15) cites credential Tech Spec §3.6 lines 928-996 — the very lines that carry the supersession header pointing to §15.7. The cross-pin happened at the doc-citation level but did not propagate to the trait-shape level.

**Suggested next steps** (tech-lead authority — architect proposes, does not commit):

1. **Route R1 + R2 amendment scope to tech-lead** as the two 🔴 STRUCTURAL gaps blocking implementation. Tech-lead decides whether the amendments land:
   - Atomically as a single cross-cascade Tech Spec amendment ("cross-cascade reconciliation amendment, 2026-04-26"), OR
   - Separately as resource Tech Spec amendment + action Tech Spec amendment under ADR-0035 amended-in-place precedent.

2. **Route I1-I4 amendment scope to tech-lead** as the four 🟠 INCOMPLETE gaps that reduce implementer ambiguity. Each can be a doc-only narrative addition; no signature changes; SHOULD land before implementation but does not block implementation in the way R1/R2 do.

3. **Defer §1.2.8 + §4.2.4 (🟡 MINOR)** to credential Tech Spec author cross-section pass + workspace-style housekeeping respectively. Neither affects implementation.

4. **Cascade-queue.md slot 1 + slot 2 expansions (I3 + I4)** can be batch-edited as `docs/tracking/cascade-queue.md` revision per slot governance (line 35-40); architect can author the edit text once tech-lead approves the recommended text shapes.

5. **Implementation can begin** for the 15 🟢 COMPOSE-CLEAN seams in parallel with R1/R2 amendments — phantom-shim composition (Seam 1), sealed convention application (Seam 4), most of credential × resource composition (Seam 3 except §3.2.1), and cascade-queue.md slots 3-8 (Seam 5 except slot 1, slot 2). Implementation kickoff for the credential CP6 cascade (slot 1) and engine cluster-mode cascade (slot 2) **must wait** for R1/R2 closure.

### §8.1 Architect's reflection

The cross-cascade review surfaced two patterns worth noting for future cascade governance:

**Pattern 1 — supersession propagation discipline.** Credential CP5 supersession of §3.6 → §15.7 was internally rigorous (spike iter-3 validated the lifetime-pin refinement; §15.7 line 3503-3516). But the resource cascade authoring referenced the **pre-supersession line range** (§3.6 lines 928-996) as authoritative. A cross-cascade pass at credential CP5 ratification — explicitly identifying which downstream Tech Specs cite the superseded section — would have caught the resource Tech Spec's stale shape adoption before its own freeze. Future cross-cascade governance MIGHT include "supersession-propagation pass" at every credential / action / resource Tech Spec super-amendment.

**Pattern 2 — stub trait declarations should be marked or banned.** Action Tech Spec's stub `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }` is structurally well-meaning (it lets §2.2.4's `ResourceAction::Resource: Resource` bound parse during action Tech Spec drafting before the resource Tech Spec landed) but creates the parallel-shape collision risk this review surfaces. A discipline of "every cross-crate trait reference is `use crate_name::Trait;` import-only, or explicitly marked as forward-declaration" would prevent this class of gap.

### §8.2 Audit hand-off

- **tech-lead** — please ratify the amendment routing decisions (R1 + R2 + I1 + I2 + I3 + I4) per §7. This review does not propose specific amendment text; that is a separate dispatch per cascade-prompt's "Don't propose specific amendment text" constraint.
- **spec-auditor** — this review is itself a review document, not a spec; no auditor pass requested. If tech-lead routes R1 + R2 + I1 + I2 amendments to action / resource / credential Tech Spec authors, post-amendment spec-auditor cross-section pass per each Tech Spec's §0.3 freeze policy SHOULD verify the cross-cascade reconciliation.
- **architect (self)** — available to author amendment text under tech-lead direction; available to draft the cascade-queue.md slot 1 + slot 2 edits under tech-lead approval.

---

## §9 References

### §9.1 Primary cascade documents

- [Action redesign Strategy](../specs/2026-04-24-action-redesign-strategy.md) (FROZEN CP3)
- [Action Tech Spec](../specs/2026-04-24-nebula-action-tech-spec.md) (FROZEN CP4 + Q1+Q6+Q7+Q8 amendments)
- [Credential redesign Strategy](../specs/2026-04-24-credential-redesign-strategy.md)
- [Credential Tech Spec](../specs/2026-04-24-credential-tech-spec.md) (CP6 frozen + §15.7 CP5 supersession)
- [Credential refresh coordination](../specs/2026-04-24-credential-refresh-coordination.md)
- [Resource redesign Strategy](../specs/2026-04-24-nebula-resource-redesign-strategy.md) (FROZEN CP3)
- [Resource Tech Spec](../specs/2026-04-24-nebula-resource-tech-spec.md) (FROZEN CP4 2026-04-25)
- [Resource redesign summary](../specs/2026-04-24-nebula-resource-redesign-summary.md)

### §9.2 ADRs

- [ADR-0035 phantom-shim capability pattern](../../adr/0035-phantom-shim-capability-pattern.md) (proposed; amended 2026-04-24-B / 2026-04-24-C)
- [ADR-0036 Resource::Credential adoption + Auth retirement](../../adr/0036-resource-credential-adoption-auth-retirement.md) (accepted 2026-04-24)
- [ADR-0037 Daemon + EventSource engine fold](../../adr/0037-daemon-eventsource-engine-fold.md) (accepted 2026-04-25 amended-in-place)
- [ADR-0038 action trait shape](../../adr/0038-action-trait-shape.md) (accepted 2026-04-25)
- [ADR-0039 action macro emission](../../adr/0039-action-macro-emission.md) (accepted 2026-04-25 amended-in-place)
- [ADR-0040 ControlAction seal + canon §3.5 revision](../../adr/0040-controlaction-seal-canon-revision.md) (proposed pending user)

### §9.3 Cascade tracking

- [cascade-queue.md](../../tracking/cascade-queue.md) (8-slot table + governance rules)

---

*Review-only document; no Tech Spec or ADR modifications proposed. Amendment routing is tech-lead authority. This document is the architect's framing of the problem space; tech-lead picks the resolution path.*
