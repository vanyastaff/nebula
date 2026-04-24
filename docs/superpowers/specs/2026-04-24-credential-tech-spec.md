---
name: credential tech spec (implementation-ready design)
status: Checkpoint 1 — §0–§3 written. §4–§16 follow in Checkpoints 2a/2b/3/4.
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

### §2.1 `Credential` trait

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
                slot_type: SlotType::CapabilityBound {
                    capability: Capability::Bearer,
                },
                resolve_fn: resolve_bearer_slot,
            },
        ];
        SLOTS
    }
}
```

Engine uses the slot metadata to drive resolution — it knows each slot's capability requirement.

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

**Cascade revocation.** Revoking a "parent" credential (e.g., a long-lived OAuth2 refresh token) cascades to dependent tokens (the access tokens it minted). Detection: `credential_audit` foreign-key relationships + per-type `Credential::dependent_credentials()` hook (default empty). Rare — only credentials with explicit dependency relationships.

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

---

**Checkpoint 2a ends here.** §6 Security + §7 Operational + §8 Testing follow in Checkpoint 2b. §9–§16 in Checkpoints 3 + 4.
