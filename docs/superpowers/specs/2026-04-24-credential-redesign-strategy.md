---
name: credential redesign — strategy (checkpoint 1)
status: Checkpoint 1 — §0–§3 written. §4–§6 follow in Checkpoints 2–3.
date: 2026-04-24
authors: [vanyastaff, Claude]
scope: cross-cutting — nebula-credential, nebula-storage, nebula-engine, nebula-api, nebula-resource, nebula-action, nebula-core, nebula-schema
supersedes: []
related:
  - docs/superpowers/drafts/2026-04-24-credential-redesign/
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0032-credential-store-canonical-home.md
  - docs/adr/0033-integration-credentials-plane-b.md
  - docs/research/n8n-credential-pain-points.md
  - docs/research/n8n-auth-architecture.md
---

# Credential redesign — strategy (Checkpoint 1)

## §0 Meta

**Scope of this document.** Strategy-level decisions that block prototype spike dispatch. Narrow by design — no Tech Spec content.

**Not in scope here:**

- Compile-able Rust signatures for trait shapes (prototype produces these).
- Full lifecycle / security / operational / testing / discovery / multi-mode decisions (→ Tech Spec after prototype).
- Sub-spec material (Trigger integration, ProviderRegistry seeding, mid-refresh race with rotated refresh_token, schema migration on encrypted rows, WebSocket events, multi-step persistent flows) — tracked in `docs/tracking/credential-concerns-register.md` (file seeded in Checkpoint 2; not yet present at Checkpoint 1 freeze), land as separate documents.

**Relationship to existing artefacts.**

| Artefact | Role | Status after this doc |
|---|---|---|
| `drafts/2026-04-24-credential-redesign/` | Exploratory notes, 37 findings | Superseded by §2–§3 decisions; remains archival |
| `specs/2026-04-20-credential-architecture-cleanup-design.md` | P6–P11 rollout cleanup spec | Still valid; predates redesign |
| ADR-0028 through 0033 | Canon invariants for credential architecture | §2 preserves all; §6 (Checkpoint 3) may supersede ADR-0031 pending prototype outcome |
| `research/n8n-credential-pain-points.md` | Pain data motivating redesign | Primary evidence basis for §1 |

**Reading order.** §0 → §1 (why) → §2 (foundational) → §3 (type system contract). §4–§6 land in Checkpoints 2–3.

**Checkpoint path.**

1. **Checkpoint 1** (this document): §0–§3 — blocks prototype spike dispatch.
2. **Checkpoint 2** (parallel to spike execution): §4 Concerns classification (5-label matrix, summary here; full ~124 rows in Deferred Concerns Register) + §5 Prototype spike plan (revised — A1 budget, three hypotheses, macro out-of-scope, inter-iteration checkpoint, blanket sub-trait validation + fallback, compat sketches required).
3. **Checkpoint 3** (after spike): §6 Post-prototype roadmap — freezes Strategy; Tech Spec kickoff signal.

**Freeze policy.** §2 Foundational decisions and §3 Type system contract are frozen after Checkpoint 1 review. Supersede requires ADR. §4–§6 may evolve through Checkpoints 2–3.

**Terminology.** "Strategy Document" = this file (decisions). "Tech Spec" = single production document written post-prototype (implementation-ready design). "Deferred Concerns Register" = `docs/tracking/credential-concerns-register.md` (living tracking doc, updated as sub-specs land).

## §1 Problem statement

### §1.1 Why redesign

After P6–P11 cleanup landed (storage-owns-persistence, engine-owns-orchestration, api-owns-OAuth ceremony, trait/impl split, Plane B vocabulary), the cross-crate layer map looks coherent. Three sources of evidence show it is not done:

