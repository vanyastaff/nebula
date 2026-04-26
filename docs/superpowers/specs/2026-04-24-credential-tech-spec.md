---
name: credential tech spec (implementation-ready design)
status: complete CP6 (active-dev endorse-phased 2026-04-24 Round 7) — П1 in-implementation 2026-04-26 (worktree `worktree-credential-p1`, will be commit `<SHA>` at merge). CP5 amendments §15.3-§15.11 added post 3-stakeholder consensus session Rounds 0-5 (Path A-Hybrid, initially adoption-deferred). CP6 Rounds 6-7 corrected: tech-lead flipped to endorse-phased under active-dev framing; P10 verified NOT landed; 3 concrete gates added (§15.12). П1 lands the type-shape scaffolding (§15.4-§15.8) per [Stage 8 plan](../plans/2026-04-26-credential-p1-trait-scaffolding.md).
date: 2026-04-24
authors: [vanyastaff, Claude]
scope: cross-cutting — nebula-credential, nebula-credential-builtin (NEW), nebula-storage, nebula-engine, nebula-api, nebula-resource, nebula-action
supersedes: []
related:
  - docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/tracking/credential-concerns-register.md
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0032-credential-store-canonical-home.md
  - docs/adr/0033-integration-credentials-plane-b.md
---

# Credential Tech Spec (implementation-ready design)

## §0 Meta

**Scope of this document.** Implementation-ready design for the credential subsystem. Built on top of the frozen [Strategy Document](2026-04-24-credential-redesign-strategy.md) (Checkpoint 3, commit `4316a292`) and [ADR-0035](../../adr/0035-phantom-shim-capability-pattern.md) (amended 2026-04-24-B post spike iter-2).

**Builds on partially-completed P6-P11 cleanup** (CP6 correction 2026-04-24 Round 7 — prior CP5 claim «all landed» was wrong). ADRs [0028](../../adr/0028-cross-crate-credential-invariants.md) (cross-crate invariants), [0029](../../adr/0029-storage-owns-credential-persistence.md) (storage owns persistence), [0030](../../adr/0030-engine-owns-credential-orchestration.md) (engine owns orchestration), [0031](../../adr/0031-api-owns-oauth-flow.md) (API owns OAuth HTTP), [0032](../../adr/0032-credential-store-canonical-home.md) (CredentialStore home), [0033](../../adr/0033-integration-credentials-plane-b.md) (Plane B integration credentials), and [0034](../../adr/0034-schema-secret-value-credential-seam.md) (SecretValue schema seam) all accepted.

**Actual landing status** (verified 2026-04-24 Round 7 + Gate 1 amendment 2026-04-24):
- **P6-P9 landed** ✓ — `crates/storage/src/credential/{backup.rs,key_provider.rs,layer/,memory.rs,pending.rs}` + `crates/engine/src/credential/{executor.rs,refresh.rs,registry.rs,resolver.rs,rotation/}`.
- **P10 landed under axum convention** ✓ (revised Gate 1 finding — see ADR-0031 amendment 2026-04-24-A). CP6 initially claimed «P10 NOT landed» based on incomplete verification (`ls crates/api/src/credential/` returning empty). Actual structure: `crates/api/src/handlers/credential_oauth.rs` (594 LOC controller) + `crates/api/src/handlers/credential.rs` (wrapper) + `crates/api/src/services/oauth/{flow.rs, http.rs, state.rs, mod.rs}` + route wiring in `routes/workspace.rs` + `tests/e2e_oauth2_flow.rs` (316 LOC E2E). Feature-gated under `credential-oauth`. Security invariants §4.1-§4.6 of ADR-0031 preserved at landed paths. Axum convention (`handlers/` + `services/`) is idiomatic for `nebula-api`; the `credential/` subdirectory ADR-0031 §1 originally prescribed would split one domain along a path axis inconsistent with webhook/auth/workflow handlers. [ADR-0031 amendment 2026-04-24-A](../../adr/0031-api-owns-oauth-flow.md) reconciles the aspirational paths with landed structure.
- **P11 partial** — import migration done (`nebula_credential::…` + `nebula_storage::credential::…` paths used by consumers); consumer migrations complete for routes/handlers wiring; MATURITY row for `nebula-api` stays at «credential-oauth feature-gated; awaiting E2E stability» honest status until feature flips to default. Tech Spec assumes the post-cleanup architecture: `nebula-credential` = pure contract trait + scheme DTOs + §12.5 primitives; `nebula-storage` owns persistence layers (`EncryptionLayer` / `CacheLayer` / `AuditLayer` / `ScopeLayer` + `KeyProvider`); `nebula-engine` owns orchestration (`CredentialResolver` / `RefreshCoordinator` / rotation scheduler / `token_refresh.rs` HTTP); `nebula-api` owns OAuth flow HTTP ceremony (`oauth_controller.rs` / `flow.rs` / `state.rs`).

**Tech Spec re-shapes the contract trait, not the layer boundaries.** Amendments §15.3-§15.10 (added 2026-04-24 CP5 per 3-stakeholder consensus session) introduce capability sub-trait split, `AuthScheme` sensitivity dichotomy, fatal duplicate-KEY registration, `SchemeGuard` + `SchemeFactory` for refresh hook, capability-from-type authority, and `PendingStore::consume` atomicity contract. **The sub-trait split (§15.4) preserves ADR-0035 phantom-shim dyn-safety** — Pattern 2 / Pattern 3 `dyn XPhantom` continues to work; if `dyn Refreshable` etc. are needed for runtime dispatch, parallel phantom shims are introduced via the same ADR-0035 mechanism (or ADR-0035 superseded if a structural gap surfaces during П1 scaffolding).

**Reading order.** §0 → §1 (scope & audience) → §2 (trait contract) → §3 (runtime model) → §4 (lifecycle) → §5 (storage schema) → §6 (security) → §7 (operational) → §8 (testing) → §9–§13 (interface) → §14–§16 (meta + open items + handoff).

**Checkpoint path.**

1. **Checkpoint 1** (this document, §0–§3): foundational — scope, trait contract, runtime model.
2. **Checkpoint 2a** (§4–§5): lifecycle + storage schema (foundation for security/operational).
3. **Checkpoint 2b** (§6–§8): security + operational + testing.
4. **Checkpoint 3** (§9–§13): interface surface — discovery, OAuth flows, multi-mode, integration, evolution.
5. **Checkpoint 4** (§14–§16): meta + open items (`critique-c9`, `arch-authscheme-clone-zeroize`) + implementation handoff.

**Freeze policy.** Each checkpoint freezes after review. Supersede requires ADR. Strategy Document authority supersedes Tech Spec on conflict; Tech Spec wins over sub-spec and implementation plans. Register rows update to `decided` with Tech Spec pointer as sections land.

**Relationship to authority documents.**

```
Strategy (primary entry, frozen Checkpoint 3)
  + ADR-0028/29/30/32/33 (cross-crate invariants)
  + ADR-0035 (amended — phantom-shim canonical form)
    → Tech Spec (this doc — implementation-ready)
        + sub-specs (per-concern separate documents)
          → Implementation phases (П1…Пn plans)
            → Code (Rust crates)
```

## §1 Scope & audience

### §1.1 What Tech Spec is

Tech Spec is **implementation-normative**. Where Strategy established decisions and ADR-0035 established canonical form, Tech Spec provides:

- Concrete Rust signatures with real error types (not pseudo-Rust).
- Storage DDL with indices, foreign keys, and dialect-parity constraints.
- Layer ordering with explicit invariants at each boundary.
- Test matrix with specific test categories, tools, and coverage gates.
- Operational runbooks pointers + failure-modes matrix.
- Open-item decisions (§15): `critique-c9` and `arch-authscheme-clone-zeroize` resolved here.

Readers treat Tech Spec as authoritative for implementation. Strategy + ADRs provide decision record; sub-specs cover deferred concerns.

### §1.2 Non-scope

**Sub-spec items** — lands as separate documents per Strategy §6.5 landing queue.

**10 OUT markers covering 10 sub-spec pointers** (referenced in Tech Spec sections as pointers, not inlined):