1. **n8n field data** (428 credential types in production, `docs/research/n8n-credential-pain-points.md`) shows classes of regression the current Nebula design only partially defends against:
   - Concurrent refresh race with rotated refresh_token (n8n #13088) — in-proc RefreshCoordinator does not cover multi-replica.
   - Encryption key rotation operational pain (n8n #22478) — envelope walker CLI undocumented.
   - Git-pull wipes tokens (n8n #26499) — requires config/runtime split in storage schema.
   - Community node credential leak (n8n #27833) — needs `workflow_id` invariant in resolver.
2. **Two rounds of paper design failed to typecheck.** 8 BROKEN type-system findings in `drafts/.../05-known-gaps.md` (Pattern 2 default mismatch, `ctx.credential::<C>()` ambiguity, dyn-safety with 4 assoc types, multi-credential resource, `CredentialGuard<C::Scheme>` projection) are not resolvable on paper. This Strategy exists to break the cycle by gating production spec on prototype-validated trait shapes.
3. **Feature gates remain** (`credential-oauth` in `nebula-api`) as rollout-only artefact. ADR-0031 justification ("n8n parity") is weak under scrutiny — n8n shape is one project's reality, not a principled invariant.

### §1.2 Non-goals

- Replace §12.5 crypto primitives (AES-256-GCM + AAD) — preserved bit-for-bit.
- Relax zeroize invariants — preserved.
- Move HTTP into `nebula-credential` — stays out per ADR-0028.
- Replace `CredentialStore` trait location — stays in `nebula-credential` per ADR-0032.
- Add new public surface before implementation exists (PRODUCT_CANON §4.5 — operational honesty).
- Replace Plane A / Plane B separation (ADR-0033) — preserved.

### §1.3 Success criteria

1. Every decision that constrains prototype trait shape is locked here before spike dispatch.
2. Prototype validates (or refutes with documented rationale) trait shape against 5+ realistic credential types, 3 resources, 2–3 actions across Pattern 1 / Pattern 2 / Pattern 3.
3. Post-prototype Tech Spec has zero `TBD` holes on type-system-level concerns. Fallback activation per §3.7 (A or B) is an **explicit decision**, not a TBD hole — Tech Spec documents the fallback state if triggered.
4. Deferred concerns have an explicit tracked home in the Register — no silent drops.

## §2 Foundational decisions (frozen)

Locked after Checkpoint 1 review. Supersede requires ADR.

### §2.1 Sealed trait policy

**Decision.** `Credential` trait is sealed for API surface cleanliness. Third-party credential types extend via `#[plugin_credential]` macro escape hatch.

**Rationale (precise).** Sealed trait chosen for API surface cleanliness and intentional extension points, **not for security**. Plugin execution security is handled at the execution model layer (in-process / process-isolated / WASM — separate ADR, see Deferred Concerns Register). Sealing the trait does not prevent a hostile in-process plugin from reading plaintext credentials or exfiltrating them — a malicious plugin using the stabby ABI has the same memory access as the host.

What sealing actually purchases:

- **API surface cleanliness** — legitimate plugin authors cannot accidentally misuse the trait shape.
- **Intentional extension points** — every new credential type passes through the macro, making extension a considered act rather than an ambient affordance.

`#[plugin_credential]` escape hatch with signed manifest provides:

- (a) **Author accountability** for audit — signed key identifies the author.
- (b) **Revocation mechanism** for compromised plugins — signatures can be revoked.
- (c) **Explicit ceremony** — macro expansion is discoverable in audit logs, making extension a considered act.

**Manifest signing infrastructure is acknowledged as a separate sub-project.** Desktop (offline signature verification model), self-hosted (operator as root-of-trust), and cloud (Anthropic as CA) — three distinct trust-anchor models requiring separate sub-spec. Tracked in Deferred Concerns Register.

**Interim Strategy.** Manifest signing is deferred to post-MVP. `#[plugin_credential]` macro works without signing until signing infrastructure lands. This Strategy does not block on it.

### §2.2 Pattern defaults

| Pattern | When to use | Default? |
|---|---|---|
| **Pattern 1** — concrete per-credential-type, no service trait | single-auth services (Anthropic API key, Discord webhook, fixed-schema API tokens) | Default for single-auth |
| **Pattern 2** — service trait as pure marker + blanket sub-trait for capability binding (§3.2–§3.3) | multi-auth services (Bitbucket, Jira, GitHub, Slack, Salesforce, Stripe, HubSpot, Notion) | **Default for multi-auth** |
| **Pattern 3** — capability-only binding (`dyn AcceptsBearer`, `dyn AcceptsSigning`) | service-agnostic utilities (generic HTTP bearer client, generic SigV4 signer) | For utilities only |
| **Generic OAuth2 fallback** — `GenericOAuth2Credential` concrete type | user-provided OAuth2 endpoints for unknown/custom providers | Treated as Pattern 3 consumer (implements `AcceptsBearer`; no service trait) |

Pattern 2 as default for multi-auth is re-derived from n8n field data: majority of popular services are multi-auth. The paper-design default of Pattern 1 was wrong — this Strategy corrects it.

**Pattern 1 → Pattern 2 promotion policy.** When a service starts as single-auth (Pattern 1) and later acquires a second auth method, `CredentialRef<AnthropicApiKeyCredential>` consumers cannot transparently accept the new shape — they must migrate to `CredentialRef<dyn AnthropicCredential>`. Two policies considered:

- **(a) Accept as breaking change.** Pattern 1 → Pattern 2 promotion is a contract change for consuming actions; treated as a major version bump per semver. Promotion procedure: introduce service trait, mark old `CredentialRef<C>` deprecated for one minor cycle, then remove in next major.
- **(b) Defensive Pattern 2 always.** Even single-auth services declare a one-impl service trait. No migration needed because consumers already use `CredentialRef<dyn ServiceCredential>`. Cost: boilerplate trait + impl for ~100+ single-auth services in `nebula-credential-builtin`.

**Decision: (a)**, with explicit acknowledgement that promotion is a real contract change and major version bump is the appropriate signal. (b) rejected because the per-service boilerplate cost is paid by **every** single-auth service (frequent), while migration cost is paid only by services that actually grow a second auth (rare event with clear semver signal). Open to revision in Tech Spec if multi-auth promotion turns out frequent in practice; tracked as Tech-spec-material in the Concerns Register.

### §2.3 Resource-per-capability

**Decision.** When a service exposes multiple auth capabilities (Bearer + Basic, Bearer + mTLS, Bearer + SigV4, …), emit **one Resource type per capability** — not a single Resource with builder polymorphism.

**Example — Bitbucket:**

- `BitbucketBearerClient` — consumes `dyn BitbucketBearer` (satisfied by OAuth2, PAT).
- `BitbucketBasicClient` — consumes `dyn BitbucketBasic` (satisfied by AppPassword).

Not: `BitbucketClient::new(...).authenticated_bearer()` / `.authenticated_basic()`.

**Rationale.** Capability-matching lives in the type system, not in runtime branching. An action that requires Bearer semantics should not silently accept Basic-shaped credentials; compile error is the correct outcome. Builder polymorphism hides the match at call site and pushes failure to runtime.

**Macro contract.** `#[action]` macro verifies that the `credential` field's capability bound matches the declared `resource`'s accepted auth. Mechanism of verification (trait-resolution-based vs compile-time registry) is itself a prototype validation item — see §3.5.

### §2.4 Layer ownership (reaffirmed)

Existing layer map from `drafts/2026-04-24-credential-redesign/02-layer-map.md` is preserved. No new crates proposed beyond those already identified there. Summary for clarity:

- **`nebula-credential`** — contract (traits, DTOs, crypto primitives, Zeroize wrappers). **No HTTP, no orchestration.**
- **`nebula-credential-builtin`** (NEW) — concrete credential types + service traits (`SlackOAuth2Credential`, `BitbucketCredential` markers, `AwsSigV4Credential`, …). **Split rationale:** plugin authors depend only on the contract crate (`nebula-credential`); built-in concrete types live in a separate crate so the trait-only dependency surface stays clean for third-party consumers and so built-in types can evolve (add credential types, bump dependencies, refactor concrete impls) without touching the contract crate's stability surface.
- **`nebula-storage`** — persistence, encryption layer, cache layer, audit layer (fail-closed + degraded read-only), scope layer, `KeyProvider`, `PendingStore`, new repos (`RefreshClaimRepo`, `RotationLeaderClaimRepo`, `ProviderRegistryRepo`).
- **`nebula-engine`** — orchestration (resolver, registry, two-tier coordinator, rotation, OAuth HTTP ceremony, `ExecutionCredentialStore`, health probe scheduler).
- **`nebula-api`** — HTTP gateway (OAuth callback, CRUD, registry admin, WebSocket events).
- **`nebula-resource`, `nebula-action`** — consumers (Resource trait, Action trait, `#[action]` macro, `ActionContext`).

Supporting crates (`nebula-core`, `nebula-schema`, `nebula-metadata`, `nebula-error`, `nebula-resilience`, `nebula-eventbus`, `nebula-metrics`, `nebula-log`) and the full dependency matrix + deny.toml enforcement rules are taken **verbatim** from the draft's layer map. They are decisions of prior ADRs (0028–0033) and not revisited here.

**ADR-0031 supersede candidacy.** Whether OAuth HTTP ceremony stays in `nebula-api` (ADR-0031) or moves to `nebula-engine` (draft layer-map recommendation) is a **Tech-spec-material** concern, deferred to Checkpoint 3. Prototype does not depend on this choice — spike uses mock HTTP regardless.

## §3 Type system contract (frozen decisions + named hypotheses)

Locked decisions plus hypotheses for prototype validation. **No compile-able Rust** — that is the spike's output. Pseudo-Rust below is illustrative intent, not a compile claim.

### §3.1 `Credential` trait — shape held

`Credential` keeps its current shape:

- 4 associated types — `Input`, `State`, `Scheme`, `Pending`.
- `const CAPS: Capabilities` — bitflags, 12 flags (see draft §01-type-system).
- Async methods — `resolve`, `continue_resolve`, `refresh`, `revoke`, `test`. Rotation is engine-orchestrated per ADR-0030, **not a trait method**.

`continue_resolve` is retained for OAuth2 callback continuation even under atomic-only multi-step (draft finding #22 direction). Persistent N-step accumulator is a separate sub-spec; current `continue_resolve` signature handles the single-continuation case.

**Trait-heaviness acknowledged.** Every associated type, every default-impl method, every capability flag has engineering cost — for dyn-safety, plugin ergonomics, readability. §3.6 records the discipline that controls additions.

### §3.2 Service trait — pure marker

Service traits (`BitbucketCredential`, `JiraCredential`, `GitHubCredential`, `SlackCredential`, …) are **pure markers**. No `type Scheme` bound, no associated types beyond what `Credential` supertrait provides.

Intent (pseudo-Rust, illustrative):

```rust
// Pure marker — dyn-safe by construction.
pub trait BitbucketCredential: Credential + Sealed {}
```

This closes finding #32 of the draft (non-dyn-safe projections of `C::Scheme` in paper design) by refusing to carry scheme information in the service trait at all.

**On `dyn` semantics — what the spike must validate.** The `dyn BitbucketBearer` in `CredentialRef<dyn BitbucketBearer>` is a **nominal bound** for compile-time type-checking, **not a classical vtable trait object**. `Credential` itself has 4 associated types and methods using them — turning `Credential` directly into a vtable trait object requires either specifying all four assoc types (`dyn Credential<Input=_, State=_, Scheme=_, Pending=_>`) or excluding methods via `where Self: Sized`. `dyn BitbucketBearer` inherits the same constraint through its supertrait chain.

Runtime path is therefore **type-erased**: the handle is `CredentialKey` (carried alongside `PhantomData<fn() -> dyn BitbucketBearer>` in H1, or via macro-generated binding in H2/H3); resolve returns `Box<dyn AnyCredential>` — `AnyCredential` is a separate, narrower object-safe trait by design (no assoc types in its method signatures, by construction); downcast to concrete `T: BitbucketBearer` happens at the use site. **The `dyn` in `CredentialRef<dyn BitbucketBearer>` is for compile-time signature checking; runtime never holds a vtable pointer to `dyn BitbucketBearer` directly.**

Spike must confirm this resolution path compiles end-to-end on the Bitbucket triad and that the macro-generated downcast at the use site type-checks. If reader (or spike agent) attempts to materialize `dyn BitbucketBearer` as a classical vtable object, that will fail compile — and the failure is **expected**, not evidence the pattern is broken.

### §3.3 Capability binding — blanket sub-trait pattern

Capability requirements live on a separate layer — a sub-trait with blanket impl over service-trait types whose `Credential::Scheme` satisfies the capability.

Intent (pseudo-Rust, illustrative):

```rust
// Capability-constrained sub-trait — derived, not hand-implemented.
pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{}

// Consumer (action) demands capability-bound service trait:
#[action(credential)]
pub bb: CredentialRef<dyn BitbucketBearer>,
```

Resolution walk:

- `BitbucketOAuth2` → `Scheme = BearerScheme` → `AcceptsBearer` → satisfies `BitbucketBearer` ✓
- `BitbucketPat` → `Scheme = BearerScheme` → `AcceptsBearer` → satisfies `BitbucketBearer` ✓
- `BitbucketAppPassword` → `Scheme = BasicScheme` → does not implement `AcceptsBearer` → does not satisfy `BitbucketBearer` — **compile error** at action resolution ✓

Service grouping (Bitbucket-ness) lives in one layer; capability (Bearer-ness) lives in another. Orthogonal. The `dyn BitbucketBearer` in `CredentialRef<dyn BitbucketBearer>` is a nominal bound for compile-time type-checking per §3.2 — the runtime path is type-erased through `AnyCredential` + downcast, not a direct vtable trait object on `BitbucketBearer`.

### §3.4 `CredentialRef<C>` — three hypotheses for prototype

The runtime shape of `CredentialRef<C>` — the handle actions hold to demand a credential — is a prototype validation item. Three hypotheses; spike must attempt all three before picking.

**H1 — PhantomData + TypeId registry.**

```rust
struct CredentialRef<C: ?Sized> {
    key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}
```

Runtime lookup uses `TypeId::of::<C>()` against a `CredentialRegistry` populated at plugin registration. Dispatch cost: one HashMap lookup per resolve. Compatible with `dyn` bounds via type-erased entries.

**H2 — proc-macro binding table.** `#[action]` macro emits a compile-time binding table, one entry per `CredentialRef` field in the action struct. Runtime is an index lookup into the pre-computed table. No `TypeId` introspection. Dispatch cost: array index.

**H3 — typed accessor methods.** Instead of `CredentialRef<C>` fields, `#[action]` macro generates typed accessor trait implementations for the action: `fn slack(&self) -> CredentialGuard<…>`, `fn bitbucket(&self) -> CredentialGuard<…>`. No shared runtime representation — each field is its own generated method. Dispatch cost: direct call.

Prototype attempts all three, picks one with rationale documented in `NOTES.md`. Selection criteria: dyn-safety preserved (per §3.2 type-erased semantics), ergonomics acceptable on realistic actions, hot-path resolve performance measured via micro-benchmark.

**Performance budget.** ≤1µs per cached resolve as **upper bound** (ceiling, not aspiration). Goal is no regression from the current `resolve_any` baseline — spike measures baseline first, then reports each hypothesis as delta from baseline. The 1µs ceiling accommodates `TypeId` check + downcast overhead headroom over the typical 200–500ns of `Box<dyn Any + Send + Sync>` + `HashMap` lookup; if any hypothesis exceeds 1µs cached, it is rejected on performance grounds regardless of ergonomics.

### §3.5 Macro-enforced capability ↔ resource match — validation item

Decision in §2.3: `#[action]` macro verifies that the `credential` field's capability bound matches the declared `resource`'s accepted auth. The **mechanism** of this verification is not yet proven. Two candidate mechanisms:

- **(i) Trait-resolution cross-check.** Resource declares `type AcceptedAuth: SchemeInjector`; macro checks trait-resolution compatibility between action's `CredentialRef<dyn Bound>` and resource's `AcceptedAuth` using `where` clauses emitted in generated code. Compile fails if mismatch.
- **(ii) Compile-time capability registry.** `inventory`-style or explicit `register_resource_auth::<R, C>()` at plugin init; macro performs lookup of tag pairs at expansion. Compile fails if pair absent.

Added to prototype spike scope as sub-question under Q3 (capability registry dispatch): **"Does Resource `AcceptedAuth` declaration + Action credential bound cross-check compile-enforce correctly?"** Spike attempts at least one mechanism end-to-end on the Bitbucket triad (BearerClient + BasicClient + three credential types). If neither mechanism compiles cleanly, capability-match escalates to runtime check and **§3.7.B** fallback activates (Pattern 2 retained; only capability-resource match downgraded to runtime).

### §3.6 Trait-heaviness discipline

Every new addition to `Credential` trait (new associated type, new default-impl method, new capability flag) requires:

1. Explicit rationale in ADR or Tech Spec.
2. Alternative considered — helper crate / separate trait / runtime registry / different abstraction layer.
3. dyn-safety impact assessed.

This is a policy, not a mechanism. Violations are caught in review, not at compile. Recorded here because draft §05-known-gaps finding #11 (trait heaviness un-flagged) is real — without explicit discipline, the trait accretes.

### §3.7 Fallbacks — two distinct failure modes

§3.3 pattern failure (type-level) and §3.5 macro enforcement failure (tooling-level) have different blast radius and warrant separate fallbacks. **Spike evaluates §3.3 first**; if §3.3 fails, §3.5 is skipped and Fallback A activates; if §3.3 passes, §3.5 is evaluated independently. Spike must distinguish which failure mode (if any) it hits and report explicitly in `NOTES.md`.

#### §3.7.A — Fallback A: type-level failure (§3.3 pattern broken)

**Trigger.** Blanket sub-trait pattern (§3.3) fails type-level validation — blanket impl does not constrain correctly under the actual supertrait chain, or the type-erased resolution path described in §3.2 cannot be made to type-check end-to-end on the Bitbucket triad.

**State.** Pattern 2 dropped entirely. Service grouping expressed only in `CredentialMetadata` (UI discovery) and runtime dispatch. Action declares `CredentialRef<dyn AcceptsBearer>` (Pattern 3); resolver filters by service-id metadata at runtime; UI restricts credential picker to service-compatible credentials.

**Cost — explicit.** Compile-time guarantee "this action accepts only Bitbucket credentials" is **lost**. Service mismatch becomes a runtime error at resolve, with UI as sole prevention. A user invoking the action directly via API with a wrong-service credential gets a runtime failure, not a compile error. **Fallback A is a valid-but-degraded path, not equivalent to Pattern 2.** If prototype reaches Fallback A, Tech Spec must explicitly document this degradation for consumers and surface it in per-action documentation.

#### §3.7.B — Fallback B: tooling-level failure (§3.5 macro enforcement broken)

**Trigger.** §3.3 pattern works (Pattern 2 type-level validation succeeds) but neither §3.5 mechanism (trait-resolution cross-check or compile-time capability registry) can be made to compile-enforce capability ↔ resource match. Only the macro check fails.

**State.** Pattern 2 retained — service-match remains compile-enforced at the action's `CredentialRef<dyn BitbucketBearer>` declaration. Capability-resource match downgraded to runtime — engine validates at action invocation that the resolved credential's `Scheme` is accepted by the resource's declared `AcceptedAuth`. UI prevention strengthened to compensate (capability filter on credential picker per resource type).

**Cost — explicit.** Mismatch between resource and credential capability surfaces at action invocation, not at compile. Significantly less severe than Fallback A — service-grouping guarantee retained; only the capability-pair check moves to runtime. Tech Spec documents the runtime-check semantics if Fallback B activates.

#### Fallback selection rules

- Fallback A and Fallback B are **not mutually exclusive**: if §3.3 fails, Fallback A activates regardless of §3.5 status (since §3.5 mechanism presupposes the §3.3 pattern works).
- If §3.3 succeeds and §3.5 succeeds → no fallback; Pattern 2 + macro-enforced match.
- If §3.3 succeeds and §3.5 fails → Fallback B only.
- If §3.3 fails → Fallback A (regardless of §3.5).
- `NOTES.md` records: §3.3 outcome, §3.5 outcome, fallback selected (none / A / B), with reproducible failing test where applicable.

---

**Checkpoint 1 ends here.** §4 Concerns classification and §5 Prototype spike plan land in Checkpoint 2 (parallel to spike execution). §6 Post-prototype roadmap lands in Checkpoint 3 (after spike completes).