- §2.11 Signed manifest infrastructure → `arch-signing-infra` (queue #7, post-MVP, independent track).
- §4.6 Migration on encrypted rows v1→v2 → `draft-f36` (queue #4).
- §4.7 Import/export (encrypted backup + n8n-compat import) → queue #12.
- §6.10 Compromise response runbook → queue #8.
- §7.1 / §7.6 Refresh multi-replica + rotation leader → `draft-f17` (already in flight) + queue #2.
- §10.3 Multi-step persistent flow accumulator → `draft-f22` (queue #3, deprioritized — Pending enum suffices for atomic flows).
- §12.3 OIDC/SSO federation → Plane A (ADR-0033), permanent OUT of credential scope.
- §12.4 Plugin execution sandbox → execution-model ADR (product-policy row `user-int-plugin-sandbox`).
- §14.1 Threat model document → queue #9 (quarterly review cadence).
- §14.4 Incident response runbooks (×3: leak / key compromise / IdP outage) → queue #10.

**Product-policy items** — decided by product, not engineering. Tech Spec describes engagement, does not decide policy:

- Sealed policy for `Credential` trait (Strategy §2.1 — frozen).
- Compliance certifications (SOC 2 / ISO 27001 / HIPAA) — mapping owned by product.
- Deployment mode gates (what's cloud-only vs shared) — §11 describes the matrix; gate decisions are product.
- GDPR lawful basis for storage — compliance sub-spec.

**Permanent out-of-scope:**

- OIDC/SSO federation (Plane A per ADR-0033). Credential subsystem (Plane B) does not federate identity.
- Plugin execution model (in-process / process-isolated / WASM). Credential subsystem assumes the execution model is decided elsewhere.

**ProviderRegistry scope split.** Tech Spec describes ProviderRegistry's **consumer-side** interface — how credential resolution, storage layer, and multi-mode feature matrix interact with it (referenced in §5.4 / §6.8 / §11.2 / §7 where applicable). Registry's **producer-side** design (schema, versioning, URL template semantics, seeding protocol, admin API) lives in sub-spec `draft-f18/f19/f20` per Strategy §6.5 queue item 2. Tech Spec references the consumer interface as if registry design already exists; sub-spec fills in producer side. Chicken-and-egg resolved: Tech Spec ships referencing a **defined consumer interface**; sub-spec finalizes producer independently. Changes to the producer surface cannot break the consumer API without ADR.

### §1.3 Audience

**Primary:**

- Implementation engineers landing credential subsystem crates (`nebula-credential`, `nebula-credential-builtin`, `nebula-storage` extensions, `nebula-engine` extensions).
- Third-party plugin authors using `#[plugin_credential]` macro + capability phantom-shim (§2.3) to extend credential types for their services.
- Reviewers for implementation-phase PRs.

**Secondary:**

- Security reviewers assessing encryption, audit, and zeroization invariants (§6).
- Tech writers producing user-facing docs + runbooks referencing Tech Spec §14.

### §1.4 Relationship to Strategy, ADRs, and register

Tech Spec is one level below Strategy in the authority chain:

| Document | Authority | Mutation mechanism |
|---|---|---|
| PRODUCT_CANON | Product invariants | Product ADR |
| Strategy (frozen Checkpoint 3) | Credential-redesign decisions + canonical forms | New ADR with inline forward-pointer per §0 freeze policy |
| ADR-0028 … 0035 | Cross-crate invariants + phantom-shim form | New ADR supersede |
| **Tech Spec (this doc)** | **Implementation-ready design** | **Checkpoint review + new Tech Spec version** |
| Sub-specs | Per-concern design depth | Sub-spec freeze after review |
| Implementation plans | Phased execution | Plan revision |

**Register** (`docs/tracking/credential-concerns-register.md`) is a living tracking surface. Tech Spec sections land → register rows update from `locked-post-spike` to `decided` with Tech Spec pointer. Zero silent drops per register maintenance rule.

### §1.4.1 Adoption rationale — active-dev endorse-phased (CP6 re-framed 2026-04-24 Round 7)

**CP5 adoption-deferred-per-triggers framing SUPERSEDED.** CP5 framed adoption as «no consumer wall → wait». Under active-dev stage + hard breaking changes OK + verified live hazards in current production code, «wait for consumer» is the wrong question per `feedback_active_dev_mode.md` («Active dev ≠ prod release. Never settle for 'deferred'...»). Round 7 re-engaged tech-lead with active-dev framing; tech-lead flipped from «adoption-deferred» to **endorse-phased** with 3 gates before П1 (see §15.12).

The three §15.12 gates are **engineering-derived sequencing**, not consumer-derived deferral:

1. **Gate 1 — P10 cleanup completion** (honest baseline): OAuth HTTP ceremony actually moves to `crates/api/src/credential/` per ADR-0031 + `p6-p11.md:26-33` «Landed» claim corrected. Unblocks truthful §0 citation and clears doc↔code gap before П1 scaffolds onto it.
2. **Gate 2 — N7 standalone fix** (live hazard): `crates/engine/src/credential/registry.rs:31` gets `tracing::warn!` + reject-second-registration policy (silent overwrite with zero tracing is live today, not hypothetical).
3. **Gate 3 — spike iter-3 narrow-scope** (CP5 shape validation): sub-trait × ADR-0035 phantom-shim composition validated on 2-3 credential types (api_key static, oauth2 refreshable, 1 interactive). Closes «paper design relative to dyn-safety» risk that tech-lead Round 4 condition 1 flagged.

**П1 starts after all 3 gates close.** No consumer-wall dependency. No half-life timer. Tech-correct sequencing for active-dev stage.

**What «deferred» is NOT the same as:** deferred-to-consumer-wall (CP5, wrong for active-dev) ≠ deferred-to-3-engineering-gates (CP6, tech-correct for active-dev). The difference is authority: consumer = product signal; gates = engineering judgment.

Recorded in [`docs/MATURITY.md`](../../MATURITY.md) (`nebula-credential` row: «trait redesign endorsed-phased; П1 gated on §15.12 3 gates»); register row `tech-spec-adoption-status` flipped to `active-dev-3-gate-path` status.

### §1.5 Success criteria

Tech Spec is DONE when:

1. Every register `tech-spec-material` row has a resolution pointer to a specific Tech Spec section (zero silent drops).
2. Two `open` items (`critique-c9` `PROVIDER_ID` + `arch-authscheme-clone-zeroize` Clone bound) have explicit decisions in §15 with rationale + impact analysis.
3. No `TBD` holes in §1–§16.
4. Implementation engineers can scaffold `nebula-credential` + `nebula-credential-builtin` crates from Tech Spec without re-interpreting Strategy or ADRs.
5. Storage DDL (§5.1) is reviewable by SQL reviewers independently of Rust code.
6. Test matrix (§8) gives test authors concrete starting points per category with tool names, not just category labels.
7. Pattern 2 dispatch narrative (§3.4) survives review by Rust reviewers — no hand-waving at the phantom / where-clause / downcast boundary. The narrative must compile in a spike-level reproduction if pressed.
8. Every OUT marker in §1.2 is a pointer, not a hole — reader can navigate to the sub-spec or product decision.

## §2 Trait contract

> **⚠️ CP5/CP6 amendment overlay (read first).** Sections §2.1, §2.2, §2.6 below retain CP1-CP4 baseline text for review-history diff readability. **The canonical CP5/CP6 shape is in §15.3-§15.12**, which supersedes the sections as marked. Specifically:
>
> - §2.1 `Credential` trait — const capability bools + defaulted method bodies → **superseded by §15.4** (capability sub-trait split: `Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic` — no defaults, no silent downgrade). `Pending` assoc type moves under `Interactive`.
> - §2.1 `State: CredentialState` — no `ZeroizeOnDrop` bound → **superseded by §15.4** amendment: `CredentialState: Serialize + DeserializeOwned + Send + Sync + ZeroizeOnDrop + 'static`.
> - §2.2 `AuthScheme: Send + Sync + Clone + 'static` — → **superseded by §15.2 + §15.5**: Clone relaxed (per-scheme opt-in); `AuthScheme` split into `SensitiveScheme + ZeroizeOnDrop` vs `PublicScheme`; derive-time field audit; non-sensitive carve-out removed.
> - §2.6 `#[capability]` macro contract — currently covers `AcceptsBearer`/`AcceptsBasic`/`AcceptsSigning` markers only → **extended by §15.4** to emit `Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic` sub-trait impl blankets when `#[plugin_credential]` is applied. §2.6 text not yet updated (deferred to Gate 3 spike iter-3 + П1 scaffolding PR).
>
> Implementers scaffolding П1 code read §15.3-§15.12 as authoritative, then §2 for context on what was. Reviewers reading Tech Spec linearly should expect the §15 amendments to resolve apparent §2 contradictions.

### §2.1 `Credential` trait

> **Superseded by §15.4 + §15.5 (CP5 2026-04-24).** Text below reflects CP1-CP4 baseline. Canonical CP5/CP6 form: capability sub-trait split eliminates `const INTERACTIVE/REFRESHABLE/REVOCABLE/TESTABLE/DYNAMIC` bools + defaulted method bodies; `CredentialState` gains `ZeroizeOnDrop` supertrait bound.

Canonical shape held since Strategy Checkpoint 1 §3.1. Implementation-level signatures:

```rust
/// Primary trait. Every concrete credential type implements this.
///
/// Sealed — external crates cannot impl directly. Plugins use the
/// #[plugin_credential] macro, which emits sealed::Sealed blanket as
/// part of its expansion (§2.11).
pub trait Credential: sealed::Sealed + Send + Sync + 'static {
    /// User-facing configuration input. Typed schema via HasSchema.
    /// Must be Deserialize from API / UI; Zeroize on drop if contains
    /// SecretField per nebula-schema conventions.
    type Input: HasInputSchema + Send + Sync + 'static;

    /// Runtime scheme output — what Resources consume. Examples:
    /// BearerScheme, BasicScheme, SigV4Scheme, TlsIdentityScheme.
    type Scheme: AuthScheme;

    /// Encrypted-at-rest state. Serialized via serde for storage;
    /// SecretString / SecretBytes wrappers for sensitive fields
    /// (preserve §12.5 crypto invariants per Strategy §1.2 non-goal).
    type State: CredentialState + Send + Sync + 'static;

    /// Pending state for interactive flows (OAuth2 callback, multi-step
    /// chain). Unit type `NoPendingState` for non-interactive credentials.
    type Pending: PendingState + Send + Sync + 'static;

    /// Stable key for registry lookup. Per-type, not per-instance.
    /// Examples: "slack.oauth2", "bitbucket.pat", "aws.sigv4".
    const KEY: &'static str;

    /// Whether this credential requires multi-step interactive resolution
    /// (OAuth2 authorize→callback, device code flow).
    const INTERACTIVE: bool = false;

    /// Whether this credential can refresh its State (OAuth2 refresh_token,
    /// STS session renewal).
    const REFRESHABLE: bool = false;

    /// Whether this credential supports explicit revocation at the provider.
    const REVOCABLE: bool = false;

    /// Whether `test()` is meaningful (health probe endpoint exists).
    const TESTABLE: bool = false;

    /// Build initial State from user Input. May initiate interactive
    /// flow (returns `ResolveResult::Pending`). Async because may call
    /// IdP endpoints (OAuth2 token exchange, AWS STS AssumeRole, etc).
    async fn resolve(
        ctx: &CredentialContext<'_>,
        input: &Self::Input,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError>
    where
        Self: Sized;

    /// Continue a Pending flow. Used for OAuth2 callback (code + state
    /// from IdP), multi-step chain continuation (step-N output).
    /// Returns `ResolveResult::Ready(State)` on completion, or
    /// `ResolveResult::Pending(next)` if more steps remain.
    async fn continue_resolve(
        ctx: &CredentialContext<'_>,
        pending: Self::Pending,
        continuation: &Continuation,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError>
    where
        Self: Sized;

    /// Project runtime Scheme from stored State. Synchronous, pure.
    /// `where Self: Sized` — see §2.10 for the dispatch implication.
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    /// Refresh expired State (OAuth2 access_token via refresh_token,
    /// STS session token renewal). Coordinated by engine's
    /// RefreshCoordinator (L1 in-proc, L2 cross-replica per sub-spec
    /// `draft-f17`).
    async fn refresh(
        ctx: &CredentialContext<'_>,
        state: Self::State,
    ) -> Result<Self::State, RefreshError>
    where
        Self: Sized;

    /// Revoke State at provider (best-effort for the provider; storage
    /// tombstone is separate — lifecycle §4.3). Some providers have no
    /// revoke endpoint; implementations return Ok(()) with no-op.
    async fn revoke(
        ctx: &CredentialContext<'_>,
        state: &Self::State,
    ) -> Result<(), RevokeError>
    where
        Self: Sized;

    /// Health probe. Engine calls periodically per
    /// CredentialMetadata::test_cadence. Must not have side effects
    /// beyond read operations (no token mint, no resource creation).
    async fn test(
        ctx: &CredentialContext<'_>,
        state: &Self::State,
    ) -> Result<TestOutcome, TestError>
    where
        Self: Sized;
}
```

**Rotation is NOT a trait method.** Per ADR-0030, rotation orchestration lives in `nebula-engine`. The trait provides primitives (`refresh` + `revoke`); engine composes them into rotation cycles per scheduled policy.

**Note on capability encoding — Strategy drift disclosure.** Strategy §3.1 (frozen Checkpoint 1, line 168) mentions `const CAPS: Capabilities` bitflag with 12 flags as the shape. That text is **pre-spike aspirational** and was not updated during Checkpoint 3 when iter-2 outcomes landed. Spike iter-1 + iter-2 validated, and **current production code** (`crates/credential/src/contract/credential.rs`) uses, the **per-flag bool form above** with 4 flags (`INTERACTIVE`, `REFRESHABLE`, `REVOCABLE`, `TESTABLE`, each default `false`). Tech Spec matches the validated + in-production shape. Strategy §3.1 line 168 has a minor drift from validated reality — scheduled as a follow-up correction (small amendment, not ADR-scale — information content is equivalent, just different encoding). If bitflag ergonomics become desirable later (e.g., single-line `CAPS` declaration in plugin code), that migration is material change requiring ADR discipline.

**Error types** — all errors impl `Classify` per `nebula-error` taxonomy (`Transient` / `Permanent` / `Capability` / `Context`). Detailed in §2.12 (TBD — Checkpoint 1 placeholder to be filled in §15 once open items decided).

### §2.2 `AuthScheme` + capability markers

> **Superseded by §15.2 (Clone relax) + §15.5 (sensitive/public dichotomy) (CP4 + CP5 2026-04-24).** Text below reflects CP1 baseline. Canonical form: `AuthScheme: Send + Sync + 'static` (no `Clone`); split into `SensitiveScheme: AuthScheme + ZeroizeOnDrop` + `PublicScheme: AuthScheme`; derive-time field audit forbids plain `String` for sensitive-tagged schemes. «Non-sensitive carve-out» rationalization loophole removed.

```rust
/// Base trait for runtime scheme output. Implementations are concrete
/// structs holding decrypted scheme material, zeroized on drop where
/// sensitive material lives.
///
/// `Clone` bound — see §15.2 decision for rationale (one of three
/// candidates resolved at Checkpoint 4).
pub trait AuthScheme: Send + Sync + Clone + 'static {}

/// Capability markers. Each concrete Scheme opts in to one or more.
/// Empty traits — capability membership is the entire contract;
/// semantics enforced via service trait blankets (§2.5) + #[action]
/// macro bindings (§2.7) + resolve-site where-clauses (§3.4).
pub trait AcceptsBearer: AuthScheme {}
pub trait AcceptsBasic: AuthScheme {}
pub trait AcceptsSigning: AuthScheme {}
pub trait AcceptsTlsIdentity: AuthScheme {}

/// Concrete scheme — HTTP Bearer token.
#[derive(Clone)]
pub struct BearerScheme {
    pub token: SecretString,
}
impl AuthScheme for BearerScheme {}
impl AcceptsBearer for BearerScheme {}

/// Concrete scheme — HTTP Basic (username + password).
#[derive(Clone)]
pub struct BasicScheme {
    pub user: String,
    pub pass: SecretString,
}
impl AuthScheme for BasicScheme {}
impl AcceptsBasic for BasicScheme {}

/// Concrete scheme — AWS SigV4 signing material (static creds or
/// STS-minted session creds).
#[derive(Clone)]
pub struct SigV4Scheme {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
    pub region: String,
}
impl AuthScheme for SigV4Scheme {}
impl AcceptsSigning for SigV4Scheme {}

/// Concrete scheme — mutual TLS client identity.
#[derive(Clone)]
pub struct TlsIdentityScheme {
    pub cert_pem: SecretBytes,
    pub key_pem: SecretBytes,
}
impl AuthScheme for TlsIdentityScheme {}
impl AcceptsTlsIdentity for TlsIdentityScheme {}
```

Extension: non-sensitive schemes (`DiscordWebhookScheme`, `NoAuthScheme`) exist at the contract level — they impl `AuthScheme` but hold no `SecretString`-wrapped material. This is why `AuthScheme` does not mandate `ZeroizeOnDrop` at the trait level — zeroization is per-type responsibility, driven by whether the Scheme contains sensitive fields.

Full builtin scheme catalog (6 schemes at shipping) is in §2.x TBD Checkpoint 2a (§4 lifecycle expansion covers all builtin types).

### §2.3 Phantom-shim canonical form

Per [ADR-0035](../../adr/0035-phantom-shim-capability-pattern.md) §1 (amended 2026-04-24-B post spike iter-2 at commit `1c107144`). Every capability trait that appears in `dyn` positions has a paired phantom trait. The phantom has no `Credential` supertrait chain, making `dyn X` well-formed as a Rust type.

**Canonical form per capability-trait-defining crate:**

```rust
// At crate root (src/lib.rs), declared once per crate.
// NOT re-exported. sealed_caps is crate-private.
mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait SigningSealed {}
    pub trait TlsIdentitySealed {}
    // ... one inner trait per capability this crate exposes.
}

// "Real" capability trait — supertrait-chained to service trait for
// compile-time constraint. Used only for blanket-impl eligibility.
// NOT usable in dyn positions (inherits Credential's 4 unspecified
// assoc types via BitbucketCredential → Credential).
pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{}

// Sealed blanket — only types satisfying BitbucketBearer gain
// sealed_caps::BearerSealed membership. sealed_caps is crate-private,
// so external crates cannot impl BearerSealed for their own types.
impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}

// "Phantom" capability trait — dyn-safe marker for dyn positions.
// Supertrait is BearerSealed + Send + Sync. `'static` dropped per
// ADR-0035 §5 (redundant under Rust 2021+ default-object-lifetime).
pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}

impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}
```

**Per-capability inner Sealed** (not single shared `Sealed`) — ADR-0035 §3 amendment. A single shared `Sealed` would cause Rust coherence collision when two capabilities share a service supertrait: `impl<T: BitbucketBearer> Sealed for T {}` + `impl<T: BitbucketBasic> Sealed for T {}` declared overlapping by coherence even when no concrete type satisfies both. Per-capability inner sealed traits sidestep: each blanket targets its own `BearerSealed` / `BasicSealed` / etc, no overlap.

**User obligations** (macro assumes, does not emit):

- Declare `mod sealed_caps { pub trait BearerSealed {} … }` at crate root with one inner trait per capability. Missing → `E0433` at emitted blanket impl.
- Declare the service supertrait (`BitbucketCredential`) before `#[capability]` annotation.

### §2.4 Service marker traits

Service traits are pure markers — no methods, no associated types beyond the `Credential` supertrait. Express "this credential belongs to this service":

```rust
pub trait BitbucketCredential: Credential + sealed::Sealed {}

impl BitbucketCredential for BitbucketOAuth2 {}
impl BitbucketCredential for BitbucketPat {}
impl BitbucketCredential for BitbucketAppPassword {}
```

`sealed::Sealed` here is the **Credential-level** sealed (see §2.11) — distinct from `sealed_caps::*` for capability phantoms. The two mechanisms are orthogonal.

Service trait enables Pattern 2 actions to bind at the service layer (via `dyn ServiceCapabilityPhantom`) without committing to a concrete credential type.

### §2.5 Capability sub-trait pattern

For each (service × capability) intersection the crate supports:

```rust
#[capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait BitbucketBearer: BitbucketCredential {}

#[capability(scheme_bound = AcceptsBasic, sealed = BasicSealed)]
pub trait BitbucketBasic: BitbucketCredential {}
```

(Macro output detailed in §2.6.)

**Resolution walk on Bitbucket triad:**

- `BitbucketOAuth2` → `Scheme = BearerScheme` → `AcceptsBearer` ✓ → `BitbucketBearer` (blanket) ✓ → `BearerSealed` (blanket) ✓ → `BitbucketBearerPhantom` (blanket) ✓
- `BitbucketPat` → `Scheme = BearerScheme` → same path ✓
- `BitbucketAppPassword` → `Scheme = BasicScheme` → not `AcceptsBearer` ✗ → not `BitbucketBearer` → not `BearerSealed` → not `BitbucketBearerPhantom` → **compile error** at action declaration `CredentialRef<dyn BitbucketBearerPhantom>` when wired with AppPassword.

### §2.6 `#[capability]` macro

> **Extended by §15.4 (CP5 2026-04-24).** Text below covers `AcceptsBearer`/`AcceptsBasic`/`AcceptsSigning`/`AcceptsTlsIdentity` capability markers (service/scheme intersection). CP5 adds `#[plugin_credential]` macro emission of `Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic` sub-trait impl blankets derived from credential author's opt-in declaration (`#[credential(refreshable)]` etc.). Macro emission contract for sub-traits will be finalized in Gate 3 spike iter-3 + formalized alongside П1 scaffolding PR.

Per ADR-0035 §4 (amended). Emits real + sealed-blanket + phantom from a single user-written declaration.

**Input:**

```rust
#[capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait BitbucketBearer: BitbucketCredential {}
```

**Output (hand-expanded equivalent):**

```rust
pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{}

impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}

pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}

impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}
```

**Macro does NOT emit `mod sealed_caps`.** Proc-macros in stable Rust cannot share state across invocations — "emit once, skip thereafter" is not implementable without external mechanisms (rejected per ADR-0035 §4.2). The crate author declares `mod sealed_caps { … }` manually at crate root once, with one inner trait per capability.

**Diagnostic on missing sealed module:**

```
error[E0433]: failed to resolve: use of undeclared crate or module `sealed_caps`
  --> src/lib.rs:17:1
   |
17 | pub trait BitbucketBearer: BitbucketCredential {}
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   = help: ensure `mod sealed_caps { pub trait BearerSealed {} }` is declared at crate root
```

Tech Spec documents the `mod sealed_caps { … }` onboarding step in plugin-author guide (Checkpoint 3 §9.1 Registration).

### §2.7 `#[action]` macro translation

Action structs have fields typed `CredentialRef<dyn BitbucketBearer>` in user-facing syntax. `#[action]` macro **rewrites silently** to `CredentialRef<dyn BitbucketBearerPhantom>` in generated code.

**Decision — rewrite silently (not reject with guidance).** Rationale:

- User-facing syntax reads naturally: "my action needs a Bitbucket credential that supports Bearer".
- Phantom is an implementation detail of ADR-0035 canonical form; requiring users to learn the `Phantom` suffix leaks implementation into API ergonomics.
- Diagnostic chain on mismatch stays readable through the phantom (verified spike iter-2 at commit `1c107144` — `E0277` chain: scheme → real trait → phantom).
- Pattern 1 (concrete `CredentialRef<SlackOAuth2Credential>`) needs no translation — pass-through unchanged.

**Input (Pattern 2 capability-bound):**

```rust
#[action]
pub struct BitbucketRepoFetchAction {
    #[credential]
    pub bb: CredentialRef<dyn BitbucketBearer>,
    // ...
}
```

**Output:**

```rust
pub struct BitbucketRepoFetchAction {
    pub bb: CredentialRef<dyn BitbucketBearerPhantom>, // rewritten
    // ...
}

// + ActionMetadata impl per §3.4 dispatch (generated).
```

**Input (Pattern 1 concrete):**

```rust
#[action]
pub struct SlackPostMessageAction {
    #[credential]
    pub slack: CredentialRef<SlackOAuth2Credential>,
}
```

**Output:** identical (no phantom translation — `SlackOAuth2Credential` is `Sized`, not `dyn`).

### §2.8 `AnyCredential` — object-safe runtime handle

Narrow object-safe supertrait. Runtime holds `Box<dyn AnyCredential>`; downcast to concrete type happens at engine resolve-site per §3.4.

```rust
/// Object-safe narrow shadow of Credential. Part of the stable plugin
/// API — changes require ADR (§13.4).
pub trait AnyCredential: Any + Send + Sync + 'static {
    /// Stable key from Credential::KEY, for registry-independent identity.
    fn credential_key(&self) -> &'static str;

    /// TypeId for downcast. Returned by TypeId::of::<C>() on the
    /// concrete impl.
    fn type_id_marker(&self) -> TypeId;

    /// Expose as &dyn Any for downcast_ref. Narrow path — use the
    /// TypeId above for lookup, downcast via as_any.
    fn as_any(&self) -> &dyn Any;

    /// Metadata (display name, icon, help text, test cadence,
    /// capability flags). Returned by reference — CredentialMetadata
    /// lives in nebula-metadata, shared across crates.
    fn metadata(&self) -> &CredentialMetadata;
}

// Blanket impl — every Credential gets AnyCredential for free.
impl<C> AnyCredential for C
where
    C: Credential + CredentialMetadataSource,
{
    fn credential_key(&self) -> &'static str {
        C::KEY
    }
    fn type_id_marker(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn metadata(&self) -> &CredentialMetadata {
        <C as CredentialMetadataSource>::metadata()
    }
}
```

`CredentialMetadataSource` is a companion trait that concrete impls provide (via `#[plugin_credential]` macro or hand-written). Separated from `Credential` for two reasons:

1. **Conceptual separation.** `Credential` is runtime behavior (`resolve`, `refresh`, `revoke`, `test`, `project`); `CredentialMetadataSource` is UI binding (display name, icon, help text, test cadence). Mixing them would bloat the trait surface with UI concerns alongside the hot path.

2. **Composability for metadata overrides.** `CredentialMetadata` supports two-layer overrides per register row `draft-f33` (`::defaults()` + `::with_override(MetadataOverrides)` via registry / per-tenant config). Separating the source trait keeps metadata evolution orthogonal to the ABI-stable `Credential` + `AnyCredential` surfaces — metadata fields can be added without disturbing the `AnyCredential` vtable or the plugin ABI promise (§13.4).

The implication: plugin authors always provide both (`Credential` + `CredentialMetadataSource`). The `#[plugin_credential]` macro emits both from a single annotated declaration.

### §2.9 `CredentialRef<C>` — typed handle

Per Strategy §6.1 hypothesis decision — H1 picked:

```rust
/// Typed handle to a credential by key. C may be Sized (Pattern 1
/// concrete) or unsized dyn CapabilityPhantom (Pattern 2/3 bounded).
///
/// Runtime representation = CredentialKey + PhantomData. No runtime
/// type reflection stored; dispatch via TypeId lookup against
/// CredentialRegistry per §3.3.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct CredentialRef<C: ?Sized> {
    key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}

impl<C: ?Sized> CredentialRef<C> {
    /// Construct from key. Internal — typically emitted by #[action]
    /// macro or constructed via ActionContext::credential_at.
    pub(crate) const fn from_key(key: CredentialKey) -> Self {
        Self { key, _t: PhantomData }
    }

    pub fn key(&self) -> &CredentialKey {
        &self.key
    }
}
```

**Why `PhantomData<fn() -> C>` not `PhantomData<C>`:**

- `fn() -> C` is covariant in return position — matches expected variance for `CredentialRef<dyn X>` subtyping.
- Auto-derives `Send + Sync` unconditionally (a `fn` pointer is always `Send + Sync`), which allows `CredentialRef<dyn Phantom>` to be `Send + Sync` without the phantom trait mandating those on its concrete type argument.
- Drop check ignores the phantom — no ownership implication.

### §2.10 `Credential::project` — dispatch complementarity

`project` is `where Self: Sized`. This is load-bearing for the Pattern 2 dispatch narrative (§3.4).

```rust
impl Credential for BitbucketOAuth2 {
    // ...
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized,
    {
        BearerScheme {
            token: state.access_token.clone(),
        }
    }
}
```

**Implications of `where Self: Sized`:**

- `project` is excluded from any object-safe vtable. `dyn Credential` / `dyn AnyCredential` do not have `project` in their vtable.
- Callers must have concrete-type knowledge of `C` at call-site. This forces the dispatch narrative in §3.4: engine downcast first, then call `project`.
- Complementary to the declaration-site phantom check — together they form the two-layer compile-time gate: (a) action declaration can only wire correct-capability types (phantom); (b) engine can only project correct-scheme types (`where C: Credential<Scheme = …>`).

### §2.11 Plugin extension

**Two distinct sealed mechanisms** — do not confuse:

**1. `Credential`-level `sealed::Sealed`** — protects the primary `Credential` trait from external impls. Crate-private:

```rust
// In nebula-credential:
mod sealed {
    pub trait Sealed {}
}
pub trait Credential: sealed::Sealed + Send + Sync + 'static { /* … */ }

// Internal blanket (in nebula-credential-builtin or via macro):
impl sealed::Sealed for BitbucketOAuth2 {}
impl Credential for BitbucketOAuth2 { /* … */ }
```

Only `nebula-credential-builtin` crate and `#[plugin_credential]` macro-generated code can impl `Credential`. External crates cannot impl directly.

**2. Per-capability `sealed_caps::XSealed`** — protects capability **phantom** traits in dyn positions. Per ADR-0035 §3 amendment (§2.3 in this Tech Spec). Detailed in §2.3.

**Orthogonality:** `sealed` protects the Credential trait (cross-crate API surface control). `sealed_caps` protects capability phantom traits (dyn-type well-formedness + coherence correctness). They share the sealed-pattern shape but serve different goals.

**`#[plugin_credential]` macro** — the only entry point for external crates to add credential types. Macro emits the `sealed::Sealed` blanket + `Credential` impl + `CredentialMetadataSource` impl. Registration happens at runtime via explicit `register::<C>()` on plugin init (Strategy §2.1).

**Registration invariant — registry is append-only after startup.** `CredentialRegistry` (§3.1) is mutated only during service initialization (plugin registration phase). Runtime credential resolution never mutates the registry. **Hot-reload of credential types is explicitly OUT of scope** — restarting the service is the mechanism for picking up new credential types (e.g., after loading a new plugin). This invariant enables the lock-free read path in §3.1 and is enforced by the `register` method being pub-crate on `CredentialRegistry` — only the plugin init code paths can call it.

**Signed manifest infrastructure** — **OUT** (`arch-signing-infra` sub-spec, Strategy §6.5 queue #7, post-MVP). The macro works without signing until signing infra lands; plugins are identified by explicit registration for now.

**Capability extension for plugins:** plugin crates declare their own `mod sealed_caps { pub trait CustomServiceBearerSealed {} … }` at the plugin's crate root and use `#[capability]` to emit phantom chains against that local `sealed_caps`. Cross-crate sealed sharing is neither requested nor permitted — each crate protects only its own phantoms.

### §2.12 Error type placeholders

Error types (`ResolveError`, `RefreshError`, `RevokeError`, `TestError`) impl `Classify` per `nebula-error` taxonomy. Full error catalog is in §6.7 failure modes matrix (Checkpoint 2b). `Capability` axis per register `draft-f13` row.

Placeholders held until §15 open items decided, because `critique-c9` (`PROVIDER_ID` for non-OAuth) influences the `Capability(WrongScheme)` vs `Capability(NotSupported)` axis shape.

## §3 Runtime model

### §3.1 `CredentialRegistry`

Per Strategy §6.1 iter-2 final shape (commit `1c107144`):

```rust
use ahash::AHashMap;
use std::borrow::Borrow;
use std::sync::Arc;

pub struct CredentialRegistry {
    entries: AHashMap<Arc<str>, Box<dyn AnyCredential>>,
}

impl CredentialRegistry {
    pub fn new() -> Self {
        Self { entries: AHashMap::new() }
    }

    /// Register a concrete credential. Called at plugin init / service
    /// startup. Duplicate keys are a programming error — panic in debug,
    /// warn + overwrite in release (instrumented).
    pub fn register<C: Credential>(&mut self, instance: C) {
        let key: Arc<str> = C::KEY.into();
        let existing = self.entries.insert(key, Box::new(instance));
        debug_assert!(
            existing.is_none(),
            "duplicate credential key: {}", C::KEY
        );
        if existing.is_some() {
            tracing::warn!(
                key = C::KEY,
                "duplicate credential key registered — overwriting"
            );
        }
    }

    /// Lookup by key — zero-allocation hot path via Borrow<str>.
    pub fn resolve_any(&self, key: &str) -> Option<&(dyn AnyCredential + 'static)> {
        self.entries.get(key).map(|b| &**b)
    }

    /// Typed lookup — downcast after any-resolve. Safe per TypeId
    /// check inside downcast_ref.
    pub fn resolve<C: Credential>(&self, key: &str) -> Option<&C> {
        self.entries
            .get(key)?
            .as_any()
            .downcast_ref::<C>()
    }
}
```

**Key design choices (iter-2 decisions):**

- **Single-keyed by `Arc<str>`.** Not tuple-keyed by `(key, TypeId)`. `TypeId` safety comes from `downcast_ref` at concrete-type boundary, not from key structure. Simplifies lookup API + reduces key hash input.
- **ahash (v0.8) hasher** — ~3× faster than default SipHash at credential-key sizes (10–50 bytes). Sufficient for iter-2 baseline bench (~5.5 ns mean; 150× under 1 µs ceiling).
- **No locking on read path.** Registry is **append-only after startup** — all registration happens during plugin init before any resolve. Hot path sees `&AHashMap::get`, no `RwLock` / `Mutex` overhead.
- **`Box<dyn AnyCredential>` storage** — `AnyCredential` is object-safe by construction (§2.8). Credentials kept owned by the registry; `Credential::resolve` returns `State`, not the credential instance, so cloning is rare.

### §3.2 `CredentialKey` — `Arc<str>` newtype

Per Strategy §6.1 iter-2 decision — eliminates `String::clone` on hot path:

```rust
/// Stable identifier for a credential instance. Shares underlying
/// allocation via Arc<str>; zero-alloc lookup via Borrow<str>.
///
/// Invariant: always corresponds to a Credential::KEY. Never
/// constructed for ad-hoc strings — either from static or from
/// storage-layer Arc<str>.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct CredentialKey(Arc<str>);

impl CredentialKey {
    pub fn from_static(s: &'static str) -> Self {
        Self(Arc::from(s))
    }
}

impl Borrow<str> for CredentialKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<Arc<str>> for CredentialKey {
    fn from(s: Arc<str>) -> Self {
        Self(s)
    }
}

impl fmt::Debug for CredentialKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CredentialKey").field(&&*self.0).finish()
    }
}
```

`Borrow<str>` enables `HashMap::get(&str)` — no allocation for lookup. Hash delegates to `str` hash; equality is byte-level. The `Arc<str>` is cheap to clone (refcount bump) for key propagation in async contexts.

Construction is deliberately restricted:

- `from_static` for compile-time `const KEY` sources (most common).
- `From<Arc<str>>` for storage-layer reads where the key is already `Arc<str>`-shared.

No `From<String>` or `From<&str>` — those would hide allocation; callers must be explicit about the allocation site.

### §3.3 Resolver dispatch — Pattern 1 path

For **Pattern 1** actions (`CredentialRef<ConcreteCredential>`):

```rust
impl ActionContext<'_> {
    /// Pattern 1: concrete credential type, no dyn.
    pub fn resolve<C: Credential>(&self, cref: &CredentialRef<C>) -> Option<&C> {
        self.registry.resolve::<C>(cref.key().borrow())
    }
}
```

Type-level direct: action body has concrete `&C`, can call `C::project(&state)` directly. No dispatch narrative required — the action is already at full type knowledge.

For **Pattern 2/3** actions (capability-bound dyn), see §3.4.

### §3.4 Pattern 2 dispatch — end-to-end

This is the load-bearing narrative per Strategy §6.1. Tech Spec documents it explicitly so readers do not reinvent it.

**Scenario:** action accepts any Bitbucket credential with Bearer capability.

```rust
#[action]
pub struct BitbucketRepoFetchAction {
    #[credential]
    pub bb: CredentialRef<dyn BitbucketBearer>,  // user-facing syntax;
    // macro rewrites to CredentialRef<dyn BitbucketBearerPhantom>.
}
```

**Step 1 — Declaration-site phantom check (compile-time).**

The `#[action]` macro-rewritten field `CredentialRef<dyn BitbucketBearerPhantom>` forces the supplied credential type `T` to satisfy `BitbucketBearerPhantom`, which requires `T: BitbucketBearer` via the sealed_caps blanket chain.

Wrong-capability wiring rejects at compile time:

```rust
// Action author wires AppPassword by mistake:
let action = BitbucketRepoFetchAction {
    bb: CredentialRef::<BitbucketAppPassword>::from_key(key),
    // ...
};
// Compile error E0277:
//   BasicScheme: AcceptsBearer not satisfied
//   → required for BitbucketAppPassword to implement BitbucketBearer
//   → required for BitbucketAppPassword to implement BitbucketBearerPhantom
//   → required for CredentialRef<BitbucketAppPassword> to coerce to
//      CredentialRef<dyn BitbucketBearerPhantom>
```

The action **cannot compile** with a wrong-capability credential. Phantom is the declaration-site gate.

**Step 2 — Engine iterates action's credential slots at invocation.**

Engine does NOT pass `&dyn BitbucketBearerPhantom` to the action body. Instead, the `#[action]` macro generates slot metadata:

```rust
// Macro-generated (conceptual):
impl ActionSlots for BitbucketRepoFetchAction {
    fn credential_slots(&self) -> &[SlotBinding] {
        const SLOTS: &[SlotBinding] = &[
            SlotBinding {
                field_name: "bb",
                slot_type: SlotType::ServiceCapability {
                    capability: Capability::Bearer,
                    service: ServiceKey::Bitbucket,
                },
                resolve_fn: resolve_bearer_slot,
            },
        ];
        SLOTS
    }
}
```

Engine uses the slot metadata to drive resolution — it knows each slot's capability requirement.

The `resolve_fn` field uses the **same HRTB function-pointer pattern** as `RefreshDispatcher::refresh_fn` described in §7.1: `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>`. Erasure over concrete `C` is achieved via a blanket builder called at slot registration time (same mechanism as `RefreshDispatcher::for_credential<C>()`). Concrete `resolve_bearer_slot` function points at a monomorphization of `resolve_as_bearer<C>` for each registered `C: Credential<Scheme = BearerScheme>`.

**Step 3 — Typed resolve via capability-specific helper (resolve-site where-clause).**

```rust
/// Engine-owned capability-specific resolve. Takes the concrete type
/// C (known at engine-side macro expansion time via SlotBinding's
/// resolve_fn pointer) and enforces Scheme = BearerScheme at the type
/// system level.
fn resolve_as_bearer<C>(
    ctx: &CredentialContext<'_>,
    key: &str,
) -> Result<BearerScheme, ResolveError>
where
    C: Credential<Scheme = BearerScheme>,
{
    let cred: &C = ctx.registry.resolve::<C>(key)
        .ok_or(ResolveError::NotFound { key: key.into() })?;
    let state: &C::State = ctx.load_state::<C>(key)?; // decrypt + audit
    let scheme: BearerScheme = C::project(state);
    Ok(scheme)
}
```

The `where C: Credential<Scheme = BearerScheme>` is **resolve-site enforcement**. Engine cannot instantiate this helper with a wrong-Scheme concrete type — compile error `E0271`:

```
error[E0271]: type mismatch resolving
  `<BitbucketAppPassword as Credential>::Scheme == BearerScheme`
  --> src/engine.rs:123
   |
123 | resolve_as_bearer::<BitbucketAppPassword>(ctx, key)?
   |                       ^^^^^^^^^^^^^^^^^^^ expected `BearerScheme`,
   |                                           found `BasicScheme`
```

**Step 4 — Action body receives `&Scheme`.**

The action body never sees `&dyn BitbucketBearerPhantom`. It sees `&BearerScheme` directly:

```rust
impl Action for BitbucketRepoFetchAction {
    async fn execute(
        &self,
        ctx: &ActionContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let bearer: &BearerScheme = ctx.resolved_scheme(&self.bb)?;
        let client = reqwest::Client::new();
        let resp = client
            .get(&input.url)
            .bearer_auth(bearer.token.expose_secret())
            .send()
            .await?;
        // ...
    }
}
```

Engine does the type reflection (step 3's `where`-clause + `downcast_ref`). Action body stays at concrete-type knowledge — no `downcast_ref` in user code.

**Complementarity summary:**

| Gate | Site | Enforces |
|---|---|---|
| Phantom bound | Declaration site (action struct) | "Can this action be wired with credential `C`?" |
| `where C: Credential<Scheme = X>` | Resolve site (engine helper) | "Can engine actually project `C`'s state to the expected scheme?" |

Both checks are compile-time. Runtime sees only `&Scheme`. The two gates cover disjoint failure modes — phantom catches capability mismatch at wiring; `where`-clause catches scheme mismatch at engine-side dispatch instantiation (which in practice is a macro-emitted `match` against slot metadata + capability enum).

**Why not naïve `downcast_ref` enumeration:** the alternative would be action body receiving `&dyn Phantom` and enumerating downcasts. This does not scale — action author cannot enumerate plugin-registered concrete types at action-compile-time. The phantom + where-clause approach delegates enumeration to the engine, which has the full registry at runtime and compile-time bounds at macro expansion time.

### §3.5 `ExecutionCredentialRef<C>` — typed newtype distinction

Per register row `draft-f24`: ephemeral credentials used only within a single execution (e.g., DYNAMIC credential resolution via `ExternalProvider` Vault pull, execution-scoped test credentials) have a distinct typed handle:

```rust
/// Execution-scoped credential reference. Distinct from CredentialRef
/// at the type level — cannot be mixed with persistent references at
/// the type system level.
///
/// Storage backend is ExecutionCredentialStore (lives in nebula-engine,
/// per-execution scope, cleaned up at execution teardown).
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct ExecutionCredentialRef<C: ?Sized> {
    key: CredentialKey,
    execution_id: ExecutionId,
    _t: PhantomData<fn() -> C>,
}
```

**Type-level distinction means:**

- Storage layer cannot accidentally persist an `ExecutionCredentialRef`'s state to the `credentials` table — storage API signatures take `CredentialRef<C>`, not `ExecutionCredentialRef<C>`.
- `ActionContext` provides separate resolve methods — `resolve` (persistent) vs `resolve_execution` (ephemeral) — to prevent accidental cross-use.
- Zeroization on execution teardown via explicit `cleanup()` method, not Drop best-effort (per register row `draft-f25`).

Detailed cleanup semantics in Checkpoint 2a §4 lifecycle.

### §3.6 `on_credential_refresh` — connection-bound resources

> **Superseded by §15.7 (CP5 2026-04-24).** Signature below takes `&<Self::Credential as Credential>::Scheme` (borrowed reference). Canonical CP5 form: `SchemeGuard<'_, Self::Credential>` — owned, `!Clone`, `ZeroizeOnDrop`, `Deref<Target = Scheme>`, lifetime-bound to call. Plus `SchemeFactory<C>` companion for long-lived resources needing re-acquisition (worked `OAuth2HttpPool` example in §15.7). Compile-fail probes in §16.1.1 (#6, #7) enforce no-retention + no-clone properties.

Per register rows `draft-f26` / `draft-f27`: connection-bound resources (Postgres pool, Kafka producer) may outlive individual credential resolves. When the credential refreshes, the resource needs to rebuild its connection.

**Resource trait:**

```rust
pub trait Resource {
    type Credential: Credential;
    type Error: Classify + Send + Sync + 'static;

    async fn create(
        ctx: &ResourceContext<'_>,
        scheme: &<Self::Credential as Credential>::Scheme,
    ) -> Result<Self, Self::Error>
    where
        Self: Sized;

    /// Optional hook — called when engine detects credential scheme
    /// change (refresh or rotation). Default: no-op. Most resources
    /// are per-request, not connection-bound.
    async fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> Result<(), Self::Error> {
        let _ = new_scheme;
        Ok(())
    }
}
```

**Blue-green pool swap pattern** (canonical example, Postgres):

```rust
pub struct PostgresPool {
    inner: Arc<RwLock<deadpool_postgres::Pool>>,
}

impl Resource for PostgresPool {
    type Credential = PostgresConnectionCredential;
    type Error = PostgresError;

    async fn create(
        _ctx: &ResourceContext<'_>,
        scheme: &PostgresConnectionScheme,
    ) -> Result<Self, PostgresError>
    where
        Self: Sized,
    {
        let pool = build_pool_from_scheme(scheme).await?;
        Ok(Self { inner: Arc::new(RwLock::new(pool)) })
    }

    async fn on_credential_refresh(
        &self,
        new_scheme: &PostgresConnectionScheme,
    ) -> Result<(), PostgresError> {
        let new_pool = build_pool_from_scheme(new_scheme).await?;
        let mut guard = self.inner.write().await;
        *guard = new_pool;
        // Old connections drain naturally as their RAII guards drop;
        // new queries use the new pool (read lock acquires against
        // the new inner after swap).
        Ok(())
    }
}
```

**Per `draft-f27`:** most resources accept the trait method as a default no-op. Cost is one unused `async fn` per `Resource` impl — minor overhead. Concrete use case: AWS IAM Database Authentication (15-minute token TTL) requires the pool to rebuild when the auth token refreshes.

## §4 Lifecycle

Lifecycle covers a credential instance's existence from creation to deletion. Per Strategy §4 and register `user-lifecycle-*` cluster (7 rows). State transitions map to the production `state_kind` enum (`active` / `refreshing` / `expired` / `revoked` / `suspended`) per migration `0017_credentials_v3.sql`.

**Note on state vocabulary — two overlapping vocabularies appear throughout §4 and §7:**

- **In-memory flow states** (transient, engine-managed). State-machine diagrams below track transitions through operations: `idle`, `resolving`, `pending`, `continuing`, `discarded`, `failed`, `refresh_pending`, `refreshing`, `reauth_required`, `revoked-grace`. **Not persisted** to `credentials.state_kind`.
- **Persisted `state_kind` values** (from `credentials.state_kind` column per migration `0017_credentials_v3.sql`): `active` / `refreshing` / `expired` / `revoked` / `suspended`.

Terminal diagram states with same spelling (`active`, `revoked`) equal the persisted `state_kind`. Transient states map to persisted values per the mapping table in §7.1 (refresh strategy): `refresh_pending` + `refreshing` → persisted `refreshing`; `reauth_required` → `suspended`; `revoked-grace` → `revoked` after grace expiry; transient `failed` (refresh-transient-error path) stays persisted `active` pending retry.

### §4.1 Creation strategies

Four creation strategies per register row `user-lifecycle-creation`. Each maps to a distinct entry point + invariant set.

**(a) Interactive (OAuth2 / device code).** User clicks "Connect" in UI; engine initiates a multi-step `Credential::resolve` returning `ResolveResult::Pending(p)`; pending state encrypted and persisted to `pending_credentials` table; user redirected to IdP; IdP callback hits `nebula-api`; api dispatches to `Credential::continue_resolve(pending, continuation)` which returns `ResolveResult::Ready(state)` on success; state encrypted and persisted to `credentials` table with `state_kind = 'active'`.

State machine:

```
[idle] --user clicks Connect--> [resolving]
[resolving] --returns Pending--> [pending]
[pending] --IdP callback--> [continuing]
[continuing] --returns Ready--> [active] (write credentials row)
[continuing] --returns Pending--> [pending]  (multi-step chain)
[pending] --timeout (10 min)--> [discarded]  (GC sweep deletes row)
[continuing] --error--> [failed]  (audit + delete pending)
```

**(b) Programmatic (API call with plaintext input).** Caller posts to `POST /credentials` with `Input` body (typed schema per §2.1 `Credential::Input`); engine calls `Credential::resolve` synchronously; non-interactive credentials return `ResolveResult::Ready` directly; state persisted. Used for static credentials (API key, Basic auth, signing keys imported from KMS).

**(c) Imported (from file or external secret store).** Operator imports via CLI or admin API with file path or external provider URI; engine reads via `ExternalProvider` trait (Vault, AWS SM, GCP SM, Azure KV — impls in `nebula-storage/src/external_providers/`); resolves to `State`; persisted as Pattern (b).

**(d) Bootstrapped (from environment at startup).** Service-level credentials configured via env vars (e.g., `NEBULA_DEFAULT_OAUTH_GOOGLE_CLIENT_ID` + secret material); read at engine startup; registered in `CredentialRegistry` per §3.1; persisted to `credentials` table on first use. Distinct from (c) by who owns the source: env = service-operator at boot; file/vault = explicit operator action.

**Validation hooks** (per `user-disc-validation` register row, full detail Checkpoint 3 §9.3):

- **Schema validation** — `<Self::Input as HasSchema>::schema()` validates input shape before any IdP call.
- **Semantic validation** — optional `Credential::test()` post-resolution to verify the resulting state actually works against the provider.
- **UX validation** — form hints from `CredentialMetadata::field_hints()` rendered in UI; validates client-side before submission.

### §4.2 Update & rotation

Four update sub-strategies per register row `user-lifecycle-update`. All eventually mutate the `credentials` row's `encrypted_secret` + bump `version` (CAS) + write `credential_audit` entry with `operation = 'rotated'` (or `refreshed` for non-rotation refreshes).

**(a) User-initiated update.** Operator uploads new credential material via UI/API; engine calls `Credential::resolve` with new input; old state replaced atomically (CAS on `version`).

**(b) Provider-initiated refresh.** Engine detects state nearing expiry (`expires_at - now < refresh_lead_time`) and calls `Credential::refresh(state)`. Coordinated by `nebula-engine::credential::refresh::RefreshCoordinator` (in-proc L1: `parking_lot::Mutex` keyed by `credential_id`); cross-replica coordination via `RefreshClaimRepo` per **OUT** sub-spec [`draft-f17`](2026-04-24-credential-refresh-coordination.md). The refresh path:

```
[active] --expires_at - now < lead_time--> [refresh_pending]
[refresh_pending] --L1 claim--> [refreshing]   (state_kind transition)
[refreshing] --refresh() ok--> [active]        (new state, version++)
[refreshing] --refresh() Permanent err--> [reauth_required]
[refreshing] --refresh() Transient err--> [active] (retry per backoff)
[reauth_required] --user re-auths via §4.1(a)--> [active]
```

**(c) Scheduled rotation.** Per `CredentialMetadata::rotation_policy`, engine's rotation scheduler (per ADR-0030) periodically composes `revoke` (best-effort) + new `resolve` cycle. **Multi-replica leader election** for the scheduler is **OUT** — `RotationLeaderClaimRepo` sub-spec (Strategy §6.5 queue #2).

**(d) Emergency rotation.** Triggered by compromise-response runbook (**OUT** queue #8). Synchronous force-rotate that bypasses normal backoff; revokes all in-flight tokens; requires re-auth.

### §4.3 Revocation

Three revocation modes per register row `user-lifecycle-revocation`. All update `state_kind` and write `credential_audit`.

**Soft revocation (default).** `state_kind = 'revoked'` + `revoked_at = now()`; in-flight resolves continue for the configured grace window (default 30 s, per `CredentialMetadata::revocation_grace`). After grace, all resolves return `ResolveError::Revoked`. The credential row is preserved for audit; restoration is possible within the retention window (§4.4).

**Hard revocation.** Immediately invalidate; in-flight resolves fail with `ResolveError::Revoked` even if they started before revocation. No grace window. Used for compromise response.

**Cascade revocation.** Revoking a "parent" credential (e.g., a long-lived OAuth2 refresh token) cascades to dependent tokens (the access tokens it minted). Detection relies on `credential_audit` foreign-key relationships (via `credential_id` on audit entries + parent-child markers in audit `detail` JSONB) + operator intervention. Rare — only credentials with explicit dependency relationships. **No trait method on `Credential`**; cascade is handled by storage-layer query + manual operator action to preserve the lean trait surface (Strategy §3.6 trait-heaviness discipline). See §6.11 for the audit-FK-based cascade revocation handler.

State transitions:

```
[active] --soft revoke--> [revoked-grace] --grace expires--> [revoked]
[active] --hard revoke--> [revoked]
[any] --cascade revoke--> apply same to dependents
```

The provider-side revocation is **best-effort** — `Credential::revoke(state)` is called but failures are logged, not propagated. Storage tombstone is the source of truth.

### §4.4 Deletion

Two deletion modes per register row `user-lifecycle-deletion`.

**Soft delete (default).** `deleted_at = now()` set on the `credentials` row; row remains in storage. Workflow references that resolve to a deleted credential get `ResolveError::Deleted`. Retention window (default 90 days, per service config) before purge eligibility.

**Hard purge.** Run by retention sweep (engine background task) or operator-initiated `DELETE /credentials/:id?purge=true`. Wipes the row; audit log retains the deletion record + final state hash for forensics.

**Cascading on workflow refs.** Workflow definitions hold `CredentialKey` strings, not foreign keys. Workflows referencing a soft-deleted credential are flagged by the workflow validation pass; operator notified to update or unbind. Hard purge does NOT cascade to workflow rewrites — workflows referencing purged credentials simply fail at execution with `ResolveError::NotFound`.

DDL touch:

```sql
-- credentials.deleted_at TIMESTAMPTZ (already in production migration 0008)
-- index on deleted_at for retention sweep
CREATE INDEX idx_credentials_purge_eligible
    ON credentials (deleted_at)
    WHERE deleted_at IS NOT NULL;
```

Retention sweep query (executed daily):

```sql
DELETE FROM credentials
 WHERE deleted_at IS NOT NULL
   AND deleted_at < now() - INTERVAL '90 days';
```

### §4.5 Expiration

Per register row `user-lifecycle-expiration`. Three behaviors at expiry, configured per credential type via `CredentialMetadata::expiry_behavior`:

**Auto-refresh** (default for `REFRESHABLE` credentials). When `expires_at - now() < refresh_lead_time` (default 5 min), engine triggers refresh per §4.2(b). State transitions through `refresh_pending → refreshing → active` (or `reauth_required` on failure).

**Mark expired.** `state_kind = 'expired'`; no auto-refresh attempted. Used for credentials with finite manual lifetimes (API keys with provider-mandated rotation). Resolves return `ResolveError::Expired`. UI surfaces a "Renew" prompt to the operator.

**Notify only.** State stays `active`; engine emits `CredentialEvent::ExpiringSoon` to the eventbus at `expires_at - notify_lead_time` (default 24 h). UI / Slack / email integrations consume the event.

Grace period: if `expires_at` is reached but the credential has not been refreshed (transient failure path), state stays `active` for `expiry_grace` (default 60 s) to allow the next refresh attempt to complete. After grace, `state_kind = 'expired'`.

### §4.6 Migration v1→v2

**OUT — sub-spec `draft-f36`.** Schema migration on encrypted rows is non-trivial (must decrypt → migrate → re-encrypt without downtime). Pattern proposed in register row resolution: lazy migration on resolve (decrypt v1 → migrate to v2 → re-encrypt) + bulk migration CLI for batch processing.

Tech Spec consumers expect: a stable `Credential::State` shape per credential type, with version tagging (`#[credential_state(version = 2, migrate_from = v1)]` or equivalent). The migration mechanism itself lives in the sub-spec.

### §4.7 Import / export

**OUT — Strategy §6.5 queue #12 (low priority, post-Tech-Spec).** Encrypted backup format + n8n-compat import. Tech Spec consumers expect: a self-describing encrypted tarball format with envelope keys + state material; an importer that maps n8n's credential JSON shape to nebula's typed `Credential::Input`. Sub-spec defines the format.

## §5 Storage schema & layer boundaries

Production already has substantial credential storage infrastructure (see migrations `0008_credentials.sql` + `0017_credentials_v3.sql` + `0015_audit.sql`). Tech Spec §5 documents the existing reality + adds three NEW tables required by the redesign (RefreshClaimRepo, RotationLeaderClaimRepo, ProviderRegistryRepo per Strategy §6.5).

### §5.1 Layer stack (canonical order)

The credential storage layers wrap each other in a fixed order. From outermost to innermost:

```
            ↓ caller (engine, api)
┌───────────────────────────────────────┐
│  ScopeLayer    — tenant × workspace  │
│                  × workflow filter   │
├───────────────────────────────────────┤
│  AuditLayer    — fail-closed write   │
│                  + degraded mode     │
├───────────────────────────────────────┤
│  CacheLayer    — L1 in-proc + TTL    │
│                  + invalidation      │
├───────────────────────────────────────┤
│  EncryptionLayer — AES-256-GCM + AAD │
│                  + KeyProvider envelope│
├───────────────────────────────────────┤
│  CredentialStore — raw persistence   │
│                  (Postgres/SQLite/mem)│
└───────────────────────────────────────┘
            ↓ database
```

**Why this order — outer-to-inner rationale:**

1. **Scope is outermost** — every operation is filtered by tenant/workspace before any other layer sees it. A scope-mismatch never reaches encryption (no wasted decrypt) or audit (no audit pollution from forbidden access attempts; those go to a separate access-violation log).
2. **Audit is above cache** — audit writes happen on every operation regardless of cache hit. Cache hits still produce audit entries (with `operation = 'accessed'`). Audit fail-closed semantics protect: if audit is unavailable, no operation proceeds (fall through to degraded mode per §6.5 in CP2b).
3. **Cache is above encryption** — cache stores the **decrypted** `State` for hot-path resolves. Cache TTL bounded; cache invalidation channel via `nebula-eventbus::CacheInvalidation` per §6.2 (CP2b). Decryption is amortized across cache lifetime.
4. **Encryption is innermost** — AES-256-GCM + AAD + KeyProvider envelope per `0017_credentials_v3.sql` `envelope JSONB` shape. Bit-preserved per Strategy §1.2 non-goal. Storage layer below sees only opaque bytes.
5. **CredentialStore is the bottom** — raw key-value persistence. Three impls: Postgres (production), SQLite (desktop / dev), in-memory (tests).

This ordering matches existing production layer modules (`crates/storage/src/credential/layer/{scope,audit,cache,encryption}.rs`).

### §5.2 Existing schema (production)

Three production tables. DDL summarised; full DDL in migrations.

**`credentials`** (primary credential storage, migrations 0008 + 0017_v3):

```sql
CREATE TABLE credentials (
    id                  BYTEA PRIMARY KEY,           -- cred_ ULID
    org_id              BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id        BYTEA REFERENCES workspaces(id) ON DELETE CASCADE,
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    kind                TEXT NOT NULL,               -- credential type key
    scope               TEXT NOT NULL,               -- 'workspace' or 'org'
    encrypted_secret    BYTEA NOT NULL,              -- envelope-encrypted
    encryption_version  INT NOT NULL,                -- key rotation
    envelope            JSONB,                       -- {kek_id, encrypted_dek, algorithm, nonce, aad_digest}
    state_kind          TEXT NOT NULL DEFAULT 'active',  -- active|refreshing|expired|revoked|suspended
    lease_id            TEXT,                        -- dynamic secret lease ID
    expires_at          TIMESTAMPTZ,                 -- lease expiry / cred TTL
    allowed_workspaces  BYTEA[],                     -- for org-level
    metadata            JSONB,                       -- non-secret data
    created_at          TIMESTAMPTZ NOT NULL,
    created_by          BYTEA NOT NULL,
    last_rotated_at     TIMESTAMPTZ,
    last_used_at        TIMESTAMPTZ,
    version             BIGINT NOT NULL DEFAULT 0,   -- CAS
    deleted_at          TIMESTAMPTZ                  -- soft delete
);

-- Unique slug constraints + retention sweep index per §4.4 + expiry index.
CREATE UNIQUE INDEX idx_credentials_workspace_slug
    ON credentials (workspace_id, LOWER(slug))
    WHERE scope = 'workspace' AND deleted_at IS NULL;
CREATE UNIQUE INDEX idx_credentials_org_slug
    ON credentials (org_id, LOWER(slug))
    WHERE scope = 'org' AND deleted_at IS NULL;
CREATE INDEX idx_credentials_expiring
    ON credentials (expires_at)
    WHERE expires_at IS NOT NULL AND deleted_at IS NULL;
```

**`pending_credentials`** (in-progress interactive flows, migration 0017_v3):

```sql
CREATE TABLE pending_credentials (
    id              BYTEA PRIMARY KEY,               -- ULID
    org_id          BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id    BYTEA REFERENCES workspaces(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,                   -- credential type
    state_encrypted BYTEA NOT NULL,                  -- encrypted Pending state
    initiated_by    BYTEA NOT NULL,                  -- user who started
    created_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL             -- auto-cleanup timeout
);

CREATE INDEX idx_pending_credentials_cleanup
    ON pending_credentials (expires_at);
```

**`credential_audit`** (HMAC hash-chain tamper-evident, migration 0017_v3):

```sql
CREATE TABLE credential_audit (
    id              BYTEA PRIMARY KEY,
    org_id          BYTEA NOT NULL,
    credential_id   BYTEA NOT NULL,                  -- may reference deleted
    seq             BIGINT NOT NULL,                 -- per-credential monotonic
    principal_kind  TEXT NOT NULL,
    principal_id    BYTEA,
    operation       TEXT NOT NULL,                   -- created|rotated|refreshed|revoked|accessed|deleted
    result          TEXT NOT NULL,                   -- success|failure
    detail          JSONB,
    prev_hmac       BYTEA,                           -- HMAC of previous entry (NULL = first)
    self_hmac       BYTEA NOT NULL,                  -- hash chain anchor
    emitted_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_credential_audit_by_cred
    ON credential_audit (credential_id, seq);
CREATE INDEX idx_credential_audit_by_org
    ON credential_audit (org_id, emitted_at DESC);
```

The hash chain (each row carries `prev_hmac` + `self_hmac` derived from prev) makes tampering detectable: verifier walks the chain, recomputing `self_hmac` from `(prev_hmac || row content || HMAC-key)`. Any mid-chain mutation breaks all subsequent `self_hmac`s.

### §5.3 New tables (Tech Spec adds)

Three NEW tables required by the redesign per Strategy §6.5 sub-spec queue. Tech Spec describes the **consumer-side** schema; full producer-side design (admin API, seeding, versioning) lives in respective sub-specs.

**`refresh_claims`** (RefreshClaimRepo backing — `draft-f17` consumer surface):

```sql
CREATE TABLE refresh_claims (
    credential_id   BYTEA PRIMARY KEY REFERENCES credentials(id) ON DELETE CASCADE,
    claim_token     BYTEA NOT NULL,                  -- ULID; unique per claim attempt
    claimed_by      TEXT NOT NULL,                   -- replica identifier (hostname:pid:nonce)
    claimed_at      TIMESTAMPTZ NOT NULL,
    heartbeat_at    TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL             -- TTL after which claim auto-released
);

CREATE INDEX idx_refresh_claims_expiry ON refresh_claims (expires_at);
```

Single row per credential at a time; PRIMARY KEY on `credential_id` enforces single-claimant invariant. Claim acquisition via `INSERT … ON CONFLICT (credential_id) DO UPDATE WHERE expires_at < now() RETURNING claim_token` — atomic claim-or-existing-token. Heartbeat updates `heartbeat_at` + extends `expires_at` while refresh in progress. Release deletes the row.

**`rotation_leader_claims`** (RotationLeaderClaimRepo — Strategy §6.5 queue #2 consumer surface):

```sql
CREATE TABLE rotation_leader_claims (
    scope           TEXT PRIMARY KEY,                -- e.g., 'global', 'tenant:{org_id}'
    leader_id       TEXT NOT NULL,                   -- replica identifier
    claimed_at      TIMESTAMPTZ NOT NULL,
    heartbeat_at    TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL
);
```

Leader-elected scheduler: only one replica's rotation scheduler runs at a time per scope. Same claim/heartbeat/expire pattern as `refresh_claims`. Producer-side election protocol detail in queue #2 sub-spec.

**`provider_registry`** (ProviderRegistryRepo — `draft-f18/f19/f20` consumer surface):

```sql
CREATE TABLE provider_registry (
    provider_id         TEXT PRIMARY KEY,            -- e.g., 'slack', 'github', 'azure-ad'
    spec_version        INT NOT NULL,                -- bump on schema change
    spec                JSONB NOT NULL,              -- ProviderSpec (endpoints, scopes, template_vars)
    spec_hash           BYTEA NOT NULL,              -- digest for audit
    updated_at          TIMESTAMPTZ NOT NULL,
    updated_by          BYTEA NOT NULL               -- admin user ID
);
```

Consumer-side reads: `SELECT spec FROM provider_registry WHERE provider_id = $1`. Producer-side admin operations (insert / update / version migration / URL template substitution) live in `draft-f18/f19/f20` sub-spec.

### §5.4 Postgres ↔ SQLite parity

Production maintains parity between Postgres and SQLite migration scripts (see `crates/storage/migrations/{postgres,sqlite}/`). Tech Spec preserves this discipline.

**Dialect translation table:**

| Postgres | SQLite |
|---|---|
| `BYTEA` | `BLOB` |
| `JSONB` | `TEXT` (JSON serialized) |
| `TIMESTAMPTZ` | `INTEGER` (Unix epoch microseconds) or `TEXT` (ISO 8601) |
| `BIGINT` | `INTEGER` |
| `BYTEA[]` | `TEXT` (JSON array) — array types not native in SQLite |
| `INSERT ... ON CONFLICT ... DO UPDATE` | `INSERT ... ON CONFLICT ... DO UPDATE` (since SQLite 3.24, available since 2018) |

**CI parity gate:** every credential migration `0NNN_xxx.sql` must exist in both `migrations/postgres/` and `migrations/sqlite/`. CI script walks the two directories and fails if migration numbers differ. Per `draft-f28`.

**NoOpClaimRepo for desktop.** Single-replica desktop deployments do not need cross-replica claim coordination. `NoOpRefreshClaimRepo` returns `Ok(claim_token)` immediately without touching storage; `NoOpRotationLeaderClaimRepo` returns `Ok(LeaderHeld)` always. Engine dispatches to NoOp impls when `deployment_mode = Desktop` per §11.

### §5.5 Storage repos — consumer interfaces

Three NEW repos beyond existing `CredentialStore`. Trait surfaces below; full producer-side implementations in sub-specs.

**`CredentialStore` (existing, stable per ADR-0032):**

```rust
pub trait CredentialStore: Send + Sync {
    fn get(&self, id: &str)
        -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    fn put(&self, credential: StoredCredential, mode: PutMode)
        -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    fn delete(&self, id: &str, mode: DeleteMode)
        -> impl Future<Output = Result<(), StoreError>> + Send;

    fn list(&self, filter: ListFilter)
        -> impl Future<Output = Result<Vec<StoredCredential>, StoreError>> + Send;
}
```

Existing surface — not changed by Tech Spec. Wrapped by the §5.1 layer stack. `StoredCredential` is the encrypted-on-the-wire DTO; engine receives it after Encryption layer decrypt + Cache layer hit/miss tracking + Audit layer write.

**`CredentialContext::load_state<C>` — closes §3.4 forward-dep:**

```rust
impl CredentialContext<'_> {
    /// Load + decrypt credential state for a known concrete type C.
    /// Engine-internal — wraps the layer stack: Scope filter →
    /// Audit write (operation = 'accessed') → Cache hit-or-miss →
    /// Encryption decrypt → CredentialStore::get → State::deserialize.
    pub(crate) async fn load_state<C: Credential>(
        &self,
        key: &str,
    ) -> Result<&C::State, ResolveError> {
        // ... layer-stack invocation; cached references owned by ctx.
    }
}
```

Used by `resolve_as_X` capability-helper functions per §3.4 step 3. Cache-borrowed lifetime tied to `&self` of `CredentialContext`. Concrete impl in `nebula-engine`.

**`RefreshClaimRepo`** (consumer surface — producer in `draft-f17` sub-spec):

```rust
pub trait RefreshClaimRepo: Send + Sync {
    /// Try to claim the refresh slot for a credential. Returns the
    /// active claim token (which may be ours or held by another
    /// replica). Caller compares returned `claimed_by` to determine
    /// ownership.
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        ttl: Duration,
    ) -> Result<RefreshClaim, RepoError>;

    /// Heartbeat an existing claim — extends expires_at.
    async fn heartbeat(
        &self,
        credential_id: &CredentialId,
        claim_token: &ClaimToken,
    ) -> Result<(), RepoError>;

    /// Release the claim on success or abort.
    async fn release(
        &self,
        credential_id: &CredentialId,
        claim_token: &ClaimToken,
    ) -> Result<(), RepoError>;
}

pub struct RefreshClaim {
    pub credential_id: CredentialId,
    pub claim_token: ClaimToken,
    pub claimed_by: String,
    pub expires_at: SystemTime,
}
```

Producer-side: claim acquisition SQL, heartbeat cadence, mid-refresh crash reclaim — all in [`docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`](2026-04-24-credential-refresh-coordination.md).

**`RotationLeaderClaimRepo`** (consumer surface — producer in queue #2 sub-spec):

```rust
pub trait RotationLeaderClaimRepo: Send + Sync {
    async fn try_become_leader(
        &self,
        scope: LeaderScope,
        ttl: Duration,
    ) -> Result<LeaderStatus, RepoError>;

    async fn heartbeat(
        &self,
        scope: &LeaderScope,
        leader_id: &str,
    ) -> Result<(), RepoError>;

    async fn release(
        &self,
        scope: &LeaderScope,
        leader_id: &str,
    ) -> Result<(), RepoError>;
}

pub enum LeaderStatus {
    Acquired { leader_id: String, expires_at: SystemTime },
    HeldByOther { leader_id: String, expires_at: SystemTime },
}
```

**`ProviderRegistryRepo`** (consumer surface — producer in `draft-f18/f19/f20`):

```rust
pub trait ProviderRegistryRepo: Send + Sync {
    /// Read-only consumer access to provider specs. Producer-side
    /// admin API (insert/update/version) lives in sub-spec
    /// draft-f18/f19/f20.
    async fn get_provider(
        &self,
        provider_id: &str,
    ) -> Result<Option<ProviderSpec>, RepoError>;

    async fn list_providers(&self)
        -> Result<Vec<ProviderSpecSummary>, RepoError>;
}

pub struct ProviderSpec {
    pub provider_id: String,
    pub spec_version: u32,
    pub spec: ProviderSpecBody,            // endpoints, scopes, template_vars
    pub spec_hash: [u8; 32],
}
```

Tech Spec requires consumer code (engine OAuth flow, credential resolve, multi-mode feature matrix) to use this read-only surface only. Any producer-side mutation goes through sub-spec admin API; consumer code never inserts or updates `provider_registry` rows directly.

### §5.6 Migration discipline

**Schema versioning convention.** Migration files numbered `0NNN_descriptive_name.sql` per existing convention. Forward-only; no rollback past N (hand-rolled patches if needed). Per-table version metadata in `metadata` JSONB column where applicable.

**Dialect parity CI.** Per §5.4. Build fails if Postgres and SQLite migrations diverge in count or numbering.

**Encryption-version migration.** When `encryption_version` bumps (key rotation, algorithm change), the walker CLI per `user-sec-key-rotation` (CP2b §6.2) iterates rows: decrypt with old key → re-encrypt with new key → bump `encryption_version` + `version` (CAS). Per-table `WHERE encryption_version = $old` query drives the iteration.

**State schema migration v1→v2.** **OUT** — `draft-f36` sub-spec. Tech Spec consumers expect: lazy migration on resolve (decrypt v1 → migrate to v2 → re-encrypt) + bulk CLI for batch processing. Migration mechanism mechanism itself in sub-spec.

## §6 Security

Security contract preserves Strategy §1.2 non-goal invariants (§12.5 crypto bit-for-bit, zeroize boundaries). Implementation-level detail for 10 `user-sec-*` register rows + cascade revocation handler (§6.11 from Nit 2 resolution).

### §6.1 Encryption-at-rest (§12.5 preserved bit-for-bit)

Envelope shape per production migration `0017_credentials_v3.sql`:

```
envelope (JSONB, stored alongside ciphertext in plaintext):
  {
    kek_id: "uuid-of-kek",            // lookup key for KeyProvider
    encrypted_dek: <bytes>,           // DEK wrapped with KEK
    algorithm: "AES-256-GCM",         // fixed per §12.5
    nonce: <12 bytes>,                // 96-bit random per encrypt
    aad_digest: <32 bytes>            // SHA-256 of AAD for integrity
  }

encrypted_secret (BYTEA):
  AES-256-GCM(DEK, nonce, AAD) of serde-serialized State bytes
```

**AAD construction** (bound to specific credential to defeat replay):

```
AAD = credential_id (16 bytes ULID) ||
      kek_id         (16 bytes)     ||
      encryption_version (4 bytes big-endian u32)
```

**Decrypt flow:**

1. Read `envelope` JSONB + `encrypted_secret` BYTEA from `credentials` row.
2. `KeyProvider::get_kek(kek_id)` — returns the KEK material (opaque; lives in HSM/KMS/memory depending on mode per §11).
3. Unwrap DEK: decrypt `encrypted_dek` with KEK. DEK is an ephemeral 32-byte key.
4. Verify AAD: compute `SHA-256(AAD_bytes)` and match `aad_digest`. Mismatch → `DecryptError::AadMismatch` (possible tampering or row rebinding attack).
5. Decrypt `encrypted_secret` with DEK + nonce + AAD via AES-256-GCM. Failure → `DecryptError::Tampered`.
6. Deserialize plaintext bytes via `serde_json::from_slice::<C::State>()`.

Encrypt flow is the inverse. Zeroize the DEK + plaintext buffers immediately after use per §6.7.

**Invariants (canonical, bit-preserved per Strategy §1.2 non-goal):**

- Algorithm fixed to AES-256-GCM. No negotiation; no fallback. Changing algorithm is an ADR.
- Nonce always 96-bit random from CSPRNG. Never reused per (KEK, DEK) pair.
- AAD always includes `credential_id + kek_id + encryption_version` — enables KEK rotation without re-encrypting every row.
- DEK wrapped with KEK. Raw KEK never leaves `KeyProvider`.
- `KeyProvider` is the sole component that touches raw KEK material.

### §6.2 Key rotation

`KeyProvider` supports multiple active KEKs during a rotation window. Old KEK still decryptable for existing rows; new KEK encrypts new rows and is re-wrapping target for walker.

**`nebula credential rotate-master-key --from=<old_kek_id> --to=<new_kek_id>` walker CLI:**

```
1. Generate or ingest new KEK (KMS / HSM / env-provided per §11).
   Register new KEK in KeyProvider at a new version.
2. Walk `credentials` table (paginated, cursor by `id`):
   a. For rows with envelope.kek_id == old_kek_id:
      b. Load KEK_old via KeyProvider.
      c. Unwrap DEK with KEK_old.
      d. Re-wrap DEK with KEK_new.
      e. Update envelope.kek_id = new_kek_id + bump encryption_version.
      f. CAS on `version` to avoid concurrent-rotation clobber.
   No re-encrypt of State needed — only the DEK is rewrapped.
3. Walk `pending_credentials` same way (no CAS needed — single-writer).
4. `credential_audit` is NOT envelope-encrypted (uses HMAC hash chain,
   see §6.5) — skipped.
5. After walker completes + retention window (default 30 days), decommission
   KEK_old via KeyProvider::retire_kek(old_kek_id).
```

Walker is **online** — each row is CAS-updated independently; no global lock. Concurrent reads continue (decrypt uses whichever `kek_id` is stamped on the row).

`KeyProvider::with_legacy_keys(old_fp)` supports lazy re-wrap on resolve: if the walker hasn't reached a row yet but a read happens, the read decrypts with old KEK without re-wrapping. Walker eventually rewraps.

### §6.3 Access control — RBAC matrix

Operations × roles matrix. Roles are org-scoped; additional per-workspace scoping via `allowed_workspaces` on credentials.

| Operation | Admin | Operator | Developer | Viewer |
|---|---|---|---|---|
| `credentials.create` | ✓ | ✓ | own workspace only | ✗ |
| `credentials.read` (redacted metadata) | ✓ | ✓ | own workspace / org if scope=org | ✓ |
| `credentials.resolve` (decrypt + use) | ✓ | ✗ | own workspace only | ✗ |
| `credentials.update` | ✓ | ✓ | own + not rotated_by_another | ✗ |
| `credentials.revoke` | ✓ | ✓ (if own) | own only | ✗ |
| `credentials.delete` (soft) | ✓ | ✓ | own only | ✗ |
| `credentials.purge` (hard) | ✓ | ✗ | ✗ | ✗ |
| `credentials.rotate` (manual) | ✓ | ✓ | own only | ✗ |
| `registry.provider.admin` | ✓ | ✗ | ✗ | ✗ |
| `credentials.key_rotate` (master key walker) | ✓ | ✗ | ✗ | ✗ |

Enforcement via `ScopeLayer` (see §6.4) — RBAC decision happens at request-entry in `nebula-api` based on principal's role; rejections never reach storage.

`Developer` role's "own" means: creator (created_by) OR workspace member with write permission. Per-credential ACL can narrow further via `allowed_workspaces[]`.

### §6.4 Scope isolation

Tenant × workspace × user boundaries enforced at the storage layer via `ScopeLayer` (production — `crates/storage/src/credential/layer/scope.rs`).

**Scope hierarchy:**

1. **`org_id`** — required on every credential operation. ScopeLayer rejects requests without a valid org_id principal.
2. **`workspace_id`** — scoped within org. If `scope = 'workspace'`, only that workspace can access. If `scope = 'org'`, all workspaces in org can access (subject to `allowed_workspaces[]` filter).
3. **User principal** — tracked in audit (`created_by`, `principal_id` on audit entries). No direct access check at credential level (RBAC §6.3 governs).

**FK constraints** (existing):

- `credentials.org_id REFERENCES orgs(id) ON DELETE CASCADE`
- `credentials.workspace_id REFERENCES workspaces(id) ON DELETE CASCADE` (nullable for org-scoped)
- `credential_audit.org_id` (no FK — audit survives org deletion)

**Cross-scope access prohibition.** ScopeLayer enforces: a request with `org_id = A` cannot read/write credentials with `org_id = B`, even if principal is admin of both. Explicit admin grant (per-credential `allowed_workspaces[]`) is the only cross-scope mechanism. No superuser that bypasses scope isolation.

### §6.5 Audit

Audit is fail-closed by default with a documented degraded read-only mode for audit-storage outages. Hash-chain integrity makes tampering detectable.

**Normal mode — fail-closed.**

Every credential operation (create / read / update / revoke / delete / purge / rotate / refresh / test / access) writes a `credential_audit` row **before** the operation commits. If the audit write fails (storage error, timeout), the operation fails with `AuditError::WriteFailed`. No silent success.

**Write sequence (atomic per operation):**

```
BEGIN TRANSACTION
  1. INSERT INTO credential_audit (
       id, org_id, credential_id, seq, principal_kind, principal_id,
       operation, result='pending', detail, prev_hmac, self_hmac,
       emitted_at
     )
     -- seq is per-credential_id monotonic (engine computes via COUNT or
     -- serial sequence scoped to credential_id)
  2. EXECUTE operation on credentials/pending_credentials
  3. UPDATE credential_audit SET result = 'success' | 'failure' WHERE id = audit_id
COMMIT
```

Failure in step 2 → step 3 writes `result = 'failure'`, transaction commits with audit recording the attempt. Step 1 failure → transaction aborts, operation fails.

**HMAC hash-chain verification.**

Each row stores `prev_hmac` (HMAC of previous audit entry for same credential_id) + `self_hmac` (HMAC of this entry's content chained from prev_hmac).

```
self_hmac = HMAC-SHA-256(
    key = audit_chain_key,   // per-org secret, managed by KeyProvider
    input = prev_hmac || id || credential_id || seq || principal ||
            operation || result || detail || emitted_at
)
```

Verifier walks chain by `(credential_id, seq ASC)`, recomputing `self_hmac` from `(prev_hmac || row content || audit_chain_key)`. Any mutation of a row invalidates all subsequent `self_hmac`s in the chain. Tampering detection → `AuditError::ChainBroken(credential_id, seq)`.

**Degraded read-only mode.**

Triggered when audit storage is unreachable for >5 seconds. Engine enters degraded mode:

- **Read operations continue.** `resolve` still succeeds against cached state; audit writes go to a local file buffer.
- **Write operations blocked.** `create` / `update` / `revoke` / `delete` / `rotate` all return `ServiceUnavailable` because the audit write gate fails.
- **Refresh allowed with caveat.** Automatic refresh is blocked (can't audit); manual operator-triggered refresh via explicit override returns `DegradedMode` warning.

Local fallback sink: audit writes to a bounded file buffer (`/var/lib/nebula/audit-fallback.jsonl`, size-capped). When audit storage recovers, a drain task replays buffered entries in order, re-computing hash chain. If chain recomputation fails (e.g., entries went missing), operator notification via `CredentialEvent::AuditChainBroken`.

### §6.6 Redaction

Secrets never appear in logs, errors, debug output, or serialized state dumps. Enforced at the type layer.

**`SecretString` / `SecretBytes`** (from `nebula-schema`):

```rust
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

// Default Serialize impl: errors or writes "[REDACTED]" depending on
// the SerializeStage marker (Storage / Transport / Diagnostic).
impl Serialize for SecretString { /* … */ }

impl SecretString {
    // Single access point — caller must explicitly opt in.
    pub fn expose_secret(&self) -> &str { /* … */ }
}
```

**Rules:**

- `expose_secret()` only at injection site — HTTP header construction, DB connection string, cryptographic operation. Never at log or error-formatting site.
- Error types: `Debug` / `Display` impls formatted to omit secret material. Error messages reference credential ID (opaque ULID), never decrypted material.
- Log filter via `tracing` instrumentation: fields typed `SecretString` / `SecretBytes` auto-redacted by the subscriber.

**Checklist for reviewers** (per `credential-security-review` skill):

- No `format!("{}", secret)` or `format!("{:?}", secret)` without explicit `expose_secret()`.
- No `secret.to_string()` into log buffer.
- No `serde_json::to_string(&state_with_secrets)` for diagnostic output.

### §6.7 Zeroization invariants table

Canonical zeroization discipline per lifecycle stage:

| Stage | Memory lives | Zeroize mechanism |
|---|---|---|
| Pre-decrypt (from storage) | `Zeroizing<Vec<u8>>` ciphertext + `Zeroizing<Vec<u8>>` wrapped DEK | `ZeroizeOnDrop` (trait) + explicit `drop(buffer)` at boundary |
| Post-decrypt (plaintext State) | `SecretString` / `SecretBytes` fields inside `C::State` | `ZeroizeOnDrop` on containing struct; serde deserialization wraps in Secret types |
| Projection (`C::project(&state)`) | `C::Scheme` returned to engine | Scheme's `ZeroizeOnDrop` (where applicable; per §15.2 decision) |
| Resource boundary (per-request injection) | Scheme material borrowed for HTTP header / DB conn string | Request scope drop zeroizes Scheme |
| Execution cleanup (abort path) | `ExecutionCredentialStore` state | Explicit `cleanup()` call at execution teardown (per `draft-f25`); Drop is best-effort |
| Audit `detail` JSONB | Non-sensitive operation metadata | No secret material allowed in detail; enforced by audit-write code path |

**Non-negotiable invariants:**

- Ciphertext decrypt buffer zeroized within one async-task-local scope.
- Plaintext State never serialized to disk outside encrypted `credentials.encrypted_secret`.
- `CredentialGuard<S>` RAII wrapper zeroizes Scheme at drop. Guard lifetime tied to resource-boundary usage.
- Test code uses `SecretString::expose_secret()` only in assertion bodies, never in test output format strings.

### §6.8 Egress control — SSRF mitigation

All outgoing IdP calls go through registered provider endpoints in `provider_registry`. User cannot inject arbitrary URL via credential config — URL comes from `ProviderSpec` (registry read-only consumer surface per §5.5), not user `Credential::Input`.

**SSRF mitigations (layered):**

1. **Endpoint allowlist.** `ProviderSpec.authorize_endpoint` and `ProviderSpec.token_endpoint` are explicit strings set by operator (self-hosted/desktop) or Anthropic-curated (cloud). User-editable `Credential::Input` fields hold only binding variables (client_id, scopes, tenant for Microsoft multi-tenant template), not URLs.
2. **URL template validation.** Microsoft multi-tenant case (`draft-f20`): template variables validated against per-spec regex (`tenant` must be UUID or "common" / "organizations" / "consumers") at credential activation. Invalid binding → `ResolveError::ProviderBindingInvalid`.
3. **TLS required.** `reqwest::Client` configured with `min_tls_version(Tls12)`; plain HTTP rejected.
4. **Private IP blocklist** (cloud mode; optional in self-hosted). Block 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.0/8, 169.254.0.0/16 (AWS IMDSv1), ::1, fe80::/10. Resolved via DNS lookup before HTTP call; if destination resolves to blocked range, request rejected. Desktop mode: allowlist localhost for dev but still blocks link-local IMDSv1.
5. **Redirect policy.** `reqwest::Client::redirect(Policy::none())` or `Policy::limited(3)` — follow at most a small bounded number, and re-check target against allowlist on each hop.

### §6.9 Session binding

`PendingStore` security for interactive OAuth2 flows:

| Element | Rule |
|---|---|
| CSRF state parameter | 128-bit random from CSPRNG, single-use. Stored in `pending_credentials.state_encrypted` along with PKCE verifier. |
| PKCE code_verifier | 32 bytes random from CSPRNG, base64url-encoded (43 chars). Per RFC 7636. |
| PKCE code_challenge | `base64url(SHA-256(code_verifier))`. `code_challenge_method = S256` only. `plain` rejected. |
| Pending state TTL | 10 minutes default (per `CredentialMetadata::pending_ttl`). Expired entries swept by GC job every minute. |
| Single-use | `get_then_delete` transactional pop: rows are deleted as part of `continue_resolve`. Replay of same state → `PendingError::NotFound`. |
| Session cookie | `Secure; HttpOnly; SameSite=Lax`. Issued by `nebula-api` after successful credential creation. Lifetime bounded. |
| GC sweep | Periodic (cadence = 60 s) `DELETE FROM pending_credentials WHERE expires_at < now()`. |

### §6.10 Compromise response

**OUT — Strategy §6.5 queue #8** sub-spec. Tech Spec consumers expect: a documented runbook covering detection (failed-auth spike correlation, anomaly detection, operator-reported compromise), response (auto-revoke compromised credential, quarantine related credentials via cascade per §6.11, audit chain fork detection), and recovery (re-issue with forced reauth, notify affected workflows). Runbook owner is security-lead per register `user-sec-compromise-response`.

### §6.11 Cascade revocation handler — Nit 2 resolution

Cascade revocation uses **audit FK relationships + operator query** rather than a trait method. Rationale (preserves Strategy §3.6 trait-heaviness discipline):

- Cascade is rare — only credentials with explicit parent-child dependency (OAuth2 refresh-token → minted access tokens; AWS STS AssumeRole → session tokens).
- Dependency relationships are recorded in `credential_audit.detail` JSONB as parent-child markers at token-minting time (e.g., `detail = {"kind": "minted_from", "parent_credential_id": "<uuid>", "grant_type": "refresh_token"}`).
- Revocation cascade is an operator-driven flow, not a runtime hot path:

```
When revoking credential P with cascade flag:
  1. Query credential_audit for rows where
       detail->>'parent_credential_id' = P.id
       AND operation = 'created'
     → list of child credentials.
  2. For each child, apply soft revoke (state_kind = 'revoked') via engine.
  3. Audit each cascade-revoke entry with detail.cascade_from = P.id.
  4. Notify via CredentialEvent::Revoked with cascade_source field.
```

No trait method required. Revocation API accepts `cascade: bool` flag; handler queries audit DB and iterates. Expensive but rare — acceptable cost per frequency.

**Performance note.** The JSONB path query `detail->>'parent_credential_id'` is unindexed by default. For deployments where cascade revocation frequency rises (or audit table grows large), add a partial index:

```sql
CREATE INDEX idx_audit_cascade_parents
    ON credential_audit ((detail->>'parent_credential_id'))
    WHERE operation = 'created';
```

Current design assumes cascade is rare enough (manual operator action, not automated at rate) to not require the index. Operator adds it if cascade frequency or audit table size warrants.

## §7 Operational

Runtime behavior under load + failure. Maps register `user-op-*` cluster (7 rows) + `draft-f15/f16` (refresh two-tier coordinator) + failure modes matrix.

### §7.1 Refresh strategy — state vocabulary reconciled

Per Nit 1 from CP2a review, this section consolidates the two vocabularies.

**State vocabulary mapping table:**

| In-memory flow state | Persisted `state_kind` | Notes |
|---|---|---|
| `idle` | `active` | Credential at rest, no operation in flight. |
| `resolving` / `continuing` | N/A — row not yet created | Transient during `Credential::resolve` / `continue_resolve`. Persisted `credentials` row with `state_kind = 'active'` is written only after `ResolveResult::Ready` returns. |
| `pending` | N/A — lives in `pending_credentials` | OAuth2 callback awaited. Not in `credentials` row yet. |
| `discarded` | N/A — row deleted | Pending TTL expired or user abandoned. |
| `failed` (transient refresh error, non-terminal) | `active` | Engine retries per backoff; no `state_kind` change. |
| `refresh_pending` | `refreshing` | Refresh coordinator claimed; about to call `Credential::refresh`. |
| `refreshing` | `refreshing` | Refresh call in flight. |
| `reauth_required` | `suspended` | Refresh returned permanent error (e.g., revoked refresh_token). Operator must re-auth via §4.1(a). |
| `revoked-grace` | `revoked` (with grace window) | Soft revoke, in-flight ops within grace continue. |
| (terminal) `active` | `active` | Healthy. |
| (terminal) `revoked` | `revoked` | Post-grace soft-revoke OR hard revoke. |
| (terminal) `expired` | `expired` | TTL exceeded, no auto-refresh (or auto-refresh failed). |

**Refresh dispatch (resolves §3.4 step 2 forward-dep).**

Engine's per-credential-type refresh is driven by a `RefreshDispatcher` populated at plugin registration. The pattern mirrors resolve dispatch but operates on credential_id instead of slot bindings.

```rust
/// Per-type refresh function pointer. Closes over the concrete Credential
/// type C; erased for storage in the registry.
pub struct RefreshDispatcher {
    pub(crate) refresh_fn: for<'ctx> fn(
        &'ctx CredentialContext<'ctx>,
        &'ctx CredentialId,
    ) -> BoxFuture<'ctx, Result<RefreshOutcome, RefreshError>>,
    pub(crate) kind: &'static str, // Credential::KEY, for metrics
}

// Blanket builder — called by CredentialRegistry::register::<C>().
impl RefreshDispatcher {
    pub(crate) fn for_credential<C: Credential>() -> Self {
        Self {
            refresh_fn: |ctx, cred_id| Box::pin(refresh_worker::<C>(ctx, cred_id)),
            kind: C::KEY,
        }
    }
}

// Generic refresh worker — instantiated once per concrete C at monomorphization.
async fn refresh_worker<C: Credential>(
    ctx: &CredentialContext<'_>,
    cred_id: &CredentialId,
) -> Result<RefreshOutcome, RefreshError> {
    // 1. L1 in-proc coordinator claim.
    let l1_permit = ctx.refresh_coordinator.try_claim(cred_id).await;
    if !l1_permit.granted() {
        return Ok(RefreshOutcome::CoalescedWithOther);
    }

    // 2. L2 cross-replica claim (OUT — draft-f17 sub-spec).
    let claim = ctx.refresh_claim_repo
        .try_claim(cred_id, CLAIM_TTL)
        .await?;
    if claim.claimed_by != ctx.replica_id() {
        return Ok(RefreshOutcome::CoalescedWithReplica);
    }

    // 3. Load state (via layer stack per §5.5).
    let state: C::State = ctx.load_state::<C>(&cred_id.key()).await?.clone();

    // 4. Transition persisted state_kind: active → refreshing.
    ctx.transition_state_kind(cred_id, StateKind::Refreshing).await?;

    // 5. Call Credential::refresh — typed, dispatched at this call site.
    let outcome = match C::refresh(ctx, state).await {
        Ok(new_state) => {
            ctx.save_state::<C>(cred_id, &new_state).await?;
            ctx.transition_state_kind(cred_id, StateKind::Active).await?;
            RefreshOutcome::Refreshed
        }
        Err(e) if e.classify() == Severity::Transient => {
            // Keep state_kind = active; engine retries per backoff.
            ctx.transition_state_kind(cred_id, StateKind::Active).await?;
            return Err(e);
        }
        Err(e) => {
            // Permanent — reauth required.
            ctx.transition_state_kind(cred_id, StateKind::Suspended).await?;
            return Err(e);
        }
    };

    // 6. Release L2 claim.
    ctx.refresh_claim_repo.release(cred_id, &claim.claim_token).await?;

    // 7. Publish cache invalidation (per §7.2).
    ctx.eventbus.publish(CacheInvalidation { cred_id: cred_id.clone() });

    Ok(outcome)
}
```

The same `for<'ctx> fn(...) -> BoxFuture<...>` pattern underlies resolve dispatch in §3.4 — action macro emits slot bindings with the same erasure shape. Forward-dep closed.

**Proactive refresh.** Engine scheduler polls credentials where `expires_at - now() < refresh_lead_time` (default 5 min) and enqueues refresh tasks. Default cadence: 30 s.

**Reactive refresh.** Downstream call fails with 401 from provider → engine detects `Severity::AuthExpired` and triggers synchronous refresh + retry once.

**Multi-replica coordination** — OUT to `draft-f17`. Two-tier: L1 in-proc `RefreshCoordinator` (parking_lot Mutex keyed by credential_id), L2 cross-replica `RefreshClaimRepo` (§5.5 consumer surface).

### §7.2 Caching

Two-level cache for resolved State. Hot path reads from L1; L2 is the `credentials` table itself.

**L1 — in-proc per replica:**

```rust
pub struct ResolvedStateCache {
    entries: AHashMap<CredentialKey, CachedEntry>,
}

struct CachedEntry {
    state: Arc<ErasedState>,           // decrypted C::State, type-erased
    type_id: TypeId,                   // C's TypeId
    cached_at: Instant,
    ttl: Duration,                     // from CredentialMetadata::cache_ttl
}
```

- Default TTL: 5 minutes.
- Eviction: TTL expiry + explicit invalidation via `CacheInvalidation` eventbus subscription.
- No LRU — append-only within TTL; memory bounded by active credentials × state size.

**L2 — `credentials` table.** Every read path on cache miss decrypts from storage. Decrypt cost amortized across cache TTL.

**Invalidation channel.**

```rust
pub struct CacheInvalidation {
    pub cred_id: CredentialId,
    pub reason: InvalidationReason,   // Refreshed | Revoked | Rotated | Updated
}
```

Published on `nebula-eventbus` topic `credential.cache_invalidation`. Each replica subscribes + drops its L1 entry on receipt.

**Negative caching.** `NotFound` results cached for 10 s to prevent storm when a workflow references a deleted credential. Cleared on next successful `create` of same key.

**Per-replica vs shared.** L1 is per-replica (not shared). Cross-replica coherence via invalidation events, not shared cache. Eventual consistency with bounded staleness (≤ 1 s typical event propagation).

### §7.3 Retry taxonomy

Per `nebula-error::Classify` + register `user-op-retry`.

| Error class | Retry? | Backoff | Budget |
|---|---|---|---|
| `Transient(Network)` — connection error, DNS, 5xx | Yes | Exponential w/ jitter (100ms × 2^n, max 60s, n≤5) | Per-credential: 10/hour |
| `Transient(Timeout)` — request timed out | Yes | Same as above | Same |
| `Transient(RateLimited)` — 429 from IdP | Yes | Respect `Retry-After` header; else 60s fixed | Count against budget |
| `Permanent(AuthExpired)` — 401 from IdP | Reactive refresh + retry once; if still 401 → `Suspended` | No backoff | Trigger §7.1 reactive refresh |
| `Permanent(Forbidden)` — 403 from IdP | No | — | — |
| `Permanent(NotFound)` — 404 / credential deleted | No | — | — |
| `Capability(WrongScheme)` — resolve-site type mismatch | No — programming error | — | — |
| `Capability(NotSupported)` — operation not supported (e.g., revoke on non-revocable cred) | No | — | — |
| `Context(*)` — caller-supplied bad params | No | — | — |

Retry budget enforced per-credential to prevent feedback loops. Budget exhausted → `RateLimitExceededLocal`; operator notified via eventbus.

### §7.4 Circuit breaker

Per-credential / per-provider / per-endpoint. Implemented via `nebula-resilience` (existing crate).

Trip conditions:
- **Per-endpoint:** 5 consecutive `Transient(Network)` or `Transient(Timeout)` in 60 s window → open.
- **Per-provider:** 10 endpoints tripped within same provider → open at provider level (blocks all resolve attempts for credentials from that provider).
- **Per-credential:** 3 consecutive `Permanent` errors → open; only manual operator reset.

Half-open: after 30 s, allow 1 probe request. Success → closed; failure → extend open by 60 s.

### §7.5 Concurrency — thundering herd prevention + IdP rate limit

**Single-flight refresh.** `RefreshCoordinator` (in-proc L1 per §7.1 step 1): concurrent refresh attempts for the same credential coalesce — only one calls `Credential::refresh`; others await the result or receive `CoalescedWithOther`.

**IdP rate limit.** Per-provider token-bucket rate limiter in `nebula-resilience`. Provider-specific caps in `ProviderSpec.rate_limit`. Exceeded → `Transient(RateLimited)`.

### §7.6 Distributed coordination

**OUT** — two sub-specs:

- Multi-replica refresh coordination (L2 `RefreshClaimRepo`) — [`draft-f17`](2026-04-24-credential-refresh-coordination.md), in flight.
- Rotation leader election (`RotationLeaderClaimRepo`) — Strategy §6.5 queue #2, pending.
- Cache invalidation broadcast — sub-spec of `draft-f17` or separate mechanism-spec (eventbus channel detail).

Tech Spec consumers use the §5.5 consumer interfaces. Producer-side (claim protocol, heartbeat cadence, reclaim on crash, mid-refresh race mitigation) is in the sub-specs.

### §7.7 Failure modes matrix

Behavior per component-down scenario. Bounded degradation per Strategy §1.2 non-goal (fail-closed where safety requires, fail-open for read-only where possible).

| Component down | Reads (resolve) | Writes (create/update/revoke) | Refresh | Notes |
|---|---|---|---|---|
| IdP unreachable | OK — stale cache returned | OK (new creds fail at resolve) | Circuit-break; retry per §7.3 | No data loss; eventual recovery |
| Network partition (engine ↔ storage) | Fail-closed | Fail-closed | Fail-closed | All ops fail until partition heals |
| Storage DB down | Fail-closed | Fail-closed | Fail-closed | Fatal for service — no cache-only mode |
| Audit DB down | Fall-through to read-only mode (§6.5) | Fail-closed (audit-fail-closed) | Blocked | Fallback file sink drains on recovery |
| Cache down (L1 corrupted) | Fall-through to storage | OK | OK | Slower hot path but functional |
| KMS unreachable (cloud) | New decrypts fail; cached decrypts OK until TTL | Fail-closed (can't encrypt new) | Fail-closed | Restart after KMS recovery |
| RefreshClaimRepo down | OK (resolve reads cached/stale state) | OK | Falls back to L1-only coordination; cross-replica refresh can race. For providers that rotate refresh_tokens (Google, GitHub app installation tokens, Microsoft identity platform, etc.), racing can invalidate in-flight tokens — elevated 401 rate during refresh windows for losing-side requests. | Restore `RefreshClaimRepo` before critical workflows; expect intermittent 401s during refresh windows otherwise. See [`draft-f17`](2026-04-24-credential-refresh-coordination.md) for mid-refresh race mitigation. |
| Eventbus down | OK (cache invalidation delayed, stale for TTL) | OK | OK | Eventual consistency delay up to cache TTL |

### §7.8 Health check

Per-credential periodic `Credential::test()` via engine background task.

**Cadence.** Default 1 hour per `CredentialMetadata::test_cadence`. Override per-credential-type (e.g., DB connection credentials every 5 min; API keys every 24 h).

**Scheduler.** Engine runs `test_scheduler` task: iterates credentials where `TESTABLE = true`, elides if tested within cadence window, otherwise calls `Credential::test(&state)`. Result emitted as `CredentialEvent::HealthChanged { cred_id, outcome }`.

**No side effects.** `test()` invariant (Strategy §2.1 / Tech Spec §2.1): "must not have side effects beyond read operations (no token mint, no resource creation)".

**Result handling:** `Ok(TestOutcome::Healthy)` → no action. `Ok(TestOutcome::Degraded)` → log warning, emit event. `Err(TestError)` → emit `UnhealthyDetected`, operator alert via integration channel.

### §7.9 Observability

Three channels: metrics, traces, structured logs. All bounded-cardinality.

**Metrics (Prometheus-style, via `nebula-metrics`):**

```
# Counter — cardinality = kinds × outcomes ≈ 400 credential types × 3 outcomes = 1200 series
credential_resolve_total{kind, outcome}
  # outcome ∈ {success, error, not_found, cache_hit, cache_miss}

# Histogram — cardinality = kinds × buckets
credential_resolve_duration_seconds{kind}
  # buckets: [1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1]

# Counter
credential_refresh_total{kind, outcome}
  # outcome ∈ {refreshed, coalesced, failed_transient, failed_permanent, reauth_required}

# Gauge — cardinality = kinds
credential_cache_hit_ratio{kind}

# Gauge
credential_pending_count   # current in-progress interactive flows
credential_expiring_soon   # credentials expiring in next N hours

# Counter
audit_write_total{result}      # result ∈ {success, failure, fallback_sink}
audit_chain_broken_total       # HMAC chain break detections
```

**No per-credential-ID metric labels** — that would blow cardinality. Only credential kind (type-level) and outcome.

**Traces (OpenTelemetry-style, via `nebula-log`):**

Span per operation. Attributes:

- `credential.kind` — type key
- `credential.id_hash` — SHA-256 of credential_id (first 16 chars), NOT raw ID if sensitive
- `credential.operation` — resolve / refresh / revoke / test / create / update / delete
- `credential.scope.org` — org_id (UUID is OK to span-attribute)
- `credential.scope.workspace` — workspace_id
- `credential.result` — outcome
- `credential.cache.hit` — bool

No secret material or decrypted state in span attributes. Ever.

**Structured logs (via `tracing`):**

| Level | Contents |
|---|---|
| `error` | Fatal failures, audit chain breaks, unhandled exceptions |
| `warn` | Circuit-break trips, reauth required, fallback sink activated |
| `info` | Normal operations (resolve, refresh, revoke), with non-secret metadata |
| `debug` | Internal flow states (state_kind transitions, cache hit/miss) |
| `trace` | Off by default; enable for deep debugging only; strictly no secret material |

Structured fields: `credential_id` (ULID string), `operation`, `result`, `elapsed_ms`, `replica_id`, `trace_id`. `SecretString`-typed fields auto-redacted by `tracing` subscriber.

**Events (eventbus fan-out, via `nebula-eventbus`):**

```rust
pub enum CredentialEvent {
    Created { cred_id, kind, scope },
    Refreshed { cred_id, prev_expiry, new_expiry },
    Revoked { cred_id, cause, cascade_from: Option<CredentialId> },
    Expired { cred_id },
    ExpiringSoon { cred_id, expires_at },
    HealthChanged { cred_id, outcome },
    AuditChainBroken { cred_id, seq },
}

pub struct CacheInvalidation {
    pub cred_id: CredentialId,
    pub reason: InvalidationReason,
}
```

Consumers: WebSocket push per `draft-f34` (sub-spec queue #6), metrics, integration webhooks.

## §8 Testing

Ten test categories per register `user-test-*` cluster (10 rows). Each category has concrete deliverables, tools, and coverage gates.

### §8.1 Unit tests

**Scope.** Pure primitives: PKCE code_verifier/challenge derivation, HMAC hash-chain computation, URL template substitution, envelope JSON serialization, CredentialKey hashing, `SecretString` redaction behavior.

**Tools.** Standard `#[test]` + `#[cfg(test)]` modules inside `nebula-credential`. No async runtime required for most. `assert_eq!` / `assert!` assertions.

**Target coverage.** ≥ 85 % line coverage on `nebula-credential` primitives (measured via `cargo llvm-cov`). Hot-path code (resolve, project, refresh dispatch) ≥ 95 %.

**CI gate.** `cargo nextest run -p nebula-credential --profile ci --no-tests=pass` per `test-matrix.yml`. Fails PR on any unit test failure.

### §8.2 Integration tests

**Test crate layout:**

```
nebula-credential/
├── tests/
│   ├── credential_lifecycle.rs      # create → refresh → revoke → purge
│   ├── pending_flow.rs              # OAuth2 interactive with wiremock IdP
│   ├── pattern2_dispatch.rs         # Pattern 2 action dispatch (phantom + where-clause)
│   ├── dualauth_resource.rs         # mTLS + Bearer combined
│   ├── compile_fail_*.rs            # trybuild compile-fail probes
│   └── common/
│       └── mod.rs                   # shared fixtures (test credentials, fake registry)
```

**Tools.**

- `wiremock` — mock OAuth2 token endpoint, revoke endpoint, IdP test endpoint.
- In-memory `CredentialStore` impl (production fixture, already exists).
- `tokio::test` runtime.
- `trybuild` — compile-fail probes for phantom-shim and `#[action]` macro diagnostics.

**Coverage target.** All lifecycle transitions (§4 state machine diagrams) exercised. All 10 OUT markers have integration-level stub tests verifying the pointer works (e.g., `draft-f17` integration: mock `RefreshClaimRepo` and verify L1 L2 two-tier coalesce).

### §8.3 Contract tests (real providers)

**Scope.** Real-IdP round-trips against sandbox accounts to catch provider-side API changes that mocks miss.

**Providers (initial set):**

- Google OAuth2 — test account with test app + workspace.
- GitHub — test org with test OAuth app.
- Slack — sandbox workspace + app.
- AWS SigV4 — test IAM user with sandbox permissions.
- Azure AD — test tenant.

**Mechanism.**

- `#[ignore]` by default — contract tests don't run on every PR.
- Nightly CI job via GitHub Actions; secrets injected via `GITHUB_TOKEN` + provider-specific `CONTRACT_TEST_*` env vars.
- Failures reported to security-lead + dev team via dedicated Slack channel.
- Test runs periodically even outside PR flow (catch provider-side breaking changes, e.g., Google changing OAuth2 scopes format).

### §8.4 Security tests

**Fuzz (cargo-fuzz):**

- `fuzz_state_param` — OAuth2 state parameter parser.
- `fuzz_callback_params` — OAuth2 callback URL query parser.
- `fuzz_pending_serde` — serialized Pending state deserializer (defends against malformed stored rows).
- `fuzz_envelope_serde` — envelope JSONB deserializer.
- `fuzz_url_template` — URL template variable substitution.

Run nightly; corpus committed to `nebula-credential/fuzz/corpus/`.

**Property tests (proptest):**

- Crypto: `prop_encrypt_decrypt_roundtrip(data, key, nonce, aad) → data`.
- HMAC chain: `prop_chain_verify(entries) ⇒ all self_hmacs match`.
- Zeroize: `prop_drop_zeros_memory(secret) ⇒ buffer is zero after drop` (via miri).
- URL template: `prop_bind_then_resolve(template, vars) ⇒ concrete URL validates`.

**Miri** (unsafe + zeroize paths):

- `cargo +nightly miri test -p nebula-credential --lib zeroize`
- Target: `Zeroizing<Vec<u8>>` drop, `SecretString` drop, `CredentialGuard<S>` drop, any `unsafe` blocks in crypto primitives.

### §8.5 Concurrency tests (loom)

**Scope.** `RefreshCoordinator` L1 (in-proc) concurrent-refresh coalescing.

**Model.** Loom simulates 2–3 threads concurrently calling `refresh(cred_id)` → verify only one `Credential::refresh` call reaches the trait method; others receive `CoalescedWithOther`. Covers Mutex semantics + state transition ordering.

**Loom test** example:

```rust
#[test]
fn refresh_coalesces_under_concurrent_calls() {
    loom::model(|| {
        let coord = Arc::new(RefreshCoordinator::new());
        let call_count = Arc::new(AtomicUsize::new(0));

        let t1 = {
            let coord = coord.clone();
            let call_count = call_count.clone();
            thread::spawn(move || {
                if coord.try_claim(&cred_id).granted() {
                    call_count.fetch_add(1, Ordering::SeqCst);
                    // simulate refresh work
                    coord.release(&cred_id);
                }
            })
        };
        let t2 = { /* same pattern */ };

        t1.join().unwrap();
        t2.join().unwrap();

        // Exactly one of (t1, t2) must have claimed.
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    });
}
```

**L2 cross-replica concurrency** tests via real DB, not loom — in `draft-f17` sub-spec.

### §8.6 Failure injection — chaos

**Tool.** Custom `fault_injector` crate or existing `nebula-testing-chaos` pattern.

**Scenarios (per failure modes matrix §7.7):**

- Storage DB transaction fails mid-write (after audit-insert, before credentials-update) → verify rollback + audit shows `result = 'failure'`.
- IdP request times out during refresh → verify circuit break + retry per §7.3.
- Network partition between engine and storage → verify fail-closed to writes, reads from cache.
- Audit DB unreachable → verify degraded read-only mode, fallback file sink, drain on recovery.
- Mid-refresh crash (SIGKILL) → verify L2 claim TTL expires + reclaim succeeds (via `draft-f17` harness).

**Gate.** Chaos suite runs nightly. All scenarios must pass; any failure blocks release.

### §8.7 Upgrade tests

**Scope.** Migration correctness + no data loss + rollback safety (forward-migration-safe, no literal rollback).

**Scenarios:**

- `credentials` v1 → v2 state shape migration (when `draft-f36` sub-spec lands). Test: seed v1 rows, run migration, verify all rows readable as v2 + content semantically equivalent.
- Encryption version bump (key rotation walker per §6.2). Test: rows encrypted with kek_1, run walker, verify all rows rewrapped with kek_2 + kek_1 retirable without data loss.
- Dialect parity: run same test suite against both Postgres and SQLite backends, assert identical outcomes.

**Rollback "safety".** Not literal rollback; instead, tests verify:

- Forward migration doesn't corrupt old-version rows during transition window.
- If migration aborts mid-way, partially-migrated rows remain readable (engine handles mixed-version dataset).

### §8.8 Performance tests (CodSpeed)

**Baselines (from spike iter-2 at commit `1c107144`):**

| Bench | Baseline |
|---|---|
| `bench_resolve_hot` (H1 cached) | 6.44 ns mean |
| `bench_resolve_baseline` (synthetic HashMap+downcast) | 5.54 ns mean |
| `bench_decrypt_hot` (envelope unwrap + decrypt + deserialize) | TBD — land at implementation time; expected ~500 ns |
| `bench_refresh_inproc` (L1 coordinator path, no IdP call) | TBD — expected ~10 µs |
| `bench_audit_write` (HMAC compute + DB insert) | TBD — expected ~1 ms |

**Regression gates.** CodSpeed flags any bench >20% slower than committed baseline. CI alerts PR author + dev team.

**Hot path absolute ceiling.** Resolve p95 ≤ 1 µs per Strategy §3.4. Current baseline has ~150× headroom.

### §8.9 Determinism tests

**`Clock` trait** for time-dependent behavior:

```rust
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> SystemTime;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> SystemTime { SystemTime::now() }
}

pub struct FakeClock { current: Arc<Mutex<SystemTime>> }
impl FakeClock {
    pub fn advance(&self, d: Duration) { /* ... */ }
}
impl Clock for FakeClock { /* returns current */ }
```

**Deterministic RNG** for PKCE verifier + state parameter generation:

```rust
pub trait RandSource: Send + Sync + 'static {
    fn fill_bytes(&self, dest: &mut [u8]);
}

pub struct OsRandSource;          // production
pub struct FixedSeedRandSource;   // tests — deterministic sequences
```

Tests compose `FakeClock` + `FixedSeedRandSource` → deterministic PKCE + state + TTL outcomes. No flaky time-based or random-based tests.

### §8.10 Test fixtures

**Generated test credentials.**

```rust
// macro shorthand
let cred = fixture_credential!(
    kind: "oauth2.google",
    client_id: "test-client",
    scopes: ["email", "profile"],
);

// Emits a typed Credential struct seeded with fake OAuth2 state
// (fake access_token, fake refresh_token, fake expires_at).
```

Fixture material has a fixed deterministic pattern (e.g., `"test-" + kind + "-" + seed`) so test logs + traces are self-identifying.

**No real secrets in CI.** Enforced by:

- Pre-commit hook: `secrets-scanner` runs on staged files; rejects commits containing patterns matching real credential formats (AWS key IDs, GitHub tokens, Google OAuth client secrets).
- CI gate: secondary `detect-secrets` scan on every push.
- Contract test credentials (§8.3) injected via GitHub Actions secrets — never committed.

## §9 Discovery / UX

Maps 5 `user-disc-*` register rows. Interface surface layer — how credentials are registered, described, validated, discovered, and bound to actions.

### §9.1 Registration

Explicit `register::<C>()` at plugin / service init per Strategy §2.1. No `inventory`-style auto-registration (rejected per Strategy §2.1 — cross-crate unreliable).

**Pattern:**

```rust
// In plugin crate's init module:
pub fn register_credentials(registry: &mut CredentialRegistry) {
    registry.register(MyServiceOAuth2Credential::new());
    registry.register(MyServiceApiKeyCredential::new());
    // ... one line per credential type the plugin exposes
}

// In service startup (binary crate):
fn main() {
    let mut registry = CredentialRegistry::new();

    // Built-in credentials from nebula-credential-builtin.
    nebula_credential_builtin::register_builtin(&mut registry);

    // Plugin credentials.
    my_plugin::register_credentials(&mut registry);

    // ... engine startup with frozen registry ...
    let engine = Engine::start(registry);
}
```

**Invariants** (from §2.11 + §3.1):

- Registration happens during init only. No mutation after `Engine::start(registry)`.
- Duplicate `KEY` panics in debug (`debug_assert!`); warn + overwrite in release with `tracing::warn!`.
- Each crate registers its own credentials. Cross-crate registration (plugin registers something from another plugin) not supported.
- Hot-reload of credential types is explicitly OUT (§2.11). Restart the service to pick up new plugins.

Cross-ref: §2.11 describes the `#[plugin_credential]` macro emission protocol; §3.1 describes the append-only registry and its lock-free read path.

**Plugin authors must provide both** `Credential` + `CredentialMetadataSource` (§2.8). The `#[plugin_credential]` macro emits both from one annotated declaration.

### §9.2 Metadata — two-layer override

> **Superseded by §15.8 (CP5 2026-04-24).** `CredentialMetadata.capabilities_enabled: Capabilities` field below is REMOVED in CP5. Plugin-authored metadata does not carry capability claims — capabilities are computed at `CredentialRegistry::register<C>` time from `C`'s sub-trait membership (post §15.4 split), stored in `RegistryEntry::capabilities`. Plugin cannot self-attest false capabilities. §15.8 is the canonical shape.

Per register row `draft-f33`. `CredentialMetadata` supports static defaults + per-tenant override applied at resolution time.

```rust
pub struct CredentialMetadata {
    pub display_name: String,
    pub icon: Option<IconRef>,                 // URL or data URI
    pub help_text: Option<String>,
    pub documentation_links: Vec<DocLink>,
    pub test_cadence: Duration,                // default 1h
    pub cache_ttl: Duration,                   // default 5min
    pub pending_ttl: Duration,                 // default 10min
    pub revocation_grace: Duration,            // default 30s
    pub rotation_policy: RotationPolicy,
    pub field_hints: Vec<FieldHint>,           // per-input-field UX hints
    pub capabilities_enabled: Capabilities,    // which of INTERACTIVE/REFRESHABLE/... are live
    pub service_key: Option<ServiceKey>,       // for ServiceCapability slot matching (§9.4); None for service-agnostic credentials
    pub provider_id: Option<&'static str>,     // §15.1 decision — registry-backed OAuth provider; None for non-OAuth / self-issued credentials
}

impl CredentialMetadata {
    /// Static defaults from the credential type's impl.
    pub fn defaults<C: CredentialMetadataSource>() -> &'static Self {
        <C as CredentialMetadataSource>::metadata()
    }

    /// Apply override from registry or per-tenant config.
    pub fn with_override(
        defaults: &Self,
        overrides: &MetadataOverrides,
    ) -> Cow<'_, Self> {
        if overrides.is_empty() {
            Cow::Borrowed(defaults)
        } else {
            Cow::Owned(defaults.merge(overrides))
        }
    }
}

pub struct MetadataOverrides {
    pub display_name: Option<String>,
    pub icon: Option<IconRef>,
    pub help_text: Option<String>,
    // ... per-field optional override ...
}
```

**Override sources:**

- `provider_registry.spec` JSONB (§5.3) — operator customizes `display_name`, `icon`, `help_text` per provider. Scope: global registry.
- Per-tenant config — future extension (`tenant_metadata_overrides` sub-spec). Scope: per `org_id`.

### §9.3 Validation — three tiers

**Schema validation.** `<Self::Input as HasSchema>::schema()` validates input shape before any IdP call. Field types, required/optional, regex patterns, enum constraints. Runs at UI client (form validator) + server-side (API `/credentials` handler re-validates — don't trust client).

**Semantic validation.** Optional `Credential::test(&state)` post-resolution. Verifies the resulting state actually works against the provider (e.g., `access_token` returns 200 from a whoami endpoint). Gated by `TESTABLE` bool (§2.1); credentials without testable probe default to skip.

**UX validation.** Form hints from `CredentialMetadata::field_hints`:

- `FieldHint::Format("ghp_*")` — GitHub personal token prefix.
- `FieldHint::MinLength(32)` — minimum API key length.
- `FieldHint::RegexMatch("^sk-ant-.+$")` — Anthropic API key prefix.
- `FieldHint::ExampleValue("ghp_1234...")` — sample for "paste your token here" UX.

Rendered client-side during form entry; pre-submission validation. Server re-validates via schema (schema + UX hints are consistent by construction — schema is stricter).

### §9.4 Discovery — action → credential matching

> **Superseded by §15.8 (CP5 2026-04-24).** `iter_compatible` filter body below consults `cred.metadata().capabilities_enabled.contains(...)` — plugin-declared field. CP5 canonical form: filter consults `RegistryEntry::capabilities` (registry-computed at `register<C>` time from sub-trait membership). Same `SlotType::Concrete / ServiceCapability / CapabilityOnly` matching axes, but capability authority shifts from plugin metadata to type system.

Action declares capability requirements via field types (`CredentialRef<dyn XPhantom>`). Engine matches available credentials at runtime for user-facing picker.

**Matching pipeline:**

1. **Compile-time slot metadata.** `#[action]` macro emits `SlotBinding` per credential field (§3.4 step 2), with `slot_type` as one of three `SlotType` variants — `Concrete { type_id }` (Pattern 1), `ServiceCapability { capability, service }` (Pattern 2), or `CapabilityOnly { capability }` (Pattern 3).
2. **Runtime filter.** When user opens the action's credential picker, `CredentialRegistry::iter_compatible(slot_binding)` returns the subset of registered credentials satisfying the slot's bound, dispatched per variant:

```rust
impl CredentialRegistry {
    pub fn iter_compatible<'a>(
        &'a self,
        binding: &SlotBinding,
    ) -> impl Iterator<Item = &'a dyn AnyCredential> + 'a {
        self.entries
            .values()
            .map(|boxed| &**boxed)
            .filter(move |cred| match &binding.slot_type {
                SlotType::Concrete { type_id } =>
                    cred.type_id_marker() == *type_id,
                SlotType::ServiceCapability { capability, service } =>
                    cred.metadata().capabilities_enabled.contains(*capability)
                        && cred.metadata().service_key == Some(*service),
                SlotType::CapabilityOnly { capability } =>
                    cred.metadata().capabilities_enabled.contains(*capability),
            })
    }
}
```

3. **UI picker** renders filtered list; user selects.
4. **Binding persistence.** Selected credential's `CredentialKey` stored in workflow definition (action instance config). Workflow re-binding on credential soft-delete surfaces in validation pass.

### §9.5 Binding — compile-time enforcement

Compile-time binding per Strategy §2.3 Resource-per-capability + §2.5 blanket sub-trait pattern + Tech Spec §3.4 Pattern 2 dispatch.

**Three compile-time gates** (cumulative):

1. **Declaration-site phantom check** (§3.4 step 1). `CredentialRef<dyn BitbucketBearerPhantom>` rejects wrong-capability credential types at action struct construction.
2. **Resolve-site where-clause** (§3.4 step 3). `fn resolve_as_bearer<C>(...) where C: Credential<Scheme = BearerScheme>` rejects wrong-scheme concrete types at engine dispatch instantiation.
3. **Macro cross-check** (§3.5). `#[action]` macro verifies that Resource's `AcceptedAuth` bound matches Action's `CredentialRef<...>` bound via trait-resolution where-clause.

**Runtime guard.** UI picker only shows compatible credentials per §9.4. User cannot wire a non-matching credential even via direct API call — the API handler re-checks §9.5 binding at request deserialization; mismatch returns `400 Bad Request` with capability-mismatch reason.

## §10 OAuth & redirect flows

Maps 6 `user-flow-*` register rows. Covers OAuth2 interactive flow mechanics specific to the credential subsystem.

### §10.1 Redirect URI policy

Three policies selectable per deployment mode:

**Fixed per-instance** (cloud default). Anthropic-managed redirect URI registered with each supported provider (e.g., `https://app.nebula.dev/oauth2/callback`). All tenants share. Tenant-specific routing happens post-callback via the encrypted pending state's `org_id` field.

**Wildcarded** (self-hosted operator choice). Operator registers multiple URIs with the provider (staging, prod, dev); Nebula uses the URI matching its `NEBULA_BASE_URL` at startup.

**Per-tenant** (enterprise, self-hosted optional). Each tenant registers its own redirect URI with the provider; Nebula reads tenant's URI from `provider_registry.spec.tenant_overrides` or tenant config.

Security: redirect URI is bound to the `ProviderSpec` (§5.3), not user `Credential::Input`. User cannot inject arbitrary redirect URI — §6.8 SSRF mitigation.

### §10.2 State management

`PendingStore` backed by `pending_credentials` table (§5.2). State TTL + single-use + GC sweep per §6.9.

**Flow (happy path):**

1. `Credential::resolve(input)` returns `ResolveResult::Pending(p)` containing PKCE verifier + CSRF state + any multi-step accumulator.
2. Engine encrypts `p` via EncryptionLayer (§5.1) and INSERTs `pending_credentials` row with `state = pending_id + encrypted_p`.
3. User redirected to IdP with `state = pending_id` query parameter.
4. IdP callback hits `nebula-api /oauth2/callback?state=...&code=...`.
5. API looks up `pending_credentials` WHERE `id = pending_id`; decrypts; reads `p`; deletes row (single-use).
6. API calls `Credential::continue_resolve(p, Continuation { code })`.
7. Returns `ResolveResult::Ready(state)`; engine encrypts `state`; INSERTs `credentials` row.

**Replay prevention:** step 5's DELETE is part of the same transaction as step 6's subsequent operations. A replay hits "no row found" → `PendingError::NotFound`.

**TTL enforcement:** GC sweep (cadence 60 s) deletes expired rows per §6.9. Expired state replayed gives the same NotFound response.

### §10.3 Multi-step flow — atomic only

Per Strategy §5 decision: atomic flows only. Compat sketch #2 from iter-2 validated that `Pending` enum shape covers bounded-N multi-step without extending trait.

**Atomic bounded-N pattern** (e.g., Salesforce JWT flow — sign JWT → exchange for token, 2 steps):

```rust
pub enum SalesforceJwtPending {
    AwaitingTokenExchange { jwt: SecretString },
    // ... future variants for additional steps ...
}

impl Credential for SalesforceJwtCredential {
    type Pending = SalesforceJwtPending;

    async fn resolve(ctx: &CredentialContext<'_>, input: &Self::Input)
        -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError>
    where Self: Sized
    {
        let jwt = sign_jwt(input).await?;
        Ok(ResolveResult::Pending(SalesforceJwtPending::AwaitingTokenExchange { jwt }))
    }

    async fn continue_resolve(
        ctx: &CredentialContext<'_>,
        pending: Self::Pending,
        continuation: &Continuation,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError>
    where Self: Sized
    {
        match pending {
            SalesforceJwtPending::AwaitingTokenExchange { jwt } => {
                let token = exchange_jwt_for_token(&jwt).await?;
                Ok(ResolveResult::Ready(SalesforceJwtState { token }))
            }
        }
    }
}
```

**OUT — unbounded dynamic-N flows.** Sub-spec `draft-f22` (Strategy §6.5 queue #3, deprioritized). Requires `continue_resolve` signature extension to pass step-index; flagged as future trait-shape change if dynamic-N use case materializes.

### §10.4 Interactive vs non-interactive

**Browser-required (standard OAuth2 authorization code flow):**

- **Web app:** standard redirect flow with session cookie (§6.9).
- **Desktop app (Tauri):** custom URI scheme (§10.6) or local callback server (`http://127.0.0.1:PORT/callback`).
- **Headless (CI/CD, SSH sessions):** device code flow (RFC 8628) where provider supports; operator pre-provisioning otherwise.

**Device code flow** for headless:

1. Service requests `device_code` + `user_code` from provider's device-auth endpoint.
2. UI displays `user_code` + `verification_uri` to user (on another device with browser).
3. User enters `user_code` at the provider's URL.
4. Service polls token endpoint with `device_code` until authorized or expired.
5. Authorized → state persisted; expired → `PendingError::DeviceCodeExpired`.

Supported providers: Google, Microsoft, GitHub (for OAuth apps). Not all providers support device code — `CredentialMetadata::supports_device_code: bool` flag per type.

**Operator pre-provisioning** (fallback when no interactive path available):

- Operator uses browser-equipped device to obtain initial tokens via standard flow.
- Exports via `nebula credential export --id=cred_xxx --format=encrypted > cred.bin`.
- Imports on headless target: `nebula credential import --file=cred.bin`.
- Subsequent automatic refresh via `refresh_token` (no further interactivity needed).

### §10.5 Callback handling

API endpoint: `POST /oauth2/callback` (or provider-specific routes for non-standard flows).

**Paths:**

| Incoming | Handler action | Outcome |
|---|---|---|
| `?code=X&state=Y` (valid state, code exchange OK) | Complete resolve, persist credential, audit 'created' | Redirect UI `/credentials/:id?status=success` |
| `?error=access_denied` | Audit 'cancelled', delete pending row | Redirect UI `/credentials/new?error=user_denied` |
| `?error=*` (IdP error other than denial) | Log provider error, audit 'failure' with detail, delete pending | Redirect UI with error toast |
| No callback within pending TTL | GC sweep deletes stale pending | Silent (user abandoned) |
| `?code=X&state=Y` (valid state, code exchange FAILS — e.g., network error to token endpoint) | Retry per §7.3 Transient; if exhausted → audit failure, delete pending | Redirect UI error toast |
| Replay with same `state` | DB lookup fails (row deleted) | `400 PendingError::NotFound` to user |
| `state` invalid or missing | `400 Bad Request` without side effects | Reject; audit security event |

**Security verification steps** (per §6.9):

1. Verify `state` parameter matches DB row's `id` (CSRF binding).
2. Verify PKCE `code_challenge` derives from stored `code_verifier` — if provider returned back-channel, `verify: code_challenge == SHA-256(code_verifier)`.
3. Validate `code` exchange response signature if provider supports.

### §10.6 Deep link — native app (Tauri)

Desktop mode uses custom URI scheme `nebula://` for OAuth callbacks.

**Registration** (OS-level, at Nebula app installation):

- **macOS:** `Info.plist` `CFBundleURLTypes` entry.
- **Windows:** registry entry under `HKEY_CURRENT_USER\Software\Classes\nebula`.
- **Linux:** `.desktop` file with `MimeType=x-scheme-handler/nebula`.

**Flow:**

1. OAuth2 redirect_uri set to `nebula://oauth2/callback`.
2. IdP redirects user's browser to `nebula://oauth2/callback?code=X&state=Y`.
3. OS launches Nebula app (already running or cold start) with the URI.
4. App parses URI, extracts state + code, forwards to local engine via IPC.
5. Engine processes per §10.5 callback path.

**Security:**

- Custom scheme registration requires OS-level permission at install time; prevents other apps from intercepting after Nebula is installed.
- Pending state + CSRF + PKCE protection per §6.9 remain unchanged.
- Fallback: if custom scheme registration fails (permission denied, OS quirk), desktop mode falls back to local loopback callback (`http://127.0.0.1:PORT/callback`) with firewall prompt to user.

## §11 Multi-mode deployment

Maps 4 `user-mode-*` register rows. Three deployment modes with a feature matrix.

### §11.1 Desktop mode

**Target:** single-user local installation (developer machine, small team offline).

- **Storage:** SQLite (file-backed at `$NEBULA_DATA_DIR/db.sqlite`), single-replica.
- **Master key:** OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service via `secret-service` crate). KeyProvider reads on demand, never caches raw key.
- **Network:** no external service exposure required. IdP calls are the only outbound network.
- **Provider registry:** bundled static registry (compiled into `nebula-credential-builtin`); non-editable at runtime. Operator must release new Nebula version to add/update providers.
- **Claim repos:** `NoOpRefreshClaimRepo` + `NoOpRotationLeaderClaimRepo` (§5.4). Single replica, no coordination needed. L1 in-proc `RefreshCoordinator` handles concurrency.
- **OS keychain fallback:** headless Linux without keychain — env-based master key with `[WARN] Master key from NEBULA_MASTER_KEY env var, consider installing secret-service` startup log.

**Cross-refs:** §3.1 append-only registry (plugins pre-registered at desktop startup; no hot-reload) + §5.4 NoOpClaimRepo + §6.1 envelope encryption with OS keychain KEK.

### §11.2 Self-hosted mode

**Target:** organization's own infrastructure (on-prem, private cloud).

- **Storage:** Postgres (production-grade, supports replication across AZs/regions).
- **Master key:** env-based at startup (`NEBULA_MASTER_KEY`) OR Vault integration (`VAULT_ADDR` + token). Operator choice.
- **Network:** exposed to workflow runtime + plugin hosts + admin UI. TLS enforced on all endpoints.
- **Provider registry:** bundled defaults + admin CLI / admin web UI override (`nebula registry admin add-provider ...`).
- **Claim repos:** production `RefreshClaimRepo` + `RotationLeaderClaimRepo` (per sub-specs).
- **Operator responsibilities:**
  - Periodic master key rotation via walker CLI (§6.2).
  - Audit DB backup (retention per §6.5).
  - Certificate renewal for TLS.
  - Retention sweep query schedule (§4.4).

### §11.3 Cloud / SaaS mode

**Target:** Anthropic-operated multi-tenant SaaS.

- **Storage:** Managed Postgres (AWS RDS / GCP Cloud SQL / equivalent) with multi-AZ replication + read replicas for scale.
- **Master key:** KMS (AWS KMS / GCP Cloud KMS / Azure Key Vault). Master key never leaves KMS; envelope unwrap via API call per decrypt per §6.1.
- **Multi-tenant:** `org_id` on every credential row; `ScopeLayer` (§6.4) enforces isolation.
- **Provider registry:** Anthropic-curated; operator cannot customize. Updates via Nebula release cycle only. Tenant-scoped metadata overrides allowed per §9.2.
- **Claim repos:** production with multi-replica coordination.
- **Billing/metering:** per-tenant credential count + refresh count + audit volume tracked via `nebula-eventbus` consumer. Metered per plan tier.
- **Compliance:** SOC 2 Type 2 + ISO 27001 Annex A controls (product-policy rows).
- **Data residency:** per-region offerings (US, EU, APAC). Customer selects at signup; all data + audit stays in region.

### §11.4 Feature matrix

| Feature | Desktop | Self-hosted | Cloud |
|---|---|---|---|
| Encryption-at-rest | ✓ OS keychain KEK | ✓ env / Vault KEK | ✓ KMS KEK (never exported) |
| Key rotation walker | ✓ single-process | ✓ online CAS | ✓ online CAS + KMS |
| Multi-replica refresh coord | N/A (NoOpClaimRepo) | ✓ `RefreshClaimRepo` | ✓ `RefreshClaimRepo` |
| Rotation leader election | N/A | ✓ `RotationLeaderClaimRepo` | ✓ `RotationLeaderClaimRepo` |
| Multi-tenant isolation | N/A (single user) | Optional (via scopes) | ✓ mandatory per `org_id` |
| Billing/metering | — | — | ✓ per-tenant |
| OAuth2 browser redirect | Custom URI (Tauri) | Standard HTTPS | Standard HTTPS |
| Device code flow | ✓ where provider supports | ✓ | ✓ |
| Admin UI for registry | — (bundled only) | ✓ CLI + web | Anthropic-managed only |
| Compliance certification | — | Self-certify | ✓ Anthropic-certified (SOC 2 / ISO 27001) |
| GDPR data residency | User-local | Operator region choice | Per-region offering |
| Vault integration | — | ✓ optional | — (KMS used) |
| Audit log retention | 30 days default | Configurable | 1 year default |
| Hot plugin reload | No (§2.11) | No | No |

### §11.5 Mode-conditional compilation

Nebula ships a single binary per channel; mode selected at startup via config:

```bash
NEBULA_DEPLOYMENT_MODE=desktop    # or self-hosted or cloud
```

Feature flags gate mode-specific code paths at compile time:

```toml
# Cargo.toml
[features]
desktop = ["dep:keyring", "dep:rusqlite"]
self-hosted = ["dep:vault-sdk"]
cloud = ["dep:aws-kms-sdk", "dep:gcp-kms-sdk", "dep:azure-sdk"]
```

Release channels per mode:

- `nebula-desktop` release bundle → `--features = "desktop"`.
- `nebula-selfhosted` release → `--features = "self-hosted"`.
- `nebula-cloud` release → `--features = "cloud"`.

No runtime dispatch per mode. Mode-specific code not compiled into other channels (smaller binary size, tighter audit surface).

## §12 Integration

Maps 4 `user-int-*` register rows (2 in-scope + 2 OUT).

### §12.1 External secret store — `ExternalProvider`

Trait for pulling credential state from Vault / AWS Secrets Manager / GCP Secret Manager / Azure Key Vault:

```rust
pub trait ExternalProvider: Send + Sync + 'static {
    /// Resolve state from the external source. Returns typed Scheme
    /// after opt-in TryFrom<RawProviderOutput> per draft-f31.
    async fn resolve<S>(
        &self,
        reference: &ExternalReference,
        tenant_ctx: &TenantContext,
    ) -> Result<S, ProviderError>
    where
        S: AuthScheme + for<'a> TryFrom<&'a RawProviderOutput>;

    /// Endpoint allowlist per SSRF §6.8.
    fn endpoint_allowlist(&self) -> &EndpointAllowlist;
}

pub struct ExternalReference {
    pub provider_id: String,       // "vault.corp", "aws.secretsmanager", etc.
    pub secret_path: String,       // provider-specific (e.g., "secret/data/myapp/api_keys")
    pub version: Option<u32>,
}

pub struct RawProviderOutput {
    pub bytes: Vec<u8>,                        // raw provider response body
    pub metadata: HashMap<String, String>,     // response headers, provider-specific
}
```

Impls in `nebula-storage/src/external_providers/`:

- `VaultProvider` — HashiCorp Vault via HTTP API (kv v2 engine default).
- `AwsSecretsManagerProvider` — AWS SDK (`aws-sdk-secretsmanager`).
- `GcpSecretManagerProvider` — GCP SDK (`google-cloud-secretmanager`).
- `AzureKeyVaultProvider` — Azure SDK (`azure_security_keyvault`).

**Tenant scoping:** each provider prepends tenant namespace to `secret_path`. E.g., `VaultProvider` constructs actual path as `{tenant_namespace}/{secret_path}`. SSRF allowlist per provider prevents user-driven URL injection.

### §12.2 HSM / KMS envelope encryption

Cloud mode (§11.3) uses KMS-backed envelope per §6.1:

- Master key (KEK) lives in KMS; never exported.
- DEK per credential unwrapped via `kms_client.decrypt(encrypted_dek, kek_id)` per resolve.
- Signing operations (for HSM-signing credentials — Salesforce JWT, Azure AD federated app) via `kms_client.sign(key_id, plaintext, algorithm)`; raw key never returned.

```rust
pub struct KmsKeyProvider {
    kms_client: Box<dyn KmsClient>,  // AWS / GCP / Azure adapter
}

impl KeyProvider for KmsKeyProvider {
    async fn unwrap_dek(&self, envelope: &Envelope) -> Result<Dek, KeyError> {
        let resp = self.kms_client
            .decrypt(&envelope.encrypted_dek, &envelope.kek_id)
            .await?;
        Ok(Dek::from_bytes(resp.plaintext))
    }

    async fn sign(
        &self,
        key_id: &str,
        data: &[u8],
        algorithm: SigningAlgorithm,
    ) -> Result<Vec<u8>, KeyError> {
        let resp = self.kms_client.sign(key_id, data, algorithm).await?;
        Ok(resp.signature)
    }
}
```

Self-hosted mode can opt-in via Vault's `transit` engine — Vault provides sign/verify/decrypt without key export, equivalent to KMS.

### §12.3 OIDC / SSO federation — OUT

**OUT — Plane A per ADR-0033.** Credential subsystem (Plane B) does not federate identity. Users authenticate to Nebula via Plane A (OIDC / SSO / SAML); post-auth the user identity is available in credential operations as `principal_id` (for audit) and `user_id` (for scope).

Nebula does NOT act as an OIDC relying party for credential purposes. An OAuth2 credential to Google (used for calling Google Drive API from a Nebula workflow) is distinct from a user authenticating TO Nebula using Google SSO.

### §12.4 Plugin execution sandbox — OUT

**OUT — execution-model ADR** (separate product decision, per product-policy register row `user-int-plugin-sandbox`).

Plugin execution security (in-process / process-isolated / WASM) is orthogonal to credential subsystem. Credential subsystem assumes the execution model is decided elsewhere.

When the execution-model ADR lands, Tech Spec §2.11 may gain a reference describing plugin credential isolation guarantees per execution model. For now: credential subsystem treats plugin code as trusted at the execution boundary; sandboxing is not a credential-layer concern.

## §13 Evolution

Maps 5 `user-evo-*` register rows. Evolution policy for Tech Spec consumers.

### §13.1 Versioning — three axes

**Schema version.** Storage schema version stamped per row (`credentials.version` for CAS, `encryption_version` for key rotation). Migration scripts versioned `0NNN_*.sql` per §5.6. Breaking schema change → new migration + code-side compatibility shim for one release.

**Trait version.** `Credential` trait has no explicit version const. API compat tracked via crate semver (§13.3). Plugin authors read `nebula-credential` version in `Cargo.toml`; pin to a major version.

**Wire protocol version.** `nebula-api` REST surface versioned via URL prefix (`/api/v1/credentials`, `/api/v2/...`). Evolution policy in `nebula-api` spec (separate document). Credential Tech Spec references the wire protocol shape but does not govern its evolution.

### §13.2 Deprecation

**Credential type deprecation** (plugin removal, deprecated provider):

1. **Release N:** Mark type with `#[deprecated(note = "Use NewType; removed in N+2")]`. Type still usable; emits compile warning in consumer code.
2. **Release N+1:** `#[deprecated]` intensified to `#[deprecated(since = "N+1", note = "...")]` with `deny(deprecated)` in CI; allow only in test/fixture code. CHANGELOG highlight. UI shows deprecation badge on existing instances.
3. **Release N+2:** Type removed. Existing stored credentials must be migrated to replacement via operator CLI (`nebula credential migrate --from=old_type --to=new_type --id=cred_xxx`).

**Trait method deprecation** on `Credential` requires an ADR (material change — affects all implementors).

### §13.3 Breaking-change semver policy

Any of the following requires **major** version bump of `nebula-credential`:

- Add / remove / change a `Credential` trait method signature.
- Add / remove / change a `Credential` associated type.
- Change capability marker trait signatures (`AcceptsBearer`, etc.) or sealed convention.
- Change `AnyCredential` object-safe vtable (§13.4 stable ABI).
- Change `CredentialRef<C>` runtime representation.
- Change storage table schema incompatibly (drop column in use; rename column without compat shim).
- Change phantom-shim canonical form (ADR-0035 amendment may require major).

**Minor** version bump:

- Add new credential type to `nebula-credential-builtin`.
- Add new capability marker (opt-in by new schemes only).
- Add methods to `CredentialMetadataSource` (companion trait — not `Credential`).
- Add new migration script (forward-compatible).

**Patch** version bump:

- Bug fix in credential type impl.
- Performance improvement (no behavior change).
- Documentation fix.
- Non-public internal refactor.

### §13.4 Plugin API stability

**Explicit stable surface** — ABI-stable across minor versions of `nebula-credential`:

- `AnyCredential` trait (vtable shape).
- Capability marker traits (`AcceptsBearer`, `AcceptsBasic`, `AcceptsSigning`, `AcceptsTlsIdentity`) — empty, no methods, stable.
- Concrete scheme types (`BearerScheme`, `BasicScheme`, `SigV4Scheme`, `TlsIdentityScheme`) — structural layout stable; new fields require feature-flagged gradual rollout (§13.5).
- `CredentialRef<C>` runtime representation.
- `CredentialKey` (`Arc<str>` newtype) structure.
- `CredentialMetadataSource` trait signature.
- `SchemeInjector` trait signatures (when defined — TBD in §15 open item or future).

**Explicitly NOT stable** (internal, subject to change without major version bump):

- `Credential` trait itself — plugins use `#[plugin_credential]` macro which adapts to trait changes via generated glue code. Trait changes are major for the crate but transparent to plugins via macro regeneration.
- Layer stack composition (§5.1) — internal to `nebula-storage`.
- Refresh coordinator internals (`RefreshDispatcher` shape) — engine-internal.
- Sealed module names + internal helper traits.
- Metrics cardinality shape (new metrics can be added; names not removed without deprecation).

**ABI testing.** CI runs `cargo-public-api` (or equivalent) against the stable surface list; any change to a stable-surface signature fails CI unless accompanied by a major version bump commit message marker + CHANGELOG entry.

### §13.5 Feature flag rollout

New credential types and new capability markers roll out gradually via cargo features in `nebula-credential-builtin`:

**Three-phase cycle:**

- **Phase 1 — Preview (release N):** Type available behind `--features = "credential-xxx-preview"`. Not in default feature set. Users opt in explicitly. CHANGELOG flags as preview. API schema includes but marks `experimental: true`.
- **Phase 2 — Stabilization (release N+1):** Promoted if preview users confirm stability. Still behind feature flag but flag becomes default-off-but-documented. Bug fixes welcomed.
- **Phase 3 — Stable (release N+2):** Merged to default `nebula-credential-builtin` surface. Feature flag removed (or retained as no-op for consumer compat). Part of the stable API surface.

**Example:** Adding a new `TokenBindingScheme` for a new auth capability goes through 3 releases before being part of default builtin surface.

**Fast-track for critical fixes:** security-related additions (new capability flag to encode a security invariant) can skip phases with security-lead approval. Documented in CHANGELOG with security-advisory marker.

## §14 Meta

Maps 5 `user-meta-*` register rows. Largely pointers + closure since meta concerns are mostly OUT (sub-spec or product-policy).

### §14.1 Threat model

**OUT — Strategy §6.5 queue #9.** `docs/threat-model/credential.md` sub-spec, owned by security-lead, quarterly review cadence per register `user-meta-threat-model`. Tech Spec consumers reference the threat model for design-level threat enumeration; specific mitigations cross-referenced from individual §6 security sections.

### §14.2 Compliance — SOC 2 / ISO 27001 / HIPAA

**Product-policy** per register `user-meta-compliance`. Anthropic-managed for cloud mode (§11.3); operator self-certifies for self-hosted (§11.2); not applicable for desktop (single-user).

Compliance mapping doc (separate, product-owned) maps Tech Spec sections to specific compliance controls:

- §6.1 encryption-at-rest → SOC 2 CC6.1 / ISO 27001 A.10.1
- §6.5 audit hash-chain → SOC 2 CC4.1 / ISO 27001 A.12.4
- §6.7 zeroization → ISO 27001 A.10.1
- §11.3 multi-tenant isolation → SOC 2 CC6.1 / ISO 27001 A.9.4

Tech Spec does not contain the compliance mapping (out of scope); references the compliance doc.

### §14.3 Documentation plan

Per register `user-meta-documentation`. Implementation-phase work — ongoing as each piece lands.

- **ADR index refresh.** ADR-0031 (api-owns-oauth-flow) is a candidate for supersede if §10 OAuth flow consolidation moves HTTP ceremony to engine per §16.1 phase П7. If superseded, new ADR cites Tech Spec §10 as canonical source.
- **HLD (high-level design).** Credential subsystem section in product HLD references Strategy §2/§3 + Tech Spec §2/§3/§5 for engineering audience.
- **Runbooks.** Covered by §6.10 + §14.4 OUT pointers — separate sub-specs.
- **Per-piece doc updates.** Land with each implementation phase per §16.5 register maintenance rule.

### §14.4 Incident response

**OUT — Strategy §6.5 queue #10.** Three runbook sub-specs:

- Credential leak runbook
- Master key compromise runbook
- IdP outage runbook

Tech Spec describes detection mechanisms (failed-auth spike monitoring per §7.4 circuit breaker, anomaly detection events per §7.9, audit chain break detection per §6.5); specific response procedures live in runbooks. Owner: security-lead.

### §14.5 Change management

Per register `user-meta-change-management`. Process for credential subsystem changes:

- Any `Credential` trait change → ADR (per §13.3 semver + Strategy §3.6 trait-heaviness discipline).
- Any storage schema change → migration script + dialect parity CI gate (§5.4).
- Any cryptographic primitive change → security-lead review + ADR.
- Any deployment-mode behavior change → §11.4 feature matrix update + Tech Spec amendment.

CI enforcement:

- `cargo-public-api` for ABI stable surface (§13.4).
- Migration parity CI script (§5.4).
- Secret-scanner pre-commit + `detect-secrets` CI gate (§8.10).

Tech Spec evolution itself: per §0 freeze policy. Each checkpoint review. Supersede via new Tech Spec version (major bump) or ADR (for trait / canonical-form changes).

## §15 Open item decisions

Resolves two `open` register rows surfaced through Strategy + Tech Spec drafting. Rationale-driven decisions, not coin flips.

### §15.1 `critique-c9` — `PROVIDER_ID` for non-OAuth schemes

**Open question.** How does the credential subsystem encode the `provider_id` concept (registry-backed OAuth provider) for non-OAuth credential types where the concept does not apply (e.g., AppPassword self-issued, API keys without provider relationship, mTLS certificates)?

**Three candidates considered:**

**(a) `const PROVIDER_ID: Option<&'static str>` on the `Credential` trait.** Default `None`; OAuth credentials override.

```rust
pub trait Credential: ... {
    const PROVIDER_ID: Option<&'static str> = None;
    // ...
}
```

Pros: simple, single trait. Cons: encodes "missing concept" as `None` — readers cannot tell whether `None` is intentional ("this credential type has no registry-backed provider") or oversight ("author forgot to set it"). Pollutes the lean `Credential` trait surface (Strategy §3.6 trait-heaviness discipline) with a metadata concern.

**(b) Scheme-conditional trait extension.**

```rust
pub trait HasProvider: Credential {
    const PROVIDER_ID: &'static str;
}

impl HasProvider for SlackOAuth2 { const PROVIDER_ID: &'static str = "slack"; }
// Non-OAuth credentials simply don't impl HasProvider.
```

Pros: type-level distinction — credentials either have `PROVIDER_ID` or they don't. Cons: forces dispatch-at-site complexity (every site that uses `provider_id` must handle the absence case via specialization or runtime check); breaks `Credential` uniformity; heavy infrastructure for one concern that registries already track.

**(c) Move `provider_id` to `CredentialMetadata` as optional field.**

```rust
pub struct CredentialMetadata {
    // ... other metadata fields ...
    pub provider_id: Option<&'static str>,
    // None = no registry-backed provider concept (self-issued / non-OAuth).
    // Some(id) = look up id in provider_registry per §5.3 / §9.2.
}
```

Pros:
- `Credential` trait stays lean (Strategy §3.6 discipline preserved).
- `provider_id` IS metadata semantically — it describes "this credential has a registry-backed provider relationship". Non-OAuth credentials have no such relationship; absence of the field is the honest expression.
- Registry interaction (§5.3 `provider_registry` table + §5.5 `ProviderRegistryRepo` consumer) is metadata-driven anyway. Moving `provider_id` to `CredentialMetadata` aligns architecture with how registry is consumed.
- Per-tenant metadata overrides (§9.2 `with_override`) extend naturally to `provider_id` (e.g., enterprise tenant uses Microsoft AD-tenant-specific provider variant).
- Self-documenting at consumer sites: `metadata.provider_id.is_some()` reads as "credential has a registered provider" without ambiguity.

Cons: migration effort if existing `PROVIDER_ID` references existed (none in current production — this is greenfield post-redesign).

**Decision: (c).** `provider_id: Option<&'static str>` lives on `CredentialMetadata`, not on `Credential` trait.

**Implementation impact:**

- `CredentialMetadata` (§9.2) gains `pub provider_id: Option<&'static str>`. (Already added in §9.2 per this decision.)
- OAuth2 credential types' `CredentialMetadataSource::metadata()` impl sets `provider_id: Some("slack")` / `Some("google")` / `Some("microsoft")` / etc.
- Non-OAuth credentials (`ApiKeyCredential`, `BasicAuthCredential`, mTLS, signing-key creds) leave `provider_id: None` (struct field default).
- Engine resolve path: `if let Some(id) = metadata.provider_id { let spec = registry.get_provider(id)?; ... }` — natural `Option` chain. Skipped entirely when `None`.
- Audit + observability traces tag `credential.provider_id` only when `Some`; absent for non-OAuth credentials.

**Rationale summary.** `provider_id` is a metadata concern, not a behavior concern. Behavior trait (`Credential`) stays minimal; metadata trait (`CredentialMetadataSource`) carries descriptive fields. This is the same separation principle that motivated `CredentialMetadataSource` as a companion trait (§2.8).

Register row `critique-c9` flips from `open` → `decided` with pointer to this Tech Spec §15.1.

### §15.2 `arch-authscheme-clone-zeroize` — `AuthScheme: Clone` bound

**Open question.** The `AuthScheme: Clone` bound (Tech Spec §2.2) creates zeroization concerns for sensitive material — every `clone()` duplicates plaintext in heap, multiplying the attack surface for memory disclosure. mTLS certs, signing keys, etc. could leak via accidental clones. How should the `Clone` bound be reconciled with zeroization discipline?

**Three candidates considered:**

**(a) Relax `Clone` on the `AuthScheme` trait. Schemes opt in to `Clone` individually.**

```rust
pub trait AuthScheme: Send + Sync + 'static {}  // no Clone

#[derive(Clone)]
pub struct BearerScheme { /* ... */ }
impl AuthScheme for BearerScheme {}
// → BearerScheme is cloneable (token already in heap, refresh produces fresh).

pub struct TlsIdentityScheme { /* cert + key */ }   // no Clone derive
impl AuthScheme for TlsIdentityScheme {}
// → TlsIdentityScheme cannot be cloned at type level.
```

Pros:
- Rust idiom: don't add bounds you don't need at the trait level.
- Type-level enforcement: mTLS / signing schemes literally cannot be cloned — discipline becomes a compile-time guarantee enforced by the borrow checker.
- Zeroization story strengthens: long-lived sensitive material (cert + key, signing key, HSM-bound material) opts out of `Clone` → consumers must use borrowed access patterns → zeroize-on-drop fires deterministically at the Scheme's lifetime end.
- Schemes with safe-to-clone material (`BearerScheme` token, `BasicScheme` username/password — already heap-allocated post-decrypt) explicitly derive `Clone` per the implementor's judgement.

Cons:
- Existing code paths assuming `Scheme: Clone` need migration. (Greenfield post-redesign — minimal existing surface.)
- Some patterns become harder: capturing a Scheme in an async closure must use `Arc<S>` or borrow.

**(b) `CredentialGuard<S>` accessors instead of clones.**

```rust
pub struct CredentialGuard<S: AuthScheme> { scheme: S, /* RAII state */ }

impl<S> CredentialGuard<S> {
    pub fn with<R>(&self, f: impl FnOnce(&S) -> R) -> R { f(&self.scheme) }
}

// Consumer never holds a Scheme directly — always goes through .with(...).
```

Pros: preserves `Clone` bound semantically (callers can still "use" the scheme). RAII guard zeroizes deterministically.

Cons: every consumer site must wrap in `guard.with(|s| ...)`. Significantly more API surface. Doesn't actually prevent clones — just wraps them; mTLS / signing schemes still _could_ be cloned within the `with` closure if `S: Clone`. The zeroization concern isn't structurally addressed; only physically discouraged.

**(c) Hybrid `clone_scheme()` returning `impl ZeroizeOnDrop + Deref<Target = Self>`.**

```rust
pub trait AuthScheme: Send + Sync + 'static {
    fn clone_scheme(&self) -> impl ZeroizeOnDrop + Deref<Target = Self>;
}
```

Pros: keeps `Clone`-like semantics, RAII guarantee on result.

Cons: `impl Trait` in trait methods (RPITIT, stable since Rust 1.75) but with restrictions on `dyn`-compat — `dyn AuthScheme` would be problematic. Lifetime gymnastics for `Deref<Target = Self>` because the returned type cannot vary uniformly across implementors. Complexity vs benefit is unclear; the underlying zeroization concern still depends on the implementor's choice rather than being structural.

**Decision: (a).** Relax `Clone` on `AuthScheme` trait. Schemes opt in to `Clone` per type.

**Rationale summary.** This aligns with Rust idiom (don't force marker traits unless every implementor needs them) and converts zeroization from a discipline-and-review problem into a compile-time invariant. `BearerScheme` clones safely (post-decrypt token in heap, refresh produces fresh tokens routinely); `TlsIdentityScheme` does not (cert + key must not be duplicated). The compiler enforces the distinction. Reviewers no longer inspect every Scheme use site for accidental clones.

The trade-off — some patterns require `Arc<S>` or borrowed access for non-cloneable schemes — is a feature, not a bug. It forces conscious decisions about Scheme lifetime + sharing.

**Implementation impact:**

- `AuthScheme` trait (§2.2): drop `Clone` from supertrait bounds. New form: `pub trait AuthScheme: Send + Sync + 'static {}`.
- `BearerScheme`, `BasicScheme`, `SigV4Scheme`: keep `#[derive(Clone)]` — these are cloneable (heap material, refresh-driven rotation tolerates duplication).
- `TlsIdentityScheme`: drop `#[derive(Clone)]`. Cert + key never cloned; consumer uses `&TlsIdentityScheme` via borrowed access.
- Signing schemes (HMAC keys, RSA private keys for JWT signing): drop `#[derive(Clone)]`. Sign operations take `&self`.
- Tests use factory functions to produce fresh instances rather than cloning fixtures.
- `Resource::create<C>` signature accepts `&<C as Credential>::Scheme` rather than `<C as Credential>::Scheme` — borrowing, not consuming. Connection-bound resources (Postgres pool) extract any state needed at creation, not retaining the Scheme reference.

**Migration plan** (per phase П8 in §16.1):

1. Drop `Clone` from `AuthScheme` trait declaration in `nebula-credential`.
2. Compiler errors surface at every consumer site that previously assumed `Scheme: Clone`.
3. Each site refactored to either: borrow Scheme via `&S`, or wrap in `Arc<S>` for shared ownership, or factory-produce a fresh instance.
4. `BearerScheme` / `BasicScheme` / `SigV4Scheme` retain `#[derive(Clone)]` — no changes at consumer site for those.
5. `TlsIdentityScheme` / signing schemes: existing code that cloned now requires updates per step 3.

Register row `arch-authscheme-clone-zeroize` flips from `open` → `decided` with pointer to this Tech Spec §15.2.

### §15.3 Compile-time vs runtime gating model (introduced by 3-agent consensus session 2026-04-24)

The 3-agent consensus session (`docs/superpowers/specs/2026-04-24-credential-3agent-consensus-session.md`) closed adoption review with security-lead enumeration of N1-N10 structural findings (1 CRITICAL + 6 HIGH + 3 MEDIUM). Resolution model: **two-gate landing** — П1 closes 8 compile-time amendments; later phases close 4 runtime-only checks. **§15 amendments below capture the design closure**, not the implementation. Production code stays at current shape until П1 trigger fires per §1.4.

**Compile-time-gated amendments** (close at П1 landing):
- §15.4 — Capability sub-trait split (closes N1 / N3 / N5)
- §15.5 — `AuthScheme` sensitivity dichotomy (closes N2 / N10)
- §15.6 — Fatal duplicate-KEY registration (closes N7)
- §15.7 — `SchemeGuard` + `SchemeFactory` for refresh hook (closes N8 + tech-lead gap (i))
- §15.8 — Capability-from-type, not metadata (closes N6)
- §15.9 — `SecretString` accessors on composite-URL schemes (closes N4)

Each compile-time amendment has a corresponding **`tests/compile_fail_*.rs`** probe, mandatory per tech-lead condition 3. cargo-public-api snapshot alone catches surface drift but not semantic regression — compile-fail probes are required.

**Runtime-gated checks** (close at later phases — П2/П-later):
- §15.10 — `PendingStore::consume` atomicity contract (closes N9 — runtime, not trait shape)
- Tracing-filter for SecretString-bearing fields (defense-in-depth on §15.5; lives in `nebula-log` subscriber config, not credential crate)
- Signed-manifest for plugin registry (`arch-signing-infra` sub-spec queue #7, post-MVP — N7's structural defense beyond fatal duplicate-KEY)
- Metadata-vs-type cross-check at registration (defense-in-depth on §15.8; runtime panic if metadata.service_key disagrees with `<C::Scheme>::pattern()`)

**Disambiguation:** N1-N10 in §15.3-§15.10 refers to the **fresh enumeration from the 2026-04-24 consensus session**, derived from landed Tech Spec CP4 + production code. A separate "round-2" enumeration in `agent-memory-local/security-lead/project_credential_redesign_round2_2026-04-24.md` references the (deleted) exploratory drafts and uses different N-numbering. Cross-mapping in the consensus session document, §"Round 5 — N1-N10 enumeration disambiguation".

### §15.4 Capability sub-trait split — `Interactive` / `Refreshable` / `Revocable` / `Testable` / `Dynamic`

**Open question** (surfaced by security-lead N3+N5, formalized by 3-agent consensus session). The current trait shape (§2.1 + production `crates/credential/src/contract/credential.rs:130-160`) declares capabilities as `const X: bool = false` flags with **defaulted method bodies** for the corresponding methods (`continue_resolve` defaults to `Err(NotInteractive)`; `refresh` defaults to `Ok(NotSupported)`; `revoke` defaults to `Ok(())`; `test` defaults to `Ok(None)`; `release` defaults to `Ok(())`). A plugin author setting `const REFRESHABLE: bool = true` while forgetting to override `refresh()` produces a credential that **declares** refresh capability but **silently** returns `NotSupported` at runtime — engine treats as `Ok` outcome, no error classification, credential never refreshes, eventually expires in production with no alert. Same pattern for the other four capabilities.

**Three candidates considered:**

**(a) Replace bools with capability sub-traits.** `Interactive`, `Refreshable`, `Revocable`, `Testable`, `Dynamic` each extend `Credential` and require the corresponding method without default. Engine dispatchers bound by `where C: Refreshable`. `Pending` associated type moves under `Interactive`. Production `DYNAMIC`/`LEASE_TTL`/`release()` map to `Dynamic` sub-trait with no capability lost.

```rust
pub trait Credential: sealed::Sealed + Send + Sync + 'static {
    type Input: HasInputSchema + Send + Sync + 'static;
    type Scheme: AuthScheme;
    type State: CredentialState + Send + Sync + ZeroizeOnDrop + 'static;
    const KEY: &'static str;

    fn metadata() -> CredentialMetadata where Self: Sized;
    fn project(state: &Self::State) -> Self::Scheme where Self: Sized;
    async fn resolve(ctx: &CredentialContext<'_>, input: &Self::Input)
        -> Result<ResolveResult<Self::State, ()>, ResolveError>
    where Self: Sized;
}

pub trait Interactive: Credential {
    type Pending: PendingState + Send + Sync + ZeroizeOnDrop + 'static;
    async fn continue_resolve(
        pending: &Self::Pending,
        input: &UserInput,
        ctx: &CredentialContext<'_>,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError>;
}

pub trait Refreshable: Credential {
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;
    async fn refresh(state: &mut Self::State, ctx: &CredentialContext<'_>)
        -> Result<RefreshOutcome, RefreshError>;
}

pub trait Revocable: Credential {
    async fn revoke(state: &mut Self::State, ctx: &CredentialContext<'_>)
        -> Result<(), RevokeError>;
}

pub trait Testable: Credential {
    async fn test(scheme: &Self::Scheme, ctx: &CredentialContext<'_>)
        -> Result<TestResult, TestError>;
}

pub trait Dynamic: Credential {
    const LEASE_TTL: Option<Duration> = None;
    // Signature fixed CP6: `&self` receiver dropped (production trait
    // had vestigial `&self` at `credential.rs:274-283` — trait is
    // type-level, `Self` is ZST impl marker, `&self` gave no access).
    // Consistent with sister sub-trait method signatures
    // (Refreshable/Revocable/Testable all take `state: &[mut] Self::State, ctx`
    // without `&self`).
    async fn release(state: &Self::State, ctx: &CredentialContext<'_>)
        -> Result<(), ReleaseError>;
}
```

Engine refresh dispatcher becomes:

```rust
impl RefreshDispatcher {
    pub(crate) fn for_credential<C: Refreshable>() -> Self { /* ... */ }
}
```

A non-`Refreshable` credential cannot be passed — `E0277` at compile site.

Pros: silent-downgrade vector eliminated at type level (the failure class N3+N5 names is structurally impossible). Per-capability impl declaration is easier to read than "set bool + override method". Engine dispatchers become statically-typed, eliminating runtime branch checks. `Pending` moves to `Interactive`, removing the `NoPendingState` companion type for non-interactive credentials.

Cons: 5 extra trait names to maintain. Derive macros (`#[plugin_credential]`) need to detect which capabilities to emit per scheme. Plugin authors writing capability declarations explicitly is a one-time learning cost.

**(b) Keep bools, remove default bodies.** Plugin must override every method even if returning `Ok(NotSupported)`. Engine dispatchers stay runtime-gated on the bool.

Pros: minimal trait surface change. Cons: doesn't eliminate the silent-downgrade class — `REFRESHABLE = true` + `refresh() { Ok(NotSupported) }` is still legal. Just makes the silence explicit instead of defaulted. Doesn't fix engine static-dispatch.

**(c) Const-generic dispatch via associated type marker.** `<C as Credential>::Refreshable: TypeMarker` where TypeMarker is a sealed marker trait. Compile-time match.

Pros: keeps single-trait shape. Cons: requires unstable specialization or feature-gated nightly; not viable on stable Rust 1.95.

**Decision: (a).** Capability sub-trait split.

**Rationale summary.** Security-lead N3 + N5 identify the silent-downgrade class as 🟠 HIGH. Tech-lead's "2am test" (3am pager when token expires + no refresh) confirms the priority. Sub-trait split moves the failure mode from "documented contract that plugins must follow" to "compile error if violated" — the strongest possible mitigation. Cost (5 extra trait names) is below the threshold for over-engineering per consensus session decision: power-AND-safety user goal is satisfied because explicit per-capability declaration is **more** expressive (each capability is a typed concept), not less.

**Composition with ADR-0035 phantom-shim.** Sub-trait split is **orthogonal** to phantom-shim. Pattern 2 / Pattern 3 dyn-safety preserves: `dyn BitbucketBearerPhantom` continues to work because phantom-shim shields the unspecified-assoc-types problem. `dyn Refreshable` would inherit the same problem; if needed for runtime dispatch (e.g., engine dynamic refresh registry), a parallel phantom `dyn RefreshablePhantom` is introduced — ADR-0035 pattern repeated.

**Spike iter-3 outcome (2026-04-24, worktree `worktree-agent-afe8a4c6`, commit `f36f3739`).** Gate 3 validation confirmed sub-trait split composes cleanly with phantom-shim on 3 credential types (`ApiKeyCredential` static, `OAuth2Credential` Interactive+Refreshable+Revocable, `SalesforceJwtCredential` Interactive+Refreshable). Five findings:

1. **`dyn Credential` was never object-safe** (`const KEY` blocks `E0038`) — sub-trait split did not regress this. CP4 had the same block. Runtime dispatch against `Credential` already required `AnyCredential` type-erasure (Strategy §3.2) or phantom-shim; CP5/CP6 preserves this reality.
2. **Phantom-shim erases `C::Scheme` cleanly** with 3-assoc-type base, identical to CP4 4-assoc-type behavior. `CredentialRef<dyn BitbucketBearerPhantom>` + `CredentialRef<dyn AnyBearerPhantom>` (Pattern 3) both compile.
3. **Lifecycle sub-traits need parallel phantom-shims for dyn-dispatch.** `Refreshable: Credential` inherits `const KEY` + 3 assoc types → doubly-blocked (`E0038` + `E0191`). `RefreshablePhantom` chain (analogous to ADR-0035 Pattern 2) works cleanly. Same for all 5 lifecycle sub-traits (`InteractivePhantom` / `RevocablePhantom` / `TestablePhantom` / `DynamicPhantom`). **ADR-0035 amendment 2026-04-24-C** proposed: extend §2 Scope with Pattern 4 (lifecycle sub-trait erasure).
4. **Const-bool downgrade path is structurally absent** (spec-correct hard breaking change per `feedback_hard_breaking_changes.md`). `C::REFRESHABLE` access fails `E0599`; replacement is static bounds + phantom-shim.
5. **All 6 compile-fail probes fire with expected diagnostics** (4 mandatory per §16.1.1 + 2 bonus):
   - `compile_fail_state_zeroize.rs` → `E0277 BadState: ZeroizeOnDrop not satisfied`
   - `compile_fail_capability_subtrait_missing_method.rs` → `E0046 missing 'refresh'`
   - `compile_fail_engine_dispatch_capability.rs` → `E0277 ApiKeyCredential: Refreshable not satisfied`
   - `compile_fail_scheme_guard_retention.rs` → `E0597 ctx_stack does not live long enough`
   - `compile_fail_dyn_credential_const_key.rs` (bonus) → `E0038 Credential not dyn compatible (const KEY)`
   - `compile_fail_pattern2_service_reject.rs` (bonus) → `E0277 ApiKeyCredential: BitbucketBearerPhantom not satisfied`

**Secondary finding (§15.7 refinement needed at П1):** current `SchemeGuard<'a, C>` with only `PhantomData<&'a ()>` does NOT structurally prevent retention — `'a` is inferred `'static` in absence of an actual borrow to pin it. The barrier works only when engine passes `SchemeGuard<'a, C>` alongside `&'a ctx: &'a CredentialContext` with shared lifetime `'a`. Spike probe 4 tests the correct form. **§15.7 amendment** at П1: add explicit lifetime pinning requirement to `SchemeGuard` + `SchemeFactory::acquire` signatures; revised `on_credential_refresh` signature threads `'a` through ctx + guard. Non-blocking for П1; doc clarification + П1 scaffolding sub-task.

**Spike reproducibility:** `cd .claude/worktrees/agent-afe8a4c6/spike && cargo test --workspace` (15 tests green: 9 integration + 6 compile-fail). Spike crate is `workspace.exclude`'d from main build; `typos.toml` root excludes `spike/` for worktree hash IDs. Rust 1.95.0 pinned.

**Verdict:** Gate 3 §15.12.3 closure criteria satisfied. Ready for П1 trait-scaffolding. Sub-trait × phantom composition works. Lifecycle phantom-shims are additive, not a redesign requirement.

**Implementation impact (П1 scope):**

1. `crates/credential/src/contract/credential.rs` — `Credential` trait reduced; new sub-traits added in `crates/credential/src/contract/{interactive,refreshable,revocable,testable,dynamic}.rs`.
2. `crates/credential/src/credentials/{api_key,basic_auth,oauth2}.rs` — built-in credentials updated to declare `impl Refreshable for OAuth2Credential` etc. as appropriate.
3. `crates/engine/src/credential/rotation/` — `RefreshDispatcher::for_credential<C: Refreshable>` bound. Compile-fail probe: `Engine::refresh::<NonRefreshableCred>()` rejected.
4. `nebula-credential-builtin` — `#[plugin_credential]` macro detects scheme + emits appropriate sub-trait impls. `#[capability(refreshable)]` opt-in syntax.
5. **Compile-fail probe** `tests/compile_fail_capability_subtrait.rs` — credential declaring `impl Refreshable` without `refresh()` body fails to compile (4 cases, one per refreshable/revocable/testable/dynamic; interactive subsumed by `Pending` assoc type).

Register row `arch-capability-subtrait-split` opens with `decided` status pointing to this section.

### §15.5 `AuthScheme` sensitivity dichotomy — `SensitiveScheme` vs `PublicScheme`

**Open question** (surfaced by security-lead N2 + N10). `AuthScheme` (§2.2) has no compile-time way to express that an implementation contains secret material — every plugin-authored scheme is reviewer-gated for "field shapes use SecretString correctly." A plugin can write `pub struct MyScheme { pub token: String }` with plain `String` for a token; `impl AuthScheme for MyScheme {}` accepts it; `Debug` printing leaks the token; `Serialize` for telemetry leaks; every clone-site without redaction leaks. The §2.2 line 312 carve-out for "non-sensitive schemes" is the rationale loophole exploited by the (currently absent) `WebhookUrlScheme` — knowing a webhook URL IS knowing the bearer secret, but the §2.2 carve-out treats URL-shaped schemes as exempt.

**Two candidates considered:**

**(a) Split `AuthScheme` into sensitive vs public sub-traits.**

```rust
pub trait AuthScheme: Send + Sync + 'static {}
pub trait SensitiveScheme: AuthScheme + ZeroizeOnDrop {}
pub trait PublicScheme: AuthScheme {}
// Mutually exclusive: a scheme is one or the other (or technically both if
// declared, but the macro forbids).
```

Derive macros `#[auth_scheme(sensitive)]` and `#[auth_scheme(public)]` audit fields at expansion: `sensitive` requires every field to be `SecretString` / `SecretBytes` / nested `SensitiveScheme`; `public` requires no field to be `SecretString`. Field-name lint catches `token` / `secret` / `key` / `password` / `bearer` — those names default to sensitive even if typed `String` (forces explicit author choice).

Pros: every scheme has a typed sensitivity declaration. Reviewer can read `impl SensitiveScheme for X` and immediately know the contract. `WebhookUrlScheme` (when added) declares `SensitiveScheme` because its URL IS the secret. `InstanceBinding` (genuinely no secrets — provider/role/region identifiers) declares `PublicScheme` and gets no `ZeroizeOnDrop` overhead. Field audit at derive time catches plain `String` for sensitive fields with `E0277`.

Cons: derive-macro complexity increases. `nebula-credential-macros` needs field-type and field-name introspection.

**(b) Single `AuthScheme` + associated `const SENSITIVITY: SchemeSensitivity` + derive-time field audit.**

Pros: one less trait. Cons: const enum value vs impl-of-trait is weaker compile-time signal — readers must inspect const value rather than trait impl. `dyn SchemeWithSensitivity { c: SchemeSensitivity::Sensitive, ... }` still requires runtime branching for "is this sensitive?" use cases.

**Decision: (a).** Split into `SensitiveScheme` + `PublicScheme`.

**Rationale summary.** The non-sensitive carve-out has a precedent for misuse (security-lead N10 specifically about `WebhookUrlScheme`). Mandatory typed sensitivity declaration removes the rationalization path. `ZeroizeOnDrop` overhead on `PublicScheme` is zero (trait has no methods); `SensitiveScheme: ZeroizeOnDrop` ensures plaintext drops from heap deterministically.

**Implementation impact (П1 scope):**

1. `crates/credential/src/contract/scheme.rs` — `AuthScheme` reduced to base; `SensitiveScheme` + `PublicScheme` added.
2. `crates/credential/src/scheme/*.rs` — each scheme file updated:
   - `BearerScheme`, `BasicScheme`, `SigV4Scheme`, `TlsIdentityScheme`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, `FederatedAssertion`, `ChallengeSecret`, `OtpSeed`, `ConnectionUri`, `SharedKey` → `SensitiveScheme`
   - `InstanceBinding` (provider + role + region; no secret) → `PublicScheme`
   - `Coercion` (transformer; no own secret) → `PublicScheme` if no embedded sensitive material; else `SensitiveScheme`
3. `WebhookUrlScheme` (if added per N10 / `draft-f35` Trigger sub-spec) — declared `SensitiveScheme` with URL wrapped in `SecretString`. URL accessor returns `&SecretString`.
4. **`OAuth2Token::bearer_header(&self)` returns `SecretString`**, not `String` — closes N4 inline. (No separate §15 amendment for §15.9 SecretString accessors needed; consolidated here.)
5. **`ConnectionUri`** — closes N4. Restructured to:
   ```rust
   pub struct ConnectionUri {
       scheme: String,                        // "postgres"
       host: String,                          // safe to log
       port: Option<u16>,
       database: String,
       username: String,
       password: SecretString,
   }
   impl ConnectionUri {
       pub fn host(&self) -> &str { &self.host }
       pub fn port(&self) -> Option<u16> { self.port }
       pub fn database(&self) -> &str { &self.database }
       pub fn username(&self) -> &str { &self.username }
       pub fn password(&self) -> &SecretString { &self.password }
       pub fn as_url(&self) -> SecretString { /* reconstruct full URL inside SecretString */ }
   }
   impl SensitiveScheme for ConnectionUri {}
   ```
   Driver injection sites use `.expose_secret()` on `as_url()` exactly once, at FFI boundary.
6. §6.7 zeroize-invariants table: every "convention" row converted to "compile-time" row citing the trait bound responsible (per security-lead sign-off condition 5).
7. **Compile-fail probe** `tests/compile_fail_scheme_sensitivity.rs` — covers: (a) `impl SensitiveScheme for X` where `X` has plain `String` field for token-named field, (b) `impl PublicScheme for X` where `X` has `SecretString` field, (c) `impl SensitiveScheme for X` without `ZeroizeOnDrop` derive.
8. §2.2 line 312 carve-out paragraph removed and replaced with reference to this §15.5.

Register row `arch-scheme-sensitivity-dichotomy` opens with `decided` status.

### §15.6 Fatal duplicate-KEY registration — `Result<(), DuplicateKey>`

**Open question** (surfaced by security-lead N7). `CredentialRegistry::register<C>` (§3.1 line 664) currently signatures as `pub fn register<C: Credential>(&mut self, instance: C)`. Per §3.1 lines 662-663: "Duplicate keys are a programming error — panic in debug, warn + overwrite in release (instrumented)." In production, two plugins shipping with the same `KEY` (e.g., `"slack.oauth2"`) result in the second overwriting the first with only a `tracing::warn!` log. If the overwriting plugin is malicious (supply-chain), stale (wrong version), or simply a name collision between first-party and third-party crate, every `CredentialRef<SlackOAuth2>` resolves against the rogue credential — including its `resolve()`, `refresh()`, and stored state. The warn-log is the only signal, easily lost in the noise of startup logging.

**Two candidates considered:**

**(a) Fatal duplicate-KEY in BOTH debug and release; `register` returns `Result<(), RegisterError>`.**

```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum RegisterError {
    #[error("duplicate credential key '{key}': existing crate {existing_crate}, new crate {new_crate}")]
    DuplicateKey {
        key: &'static str,
        existing_crate: &'static str,
        new_crate: &'static str,
    },
}

impl CredentialRegistry {
    pub fn register<C>(
        &mut self,
        instance: C,
        registering_crate: &'static str,
    ) -> Result<(), RegisterError>
    where
        C: Credential
            + plugin_capability_report::IsInteractive
            + plugin_capability_report::IsRefreshable
            + plugin_capability_report::IsRevocable
            + plugin_capability_report::IsTestable
            + plugin_capability_report::IsDynamic,
    {
        let key: &'static str = C::KEY;
        if let Some(existing) = self.entries.get(key) {
            return Err(RegisterError::DuplicateKey {
                key,
                existing_crate: existing.registering_crate,
                new_crate: registering_crate,
            });
        }
        let capabilities = compute_capabilities::<C>();
        self.entries.insert(key.into(), RegistryEntry {
            instance: Box::new(instance),
            capabilities,
            registering_crate,
        });
        Ok(())
    }
}
```

> **Snippet alignment note (2026-04-26 stage5-followup-s3):** This snippet was reconciled with the landed code (`crates/credential/src/contract/registry.rs` commit `c44eb2ca`) during П1 Stage 8. Three differences from the original CP6 candidate (a) draft:
> 1. `registering_crate: &'static str` is an explicit parameter, not `env!("CARGO_CRATE_NAME")` inside the body — the macro would resolve to `nebula-credential` regardless of which plugin called `register`, defeating per-plugin attribution.
> 2. The `C: Credential + CredentialMetadataSource` bound was replaced with the five `plugin_capability_report::Is*` bounds — capability discovery (§15.8) folds into `register`, so the bounds make every registered type carry a static capability report. `CredentialMetadataSource` is a §15.8 / Stage 7+ concern.
> 3. Capability set is computed at registration via `compute_capabilities::<C>()` and stored alongside the instance. This lets `iter_compatible` (§15.8) filter without re-running the fold per call.

Plugin init uses `registry.register(MyCred::new(), env!("CARGO_CRATE_NAME"))?` — duplicate is fatal at startup. Operator must resolve via plugin uninstall, version pin, or namespace fix.

Pros: silent overwrite eliminated. Operator gets a clear, actionable error. Plugin name collision becomes a hard startup failure, not a runtime stealth issue.

Cons: existing init code in built-in plugins must add `?` propagation. Operator-facing error must be readable.

**(b) Retain warn+overwrite, gate behind config flag `allow_credential_overrides: false` default.**

Pros: backward-compat — existing init code doesn't break. Cons: still allows the hazard if operator misconfigures; relies on operator awareness; weaker default-safe posture.

**Decision: (a).** Fatal duplicate-KEY, `Result<(), RegisterError>`.

**Rationale summary.** Silent credential takeover (security-lead N7) is a 🟠 HIGH severity issue. Default behavior must be fail-closed. Operator can resolve via explicit action; default-permissive overwrite path is a worse default than default-strict-with-explicit-error. Fail-closed at startup beats fail-stealthily at runtime.

**Implementation impact (П1 scope):**

1. `crates/credential/src/contract/registry.rs` (or wherever `CredentialRegistry` lives in the post-cleanup structure) — `register` returns `Result<(), RegisterError>`. `RegisterError` added to error taxonomy.
2. Built-in plugin init code (`crates/credential/src/credentials/{api_key,basic_auth,oauth2}.rs` registration sites) — `?` propagation added.
3. **Compile-fail probe is not applicable** here — duplicate-KEY collision is data-dependent (same string in two crates), not statically detectable across crates. Instead: **runtime test** `tests/runtime_duplicate_key_fatal.rs` verifies `register` returns `RegisterError::DuplicateKey` on second registration of same KEY; engine startup harness verifies fatal.
4. **Long-term defense beyond fatal duplicate-KEY:** `arch-signing-infra` sub-spec (queue #7, post-MVP) — signed plugin manifests prove provenance, eliminating supply-chain risk entirely. §15.6 is the interim mitigation pending signing infra.

Register row `arch-registry-duplicate-fail-closed` opens with `decided` status. Sub-spec `arch-signing-infra` row updated with §15.6 cross-reference.

### §15.7 `SchemeGuard` + `SchemeFactory` for refresh hook

**Open question** (surfaced by security-lead N8 + tech-lead technical gap (i)). `Resource::on_credential_refresh(&self, new_scheme: &Scheme)` (§3.6) takes a borrowed `&Scheme`. A `Resource` impl can:
- Clone the scheme into a side-channel field (`Arc<Scheme>` retention) if `Scheme: Clone`.
- Keep the borrow's lifetime if the resource holds the reference past the call (compile-time barrier exists, but smuggling via `unsafe` or `transmute` is reviewer-only-detectable).
- Under async cancellation: a dropped future holding `&Scheme` mid-call leaves the resource with stale Scheme reference if the resource captured it pre-await.

Tech-lead's framing: resources retaining a Scheme across requests need a **factory, not `&new_scheme`**.

**Decision** (single candidate; no alternatives evaluated because the requirements are tightly constrained).

**`SchemeGuard<'a, C: Credential>` — owned, `!Clone`, `ZeroizeOnDrop`, `Deref<Target = C::Scheme>`.**

```rust
pub struct SchemeGuard<'a, C: Credential> {
    scheme: <C as Credential>::Scheme,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a, C: Credential> SchemeGuard<'a, C> {
    // Constructed by engine; not constructible by Resource impls.
    pub(crate) fn new(scheme: <C as Credential>::Scheme) -> Self { ... }
}

impl<'a, C: Credential> Deref for SchemeGuard<'a, C> {
    type Target = <C as Credential>::Scheme;
    fn deref(&self) -> &Self::Target { &self.scheme }
}

impl<'a, C: Credential> Drop for SchemeGuard<'a, C> { /* zeroize */ }

// Crucially: NO `Clone`, NO `Send` if Scheme contains !Send material;
// lifetime parameter prevents storage in struct fields outliving the call.

// New §3.6 hook signature:
pub trait Resource: Send + Sync {
    type Credential: Credential;
    type Error: std::error::Error;

    async fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
    ) -> Result<(), Self::Error> {
        let _ = new_scheme;
        Ok(())  // Default no-op
    }
}
```

**`SchemeFactory<C>` companion — for long-lived resources needing re-acquisition, not retention.**

```rust
/// Engine provides this to long-lived resources at construction.
/// Resource invokes factory on each request (or pool refresh) to get a
/// fresh SchemeGuard. Resource never retains the Scheme itself.
pub struct SchemeFactory<C: Credential> {
    inner: Arc<dyn Fn() -> BoxFuture<'static, Result<SchemeGuard<'static, C>, AcquireError>>
        + Send + Sync>,
}

impl<C: Credential> SchemeFactory<C> {
    pub async fn acquire(&self) -> Result<SchemeGuard<'_, C>, AcquireError> {
        (self.inner)().await
    }
}
```

**Worked example — HTTP connection-pool resource with refreshable OAuth2 bearer** (per tech-lead condition 5):

```rust
// Hypothetical resource: long-lived HTTPS pool that authenticates with an
// OAuth2 bearer token. Bearer rotates every ~30 minutes via Refreshable.
// Pool must not retain the SchemeGuard, but needs a fresh bearer per request.

pub struct OAuth2HttpPool {
    pool: reqwest::Client,
    bearer_factory: SchemeFactory<MyOAuth2Credential>,
}

impl Resource for OAuth2HttpPool {
    type Credential = MyOAuth2Credential;
    type Error = OAuth2HttpPoolError;

    async fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, MyOAuth2Credential>,
    ) -> Result<(), Self::Error> {
        // Optional: bust per-pool cached bearer. Most pools just rely on
        // factory.acquire() per request — `on_credential_refresh` becomes
        // a notification hook rather than a state-update path.
        // SchemeGuard drops at end of this fn; no retention possible.
        let _ = new_scheme;
        Ok(())
    }
}

impl OAuth2HttpPool {
    pub async fn fetch(&self, url: &str) -> Result<reqwest::Response, OAuth2HttpPoolError> {
        // Fresh bearer per request — factory zeroizes at scope exit.
        let bearer_guard = self.bearer_factory.acquire().await?;
        let bearer = bearer_guard.bearer_header();  // returns SecretString per §15.5
        self.pool.get(url)
            .header("Authorization", bearer.expose_secret())
            .send()
            .await
            .map_err(Into::into)
    }
}
```

Pool engineer reads this and sees: `SchemeGuard` lives only inside `fetch()`, drops at function return, zeroizes deterministically. Refresh hook is a notification, not a retention path. Factory is the re-acquisition mechanism. The compile-fail probe verifies the structural property.

**Implementation impact (П1 scope):**

1. `crates/credential/src/secrets/scheme_guard.rs` — `SchemeGuard<'a, C>` and `SchemeFactory<C>` types added (landed in Stage 6, commit `c25fc6ff`).
2. `crates/credential/src/secrets/scheme_guard.rs` — `OnCredentialRefresh<C: Credential>` parallel trait carries the spec-canonical refresh-hook signature with default no-op body. Resource trait integration (`type Credential` on `nebula-resource::Resource`) is **deferred to a later phase**: existing `Resource` carries 5 assoc types (`Config`/`Runtime`/`Lease`/`Error`/`Auth: AuthScheme`) with no `Credential` link; threading one would cascade through 28+ impls. Per `feedback_adr_revisable.md` (ADR/spec workarounds → supersede, don't patch around), the parallel `OnCredentialRefresh<C>` trait in `nebula-credential` IS canonical for П1; full `Resource` trait integration via `Credential` cascade lives in a follow-up cascade. Tracked in the concerns register as `stage6-followup-resource-integration`. Stage 6 lands the type surface; engine wiring to dispatch refreshes via `OnCredentialRefresh` lives in a follow-up cascade.
3. `crates/resource/src/manager.rs` — `on_credential_refreshed` (manager-level fan-out keyed by `&CredentialId`, line 1378) retains its current `todo!()`. Live wiring lands when `OnCredentialRefresh` integration is decided; the manager's role is dispatch-by-id, not the per-resource hook itself.
4. **Compile-fail probe** `tests/compile_fail_scheme_guard_retention.rs` — `Resource` impl that stores `SchemeGuard` in a struct field outlasting the call fails to compile (`E0597` lifetime error).
5. **Compile-fail probe** `tests/compile_fail_scheme_guard_clone.rs` — `let g2 = guard.clone()` fails (`E0599` no `clone` method).
6. Cancellation-safety contract documented inline in §3.6: `on_credential_refresh` must be cancel-safe; future drops zeroize `SchemeGuard` deterministically.

**Lifetime-gap refinement (spike iter-3 secondary finding, 2026-04-24 commit `f36f3739`):** the `SchemeGuard<'a, C>` structure above with only `PhantomData<&'a ()>` does NOT structurally prevent retention — the `'a` parameter is inferred `'static` when no actual borrow pins it. A `Resource` impl could accept `SchemeGuard<'static, C>` and store it in a field. To prevent this:

1. Engine passes `SchemeGuard<'a, C>` **alongside** `&'a CredentialContext<'a>` (or similar borrow source) sharing the same `'a` lifetime parameter. Retention attempt forces the engine-borrowed reference to also outlive the struct, triggering `E0597`.
2. Refined `on_credential_refresh` signature:
   ```rust
   async fn on_credential_refresh<'a>(
       &self,
       new_scheme: SchemeGuard<'a, Self::Credential>,
       ctx: &'a CredentialContext<'a>,  // shared 'a pins guard lifetime
   ) -> Result<(), Self::Error>;
   ```
3. `SchemeFactory::acquire` signature similarly threads a context-tied lifetime so factory callers cannot hoist the guard out of scope.

Spike probe 4 (`compile_fail_scheme_guard_retention.rs`) tests the CORRECT form. П1 scaffolding PR must include this refinement + extended compile-fail probe variants covering the retention-via-static-inference case.

Register row `arch-scheme-guard-factory` opens with `decided` status.

### §15.8 Capability-from-type, not from metadata — `iter_compatible` authority shift

**Open question** (surfaced by security-lead N6). §9.4 `iter_compatible` (line 2418-2427) filters credentials by `cred.metadata().capabilities_enabled.contains(*capability)`. The `capabilities_enabled: Capabilities` field (§9.2 line 2348) lives in plugin-authored `CredentialMetadata`. A misbehaving or compromised plugin sets `capabilities_enabled = Capabilities::ALL` to appear in slot pickers it shouldn't satisfy — Pattern 3 (`SlotType::CapabilityOnly`) accepts it without service-key cross-check, and the §15.6 fatal-duplicate-KEY mitigation does not catch this hazard (different KEY, same false capability claim).

**Two candidates considered:**

**(a) Compute `capabilities_enabled` at registration from sub-trait membership; remove the field from plugin-authored `CredentialMetadata`.**

After §15.4 sub-trait split, capability membership is a typed property: `C: Refreshable` is or is not satisfied at compile time. `CredentialRegistry::register<C>` introspects (via blanket-trait detection or generated capability table) and **constructs** `capabilities_enabled` itself, overriding any plugin-supplied value. Plugin's `CredentialMetadata` no longer has the field.

```rust
// At registration time:
fn compute_capabilities<C: Credential>() -> Capabilities {
    let mut caps = Capabilities::empty();
    // Detection mechanism — emitted by #[plugin_credential] macro at impl site:
    if <C as plugin_capability_report::Refreshable>::IS_REFRESHABLE { caps.insert(Capability::Refreshable); }
    if <C as plugin_capability_report::Revocable>::IS_REVOCABLE { caps.insert(Capability::Revocable); }
    if <C as plugin_capability_report::Testable>::IS_TESTABLE { caps.insert(Capability::Testable); }
    if <C as plugin_capability_report::Interactive>::IS_INTERACTIVE { caps.insert(Capability::Interactive); }
    if <C as plugin_capability_report::Dynamic>::IS_DYNAMIC { caps.insert(Capability::Dynamic); }
    caps
}
```

`plugin_capability_report::*` is a per-capability blanket trait emitted by `#[plugin_credential]` macro: `IS_REFRESHABLE = true` if `C: Refreshable`, `false` otherwise. Standard "trick traits" for stable Rust without specialization.

Pros: capabilities derived from type system. Plugin cannot lie. Pattern 3 `CapabilityOnly` slot binding becomes safe — the registry-computed capability set is authoritative. Self-attestation hazard structurally removed.

Cons: `#[plugin_credential]` macro carries the blanket-trait emission. Per-credential capability detection is one extra pass at expansion.

**(b) Cross-check at registration; panic if plugin-declared `capabilities_enabled` disagrees with computed type-derived set.**

Pros: backward-compat for plugins that already declare. Cons: still requires the plugin field to exist (clutter); can be fooled by a plugin that lies and never gets cross-checked at registration time.

**Decision: (a).** Capabilities-from-type, plugin metadata field removed.

**Rationale summary.** Type system is the only authority that cannot be subverted at runtime. Removing the field eliminates the field-existence hazard. Pattern 3 `CapabilityOnly` slot binding is preserved as a feature (per Strategy §2.2 `GenericOAuth2Credential` use case) but the matching is now safe.

**Implementation impact (П1 scope):**

1. `crates/credential/src/metadata.rs` — `CredentialMetadata::capabilities_enabled` field removed. Plugin-authored metadata no longer carries it.
2. `crates/credential/src/contract/registry.rs` — `register<C>` computes capabilities at registration and stores in `RegistryEntry` alongside the boxed credential.
3. `crates/engine/src/credential/discovery.rs` — `iter_compatible` filter consults `RegistryEntry::capabilities` (registry-computed), not `cred.metadata().capabilities_enabled`.
4. `nebula-credential-macros` — `#[plugin_credential]` macro emits `plugin_capability_report::*` blanket impl per capability sub-trait the credential implements.
5. **Runtime test** `tests/runtime_metadata_no_capability_field.rs` — verifies that no plugin-authored `CredentialMetadata` carries a `capabilities_enabled` field after the field removal.
6. **Compile-fail probe** `tests/compile_fail_metadata_capability_field.rs` — attempting to add `capabilities_enabled` to a plugin's `CredentialMetadata` instance fails to compile (field does not exist).

Register row `arch-metadata-capability-authority` opens with `decided` status.

### §15.9 (reserved — consolidated into §15.5)

The proposed amendment 7 (`OAuth2Token::bearer_header → SecretString` + `ConnectionUri` structured accessors) is consolidated into §15.5 implementation-impact section since the changes are mechanical extensions of the sensitivity dichotomy. No separate decision section needed; tracking in §15.5 register row.

### §15.10 `PendingStore::consume` atomicity contract — runtime-gated (security-lead N9)

**Tech-lead condition 2** — runtime-gated, NOT compile-time-gated. Closure marker for П-later phase, not П1 landing-gate.

**Open question** (surfaced by security-lead N9). `PendingStore::consume(id)` (§5.5 consumer surface) must be transactionally single-use across concurrent engine instances. Current §6.9 description states "`get_then_delete` transactional pop: rows are deleted as part of `continue_resolve`", but the GC sweep at §6.9 line 1723 runs `DELETE FROM pending_credentials WHERE expires_at < now()` independently. Race window: callback arrives at t = expires_at - ε; engine A's `continue_resolve` reads the pending row and begins exchange (500ms-2s for IdP); concurrent GC sweep on engine B sees `expires_at < now()` and deletes the row mid-flight; A's transaction-commit sees 0 rows affected; depending on `PendingStore` impl, A either silently succeeds (returning `Ready(state)` and writing `credentials` row even though pending row vanished) or returns confusing `PendingError::NotFound`.

**Decision** (single candidate; runtime-gated):

**(a) `PendingStore::consume(id)` atomic `DELETE ... RETURNING` (Postgres) or equivalent SQLite transactional pop. GC sweep adds 60s grace window.**

```sql
-- Postgres: atomic pop
DELETE FROM pending_credentials
WHERE id = $1
RETURNING state_encrypted, expires_at;

-- GC sweep with grace
DELETE FROM pending_credentials
WHERE expires_at < now() - INTERVAL '60 seconds';
```

SQLite uses equivalent transactional `BEGIN; SELECT...; DELETE...; COMMIT` with `SERIALIZABLE` isolation.

Pros: atomic single-use semantics. GC sweep grace window prevents in-flight callback collision with cleanup. `consume` and `gc_sweep` cannot both succeed on the same row because `DELETE ... RETURNING` and `DELETE WHERE expires_at < now()` serialize via row lock.

Cons: implementations must explicitly support atomic pop. NoOp `PendingStore` for desktop-mode (§11.1) trivially satisfies via in-memory single-thread.

**Implementation impact (P-later phase, NOT П1):**

1. `crates/storage/src/credential/pending.rs` — `PendingStore::consume(id)` contract specified as atomic. Postgres impl uses `DELETE ... RETURNING`. SQLite impl uses transactional `BEGIN; ...; COMMIT`.
2. `crates/storage/src/credential/gc.rs` (new) — GC sweep with 60s grace window.
3. **Concurrency test** `tests/concurrency_pending_consume_vs_gc.rs` — spawns concurrent `consume` + `gc_sweep`; verifies invariant "row consumed XOR row gc'd, never both, never neither".
4. §6.9 text updated: `PendingStore::consume` is atomic; GC sweep grace window 60s default; SQLite parity confirmed.

Register row `runtime-pending-consume-atomicity` opens with `proposed` status (П-later closure, not П1).

### §15.11 Sign-off matrix — CP6 final (2026-04-24 Rounds 0-7)

CP5 matrix superseded by CP6 after Rounds 6-7 re-engagement under active-dev framing.

| Stakeholder | Sign-off | Position | Quotable summary |
|-------------|----------|----------|------------------|
| User (vanyastaff) | endorse | Active-dev framing + sub-trait split satisfy «power AND safety»; 3 gates before П1 are engineering-correct sequencing, not consumer-deferral | «Tech-lead должен как то помочь... на этапе планирования» — Round 5 user pushback that reframed the question from adoption-deferred to active-dev gates |
| Tech-lead | endorse-phased (Round 7 flip from Round 4 endorse-with-conditions) | 3 gates before П1: (1) P10 cleanup completion, (2) N7 standalone registry fix, (3) spike iter-3 sub-trait × phantom-shim dyn-safety validation | «Position flips from adoption-deferred to endorse-phased (B). My Round 4 rested on a false premise — I wrote 'P6-P11 landed' into memory from a plan doc's self-claim without ls-ing crates/api/src/credential/, which is empty. Under active-dev framing with feedback_hard_breaking_changes.md, 'no consumer pressure' isn't a technical objection. N1 and N7 are live today (state.rs:18 lacks ZeroizeOnDrop; registry.rs:31 has zero tracing). Three gates before П1: (1) actually land P10 + correct the doc record, (2) standalone N7 registry observability + policy fix, (3) narrow spike iter-3 validating sub-trait × ADR-0035 phantom-shim dyn-safety. Then П1 starts.» |
| Security-lead | confirmed (Round 5) + partial-escalate (Round 6 — N7 tier-A immediate fix, N1/N3/N5 tier-B hard-gate-before-plugin-surface) | Gate 2 directly satisfies sec-lead Round 6 tier-A escalate; Gate 3 + post-gate П1 satisfy tier-B «before plugin surface opens» | Round 5: «All 5 tech-lead conditions either reinforce or are neutral to my sign-off conditions; none break or weaken. Sign-off confirmed.» Round 6: «I AM escalating N7 to standalone hard-block because `crates/engine/src/credential/registry.rs:31` HashMap::insert silently overwriting on duplicate KIND is a real, today-exploitable misconfiguration vector — independent of the Tech Spec.» |

**All three stakeholders endorse CP6 endorse-phased path.** No papered-over disagreements. Active-dev framing replaces prod-release framing. Tech Spec status flips `complete CP5 → complete CP6 (active-dev endorse-phased, 3 gates before П1)`.

### §15.12 Three engineering gates before П1 (CP6 — added Round 7)

Tech-lead Round 7 flip-to-B specifies exactly three gates that must close before П1 trait-scaffolding implementation starts. Each gate is a concrete PR scope, not a vague trigger.

#### §15.12.1 Gate 1 — P10 doc reconciliation (CLOSED 2026-04-24 amendment)

**Original scope (CP6 Round 7):** code migration to `crates/api/src/credential/` per ADR-0031 §1 prescription + doc correction.

**Revised scope post-verification:** P10 was **functionally landed** during the original cleanup track under axum convention (`crates/api/src/handlers/credential_oauth.rs` + `crates/api/src/services/oauth/`), not under ADR-0031's aspirational `credential/` subdirectory. CP6 Round 7 framing «P10 NOT landed» was based on incomplete verification (only `ls crates/api/src/credential/` returning empty; did not check `handlers/` or `services/`). Actual gap: docs drift (plan doc + ADR §1 paths + Tech Spec §0 claim), not code absence.

**Gate closure = doc-sync PR** (this commit):
- ADR-0031 amendment 2026-04-24-A applied (path reconciliation table + axum-convention rationale).
- `p6-p11.md:P10` line updated with landed paths + feature-gate evidence.
- Tech Spec §0 «Actual landing status» updated with landed-under-axum-convention narrative.
- Register row `gate-p10-landing` flipped to `decided` with this commit ref.

**Security invariants §4.1-§4.6 of ADR-0031 preserved at landed paths** (verified):
- PKCE S256 mandatory — `services/oauth/flow.rs` construction path
- CSRF HMAC-bound state — `services/oauth/state.rs` `OAuthStateSigner`
- reqwest TLS+timeout+body-cap — `services/oauth/http.rs`
- URL allowlist — per-credential config check (at callback handler)
- Zeroize on partial failure — request body + response path (service layer)
- Feature-gated `credential-oauth` — `crates/api/Cargo.toml` `[features]`

**What was NOT changed:** zero code moves, zero file renames. Axum convention preserved. Applying strict ADR-0031 §1 letter would require moving 594 LOC controller + multiple service files + updating all imports for no functional benefit (landed structure is already idiomatic).

**What Gate 1 closure unblocks:** Tech Spec §0 honest baseline now matches code. Gate 3 spike (already landed at `f36f3739`) can reference accurate P10 status. П1 scaffolding PR writes against post-cleanup truth, not aspirational structure.

**Remaining follow-up (not blocking Gate 2 or П1):** MATURITY row `nebula-api` integration column still reflects «credential-oauth feature-gated» honest status until E2E test stability allows feature-default flip. Tracked as post-П11 activity per ADR-0031 Follow-ups §.

#### §15.12.2 Gate 2 — N7 registry standalone fix (CLOSED 2026-04-24)

**Scope (as implemented):** `crates/engine/src/credential/registry.rs` — `register<C>` gains observability + duplicate-policy. Zero external call sites discovered during verification; migration cost was minimal.

**Implementation landed:**
```rust
pub fn register<C>(&mut self) -> Result<(), RegistryError>
where
    C: Credential,
    C::Scheme: 'static,
{
    let kind = <C::State as CredentialState>::KIND;
    if self.handlers.contains_key(kind) {
        return Err(RegistryError::DuplicateKind { kind: kind.to_string() });
    }
    tracing::info!(credential.kind = %kind, "credential kind registered");
    self.handlers.insert(kind.to_string(), Arc::new(/* project fn */));
    Ok(())
}
```

Plus new `RegistryError::DuplicateKind { kind: String }` variant with `thiserror` message embedding the active-dev policy hint («reject-second-registration; resolve via rename, `#[plugin_credential]` namespace, or remove duplicate»).

**Policy landed:** **reject-second-registration** (active-dev + hard breaking changes OK per `feedback_hard_breaking_changes.md`). No warn-and-overwrite fallback — would reintroduce the N7 silent-takeover hazard.

**Tracing on success:** `tracing::info!(credential.kind = %kind, "credential kind registered")` — zero-leak (kind is static type identifier like `"secret_token"`, not user/secret material). Prior state was zero tracing (grep `tracing|warn|log` in file returned no matches; §15.12.2 original scope documented this).

**Test landed:** `crates/engine/tests/registry_duplicate_kind_fatal.rs` — 3 tests, all green:
- `first_registration_succeeds` — sanity baseline.
- `duplicate_registration_returns_error_no_overwrite` — verifies `RegistryError::DuplicateKind` with correct `kind` string, `len()` unchanged, `contains(kind)` still true.
- `duplicate_error_message_includes_policy_hint` — verifies error message carries «duplicate credential kind», «reject-second-registration», and the colliding `kind`.

**Call-site migration:** zero external callers existed (verified via grep for `CredentialRegistry::register` / `.register::<`); `?` propagation not needed downstream. Signature change is non-breaking in practice.

**Verification:** `cargo check -p nebula-engine --all-targets` + `cargo clippy -p nebula-engine --all-targets -- -D warnings` both green. Tests: `cargo test -p nebula-engine --test registry_duplicate_kind_fatal` — 3/3 pass.

**Gate 2 closure criterion satisfied:** `registry.rs:31` no longer silently overwrites; `tracing::info!` on success; `RegistryError::DuplicateKind` on collision; runtime test confirms invariant.

#### §15.12.3 Gate 3 — Spike iter-3 narrow-scope dyn-safety validation

**Scope:** Spike validates CP5 sub-trait × ADR-0035 phantom-shim composition on 2-3 credential types. Addresses tech-lead Round 4 condition 1 («confirm the sub-traits compose cleanly with ADR-0035 phantom-shim for `dyn Credential` erasure»).

**Target types to validate:**
1. `ApiKeyCredential` — static, no sub-trait (baseline phantom-shim for service capability).
2. `OAuth2Credential` — `Refreshable` + `Revocable` (lifecycle sub-traits).
3. One interactive credential (e.g., hypothetical `SalesforceJwtCredential`) — `Interactive` + `Refreshable`.

**Spike must demonstrate:**
- **(a) `dyn Credential` object-safety** preserved when `Interactive` / `Refreshable` / `Revocable` / `Testable` / `Dynamic` are split into sub-traits (with `Pending` moved to `Interactive`).
- **(b) Phantom-shim erases `C::Scheme` cleanly** for Pattern 2 / Pattern 3 action consumers (e.g., `CredentialRef<dyn BitbucketBearerPhantom>` still compiles).
- **(c) Parallel phantom-shim needed for lifecycle sub-traits?** Open question: if `dyn Refreshable` is needed for runtime dispatch (e.g., engine's refresh registry), does it require `RefreshablePhantom` analogous to `BitbucketBearerPhantom`? Spike answers empirically.
- **(d) Capability-const downgrade path** (for backward-compat with legacy consumers reading `REFRESHABLE: bool`) still compiles if opt-in legacy adapter lands.
- **(e) Compile-fail probes from §16.1.1 actually fire** with expected diagnostics (one per amendment).

**Artifacts:**
- Spike worktree branch (new — independent of prior `worktree-agent-a23a1d2c`) with crate scaffold.
- `NOTES.md` documenting approach, findings, dyn-safety verdict per (a)-(e).
- If parallel `Phantom` shims are needed: ADR-0035 amendment 2026-04-XX-C with the composition pattern.

**Estimated effort:** 1 spike PR + writeup. 3-5 days agent-work per user Round 5 recommendation.

**Gate closes when:** spike branch lands to archive; NOTES.md answers (a)-(e); if (c) requires amendment: ADR-0035 amendment written + accepted; §15.4 updated with spike outcome.

#### §15.12.4 Gate sequencing

Per tech-lead Round 7:
- **Gates 1 and 2 run in parallel** (independent crates: `api/`, `engine/`).
- **Gate 3 starts once Gate 1 baselines honest state** (so spike knows the cleanup is complete).
- **П1 trait-scaffolding starts after all three gates close.**

Rough estimate: Gate 1 ≈ 2 PRs, Gate 2 ≈ 1 PR, Gate 3 ≈ 1 spike PR + writeup, then П1.

Register rows `gate-p10-landing`, `gate-n7-registry-observability`, `gate-spike-iter3-dyn-safety` opened with `proposed` status pointing to §15.12.1/§15.12.2/§15.12.3 respectively.

## §16 Implementation handoff

Implementation plan outline. Detailed phase plans land in `docs/superpowers/plans/<NNNN>-<phase>.md` per phase as implementation begins. Phase boundaries based on dependency ordering + landing-gate clarity.

### §16.1 Phase list

| Phase | Scope | Dependencies |
|---|---|---|
| **П1** | Trait shape scaffolding — `nebula-credential` trait + `mod sealed_caps` + phantom-shim canonical form per ADR-0035 + `nebula-credential-builtin` crate scaffold + **§15.3-§15.9 compile-time amendments** (capability sub-trait split, AuthScheme dichotomy, fatal duplicate-KEY, SchemeGuard+SchemeFactory, capability-from-type, SecretString accessors) + **8 mandatory compile-fail probes** (see §16.1.1) | None (foundational) |
| **П2** | Refresh coordination L2 — `RefreshClaimRepo` impl + [`draft-f17`](2026-04-24-credential-refresh-coordination.md) sub-spec landing + L1+L2 two-tier integration | П1 |
| **П3** | Builtin credential types — Slack, Anthropic, Bitbucket triad (OAuth2 + PAT + AppPassword), AwsSigV4 + STS, Postgres connection, mTLS, Salesforce JWT | П1 |
| **П4** | ProviderRegistry — sub-spec `draft-f18/f19/f20` landing + admin API + URL template binding | П3 + Tech Spec §11 frozen |
| **П5** | Multi-mode polish — desktop / self-hosted / cloud feature gating + cargo features per §11.5 | П1 + П3 |
| **П6** | Audit + degraded mode — fail-closed write sequence + degraded read-only + fallback file sink + drain on recovery | П1 (storage layer wraps) |
| **П7** | OAuth flow consolidation — engine HTTP ceremony + ADR-0031 supersede if needed | П3 + Tech Spec §10 frozen |
| **П8** | §15.1-§15.2 decisions implementation — `provider_id` move to `CredentialMetadata` + `AuthScheme: Clone` relax + per-scheme `Clone` derive cleanup + consumer site migration. **(§15.3-§15.9 are П1 scope per CP5 closure; §15.10 PendingStore atomicity is П-later runtime gate.)** | П1 + П3 |
| **П9** | Migration v1→v2 — `draft-f36` sub-spec landing + lazy migration on resolve + bulk CLI | П1 + П3 |
| **П10** | Trigger ↔ credential — `draft-f35` sub-spec landing + Trigger trait integration | П1 + Trigger track (separate workstream) |

### §16.1.1 П1 sub-gates — 8 mandatory probes (7 compile-fail + 1 runtime) (added CP5 per tech-lead condition 3; clarified CP6 per Gap 8)

П1 landing-gate is **not satisfied** until all 8 probes exist and fail with the expected diagnostics or error returns. 7 probes are compile-fail (trybuild / equivalent harness); 1 probe is runtime (`RegisterError::DuplicateKind` on duplicate-KEY registration — duplicate keys across crates are not statically detectable by rustc alone). Cargo-public-api snapshot alone catches surface drift but not semantic regressions, so these probes are mandatory not optional.

| # | Probe file | Verifies | Expected diagnostic |
|---|------------|----------|---------------------|
| 1 | `tests/compile_fail_state_zeroize.rs` | `CredentialState` impl without `ZeroizeOnDrop` | `E0277` — `ZeroizeOnDrop` not satisfied |
| 2 | `tests/compile_fail_scheme_sensitivity.rs` | `SensitiveScheme` impl with plain `String` field for `token`-named field; `PublicScheme` impl with `SecretString` field; `SensitiveScheme` impl without `ZeroizeOnDrop` | `E0277` — `ZeroizeOnDrop` not satisfied; macro audit error |
| 3 | `tests/compile_fail_capability_subtrait.rs` | `impl Refreshable for X` without `refresh()` body; same for `Revocable`, `Testable`, `Dynamic` | `E0046` — required method missing |
| 4 | `tests/compile_fail_engine_dispatch_capability.rs` | `RefreshDispatcher::for_credential::<NonRefreshableCred>()` rejected; same for other dispatchers | `E0277` — bound `Refreshable` not satisfied |
| 5 | `tests/runtime_duplicate_key_fatal.rs` **(runtime probe, not compile-fail — duplicate KEYs across crates not statically detectable by rustc)** | `register::<DupCred1>()` then `register::<DupCred2>()` returns `RegisterError::DuplicateKey` | `RegisterError::DuplicateKey` returned + engine startup harness panics if operator ignores the error |
| 6 | `tests/compile_fail_scheme_guard_retention.rs` | `Resource::on_credential_refresh` impl that stores `SchemeGuard` in struct field outlasting the call | `E0597` — borrowed value does not live long enough |
| 7 | `tests/compile_fail_scheme_guard_clone.rs` | `let g2 = guard.clone()` on `SchemeGuard` | `E0599` — no method `clone` |
| 8 | `tests/compile_fail_metadata_capability_field.rs` | Plugin `CredentialMetadata` instance with `capabilities_enabled` field | `E0560` — no field `capabilities_enabled` (field removed) |

Each probe lives at `crates/credential/tests/compile_fail_*.rs` and is run via `trybuild` (or equivalent compile-fail harness) per `cargo nextest run -p nebula-credential --profile ci`. CI required-job; П1 cannot land if any probe is missing or passes-when-it-should-fail.

### §16.2 Dependency graph

```
                   П1 (trait shape — foundational)
                  /  |  |  |  |  |  \
                 П2  П3 П5 П6 П8 П9
                     |
                     П4 (after Tech Spec §11 frozen)
                     |
                     П7 (after Tech Spec §10 frozen)

                П10 (П1 + Trigger track)
```

П1 is the foundational gate. П2–П10 fan out from П1 with additional cross-dependencies on Tech Spec sections being frozen and sub-specs landing. П4 + П7 have hard dependencies on §11 + §10 freeze respectively.

### §16.3 Landing gates per phase

Each phase must satisfy before merge:

- **Tests pass.** `cargo nextest run -p <crate> --profile ci --no-tests=pass`.
- **Benches within baseline.** CodSpeed regression gate (no >20% regression vs committed baseline per §8.8).
- **Docs synced.** Tech Spec sections referenced by phase remain consistent (no contradiction). Phase plan (`docs/superpowers/plans/<NNNN>-<phase>.md`) updated.
- **Register row updates.** Affected register rows flip status (`locked-post-spike` → `decided` with phase commit pointer; or `pending-sub-spec` → `in-implementation` for sub-spec phases).
- **ABI stability check.** `cargo-public-api` confirms no stable-surface change without semver bump (§13.4).
- **Security review.** Phases touching crypto / audit / zeroization invariants get security-lead review before merge.

### §16.4 Post-Tech-Spec ADRs anticipated

Likely ADRs surfacing during implementation:

- **ADR-0036** — Rotation leader election protocol (queue #2 sub-spec). Cross-replica leader election + heartbeat / claim TTL discipline.
- **ADR-0037** — ProviderRegistry versioning + URL template binding (sub-spec `draft-f18/f19/f20`). Schema versioning + Microsoft multi-tenant template handling.
- **ADR-0038** — Schema migration discipline (sub-spec `draft-f36`). v1→v2 migration mechanism + walker patterns.
- **ADR-0031 supersede** — if §10 OAuth ceremony moves to engine per Strategy §6.4 ADR-0031 supersede candidacy + Tech Spec §10 implementation outcome.
- **§15 decision ADRs** — only if implementation reveals issues with the chosen path. Default: §15 decisions are Tech-Spec-frozen, not ADR-level.

### §16.5 Register maintenance during implementation

Per register's own maintenance rules + Tech Spec §13.4 evolution:

- Each landed phase: walk affected register rows; flip status + add Tech Spec / phase plan / commit pointer.
- New concerns surfaced during implementation: triage to one of 6 labels within 2 working days per register maintenance rule.
- Tech Spec section completes implementation: status flips to `in-implementation` then to `done` when phase merges + all tests pass.
- Register totals re-audited at every revision (no silent count drift).

### §16.5.1 П1 implementation tracker (2026-04-26)

Phase plan: [`docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md`](../plans/2026-04-26-credential-p1-trait-scaffolding.md).

Worktree: `.claude/worktrees/credential-p1` on branch `worktree-credential-p1`. 8 stages landed sequentially, one commit per task; the merge PR carries all 28 commits onto `main`.

Landing-gate checklist (verified at Stage 8):

- [x] All 8 mandatory probes (per §16.1.1) PASS — `runtime_duplicate_key_fatal` + 7 compile-fail bundles. Stage 3 ships 4 capability sub-trait probes (`refreshable` / `revocable` / `testable` / `dynamic` each missing-method); Stage 1 ships 4 sensitivity probes (`plain_string` / `plain_apikey_string` / `public_with_secret` / `public_with_option_secret` / `sensitive_no_zeroize`); Stage 2 ships `state_zeroize_missing`; Stages 6 + 7 ship guard retention/clone + metadata field probes; Stage 5 ships the runtime duplicate-KEY assertion.
- [x] Bonus probes from spike iter-3 PASS — `compile_fail_dyn_credential_const_key` + `compile_fail_pattern2_service_reject`.
- [x] Full local gate PASS (fmt nightly + clippy `--workspace --all-targets -D warnings` + `cargo nextest run --workspace --profile ci --no-tests=pass` + `cargo test --workspace --doc`).
- [x] cargo-public-api post-П1 snapshots captured for `nebula-credential` and `nebula-credential-builtin` (attached to merge PR).
- [x] MATURITY rows updated for `nebula-credential` + `nebula-credential-builtin` (preview status, П1 trait scaffolding landed).
- [x] Register status flips landed for the five architectural rows (arch-capability-subtrait-split, arch-registry-duplicate-fail-closed, arch-scheme-sensitivity-dichotomy, arch-scheme-guard-factory, arch-metadata-capability-authority) → `in-implementation` with phase-plan + Stage commit pointers. The five `stage5-followup-*` process rows resolved (i1+i2+s1+s2+s3); new row `stage7-followup-engine-discovery` added for the post-П1 slot-picker consumer wiring.
- [x] CHANGELOG / READMEs reflect new shape — `crates/credential/README.md` "П1 trait shape" section added.
- [x] §15.6 candidate (a) snippet aligned with landed `register` signature (stage5-followup-s3 closure).

Out-of-scope for П1 (deferred to follow-up cascades, all tracked in register):

- `stage6-followup-resource-integration` — engine-side `OnCredentialRefresh<C>` fan-out wiring (manager dispatch — RPITIT vs `BoxFuture` desugaring choice).
- `stage7-followup-engine-discovery` — engine slot-picker consumer routes via `iter_compatible` (Pattern 3 capability-only).
- `runtime-pending-consume-atomicity` (§15.10) — runtime-gated, П-later phase.
- `arch-techspec-section-sync` — inline §2/§3/§9 edits to remove §15.3-§15.12 forward-pointer overlay.

---

**Tech Spec complete — Checkpoint 6 ends here.**

CP4 (2026-04-24) drafted all 16 sections. CP5 (2026-04-24 Rounds 0-5) added §15.3-§15.11 closing 1 CRITICAL + 6 HIGH + 3 MEDIUM security-lead findings (N1-N10) at the trait/type level. CP6 (2026-04-24 Rounds 6-7) corrected two material errors in CP5: (i) P10 «Landed» claim wrong (`crates/api/src/credential/` directory empty); (ii) adoption-deferred-per-triggers framing wrong for active-dev stage. CP6 adds §15.12 (3 engineering gates before П1) and flips sign-off matrix §15.11 to **all-three-endorse-phased** outcome under active-dev framing.

**Adoption status: endorse-phased — 3 gates before П1.**
- **Gate 1 (§15.12.1):** P10 OAuth HTTP migration to `crates/api/src/credential/` per ADR-0031 + plan doc + §0 corrections. 2 PRs.
- **Gate 2 (§15.12.2):** N7 registry standalone fix — `registry.rs:31` gets `tracing::warn!` + reject-second-registration policy. 1 PR.
- **Gate 3 (§15.12.3):** Spike iter-3 narrow-scope — sub-trait × ADR-0035 phantom-shim dyn-safety validation on 2-3 credential types. 1 spike PR + writeup.

**П1 starts after all 3 gates close** — engineering-derived sequencing, not consumer-derived deferral.

Tech Spec ready for `writing-plans` skill invocation to produce Gate 1 / Gate 2 / Gate 3 plans (separate PRs), then П1–П10 phased implementation plans.

Consensus session document: [`2026-04-24-credential-3agent-consensus-session.md`](2026-04-24-credential-3agent-consensus-session.md) (Rounds 0-7 verbatim).
