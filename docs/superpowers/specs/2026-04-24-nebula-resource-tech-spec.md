---
name: nebula-resource tech spec (implementation-ready design)
status: FROZEN (CP4) — ratified by architect + tech-lead 2026-04-25 (amended-in-place 2026-04-26 — cross-cascade R2 per §15.7 — `on_credential_refresh` signature re-pin to credential CP5 §15.7 `SchemeGuard<'a, _>` shape; ADR-0036 §Decision counterpart amendment-in-place per §15.7.5)
date: 2026-04-25
authors: [architect (subagent dispatch)]
scope: nebula-resource — single-crate redesign; 5 in-tree consumers migrate atomically
cascade_phase: Phase 6 complete (all 4 CPs ratified)
strategy: docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md (FROZEN CP3)
adrs:
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md (accepted)
  - docs/adr/0037-daemon-eventsource-engine-fold.md (accepted, amended-in-place 2026-04-25)
spike: docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/
credential_cross_ref:
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §3.6 (lines 928-996)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §Credential::revoke (line 228)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §4.3 revocation modes (lines 1062-1068)
related:
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md
  - docs/PRODUCT_CANON.md §3.5 §4.5 §12.5 §12.7
---

# nebula-resource Tech Spec (implementation-ready design)

## §0 — Status, scope, freeze policy

### §0.1 Status

**Checkpoint 1 draft** — sections §0, §1, §2, §3 complete; §4-§16 follow in CP2 (§4-§8), CP3 (§9-§13), and CP4 (§14-§16) per the cadence locked in [`03-scope-decision.md` §6, line 169](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md).

CP1 is the foundation: trait contract + runtime model with full Rust signatures. CP1 ratification by tech-lead flips both [ADR-0036](../adr/0036-resource-credential-adoption-auth-retirement.md) and [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md) from `proposed` to `accepted` per their respective acceptance gates. Until CP1 ratifies, both ADRs remain `proposed`.

This Tech Spec is **implementation-normative**. Where the [Strategy Document](2026-04-24-nebula-resource-redesign-strategy.md) §4 establishes binding decisions and the ADRs record the architectural rationale, this Tech Spec provides the compile-able Rust shapes, runtime invariants, storage paths, and migration mechanics that engineers consume directly.

### §0.2 Scope (cascade gate-pass record)

This Tech Spec carries the gate-pass record for the nebula-resource redesign cascade. Phase order:

- **Phase 1** — pain enumeration. 28 findings, 6 🔴-class. [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md).
- **Phase 2** — scope decision. Co-decision body (architect + tech-lead + security-lead) locked Option B in round 1 of max-3. [`03-scope-decision.md`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md).
- **Phase 3** — Strategy document (3 checkpoints, frozen 2026-04-24). [Strategy](2026-04-24-nebula-resource-redesign-strategy.md).
- **Phase 4** — Phase 4 spike. PASSED iter-1; all 7 exit criteria met; iter-2 deferred. [`spike/NOTES.md`](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md).
- **Phase 5** — ADR authoring. ADR-0036 (primary trait reshape), ADR-0037 (Daemon/EventSource extraction).
- **Phase 6** — this Tech Spec.

Strategy §4 decisions are the binding contract. CP1 §2-§3 elaborate Strategy §4.1 (trait reshape), §4.2 (revocation), §4.3 (rotation dispatch) into compile-able Rust. CP1 does not re-litigate Strategy decisions; the freeze policy in [Strategy §0](2026-04-24-nebula-resource-redesign-strategy.md) governs.

### §0.3 Freeze policy (per-CP)

**Each checkpoint freezes after review**. CP1 freezes after spec-auditor + rust-senior + tech-lead sign-off. After freeze:

- **Bug fixes** (typo, broken intra-doc link, clarification that does not change semantics) land via "docs(spec)" PR. Reviewer: spec-auditor.
- **Semantic amendments** (Rust signature changes, invariant shifts, decision reversal) require a co-review cycle — architect drafts amendment rationale, tech-lead ratifies, plus relevant specialty reviewer (rust-senior for trait, security-lead for revocation).
- **Strategy §4 supersession** — if a CP discovers Strategy §4 ambiguity, the Tech Spec records the extension with explicit "extends Strategy §X.Y" annotation. If the extension changes Strategy semantics, the Strategy amendment cycle from [Strategy §0](2026-04-24-nebula-resource-redesign-strategy.md) gates first — Tech Spec cannot supersede Strategy in-place.
- **ADR amendments** — if a CP discovers an ADR-0036 / ADR-0037 decision needs adjustment, the ADR's "amended in place on" section records the delta (per ADR-0035 amended-in-place pattern). Material structural changes require new ADR.

Strategy authority supersedes Tech Spec on conflict; Tech Spec wins over sub-spec and implementation plans. ADRs are point-in-time records of the rationale at acceptance; future code may diverge if the ADR is amended or superseded.

### §0.4 Reading order

§0 → §1 (goals + non-goals) → §2 (trait contract) → §3 (runtime model) → §4 (lifecycle, CP2) → §5 (implementation specifics, CP2) → §6 (operational/observability, CP2) → §7 (testing strategy, CP2) → §8 (storage/state, CP2) → §9 (Manager file split function-level cuts, CP3) → §10 (Public API surface, CP3) → §11 (Adapter authoring contract, CP3) → §12 (Daemon/EventSource engine landing, CP3) → §13 (Evolution policy, CP3) → §14-§16 (meta + open items + handoff, CP4).

CP1 readers: load Strategy §4, ADR-0036 §Decision, ADR-0037 §Decision, [credential Tech Spec §3.6 lines 928-996](2026-04-24-credential-tech-spec.md), and [`spike/NOTES.md`](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md) before reading §2-§3 of this document. Citations are dense; load order matters.

CP3 readers: load CP1 §2 + §3 (trait + runtime contract — §9 enumerates them at function level), CP2 §5.4 (file-split structure — §9 takes the seven submodules and assigns functions), CP2 §5.6 (deferrals — §10.5 confirms `tainting_policy` does NOT enter CP3 surface), [ADR-0037 amended-in-place gate text](../adr/0037-daemon-eventsource-engine-fold.md) (§12 specifies the engine landing site), and the current [`crates/resource/docs/adapters.md`](../../../crates/resource/docs/adapters.md) (§11 is the rewrite content spec).

## §1 — Goals and non-goals

### §1.1 Primary goals

- **Replace `Resource::Auth` with `Resource::Credential`.** Adopt [credential Tech Spec §3.6 lines 935-955](2026-04-24-credential-tech-spec.md) shape verbatim per [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md). `type Credential: Credential` becomes the trait-level credential binding; `<Self::Credential as Credential>::Scheme` flows into `create` and the new rotation hooks. `type Auth: AuthScheme` is removed (no shim, no alias) per `feedback_no_shims.md` discipline encoded in [Strategy §2.4](2026-04-24-nebula-resource-redesign-strategy.md).
- **Eliminate the silent revocation drop.** Resolve [Phase 1 finding 🔴-1](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md): `Manager::on_credential_refreshed` at [`crates/resource/src/manager.rs:1360`](../../../crates/resource/src/manager.rs) and `on_credential_revoked` at [`:1386`](../../../crates/resource/src/manager.rs) terminate in `todo!()` over an empty reverse-index. The reverse-index write path lands atomically with the dispatchers in this spec.
- **Per-resource rotation hooks with isolation.** New trait methods `on_credential_refresh` + `on_credential_revoke` with default no-op bodies. Manager dispatches via parallel `join_all` with per-resource timeout enforcement per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) + [security amendment B-1](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). One slow / failing resource cannot block siblings.
- **Atomic landing.** Trait reshape, reverse-index write path, dispatcher implementation, observability (trace + counter + event), 5-consumer migration, doc rewrite, Daemon/EventSource extraction land in one PR wave per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md).

### §1.2 Secondary goals

- **Daemon and EventSource extraction.** Per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md), Daemon and EventSource topologies leave `nebula-resource` and fold into the engine layer. The `TopologyRuntime<R>` enum at [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) shrinks 7 → 5 variants. Pool / Resident / Service / Transport / Exclusive remain. Engine-side landing site (module layout, primitive naming, `EventSource → TriggerAction` adapter signature) is CP3 §13 deliverable.
- **`manager.rs` file-split.** [Strategy §4.5](2026-04-24-nebula-resource-redesign-strategy.md) keeps the `Manager` type monolithic and splits the 2101-line file into submodules. CP2 §5.4 finalises seven cut points: `mod.rs`, `options.rs`, `registration.rs`, `gate.rs`, `execute.rs`, `rotation.rs`, `shutdown.rs` — extending Strategy §4.5's five-submodule proposal with `registration.rs` (concrete site for the 🔴-1 reverse-index write fix per §3.1) and `shutdown.rs` (concrete site for the 🔴-4 `set_phase_all_failed` fix per §5.5). The two additions are concrete cuts that emerged when laying out the lifecycle, not a Strategy supersession; the Strategy §4.5 proposal did not foreclose extension. Public API does not change.
- **Drain-abort fix bundled.** Wire `ManagedResource::set_failed()` into the `DrainTimeoutPolicy::Abort` path per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md). Resolves [Phase 1 finding 🔴-4](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md). Lands atomically with the file-split PR.
- **Documentation rebuild.** Per [Strategy §4.7](2026-04-24-nebula-resource-redesign-strategy.md), all `crates/resource/docs/*` rewritten against the new shape. CP3 §13 enumerates per-file change list.

### §1.3 Non-goals (deferred per Strategy §5)

- **`AuthenticatedResource: Resource` sub-trait.** Explicitly rejected at [Phase 2](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) and [Strategy §2.4](2026-04-24-nebula-resource-redesign-strategy.md). `type Credential = NoCredential;` is the idiomatic opt-out.
- **`Runtime`/`Lease` collapse.** [Phase 1 finding 🟠-11](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — 9/9 test resources set `Lease = Runtime`. Deferred per [Strategy §5.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **`AcquireOptions::intent/.tags` wiring.** Engine integration ticket #391 drives the shape. Deferred per [Strategy §5.2](2026-04-24-nebula-resource-redesign-strategy.md); CP2 §5 picks interim posture (`#[doc(hidden)]` vs `#[deprecated]`).
- **Service/Transport merge.** [Phase 1 🟡-22](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md). `feedback_boundary_erosion.md` applies; do not merge without strong evidence.
- **`FuturesUnordered` fan-out cap.** [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) explicitly defers; current N (resources sharing one credential) is small. Future optimization if operational signal demands it.
- **L2 cross-replica rotation coordination.** Belongs to credential cascade per `draft-f17`. Resource-side `on_credential_refresh` is a consumer of L1-coordinated rotations.

### §1.4 Success criteria

CP1 ratification requires:

- All five Phase 4 spike-surfaced open questions resolved with rationale (§2.5).
- Full Rust signature for the `Resource` trait in §2.1 — every method, every associated type, every default body.
- `NoCredential` location decided and the concrete `Credential` impl signed off (§2.2).
- The five topology sub-traits (Pool / Resident / Service / Transport / Exclusive) elaborated with their inherited `type Credential` and any topology-specific shape touched by the rotation contract (§2.4).
- Manager runtime model in §3 with the reverse-index write path (Phase 1 🔴-1 resolved), parallel dispatcher, per-resource timeout configuration, and failure aggregation, all with concrete signatures.
- Phase 1 🔴-1 (silent revocation drop) and 🔴-4 (drain-abort phase corruption) explicitly resolved with file:line cross-references (§3.6).
- Every claim in §2-§3 traceable to: spike file, current code, Strategy §, ADR §, or credential Tech Spec line. No invention.

CP2 / CP3 / CP4 success criteria are recorded in their own checkpoint scope when those drafts open.

## §2 — Trait contract

Full Rust signatures for the reshaped `Resource` trait, the `NoCredential` opt-out, the rotation hooks with their default bodies and invariants, the five topology sub-traits, and the concrete resolutions of all five Phase 4 spike open questions. Section §2 is the binding compile-time contract; §3 is the runtime obligation that consumes it.

### §2.1 Resource trait (full Rust signature)

The reshaped `Resource` trait. Replaces the current trait at [`crates/resource/src/resource.rs:220-298`](../../../crates/resource/src/resource.rs). Imports per spike `resource.rs:17-19` (production version uses `nebula_credential::Credential` + `nebula_credential::CredentialId`).

```rust
use std::future::Future;

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialId, NoCredential};

use crate::context::ResourceContext;

/// Core resource trait — 5 associated types + 6 lifecycle methods.
///
/// Uses return-position `impl Future` (RPITIT) — no `async_trait`, no
/// `Box<dyn Future>` allocation on the hot path. Default-no-op rotation
/// hooks use `async fn` shorthand on the trait body to keep the default
/// readable; impl sites SHOULD use `async fn` per spike NOTES.md clippy
/// finding (`manual_async_fn` discourages `fn ... -> impl Future`
/// matching the trait declaration verbatim — see §2.1.1 for the
/// idiomatic impl form).
///
/// # Associated types
///
/// | Type        | Purpose                                                     |
/// |-------------|-------------------------------------------------------------|
/// | `Config`    | Operational config (no secrets) — `HasSchema` super-trait   |
/// | `Runtime`   | The live resource handle (connection, client, etc.)         |
/// | `Lease`     | What callers hold while using the resource                  |
/// | `Error`     | Resource-specific error type                                |
/// | `Credential`| What the engine binds — opt out via `= NoCredential`        |
///
/// # Lifecycle (post-redesign)
///
/// ```text
/// create(scheme) → Runtime
///   ↓
/// check()  → Ok | Err
///   ↓
/// on_credential_refresh(new_scheme) → Ok | Err   (default no-op)
/// on_credential_revoke(credential_id) → Ok | Err (default no-op)
///   ↓
/// shutdown() → graceful wind-down
///   ↓
/// destroy()  → final cleanup (consumes Runtime)
/// ```
pub trait Resource: Send + Sync + 'static {
    /// Operational configuration type (no secrets).
    type Config: ResourceConfig;
    /// The live resource handle.
    type Runtime: Send + Sync + 'static;
    /// What callers hold during use.
    type Lease: Send + Sync + 'static;
    /// Resource-specific error type.
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    /// What this resource binds to.
    ///
    /// Use [`NoCredential`] to opt out (`type Credential = NoCredential;`).
    /// Otherwise pick a real `Credential` impl. The projected scheme
    /// `<Self::Credential as Credential>::Scheme` is what `create` and
    /// `on_credential_refresh` receive from the credential resolver.
    ///
    /// Replaces `type Auth: AuthScheme` per Strategy §4.1 + ADR-0036.
    type Credential: Credential;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new runtime instance from config and resolved scheme.
    ///
    /// `scheme` is borrowed from the credential resolver — implementations
    /// MUST NOT clone the scheme onto the runtime per Strategy §4.3
    /// hot-path invariant (each clone is another zeroize obligation).
    /// Pull what is needed (token, connection string), let the borrow end.
    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Health-checks an existing runtime.
    ///
    /// Default: always succeeds. Connection-bound resources should
    /// override with a fast read-only probe (e.g., `SELECT 1`).
    fn check(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Rotation hook — called when the engine detects credential refresh.
    ///
    /// **Cross-cascade re-pin per credential CP5 §15.7 supersession:**
    /// `new_scheme` is `SchemeGuard<'a, Self::Credential>` (owned, `!Clone`,
    /// `ZeroizeOnDrop`, `Deref<Target = Scheme>`, lifetime-bound to call)
    /// per [credential Tech Spec §15.7 line 3394-3429](2026-04-24-credential-tech-spec.md)
    /// canonical CP5 form, NOT borrowed `&Scheme` per superseded §3.6 shape.
    /// `ctx: &'a CredentialContext<'a>` shares the `'a` lifetime per
    /// credential Tech Spec §15.7 iter-3 lifetime-pin refinement
    /// (line 3503-3516). The owned guard enforces the zeroize / no-Clone /
    /// no-retention discipline at the type level — impls cannot store the
    /// scheme beyond the call (compile-fail probe coverage per credential
    /// Tech Spec §16.1.1 probes #6, #7).
    ///
    /// Default: no-op. Connection-bound resources (Postgres pool, Kafka
    /// producer) override with the **blue-green pool swap** pattern from
    /// credential Tech Spec §3.6 lines 961-993: `Arc<RwLock<Pool>>` +
    /// write-lock swap inside the impl. `Manager` does NOT orchestrate
    /// the swap.
    ///
    /// # Invariant — per-resource isolation
    ///
    /// Manager dispatches via parallel `join_all` with per-resource
    /// timeout enforcement (§3.2). One resource's slow / failed refresh
    /// MUST NOT block sibling dispatches. This hook is bounded by the
    /// per-resource budget configured at register time (§3.3).
    ///
    /// # Error semantics
    ///
    /// Returning `Err(e)` causes Manager to emit
    /// `ResourceEvent::CredentialRefreshed { outcome: Failed(...) }`
    /// and a per-resource error tracing span. Sibling dispatches are
    /// unaffected (Strategy §4.3, security amendment B-1).
    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let _ = (new_scheme, ctx);
        async { Ok(()) }
    }

    /// Revocation hook — called when the engine signals credential revocation.
    ///
    /// Default: no-op. Override invariant per Strategy §4.2:
    /// **post-invocation, the resource emits no further authenticated
    /// traffic on the revoked credential.**
    ///
    /// The mechanism (destroy pool / mark tainted / wait-for-drain /
    /// reject new acquires) is impl-defined. Connection-bound resources
    /// SHOULD destroy or taint pool instances synchronously inside the
    /// hook; resources holding short-lived per-request schemes MAY rely
    /// on the next `create` call rejecting (the new scheme will not be
    /// available — `<Self::Credential as Credential>::Scheme` cannot
    /// be projected from a `revoked` `state_kind` per credential Tech
    /// Spec §4.3 lines 1062-1068).
    ///
    /// # Default-body rationale
    ///
    /// The default body returns `Ok(())` — it does NOT enforce the
    /// invariant for the implementer. The invariant is a contractual
    /// obligation on overriding implementations. Manager has no way to
    /// verify the invariant at runtime; it relies on the impl honoring
    /// it. CP2 §6 (security) records the audit/test gates that catch
    /// invariant violations.
    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }

    /// Gracefully winds down a runtime.
    fn shutdown(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Final cleanup — consumes the runtime.
    fn destroy(
        &self,
        runtime: Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = runtime;
        async { Ok(()) }
    }

    /// Returns the schema for this resource's configuration.
    fn schema() -> nebula_schema::ValidSchema
    where
        Self: Sized,
    {
        <Self::Config as nebula_schema::HasSchema>::schema()
    }

    /// Returns metadata for UI and diagnostics.
    fn metadata() -> ResourceMetadata
    where
        Self: Sized,
    {
        ResourceMetadata::for_resource::<Self>(
            Self::key(),
            Self::key().to_string(),
            String::new(),
        )
    }
}
```

#### §2.1.1 Idiomatic impl form

Trait declarations use `impl Future<...> + Send`. Impl sites SHOULD use `async fn` shorthand per spike NOTES.md clippy finding — clippy's `manual_async_fn` (1.95 default `clippy::all`) prefers the desugared form. Both forms are interchangeable from the trait perspective; only the impl side has the lint preference.

```rust
impl Resource for PostgresPool {
    // ... associated types ...

    async fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> { /* ... */ }

    async fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext<'a>,
    ) -> Result<(), Self::Error> {
        // `new_scheme` derefs to `&Scheme` per credential Tech Spec §15.7
        // line 3394-3429 `Deref<Target = Scheme>` impl. Pull what is
        // needed inside the await window; do NOT clone the scheme onto
        // the new pool (clone would be another zeroize obligation per
        // PRODUCT_CANON §12.5 + credential Tech Spec §15.7 `!Clone`
        // discipline).
        let new_pool = build_pool_from_scheme(&*new_scheme).await?;
        let mut guard = self.inner.write().await;
        *guard = new_pool;
        // `new_scheme` zeroizes on Drop at end of scope (per credential
        // Tech Spec §15.7 line 3412 Drop ordering); `ctx` is borrowed.
        Ok(())
    }
}
```

The blue-green swap example mirrors [credential Tech Spec §3.6 lines 981-993](2026-04-24-credential-tech-spec.md). Old connections drain naturally as their RAII guards drop; new queries use the new pool (read lock acquires against the new inner after swap). The `SchemeGuard<'a, _>` parameter (per [credential Tech Spec §15.7 line 3394-3429](2026-04-24-credential-tech-spec.md) canonical CP5 form) zeroizes on Drop at the end of the impl body's await window; the resource impl pulls what it needs (connection string, token) via `Deref` without cloning the scheme.

### §2.2 NoCredential type (location + impl)

`NoCredential` lives in **`nebula-credential`** (resolution to spike Q1; full rationale in §2.5). It IS a `Credential` impl, so structurally it belongs next to the trait. Resource-side imports it as `nebula_credential::NoCredential`.

Concrete implementation, adapted from spike `no_credential.rs:36-95`:

```rust
// Inside crates/credential/src/no_credential.rs (NEW FILE).

use serde::{Deserialize, Serialize};

use crate::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialError,
    CredentialMetadata, CredentialState, NoPendingState, ResolveResult,
    credential_key,
};

/// Zero-sized scheme that asserts "no authentication".
///
/// Pattern is [`AuthPattern::NoAuth`]. Mirrors the existing `()` impl
/// on `AuthScheme` but as a real named type so it composes with
/// [`Credential`]'s `type Scheme: AuthScheme` bound.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct NoScheme;

impl AuthScheme for NoScheme {
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

impl CredentialState for NoScheme {
    const KIND: &'static str = "no_scheme";
    const VERSION: u32 = 1;
}

/// `Credential` opt-out marker.
///
/// `type Credential = NoCredential;` declares that a `Resource` does
/// not bind to a credential. The reshaped `Manager` (see
/// [`nebula_resource::Manager`]) special-cases this at the type level:
/// registering an `R` whose `Credential = NoCredential` does NOT
/// populate the credential reverse index, and `on_credential_refreshed`
/// / `on_credential_revoked` will never reach this resource.
pub struct NoCredential;

impl Credential for NoCredential {
    type Input = ();
    type Scheme = NoScheme;
    type State = NoScheme;
    type Pending = NoPendingState;

    const KEY: &'static str = "no_credential";

    fn metadata() -> CredentialMetadata
    where
        Self: Sized,
    {
        CredentialMetadata::builder()
            .key(credential_key!("no_credential"))
            .name("No credential")
            .description("Opt-out marker for resources without an authenticated binding.")
            .schema(<() as nebula_schema::HasSchema>::schema())
            .pattern(AuthPattern::NoAuth)
            .build()
            .expect("static metadata fields are all set above")
    }

    fn project(_state: &NoScheme) -> NoScheme {
        NoScheme
    }

    async fn resolve(
        _values: &nebula_schema::FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<NoScheme, NoPendingState>, CredentialError> {
        // Returning `Complete(NoScheme)` keeps the trait obligations
        // honest. In practice the dispatcher never reaches this method
        // for a `NoCredential`-typed resource — Manager skips the
        // reverse-index write at register time (see §3.1).
        Ok(ResolveResult::Complete(NoScheme))
    }
}
```

**Re-export at the resource crate root.** `crates/resource/src/lib.rs` re-exports for ergonomics:

```rust
pub use nebula_credential::{NoCredential, NoScheme};
```

Consumers writing `type Credential = NoCredential;` import from either crate; the canonical home is `nebula-credential`.

**Registration safety.** Registering `NoCredential` against a credential store is a nonsense operation but doesn't crash — `metadata()` returns a structurally-honest tombstone. Manager's `register` emits `tracing::warn!` when called with `type Credential = NoCredential` paired with `Some(real_credential_id)` and discards the id without populating the reverse-index per §3.1.

### §2.3 on_credential_refresh + on_credential_revoke (signatures + default bodies + invariants)

Both signatures and default bodies are inlined in §2.1. This subsection records the **invariants** each method must honor.

**`on_credential_refresh(new_scheme, ctx)`:**

- **Owned-guard invariant.** `new_scheme: SchemeGuard<'a, Self::Credential>` — owned guard, `!Clone`, `ZeroizeOnDrop`, `Deref<Target = Scheme>`, lifetime-bound to call. Per [credential Tech Spec §15.7 line 3394-3429](2026-04-24-credential-tech-spec.md) canonical CP5 form (re-pinned from superseded §3.6 borrowed `&Scheme` shape per cross-cascade R2 enactment §15.7 below). The `'a` lifetime is shared with `ctx: &'a CredentialContext<'a>` per credential Tech Spec §15.7 iter-3 lifetime-pin refinement (line 3503-3516). The owned guard enforces zeroize / no-Clone / no-retention discipline at the type level — impls cannot store the scheme beyond the call. Per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) hot-path invariant: no `Scheme::clone()` on the dispatcher path; each clone is another zeroize obligation per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md). Resource impls pull what they need via `Deref` (`&*new_scheme`) inside the await window; the guard zeroizes on Drop at end of scope. Compile-fail probe coverage per credential Tech Spec §16.1.1 probes #6 (SchemeGuard retention in resource struct field), #7 (SchemeGuard clone attempt).
- **Per-resource isolation invariant.** Manager (§3.2) bounds each invocation by a per-resource timeout; sibling dispatches run in parallel via `join_all`. A slow or failed refresh on this resource MUST NOT block siblings. The default no-op body satisfies this trivially.
- **Idempotency expectation.** Manager MAY retry under specific recovery flows (CP2 §6 finalizes); impls SHOULD treat repeated `on_credential_refresh(same_scheme)` as a no-op after the first successful call. Default body satisfies; pool-swap impls are naturally idempotent because the second swap re-publishes the same pool.

**`on_credential_revoke(credential_id)`:**

- **Post-invocation invariant.** Per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md): "post-invocation, the resource emits no further authenticated traffic on the revoked credential." This is a contractual obligation on overriding impls; Manager cannot verify at runtime. CP2 §6 specifies the audit gate and test invariant for this.
- **Mechanism is impl-defined.** Strategy commits to the *invariant*, not the *implementation*. Candidates per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md): destroy pool instances; mark instances tainted with `RAII-on-release` cleanup; wait-for-drain; reject new acquires. Default body returns `Ok(())` and does not enforce the invariant — opt-out only by impls that genuinely emit no authenticated traffic (e.g., `NoCredential`-typed resources, unauthenticated caches).
- **Argument is `&CredentialId`, not `&Scheme`.** Revocation has no new scheme to swap in; the credential id is sufficient signal. Resource impls that need to look up internal per-credential state (e.g., a multi-tenant pool keyed by credential) do so via the id.
- **Failure semantics.** Manager emits `ResourceEvent::CredentialRevoked { outcome: Failed(...) }` per failed dispatch; Strategy §4.2 also requires `HealthChanged { healthy: false }` per security amendment B-2. CP2 §7 finalizes the event cardinality.

**Why two methods, not one combined `on_credential_event(EventKind)`.** Per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md) rationale: "two semantically distinct events deserve two methods." Refresh has a new scheme; revoke does not. Dual-semantics on one method would force every implementer to branch on "is this a refresh or a revoke?" via the scheme reference, which is awkward and error-prone.

### §2.4 Topology sub-traits (Pool/Resident/Service/Transport/Exclusive)

Five topology sub-traits remain in `nebula-resource` post-extraction. Daemon and EventSource leave the crate per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md). Each sub-trait extends `Resource` and inherits the new `type Credential` + rotation hooks; topology-specific methods are unchanged from current shape at [`crates/resource/src/topology/{pooled,resident,service,transport,exclusive}.rs`](../../../crates/resource/src/topology/).

The trait declarations themselves do not change shape — only the parent `Resource` trait reshape propagates through. Sub-traits stay in their existing files post-`manager.rs` file-split.

```rust
// crates/resource/src/topology/pooled.rs — UNCHANGED structure.
pub trait Pooled: Resource {
    fn is_broken(&self, _runtime: &Self::Runtime) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    fn recycle(
        &self,
        _runtime: &Self::Runtime,
        _metrics: &InstanceMetrics,
    ) -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send {
        async { Ok(RecycleDecision::Keep) }
    }

    fn prepare(
        &self,
        _runtime: &Self::Runtime,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

// crates/resource/src/topology/resident.rs
pub trait Resident: Resource where Self::Lease: Clone {
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }
    // ... (unchanged from current)
}

// crates/resource/src/topology/service.rs
pub trait Service: Resource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    fn acquire_token(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    fn release_token(
        &self,
        _runtime: &Self::Runtime,
        _token: Self::Lease,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

// crates/resource/src/topology/transport.rs
pub trait Transport: Resource {
    fn open_session(
        &self,
        transport: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    // ... + close_session etc. (unchanged from current)
}

// crates/resource/src/topology/exclusive.rs
pub trait Exclusive: Resource {
    fn reset(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}
```

**`TopologyRuntime<R>` enum shrink.** Per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md), the enum at [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) loses its `Daemon` and `EventSource` variants. Engine-side landing site (module layout, primitive naming, EventSource→TriggerAction adapter signature) is **CP3 §13 deliverable**, not CP1 — CP1 records the enum shrink as a contract, not the engine-side shape.

Spike validated cross-topology composition end-to-end: `MockPostgresPool: Pooled` (credential-bearing) and `MockKafkaTransport: Transport` (credential-bearing) bound to the same `CredentialId` both received the rotation hook in `parallel_dispatch_crosses_topology_variants` test ([`spike/.../resource-shape-test/src/lib.rs:583-607`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)). The dispatcher's type erasure works across topology variants.

### §2.5 Open question resolutions from spike

Five open questions surfaced in [`spike/NOTES.md` lines 155-202](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md). CP1 resolves each with rationale; no question deferred to CP2-CP4.

#### Q1 — `NoCredential` location: **`nebula-credential` crate**

`NoCredential` IS a `Credential` impl. Structurally it belongs next to the trait it satisfies. Three reasons:

- **Layering honesty.** Spike NOTES.md flags this as "structurally honest" — the type's semantics are credential-side, not resource-side. Placing it in `nebula-credential` mirrors how `NoPendingState` already lives there.
- **Reusability beyond resource.** Any future crate consuming `Credential` (e.g., `nebula-action` for `CredentialRef<dyn ...>` defaulting) can import `NoCredential` without taking a transitive dep on `nebula-resource`.
- **No coupling penalty.** `nebula-resource` already depends on `nebula-credential` ([`Cargo.toml`](../../../crates/resource/Cargo.toml)); `use nebula_credential::NoCredential` is a one-line import. `nebula-resource::lib.rs` re-exports for ergonomic `use nebula_resource::NoCredential` per §2.2.

**Trade-off accepted:** `nebula-credential` gains a tombstone-shaped type whose practical use case is solely resource-side opt-out. Mitigated by the type also serving `CredentialRef<dyn ...>` defaulting in actions (future cascade per [Strategy §6.5](2026-04-24-nebula-resource-redesign-strategy.md)).

#### Q2 — `TypeId` vs sealed-trait marker for opt-out detection: **`TypeId`**

Manager's `register::<R>` decides whether to populate the reverse-index by checking `TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>()` (spike `manager.rs:227`). Three reasons to keep this over a sealed-trait marker:

- **One-line check, called once per registration.** Not in the hot path. Production-relevant overhead is zero.
- **Sealed-trait marker forces NoCredential to live with the marker.** A sealed `IsCredentialBearing` (negated for `NoCredential`) trait would have to live in `nebula-credential` (because `NoCredential` does), constraining `nebula-resource::Manager` to consume yet another trait re-export. `TypeId` is host-crate-agnostic.
- **Static dispatch affordance is not load-bearing.** Sealed-trait would let the compiler optimize the branch, but the branch fires once per `register` call (not in `acquire`, not on rotation). Optimization benefit is unmeasurable at this cadence.

**Compile-fail probe added in §2.5 Q5:** the negative case (a non-`Credential` type bound) is caught by the `type Credential: Credential` bound at impl time — verified by spike compile-fail probe `_credential_bound_enforced_must_fail` ([`spike/.../resource-shape-test/src/compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).

#### Q3 — `Box::pin` per dispatch overhead: **acknowledge in observability; no extra metric**

Type erasure requires `Box<dyn ResourceDispatcher>`; trampoline returns `Pin<Box<dyn Future<...> + Send>>` because RPITIT in trait objects is not stable on 1.95 (spike NOTES.md "this almost didn't work" #1). Cost is **once per refresh dispatch per registered resource**, not per acquire.

- **Observability.** `nebula_resource.credential_rotation_dispatch_latency_seconds` histogram per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md) covers the `Box::pin` cost alongside hook execution time. No separate metric — the allocation is statistically dominated by hook execution.
- **Soft cap of 32 concurrent hooks** per credential (tech-lead Phase 2 number) becomes the default `ManagerConfig::credential_rotation_concurrency`. Tunable. N ≤ 32 runs unbounded `join_all`; N > 32 fan-out via `FuturesUnordered` deferred per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **Trait docs note the cost transparently.** CP4 §15 records "future-cleanup-when-stable" once RPITIT in trait objects stabilizes.

#### Q4 — Per-resource timeout config surface: **per-Manager default + per-Resource override via `RegisterOptions`**

Both. Default lives on `ManagerConfig::credential_rotation_timeout` (uniform default; defaults to 30 seconds — value chosen to accommodate slow blue-green pool builds while still bounding misbehaving impls). Per-resource override lives on `RegisterOptions::credential_rotation_timeout: Option<Duration>`, which when `Some(d)` overrides the Manager default for that registration.

Three reasons:

- **`RegisterOptions` already carries per-resource concerns.** [`crates/resource/src/manager.rs:220-227`](../../../crates/resource/src/manager.rs) already has `scope`, `resilience`, `recovery_gate` on `RegisterOptions`. Adding `credential_rotation_timeout` matches the existing per-resource configuration surface.
- **Operational uniformity AND precision.** Operators set a sensible default at `ManagerConfig` time; override per-resource only when a specific resource has non-uniform requirements (e.g., a remote pool with high handshake latency).
- **Spike validated the parameter shape.** Spike's `Manager::with_timeout(Duration)` ([`spike/.../resource-shape/src/manager.rs:200-205`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape/src/manager.rs)) is the per-Manager default; production extends it to per-resource via `RegisterOptions`. No structural change from spike.

Full signature in §3.3.

#### Q5 — Compile-fail probe production gaps: **trait probes carry forward; runtime gaps go in §3 invariants**

Spike's three compile-fail probes belong to the trait contract; spike NOTES.md flagged two production-relevant gaps that are runtime invariants, not trait probes:

- **Double-registration of same `ResourceKey`.** Manager-runtime invariant, not trait contract. Current code at [`crates/resource/src/manager.rs:329`](../../../crates/resource/src/manager.rs) silently replaces; semantics preserved unchanged (CP2 §5 may revisit). Runtime test added in CP2 §8.
- **NoCredential resource bound to `Some(real_id)`.** Runtime invariant per §3.1: emit `tracing::warn!`, drop the id, register the resource without reverse-index entry. Rationale: configuration mistake, not structural — warn, do not reject. Warn-emission test in CP2 §8.

**CP1 commits four trait probes:**

1. Wrong-signature `on_credential_refresh` override rejected — `_wrong_refresh_signature_must_fail` ([`spike/.../compile_fail.rs:11-95`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)) carries forward.
2. `NoScheme` cannot pretend to be `SecretToken` — `_no_credential_scheme_is_inert_must_fail` ([`spike/.../compile_fail.rs:96-115`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)) carries forward.
3. Non-`Credential` type rejected — `_credential_bound_enforced_must_fail` ([`spike/.../compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)) carries forward.
4. **NEW** for production: wrong-signature `on_credential_revoke` override rejected — symmetric to probe 1.

CP2 §8 elaborates the full test plan.

## §3 — Runtime model

Manager-side runtime contract that consumes the §2 trait shape. Replaces the broken dispatchers at [`crates/resource/src/manager.rs:1360-1401`](../../../crates/resource/src/manager.rs) (the `todo!()` panic over an empty reverse-index — Phase 1 finding 🔴-1). Spike validated the shape with 6 integration tests ([`spike/.../resource-shape-test/src/lib.rs:431-647`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)).

### §3.1 Manager registration with reverse-index write path

`Manager::register` and the five `register_*` topology helpers (`register_pooled`, `register_resident`, `register_service`, `register_transport`, `register_exclusive`) all funnel through the same internal write path, which inspects `R::Credential` to decide reverse-index population.

```rust
// Inside crates/resource/src/manager/registration.rs (post-split per Strategy §4.5).

use std::any::TypeId;
use std::sync::Arc;

use nebula_credential::{Credential, CredentialId, NoCredential};
use crate::{
    manager::Manager,
    resource::Resource,
    runtime::ManagedResource,
};

impl Manager {
    /// Internal funnel — every `register*` path ends here.
    ///
    /// Reverse-index write contract:
    /// - If `R::Credential = NoCredential` → reverse-index NOT written;
    ///   `credential_id` (if `Some`) is logged via `tracing::warn!` and
    ///   discarded (configuration mistake, not error — see Q5).
    /// - Else if `credential_id == None` → return
    ///   `Err(Error::missing_credential_id(R::key()))` — credential-bearing
    ///   resources MUST register against a specific stored credential.
    /// - Else → wrap `R` in a typed dispatcher (§3.2) and append to
    ///   `credential_resources[id]`.
    ///
    /// This method is the explicit write path Phase 1 found missing
    /// (`manager.rs:262, 370` — the field exists but no code wrote to it).
    fn register_inner<R: Resource>(
        &self,
        managed: Arc<ManagedResource<R>>,
        credential_id: Option<CredentialId>,
        options: &RegisterOptions,
    ) -> Result<(), Error> {
        let opted_out = TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>();

        match (opted_out, credential_id) {
            (true, Some(_id)) => {
                tracing::warn!(
                    resource_key = %R::key(),
                    "register: NoCredential resource bound to a credential id; \
                     the id is ignored — this is a configuration mistake but \
                     not a registration error",
                );
                // Fall through — no reverse-index write.
            }
            (true, None) => {
                // OK — NoCredential resources do not register against any id.
            }
            (false, None) => {
                return Err(Error::missing_credential_id(R::key()));
            }
            (false, Some(id)) => {
                let dispatcher: Arc<dyn ResourceDispatcher> = Arc::new(
                    TypedDispatcher {
                        managed: Arc::clone(&managed),
                        timeout_override: options.credential_rotation_timeout,
                    },
                );
                self.credential_resources
                    .entry(id)
                    .or_default()
                    .push(dispatcher);
            }
        }
        // Existing registry write at manager.rs:374 unchanged.
        Ok(())
    }
}
```

**Field type change.** [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs) currently declares `credential_resources: dashmap::DashMap<CredentialId, Vec<ResourceKey>>` (resource keys). This Tech Spec changes it to `dashmap::DashMap<CredentialId, Vec<Arc<dyn ResourceDispatcher>>>` — the dispatcher trampoline (§3.2) replaces the resource key. Bare resource keys cannot drive type-erased dispatch; the dispatcher carries the type information and the `Arc<R>` reference.

**Atomicity invariant.** Reverse-index write is part of the registration transaction. If `register_inner` returns `Err`, the registry write at [`manager.rs:374`](../../../crates/resource/src/manager.rs) does not happen (caller short-circuits before reaching it). If `register_inner` returns `Ok`, both writes complete before the `register_*` method returns. There is no observable interim state where the registry has the resource but the reverse-index does not.

**NEW error constructors** required on [`crates/resource/src/error.rs`](../../../crates/resource/src/error.rs) (called out explicitly so the implementer does not infer them silently):

- `Error::missing_credential_id(key: ResourceKey) -> Self` — used by `register_inner` (§3.1 line 641) when a credential-bearing resource (`type Credential != NoCredential`) registers with `credential_id == None`. Carries the resource key for diagnostics; surfaces a registration-time configuration error.
- `Error::scheme_type_mismatch::<R: Resource>() -> Self` — used by `TypedDispatcher::dispatch_refresh` (§3.2 line 743) when the runtime scheme `&(dyn Any + Send + Sync)` cannot downcast to `<R::Credential as Credential>::Scheme`. Generic constructor (parameterized by the registered resource type) so the error message can name the expected scheme type. Surfaces a dispatcher-wiring bug — should never fire in well-formed code, but surfaces clearly if it does.

Both are CP1 contract additions; the error variants land in `crates/resource/src/error.rs` atomically with the §3.1 / §3.2 implementations per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md). CP3 §13 enumerates the test coverage for each.

### §3.2 Rotation dispatcher (parallel join_all + per-resource isolation)

`Manager::on_credential_refreshed(id, scheme)` and `on_credential_revoked(id)` parallelize across all resources registered against `id` via `futures::future::join_all`. Each per-resource future runs in its own `tokio::time::timeout` bubble per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md). Spike NOTES.md confirmed isolation under both latency (3s sleep, 250ms budget) and errors (one failing, siblings Ok).

**Dispatcher trampoline trait** (replaces the reverse-index `Vec<ResourceKey>` value type per §3.1):

```rust
// Inside crates/resource/src/manager/rotation.rs (NEW).

use std::any::{Any, TypeId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use nebula_credential::{Credential, CredentialId};

/// Type-erased dispatcher for a single registered resource.
///
/// `dyn ResourceDispatcher` is the value type of the credential reverse-
/// index. RPITIT in trait objects is not stable on 1.95, so the trampoline
/// methods return `Pin<Box<dyn Future<...> + Send>>` — a `Box::pin`
/// allocation per dispatch (per-rotation, NOT per-acquire). See §2.5 Q3.
pub(crate) trait ResourceDispatcher: Send + Sync + 'static {
    /// Resource key (`R::key()`) — populates the per-resource event payload + tracing span field.
    fn resource_key(&self) -> ResourceKey;

    /// `TypeId` of the projected scheme. Used to fail-fast on dispatcher wiring mistakes.
    fn scheme_type_id(&self) -> TypeId;

    /// Per-resource timeout override captured at register time (§3.3).
    fn registered_timeout_override(&self) -> Option<Duration>;

    /// Calls `R::on_credential_refresh(scheme)` after downcasting. `&(dyn Any + Send + Sync)` (not
    /// `&dyn Any`) because the future must be `Send` for `join_all` on a multi-thread runtime
    /// (spike NOTES.md "needed an iteration").
    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

    /// Calls `R::on_credential_revoke(credential_id)`.
    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
}

/// Concrete dispatcher built from a typed `ManagedResource<R>`. Holds the
/// per-resource timeout override captured from `RegisterOptions` at
/// register time (§3.3).
struct TypedDispatcher<R: Resource> {
    managed: Arc<ManagedResource<R>>,
    timeout_override: Option<Duration>,
}

impl<R: Resource> ResourceDispatcher for TypedDispatcher<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn scheme_type_id(&self) -> TypeId {
        TypeId::of::<<R::Credential as Credential>::Scheme>()
    }

    fn registered_timeout_override(&self) -> Option<Duration> {
        self.timeout_override
    }

    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let scheme: &<R::Credential as Credential>::Scheme = scheme
                .downcast_ref()
                .ok_or_else(|| Error::scheme_type_mismatch::<R>())?;
            self.managed
                .resource
                .on_credential_refresh(scheme)
                .await
                .map_err(Into::into)
        })
    }

    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            self.managed
                .resource
                .on_credential_revoke(credential_id)
                .await
                .map_err(Into::into)
        })
    }
}
```

**Refresh dispatcher signature** (replaces the `todo!()` at [`manager.rs:1378`](../../../crates/resource/src/manager.rs)). Resolves Phase 1 finding 🔴-1.

```rust
impl Manager {
    pub async fn on_credential_refreshed(
        &self,
        credential_id: &CredentialId,
        scheme: &(dyn Any + Send + Sync),
    ) -> Result<Vec<(ResourceKey, RefreshOutcome)>, Error> {
        let dispatchers = self
            .credential_resources
            .get(credential_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        if dispatchers.is_empty() {
            return Ok(Vec::new());
        }

        let span = tracing::info_span!(
            "resource.credential_refresh",
            credential_id = %credential_id,
            resources_affected = dispatchers.len(),
        );

        // SAFETY (lifetime story for the dispatch trampoline):
        //   Each per-resource future captures its own `Arc<dyn ResourceDispatcher>`
        //   via the `Arc::clone(d)` move below; the inner `dispatch_refresh(scheme)`
        //   reborrows `&self` from that owned `Arc` for `'a`, and the same `&'a scheme`
        //   reference is shared by every per-resource future. `scheme` outlives
        //   `join_all` because `on_credential_refreshed` holds it for the entire
        //   scope. NO clone of `Scheme` per Strategy §4.3 hot-path invariant.
        //   Spike NOTES.md flagged this as the "iteration that needed Send-bound
        //   on `&dyn Any`" — `&(dyn Any + Send + Sync)` is load-bearing for the
        //   `Send` future on the multi-thread runtime.
        let futures = dispatchers.iter().map(|d| {
            let d = Arc::clone(d);
            let timeout = self.timeout_for(&d);
            let key = d.resource_key();
            async move {
                let outcome = match tokio::time::timeout(timeout, d.dispatch_refresh(scheme)).await {
                    Ok(Ok(())) => RefreshOutcome::Ok,
                    Ok(Err(e)) => RefreshOutcome::Failed(e),
                    Err(_) => RefreshOutcome::TimedOut { budget: timeout },
                };
                self.emit_refresh_event(&key, &outcome);
                (key, outcome)
            }
        });

        let results = span.in_scope(|| join_all(futures)).await;
        self.config.metrics.record_rotation_attempts(&results);
        Ok(results)
    }
}
```

`on_credential_revoked` is structurally symmetric — replace `dispatch_refresh(scheme)` with `dispatch_revoke(credential_id)`; emit `ResourceEvent::CredentialRevoked` per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md); same parallel `join_all`, per-resource timeout, isolation invariant.

**Hot-path borrow invariant.** `scheme: &(dyn Any + Send + Sync)` — borrowed across the entire dispatch. Each per-resource future borrows the same `&Scheme` for the duration; no `Scheme::clone()` per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) constraint 7.

### §3.3 Per-resource timeout enforcement

Per Q4 resolution, two layers: per-Manager default + per-resource override.

```rust
// Inside crates/resource/src/manager/options.rs (post-split).

/// Manager-wide rotation defaults. Applied to every registration
/// unless overridden by `RegisterOptions::credential_rotation_timeout`.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    // ... existing fields ...

    /// Default per-resource credential rotation timeout. Default 30s.
    /// Override per resource via `RegisterOptions::credential_rotation_timeout`.
    pub credential_rotation_timeout: Duration,

    /// Default per-credential rotation concurrency cap. Default 32.
    /// See §3.4 for rationale.
    pub credential_rotation_concurrency: usize,

    // ... existing fields ...
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            // ...
            credential_rotation_timeout: Duration::from_secs(30),
            credential_rotation_concurrency: 32,
            // ...
        }
    }
}

/// Per-resource registration options.
#[derive(Debug, Clone)]
pub struct RegisterOptions {
    pub scope: ScopeLevel,
    pub resilience: Option<AcquireResilience>,
    pub recovery_gate: Option<Arc<RecoveryGate>>,

    /// Per-resource override for credential rotation timeout. `None`
    /// uses `ManagerConfig::credential_rotation_timeout`.
    pub credential_rotation_timeout: Option<Duration>,
}

impl Default for RegisterOptions {
    fn default() -> Self {
        Self {
            scope: ScopeLevel::Global,
            resilience: None,
            recovery_gate: None,
            credential_rotation_timeout: None,
        }
    }
}

impl Manager {
    /// Resolves the timeout for a registered dispatcher. Per-resource
    /// override wins; falls back to Manager default.
    fn timeout_for(&self, dispatcher: &Arc<dyn ResourceDispatcher>) -> Duration {
        dispatcher
            .registered_timeout_override()
            .unwrap_or(self.config.credential_rotation_timeout)
    }
}
```

`TypedDispatcher::timeout_override` (declared in §3.2) is populated from `options.credential_rotation_timeout` at `register_inner` time per §3.1. Per-resource override wins; falls back to `ManagerConfig::credential_rotation_timeout`.

### §3.4 Concurrency cap (default + tunable)

Soft cap of 32 concurrent hooks per credential, default. Tech-lead Phase 2 mention of "~32" carries forward as the production default. Configurable via `ManagerConfig::credential_rotation_concurrency` (§3.3).

**Implementation.** CP1 commits to the `join_all` shape from spike — for N ≤ 32, `join_all` runs N parallel hooks unbounded. For N > 32, a `FuturesUnordered` cap fan-out lands when (and only when) operational signal demands it per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md). CP1 does NOT implement the FuturesUnordered cap; it records the cap value as a future-cleanup hook.

**Why 32:** 5 in-tree consumers × ~3 resources each = ~15 expected; doubling for headroom is 32. Conservative; production load testing in [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md) post-merge soak validates.

**Observability cost surface.** The `Box::pin` allocation per dispatch (Q3) compounds with N. At N=32, that's 32 box allocations per rotation event. Histogram `nebula_resource.credential_rotation_dispatch_latency_seconds` tracks per-resource latency; `nebula_resource.credential_rotation_attempts` counter labels by `outcome` per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md). Operators monitoring p99 latency at high N will see the box-allocation overhead.

### §3.5 Failure semantics + per-resource outcome aggregation

`RefreshOutcome` and `RevokeOutcome` enums encode per-resource results:

```rust
// Inside crates/resource/src/manager/rotation.rs.

/// Per-resource refresh dispatch result.
#[derive(Debug, Clone)]
pub enum RefreshOutcome {
    /// Hook completed within budget.
    Ok,
    /// Hook returned `Err(...)`.
    Failed(crate::Error),
    /// Hook exceeded its per-resource timeout (the `budget` value).
    TimedOut { budget: Duration },
}

/// Per-resource revocation dispatch result.
#[derive(Debug, Clone)]
pub enum RevokeOutcome {
    Ok,
    Failed(crate::Error),
    TimedOut { budget: Duration },
}

/// Aggregate outcome for one rotation event (refresh OR revoke). Carried in
/// `ResourceEvent::CredentialRefreshed` / `ResourceEvent::CredentialRevoked`
/// payloads (§Event broadcast contract below). One-glance summary for
/// operators reading the event stream; per-resource detail lives in tracing
/// spans, not the event payload.
#[derive(Debug, Clone)]
pub struct RotationOutcome {
    /// Count of dispatchers that returned `RefreshOutcome::Ok` / `RevokeOutcome::Ok`.
    pub ok: usize,
    /// Count of dispatchers that returned `*Outcome::Failed(_)`.
    pub failed: usize,
    /// Count of dispatchers that returned `*Outcome::TimedOut { … }`.
    pub timed_out: usize,
}
```

**Aggregation contract.** `Manager::on_credential_refreshed` returns `Result<Vec<(ResourceKey, RefreshOutcome)>, Error>`. Order matches register insertion order; one tuple per registered resource. Empty vec (wrapped in `Ok`) when no resources registered against the credential. Outer `Err(_)` reserved for setup-time failures only — per-resource errors aggregate into the per-resource `RefreshOutcome::Failed(_)` variant. `RotationOutcome` is derived from this `Vec` at event-emission time (counts of each variant).

**Per-resource isolation invariant** (security amendment B-1 per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md)):

- One resource's `Failed` does NOT poison sibling outcomes — siblings still see `Ok` or `TimedOut` per their own future.
- One resource's `TimedOut` does NOT extend wall-clock for siblings — siblings either complete within their own budget or hit their own timeout.

Validated empirically in spike `parallel_dispatch_isolates_per_resource_errors` ([`spike/.../resource-shape-test/src/lib.rs:537-578`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) and `parallel_dispatch_isolates_per_resource_latency` ([`spike/.../lib.rs:483-531`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)).

**Event broadcast contract.** Per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md), every dispatched outcome emits **one aggregate event** `ResourceEvent::CredentialRefreshed { credential_id, resources_affected, outcome: RotationOutcome }`. Symmetric `CredentialRevoked` for revocation. **CP1 commits to aggregate-only event broadcast**, with per-resource detail recorded in tracing spans (not events). The per-resource cardinality is fixed at zero for refresh/revoke aggregate events; CP2 §7 may revisit only if operational evidence post-soak ([Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md)) emerges that per-resource events are required — until then, aggregate-only is the contract.

**Revocation health asymmetry — load-bearing.** Refresh emits 1 aggregate event + N tracing spans. Revocation emits 1 aggregate `CredentialRevoked` event + 1 `HealthChanged { healthy: false }` event **per resource where `RevokeOutcome != Ok`** (security amendment B-2 per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md)). Per-resource `HealthChanged` events fire on the failure path only — successful revocations emit only the aggregate. CP2 §7 finalizes the upper-bound cardinality (the broadcast channel `events.rs` capacity at [`manager.rs:275`](../../../crates/resource/src/manager.rs) is 256 — at N=256 failed revocations the aggregate + N HealthChanged events exhaust the channel; CP2 §7 owns the bounding strategy). The asymmetry itself is the contract: refresh is span-level diagnostic, revocation is event-level health (B-2 mandates per-resource health visibility on revocation failure).

### §3.6 Resolution of Phase 1 🔴-1 (silent revocation drop) and 🔴-4 (drain-abort phase corruption)

**🔴-1 — silent revocation drop.** Resolved by §3.1 (reverse-index write path) + §3.2 (dispatcher implementation). Specifically:

- The `dashmap::DashMap<CredentialId, Vec<...>>` field at [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs) gains a write site — `register_inner` populates it for credential-bearing R per §3.1.
- The `on_credential_refreshed` `todo!()` panic at [`manager.rs:1378`](../../../crates/resource/src/manager.rs) is replaced by the parallel-dispatch implementation in §3.2.
- The `on_credential_revoked` `todo!()` panic at [`manager.rs:1400`](../../../crates/resource/src/manager.rs) is replaced by the symmetric revocation dispatcher in §3.2.
- The signature of `on_credential_refreshed` changes: takes a `&(dyn Any + Send + Sync)` scheme parameter (currently no scheme parameter); the per-resource outcome enum is renamed `ReloadOutcome` → `RefreshOutcome` to match the new trait method naming (per Strategy §4.3 + §4.9). The `Result<…, Error>` outer wrapper is preserved (matches the current shape at [`manager.rs:1363`](../../../crates/resource/src/manager.rs)) — per-resource errors aggregate into `RefreshOutcome::Failed(_)` / `RefreshOutcome::TimedOut { … }` variants, and the outer `Err(_)` is reserved for setup-time failures (e.g., reverse-index lookup faults, span construction errors). CP3 §13 enumerates the per-consumer migration impact.

**🔴-4 — drain-abort phase corruption.** Resolved per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md). The `DrainTimeoutPolicy::Abort` branch at [`crates/resource/src/manager.rs:1493-1510`](../../../crates/resource/src/manager.rs) currently calls `set_phase_all(ResourcePhase::Ready)` on line 1507 — corrupts the phase to `Ready` while returning `Err(ShutdownError::DrainTimeout)`. Tech Spec §3.6 commits to wiring `ManagedResource::set_failed(error)` ([`crates/resource/src/runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs), currently `#[expect(dead_code)]`) into the Abort path:

```rust
// Inside crates/resource/src/manager/shutdown.rs (post-split).

// REPLACES manager.rs:1493-1510 DrainTimeoutPolicy::Abort branch.
if let DrainTimeoutPolicy::Abort = self.config.drain_policy {
    let outstanding = self.drain_tracker.0.load(Ordering::SeqCst);
    let err = ShutdownError::DrainTimeout { outstanding };
    self.set_phase_all_failed(err.clone());  // NEW — was set_phase_all(Ready).
    return Err(err);
}
```

`set_phase_all_failed` invokes `ManagedResource::set_failed(err.clone())` per resource — phase becomes `Failed`, `last_error` is recorded. The `#[expect(dead_code)]` annotation on `set_failed` is removed. CP2 §6 records the test invariant.

The fix lands atomically with the `manager.rs` file-split per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md) — both touch the same shutdown path; bundling avoids context-thrash review.

## §4 — Lifecycle

Full lifecycle of a `Resource` registered with `Manager`: `register` → `acquire` → `release` → `drop` → `drain` → `shutdown`. Each subsection records pre-conditions, atomic operations, post-conditions, failure semantics, and cancel-safety. Every claim is anchored to the trait surface from §2 and the runtime surface from §3.

### §4.1 register

**Pre-conditions.** Caller holds an `Arc<Manager>` (or `&Manager`); supplies an `R: Resource` value, `R::Config`, `ScopeLevel`, `TopologyRuntime<R>`, optional resilience, optional recovery gate, and optional `RegisterOptions`. Manager is not in shutdown (no CAS race with `graceful_shutdown` per [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs)). For credential-bearing resources (`R::Credential != NoCredential`), caller MUST supply `credential_id: Some(_)` per §3.1; for `NoCredential` resources the id is ignored with `tracing::warn!` if `Some`.

**Atomic operations.** Per §3.1 + [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md):

1. `R::Config::validate()` runs first — if it returns `Err`, registration aborts with no side effects.
2. `ManagedResource<R>` is constructed in-process: `arc_swap::ArcSwap<Config>`, `topology` enum, `release_queue` `Arc::clone`, `generation: AtomicU64::new(0)`, `status: ArcSwap<ResourceStatus>` initialised to `Initialising`.
3. **Reverse-index write** runs *before* the registry write per [ADR-0036 line 103](../adr/0036-resource-credential-adoption-auth-retirement.md). For credential-bearing R: `TypedDispatcher` wraps `Arc<ManagedResource<R>>` + `RegisterOptions::credential_rotation_timeout`; pushed under `credential_resources.entry(id)` (DashMap). For `NoCredential` R: skipped silently with warn.
4. **Registry write** appends to `Registry` (the `DashMap<(ResourceKey, ScopeLevel), Arc<dyn AnyManagedResource>>` at [`manager.rs:374`](../../../crates/resource/src/manager.rs)). Same-key replacement is silent today (Strategy §4.5 preserves; CP3 §11 may revisit).
5. `ResourceEvent::Registered { key }` broadcast via `event_tx` (cap 256).

**Post-conditions.** `R::key()` is reachable from any subsequent `acquire_*` call; `on_credential_refresh` / `on_credential_revoke` dispatchers populated for credential-bearing R; status phase = `Initialising` until first acquire creates a runtime instance (or warmup, §5.2).

**Failure semantics.** Returns `Result<(), Error>`. Validation failure → `Err(Error::permanent(_))`. Missing credential id on credential-bearing R → `Err(Error::missing_credential_id(R::key()))` (NEW, §3.1). Shutdown-in-progress → `Err(Error::cancelled(_))` (per existing pattern at [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs)). All failures leave the registry and reverse-index in their pre-call state — no half-written state observable. The reverse-index write is part of the registration *transaction*: if registration fails after step 3, the dispatcher is dropped (no rollback needed because step 3 happened *before* step 4 only when validation already passed).

**Cancel-safety.** `register` is fully synchronous (no `await` points) — cancellation cannot interleave with the atomic operations above. Construction of `ManagedResource<R>` does not yield. The `event_tx.send(Registered)` is best-effort (lagging subscribers receive `Lagged` on next `recv`).

**Resolves Phase 1 🔴-1 (silent revocation drop).** Reverse-index write at step 3 is the explicit write path Phase 1 found missing ([`02-pain-enumeration.md` §4 row 🔴-1](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md), [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs)). Atomic with registry write per [security amendment B-1 / constraint #1](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md).

### §4.2 acquire

**Pre-conditions.** Resource is registered (`Manager::lookup::<R>(scope)` returns `Ok(Arc<ManagedResource<R>>)`). Manager not shut down (`is_accepting()` true). Caller holds an `AcquireOptions` (default acceptable) and a `ResourceContext`. For credential-bearing R, the engine has resolved `<R::Credential as Credential>::Scheme` from the credential store via `CredentialAccessor` ([`crates/resource/src/context.rs`](../../../crates/resource/src/context.rs)).

**Per-topology variants.** Each topology has a distinct atomic-operation chain — five total post-Daemon/EventSource extraction (per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md)):

| Topology  | Method                | What gets returned                                  |
|-----------|-----------------------|-----------------------------------------------------|
| Pooled    | `acquire_pooled`      | `ResourceGuard::guarded(_)` (pool-return on drop)   |
| Resident  | `acquire_resident`    | `ResourceGuard::shared(_)` (Arc-shared lease)       |
| Service   | `acquire_service`     | `ResourceGuard::guarded(_)` (release_token on drop) |
| Transport | `acquire_transport`   | `ResourceGuard::guarded(_)` (close_session on drop) |
| Exclusive | `acquire_exclusive`   | `ResourceGuard::owned(_)` (no return; `reset` on drop) |

**Atomic operations** (common chain):

1. `Manager::lookup::<R>(scope)` — DashMap read; cancel-safe (no `await`).
2. **Recovery gate check** (if `Some(gate)` registered) — early `Err(Error::transient_recovery_gate_open(_))` if gate is open. No await window before first refusal.
3. **Resilience wrap** — if `AcquireResilience` registered, `execute_with_resilience` wraps the acquire body with timeout + retry + circuit breaker per [`crates/resource/src/integration/`](../../../crates/resource/src/integration/).
4. **Topology-specific runtime acquire** — pool semaphore wait, service token request, transport session open, exclusive runtime build. Each is `await`-bounded by the resilience wrap (or unbounded if absent).
5. **Drain-tracker increment** — `drain_tracker.0.fetch_add(1, Relaxed)` *before* `ResourceGuard` construction. Guard's `Drop` decrements + `notify_waiters` on 1→0. This is the contract `wait_for_drain` consumes (§4.5).
6. **Metrics + event emission** — `ResourceOpsMetrics::record_acquire()` + `ResourceEvent::AcquireSuccess { key, duration }` broadcast.

**Post-conditions.** Caller holds a `ResourceGuard<R>` that derefs to `R::Lease`. Drain counter incremented by 1. On `acquire` failure, drain counter unchanged.

**Failure semantics.** Cancel-token signal during step 4 → `Err(Error::cancelled(_))` (drain counter NOT incremented because increment is at step 5). Recovery gate open → `Err(Error::transient_recovery_gate_open(_))`. Resilience-bounded timeout → `Err(Error::timeout(_))`. Topology-specific failure → `Err(R::Error.into())`. **Failure is recorded as `AcquireFailed` event** per [`events.rs:38-44`](../../../crates/resource/src/events.rs); `record_acquire_error()` increments the error counter.

**Cancel-safety.** Each step's `await` site is cancel-safe by design: lookup is sync; resilience wrap respects the `tokio::select!` parent token; topology-specific waits use `tokio::time::timeout` + cancel-token select. **The drain-tracker increment in step 5 happens AFTER the last cancel-aware `await` of the runtime acquire** — there is no window where increment happens but guard never constructs. If cancellation lands between step 5 and `ResourceGuard` construction (in practice, no `await` between them — increment is the last step before guard `new`), the panic-aware `Drop` of the partially-constructed guard recovers the counter. CP3 §11 carries the test invariant.

### §4.3 release

**Pre-conditions.** Caller drops a `ResourceGuard<R>`, or a guard variant fires its `on_release` callback. Manager may be in any phase except final shutdown.

**Two paths** — sync drop vs async ReleaseQueue:

- **Sync drop path.** `ResourceGuard::Drop` runs synchronously at the drop site. For owned guards (Exclusive), `R::reset` is *spawned* via `release_queue.submit` because `reset` is async and `Drop` is sync. For guarded guards (Pool/Service/Transport), the `on_release` closure is invoked synchronously — the closure decides whether to recycle or destroy the lease.
- **Async ReleaseQueue path.** When the on-release closure needs to run async work (close transport session, return pool connection with health probe), it submits a `TaskFactory` to `ReleaseQueue::submit` ([`release_queue.rs:97`](../../../crates/resource/src/release_queue.rs)). Round-robin to N primary workers (default 4); fallback channel on full; rescue spawn on double-full with 30s timeout cap. Tasks bounded to 30s execution ([`release_queue.rs:41`](../../../crates/resource/src/release_queue.rs)).

**Tainted vs healthy.** `ResourceGuard::tainted()` flips the `tainted` flag inside `GuardInner::Guarded { tainted, .. }`. On drop, the on-release closure inspects `tainted`: true → destroy lease (transport: `close_session`, pool: drop without recycle); false → recycle (pool: return to pool, service: `release_token`, transport: pool resume).

**Post-conditions.** `ResourceEvent::Released { key, held, tainted }` broadcast. `ResourceOpsMetrics::record_release()` increments. **Drain-tracker decrements** atomically (Acquire-Release ordering); on 1→0 transition the guard's `Drop` calls `notify_waiters` so `wait_for_drain` wakes up. On tainted-destroy path, `record_destroy()` also increments.

**Failure semantics.** ReleaseQueue full + fallback full + rescue timeout → `dropped_count` increments ([`release_queue.rs:107`](../../../crates/resource/src/release_queue.rs)). The lease leaks if the rescue task drops it before reaching the worker. Operator signal: `dropped_count > 0` is observable but not yet surfaced as a `ResourceEvent` variant (CP3 §11 may revisit; out of CP2 scope per Strategy §5).

**Cancel-safety.** `Drop` is panic-safe via `catch_unwind` around the on-release closure ([`guard.rs:96-99`](../../../crates/resource/src/guard.rs)). Permit return is decoupled from closure execution so a closure panic cannot leak a semaphore slot. Cancel-token cancellation during async release work is acceptable: the worker spawn picks up cancel and exits cleanly per release_queue's cancel-on-drain contract.

### §4.4 drop

**Pre-conditions.** A `ResourceGuard<R>` exits scope without explicit release (function return, panic unwind, `let _ = guard;`).

**`#[must_use]` enforcement.** `ResourceGuard` is annotated `#[must_use = "dropping a ResourceGuard immediately releases the resource"]` at [`guard.rs:31`](../../../crates/resource/src/guard.rs). Compiler emits a warning if the guard is constructed and dropped without binding. **CP2 affirms this annotation as load-bearing**: it is the type-level signal that drop = release.

**Rescue-timeout 30s.** Per [`release_queue.rs:60`](../../../crates/resource/src/release_queue.rs), when both the primary channel and the fallback channel are full at submit time, a short-lived rescue task is spawned that awaits fallback capacity for up to 30s. After 30s the task records the drop and exits. **This is the upper bound on tail-latency for a drop reaching a worker.**

**Atomic operations.** Identical to §4.3 release — drop IS the trigger for the release path. Difference is no caller-visible result; the guard simply ceases to exist.

**Failure semantics.** Best-effort. The rescue-timeout dropped-count metric is the operator signal that the system was overloaded.

**Cancel-safety.** Same as §4.3.

### §4.5 drain

**Pre-conditions.** `Manager::graceful_shutdown(config)` invoked. `shutting_down` CAS flips `false → true` (loser caller gets `Err(ShutdownError::AlreadyShuttingDown)` per [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs)). No interleaving caller can re-enter the drain logic against half-torn state.

**Atomic operations** — phased per [`manager.rs:1473-1559`](../../../crates/resource/src/manager.rs):

1. **Phase 1 SIGNAL.** `cancel.cancel()` — rejects new acquires (lookup checks token), tells release_queue workers to drain.
2. **`set_phase_all(Draining)`** — every registered `ManagedResource` flips status phase to `Draining` so health probes mid-drain see the correct lifecycle. Uses `Arc<dyn AnyManagedResource>` virtual call into `set_phase_erased`.
3. **Phase 2 DRAIN.** `wait_for_drain(drain_timeout)` blocks on the drain-tracker `Notify`; returns `Ok` when counter hits 0, `Err(DrainTimeoutError { outstanding })` on timeout.
4. **DrainTimeout dispatch** per `DrainTimeoutPolicy`:
   - `Abort` → **fixed in §3.6**: `set_phase_all_failed(ShutdownError::DrainTimeout { outstanding })` per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md). Phase becomes `Failed` (NOT `Ready` — current corruption per Phase 1 🔴-4); `last_error` recorded; `shutting_down` resets to false; returns `Err`.
   - `Force` → log warn, proceed to Phase 3 with `outstanding_after_drain` recorded.
5. **Phase 3 CLEAR.** `set_phase_all(ShuttingDown)` → `registry.clear()`.
6. **Phase 4 AWAIT WORKERS.** `ReleaseQueue::shutdown(handle)` bounded by `release_queue_timeout`. Failure surfaces as `Err(ShutdownError::ReleaseQueueTimeout)`.

**Post-conditions.** On success: registry empty, release_queue workers joined, `ShutdownReport { outstanding_handles_after_drain, registry_cleared: true, release_queue_drained: true }` returned. On Abort failure: registry preserved, all phases `Failed`, `Err(DrainTimeout)` returned.

**Failure semantics.** Three terminal errors per [`manager.rs:147-180`](../../../crates/resource/src/manager.rs): `AlreadyShuttingDown`, `DrainTimeout { outstanding }`, `ReleaseQueueTimeout { timeout }`. **Observability honesty restored** per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md): operators polling `health_check` after `Abort` see `Failed` phase + `last_error: ShutdownError::DrainTimeout` instead of misleading `Ready`.

**Cancel-safety.** Drain is itself cancel-aware (selects on the cancel token) but `graceful_shutdown` is the *driver* of the cancel — calling it concurrently with the cancel-token external trip is benign (the AlreadyShuttingDown CAS dedup wins).

**Resolves Phase 1 🔴-4.** [Phase 1 row 🔴-4](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — drain-abort path corruption — closed by step 4 Abort branch wiring `set_failed`.

### §4.6 shutdown

**Force vs graceful.** `graceful_shutdown` (§4.5) is the *only* public shutdown entry. Force is a *policy* (`DrainTimeoutPolicy::Force`) not a separate method — it changes whether outstanding handles are tolerated, not whether the drain machinery runs.

**ReleaseQueue drain.** Phase 4 of `graceful_shutdown` (§4.5 step 6) invokes `ReleaseQueue::shutdown(handle)`. Workers received cancel signal in Phase 1; they finish in-flight tasks bounded by `TASK_EXECUTION_TIMEOUT` (30s, [`release_queue.rs:41`](../../../crates/resource/src/release_queue.rs)). `ReleaseQueue::shutdown` joins all worker `JoinHandle`s; `release_queue_timeout` (default per `ShutdownConfig`) bounds the total await.

**Idempotency.** CAS-guarded. Second concurrent `graceful_shutdown` returns `Err(AlreadyShuttingDown)`. After successful return, repeated calls *do* re-enter (CAS succeeds because `shutting_down` was reset to false on Abort or true on success — CP3 §12 may add an explicit terminal-state guard; currently semantically safe because registry is empty so no work to do).

**Post-conditions.** Manager handle still exists; consumers may still hold `Arc<Manager>` for diagnostics, but `register` rejects new resources, `acquire_*` rejects new acquires (cancel token tripped), and `subscribe_events` returns a receiver that will only see future events (none, in practice).

**Failure semantics.** Same as §4.5 step 4 + step 6. No new error variants in §4.6.

**Cancel-safety.** Cancel-driven. The whole machinery is the cancel landing site — there is no caller-side cancel to honour beyond the cancel-token trip itself.

## §5 — Implementation specifics

The mechanism details. Six items: blue-green pool swap (§5.1), credential-bearing `warmup_pool` signature (§5.2), revocation default-hook decision (§5.3), Manager file-split execution (§5.4), resilience policy execution (§5.5), and the CP3 deferrals surfaced from this CP's mechanism work (§5.6). Each of §5.1-§5.5 commits a load-bearing decision; reviewers should scrutinise §5.3 in particular (security implication central). §5.6 records what is *not* in CP2 mechanism scope but must travel with these decisions to CP3.

### §5.1 Blue-green pool swap mechanism

**Resource owns the swap** — Manager dispatches the hook; the resource impl does the swap inside `on_credential_refresh`. This is the canonical pattern from [credential Tech Spec §3.6 lines 961-993](2026-04-24-credential-tech-spec.md), adopted verbatim per [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md). Manager NEVER orchestrates pool recreation; Manager NEVER holds `Scheme` longer than the dispatch call window. This is the key security invariant from [security review constraint #2 (lines 105-108)](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md): "the swap happens *inside* the resource impl, not inside the manager."

**Verbatim cite from credential Tech Spec §3.6 lines 981-993:**

> `pub async fn on_credential_refresh(&self, new_scheme: &PostgresConnectionScheme) -> Result<(), PostgresError> { let new_pool = build_pool_from_scheme(new_scheme).await?; let mut guard = self.inner.write().await; *guard = new_pool; Ok(()) }` — old connections drain naturally as their RAII guards drop; new queries use the new pool (read lock acquires against the new inner after swap).

CP2 does not restate the credential primitives; readers consume §3.6 directly.

**Swap point: `Arc<RwLock<Pool>>`, NOT `ArcSwap`.** The pattern from credential Tech Spec §3.6 uses `Arc<tokio::sync::RwLock<deadpool_postgres::Pool>>`. CP2 commits to this shape for connection-bound resources. Rationale (three reasons):

- **Async-safe write contention.** Pool-rebuild inside `on_credential_refresh` is `async` (`build_pool_from_scheme(new_scheme).await?`) — it must hold the lock across an `.await`. `ArcSwap` does not block writers, but it also does not provide the *exclusion* needed to prevent two concurrent refreshes from racing to rebuild. `tokio::sync::RwLock`'s write guard is `Send` and held across `.await`, so two parallel refresh dispatches against the same resource (which §3.5 isolation protects against externally, but defence-in-depth) serialise on the write guard.
- **Read affordance is identical.** Read-side query path holds `inner.read().await.get_connection().await?` — readers are non-blocking under contention because read locks are shared. Throughput on the hot acquire path is unchanged from `Arc<Pool>` direct-access.
- **Old-pool drain is RAII-natural.** When the write guard publishes the new pool, every caller that had previously taken a `Pool` clone (via `inner.read().await.clone()` or by holding a connection-guard rooted in the old pool) continues to use the old pool. RAII guards drop naturally; the old pool's connection slots reach zero and the pool drops. No explicit drain coordination required from Manager.

**ArcSwap rejected for this surface.** ArcSwap (lock-free atomic swap) is correct for `ResourceStatus` reads (§8.5), where readers want the most recent value with no contention. For pool swap, the writer's *exclusion* requirement is dispositive — a refresh races against itself otherwise. CP3 §10 enumerates the per-topology guidance.

**In-flight acquires during swap.** The acquire path (§4.2 step 4) acquires a *read* lock on `inner` to obtain the live `Pool`, then defers the actual connection `get` to the cloned pool reference. If a swap is in progress (write lock held), new read-lock acquisitions wait. Wait time is bounded by `build_pool_from_scheme(new_scheme).await` — typically sub-second; observability covers this via `nebula_resource.credential_rotation_dispatch_latency_seconds` (§6.2). In-flight queries against the OLD pool are unaffected (their connection-guard is rooted in the pre-swap `Pool` clone). On swap publication, the next acquire sees the new pool; old pool drains as RAII guards drop.

**Budget guidance** (CP3 §11 will detail; surfaced now per security-lead amendment SL-3). The resource-side `build_pool_from_scheme(new_scheme).await` SHOULD complete in significantly less than the Manager dispatch budget (default 30s per CP1 §2.5 Q4) — recommend ≤ 60% of dispatch timeout (≤ 18s under the default) to leave headroom for swap installation (write-guard publication) plus old-pool drop. Rationale: if `build_pool_from_scheme` exhausts the full dispatch budget, Manager's `tokio::time::timeout` fires while the resource-side write guard is still held; the future-drop is RAII-natural so the lock releases (no leak), but observability is poorer — the dispatch surfaces as `TimedOut` rather than `Failed` despite the resource's own rebuild having actually failed inside its allotted time. CP3 §11 specifies the per-`RegisterOptions::pool_rebuild_timeout` knob.

### §5.2 `warmup_pool` credential-bearing signature

**Decision.** `warmup_pool` takes the credential scheme as an explicit parameter; **NOT via `Scheme::default()`** (security amendment B-3 per [`phase-2-security-lead-review.md:76-82`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md), Phase 1 🟡-17 per [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). The current `R::Auth::default()` call at [`crates/resource/src/manager.rs:1268`](../../../crates/resource/src/manager.rs) is removed.

**Signature** (replaces [`manager.rs:1259-1280`](../../../crates/resource/src/manager.rs)):

```rust
impl Manager {
    /// Warmup pool with a real credential scheme.
    ///
    /// Caller resolves the scheme from the credential store before
    /// invoking warmup. Manager does NOT silently default-fill the scheme
    /// because `Scheme::default()` historically created empty-credential
    /// runtimes (Phase 1 🟡-17). For `NoCredential` resources, see
    /// `warmup_pool_no_credential` below.
    pub async fn warmup_pool<R>(
        &self,
        credential: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, credential, ctx).await;
                Ok(count)
            },
            _ => Err(Error::permanent(format!(
                "{}: warmup_pool requires Pool topology, registered as {}",
                R::key(), managed.topology.tag()
            ))),
        }
    }

    /// Warmup pool for a `NoCredential` resource. Compile-time bound
    /// `R::Credential = NoCredential` enforces opt-out at the type level —
    /// no runtime check, no scheme parameter.
    pub async fn warmup_pool_no_credential<R>(
        &self,
        ctx: &ResourceContext,
    ) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled<Credential = nebula_credential::NoCredential>,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        // ... mirrors warmup_pool body, passing &NoScheme.
    }
}
```

Two methods, one per shape, distinguished by trait-bound at compile time. Three reasons over a unified `Option<&Scheme>`:

- **Type-level enforcement of B-3.** A unified `warmup_pool` taking `Option<&<R::Credential as Credential>::Scheme>` would require runtime branching — `None` would fall back to a default. The split makes "no credential" expressible only via the dedicated `_no_credential` variant whose bound prevents calling it on credential-bearing R.
- **No `Default` trait dependency on `Scheme`.** The current `R::Auth: Default` bound at [`manager.rs:1264`](../../../crates/resource/src/manager.rs) goes away. `Scheme` types do not need `Default` per `Credential` trait shape ([`credential/src/contract/credential.rs`](../../../crates/credential/src/contract/credential.rs)).
- **Caller obligation explicit.** The credential-bearing variant requires the caller to resolve `<R::Credential as Credential>::Scheme` first — typically via `CredentialAccessor::resolve(credential_id)` from `ResourceContext`. CP3 §11 specifies the helper if a non-trivial number of consumers want a "warmup with id" convenience.

**Trade-off accepted.** Two methods instead of one. Mitigated by the type-level safety (B-3 unfaultable) and by `RegisterOptions` carrying credential-id metadata for future `warmup_pool_by_id` ergonomics if needed (deferred to CP3 §11).

### §5.3 Revocation default-hook mechanism

**Decision: option (b) — default body returns `Ok(())`; Manager unconditionally flips a per-resource `credential_revoked` atomic post-dispatch. Subsequent `acquire_*` calls fail.**

The hook's *default body* and Manager's *post-dispatch enforcement* are two layers; option (b) wires both. The user-trait method's default body is no-op (matching CP1 §2.1 declaration `async { Ok(()) }`); Manager owns the invariant-enforcement layer that flips the per-resource atomic regardless of whether the hook was overridden.

Three options were considered (per continuation prompt + [Strategy §4.2 lines 250-258](2026-04-24-nebula-resource-redesign-strategy.md)):

- **(a) Default body destroys pool (aggressive).** The default `on_credential_revoke` would tear down the runtime via `destroy()` (consume Runtime) without any user override. **Rejected** — too aggressive for the default contract: many resources have valid reasons to defer destruction (in-flight queries should complete; pool may be shared across credential ids in multi-tenant configs; `NoCredential` resources cannot meaningfully destroy). Forcing destruction by default breaks per-resource invariant authorship — every override must wrap the parent default to suppress destruction, which is the opposite of the additive-override pattern Strategy §4.2 establishes.
- **(b) Default body no-op + Manager unconditional post-dispatch taint-flip. SELECTED.** Default body of `on_credential_revoke` returns `Ok(())`; Manager flips a `credential_revoked: AtomicBool` on `ManagedResource<R>` after every dispatch (success, failure, or timeout). Subsequent `acquire_*` calls check the atomic and return `Err(Error::credential_revoked(_))` per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md) invariant ("post-invocation, the resource emits no further authenticated traffic on the revoked credential"). In-flight handles complete naturally. Resources MAY override the hook to add stronger semantics (destroy pool synchronously, taint outstanding guards via guard-side `tainted = true`) — overriding tightens, never weakens.
- **(c) Default body no-op; no Manager-side enforcement (override-only invariant).** Default body returns `Ok(())`; Manager makes no post-dispatch change. **Rejected** — a resource with default `on_credential_revoke` body satisfies the trait but VIOLATES the Strategy §4.2 invariant: it continues serving authenticated traffic from the revoked credential because nothing flips its state. The contract is uncatchable by Manager and uncatchable by `tracing::warn!` — silent invariant violation. This is the failure mode security-lead Phase 2 BLOCKED on for Option A.

**Distinction between (b) and (c).** The two diverge only on *who enforces the post-condition*. (c) puts the burden on every overrider; the default body is a structural footgun. (b) puts the burden on Manager, leaving the user's override free to *add* stronger semantics without having to *first* satisfy the base invariant manually.

**Why (b) wins on security grounds.**

- **Default body honours the invariant** (Strategy §4.2 / [ADR-0036 line 102](../adr/0036-resource-credential-adoption-auth-retirement.md)). A resource that does nothing in `on_credential_revoke` (e.g., a stateless API client that only fetches per-request) is still tainted by Manager's enforcement; the next acquire fails. There is no safe-but-silent state where Manager believes revocation succeeded but the resource continues to issue authenticated traffic on the revoked credential.
- **In-flight handles drain naturally.** Outstanding `ResourceGuard<R>` references continue to function (their `Drop` releases against the still-owned runtime). New acquires fail. This matches the soft-revocation semantics from [credential Tech Spec §4.3 lines 1062-1068](2026-04-24-credential-tech-spec.md): existing in-flight resolves continue for grace; new resolves fail.
- **Override is additive, not corrective.** A `PostgresPool` impl can override `on_credential_revoke` to *additionally* destroy the pool synchronously (immediate hard-revocation semantics). It does NOT need to also flip `credential_revoked` itself — Manager handles that layer. The override's job is the resource-specific cleanup; Manager's job is the invariant enforcement.

**Concrete wiring** (CP3 §10 for full code):

```rust
// Inside Manager::dispatch_revoke (§3.2 trampoline path).
async fn dispatch_revoke_with_tainting(
    &self,
    dispatcher: &Arc<dyn ResourceDispatcher>,
    credential_id: &CredentialId,
) -> RevokeOutcome {
    // 1. Run the override (or default no-op).
    let hook_result = dispatcher.dispatch_revoke(credential_id).await;
    // 2. Regardless of override result (Ok / Err / would-be-timeout),
    //    Manager flips the per-resource `credential_revoked` atomic on
    //    `ManagedResource`. This is the invariant-enforcement layer.
    self.set_revoked_for_dispatcher(dispatcher);
    // 3. Convert to outcome (Failed/TimedOut still surface to operators
    //    via RevokeOutcome + HealthChanged events).
    match hook_result { ... }
}
```

The taint-flip is unconditional (runs even when override returns `Err`), preserving the post-condition invariant on the failure path. The override's `Err` is recorded in `RevokeOutcome::Failed` and triggers the `HealthChanged { healthy: false }` event (security amendment B-2). **The atomic flip happens AFTER the await of `dispatch_revoke` — there is no window where the atomic flips before the override could observe its pre-revocation state.**

**Trade-off accepted.** A future revocation override that *intentionally* wants to leave the resource live (e.g., multi-tenant pool where one tenant's credential was revoked but the pool remains valid for other tenants) cannot — the taint flip is unconditional and per-resource (not per-credential within a multi-tenant pool). CP3 §11 may add a per-`RegisterOptions::tainting_policy` knob if a real consumer demands it; CP1/CP2 commits to unconditional tainting because zero in-tree consumers fit the multi-tenant exception today.

### §5.4 Manager file-split execution

Per [Strategy §4.5](2026-04-24-nebula-resource-redesign-strategy.md), the 2101-line `crates/resource/src/manager.rs` splits into submodules **without changing the `Manager` type's public surface**. CP2 commits the cut points:

| Submodule path                                | Contents                                                                 | Internal visibility |
|-----------------------------------------------|--------------------------------------------------------------------------|---------------------|
| `crates/resource/src/manager/mod.rs`          | `Manager` struct, public `register*`, `acquire*`, `subscribe_events`, `is_accepting`, `lookup` | `pub`               |
| `crates/resource/src/manager/options.rs`      | `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`, `ShutdownError`, `ShutdownReport` | `pub` (re-exported) |
| `crates/resource/src/manager/registration.rs` | `register_inner` (§3.1), reverse-index write path, error constructors `missing_credential_id` + `scheme_type_mismatch` | `pub(crate)`        |
| `crates/resource/src/manager/gate.rs`         | `GateAdmission`, `admit_through_gate`, `settle_gate_admission`           | `pub(crate)`        |
| `crates/resource/src/manager/execute.rs`      | `execute_with_resilience`, `validate_pool_config`, `wait_for_drain`      | `pub(crate)`        |
| `crates/resource/src/manager/rotation.rs`     | `ResourceDispatcher` trait, `TypedDispatcher<R>`, `on_credential_refreshed`, `on_credential_revoked`, observability scaffolding | `pub(crate)` (trait `pub` for downstream test-access only via doc(hidden)) |
| `crates/resource/src/manager/shutdown.rs`     | `graceful_shutdown`, `set_phase_all`, `set_phase_all_failed` (§3.6 fix)  | `pub(crate)`        |

**Function-level boundaries deferred to CP3 §9.** CP2 locks the file structure; CP3 enumerates which exact `impl Manager { fn ... }` block lands in which submodule. This split decouples the structural decision (which CP2 must lock for `manager.rs:262` reverse-index write to land in `registration.rs` not `mod.rs`) from the function-arrangement decision (which CP3 owns alongside the per-consumer migration list).

**`pub(crate)` discipline.** `manager/mod.rs` is the only `pub` re-export site for everything except `options.rs` (which carries types already public). `rotation.rs` exposes `ResourceDispatcher` only via `pub` — but it is intentionally an internal seam: doc-hidden re-export under `crate::__internal::ResourceDispatcher` for test access and downstream type-erasure ergonomics. CP3 §10 specifies the doc-hidden path.

**No public API change.** Every existing import at `nebula_resource::*` (5 in-tree consumers) keeps working — `lib.rs` re-exports remain untouched. The split is purely structural; it does not surface as a breaking change in the migration wave.

### §5.5 Resilience policy execution (drain-abort fix)

Per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md) + §3.6 of this Tech Spec. `ManagedResource::set_failed(error)` at [`crates/resource/src/runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs) is currently dead-coded behind `#[expect(dead_code, reason = "callers will land with the recovery-error work")]`. CP2 wires it.

**Wiring site.** The `DrainTimeoutPolicy::Abort` branch in `manager/shutdown.rs` (§5.4 split) replaces:

```rust
// REMOVED — current at manager.rs:1507
self.set_phase_all(crate::state::ResourcePhase::Ready);
```

with:

```rust
// NEW.
self.set_phase_all_failed(ShutdownError::DrainTimeout { outstanding });
```

`set_phase_all_failed` iterates `registry.all_managed()` and calls `managed.set_failed(err.clone())` for each, which:

1. Sets `status` phase to `Failed` (`ResourcePhase::Failed`).
2. Records `last_error: Some(error)` on `ResourceStatus`.
3. Emits a `ResourceEvent::HealthChanged { key, healthy: false }` per [`events.rs:54-60`](../../../crates/resource/src/events.rs).

**`#[expect(dead_code)]` removal.** Annotation lifted from [`runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs). The lint scope was correct ("callers will land with the recovery-error work") — that work is this Tech Spec's CP2 wiring.

**Test invariant** (CP3 §12):

```rust
// `tests/basic_integration.rs` — abort-policy-records-failed-phase.
let report = manager.graceful_shutdown(
    ShutdownConfig::default()
        .with_drain_timeout(Duration::from_millis(50))
        .with_abort_policy()
).await;
assert!(matches!(report, Err(ShutdownError::DrainTimeout { .. })));
let phase = manager.health_check::<MyResource>(&scope).unwrap().phase;
assert_eq!(phase, ResourcePhase::Failed);  // NOT Ready (current corruption).
```

**Resolves Phase 1 🔴-4.** [Phase 1 row 🔴-4](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — drain-abort phase corruption — closed atomically with §5.4 file-split per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md).

### §5.6 CP3 deferrals from this CP

CP2 mechanism work surfaced two CP3-track surfaces that are NOT CP2-applicable. Recorded here so they cannot be silently introduced under CP3 DX pressure without their gating constraints.

- **Future `RegisterOptions::tainting_policy` knob (security-lead amendment SL-1; §5.3 line 1250 trade-off).** A per-consumer opt-out from §5.3 option (b)'s unconditional taint flip — for example, a multi-tenant pool that wants to revoke one tenant's credential while continuing to serve others. Two gates required before introduction at CP3 §10 surface review: (1) a real in-tree consumer must surface the exception (synthetic tests do not qualify per §5.3 line 1250 deferral discipline); (2) the knob must include a security-review hook in the CP3 surface review wave that introduces it. CP3 §10 surface review confirms no premature `tainting_policy` knob ships before both gates clear. Until then, unconditional tainting is the secure default and §5.3 option (b) is the only revocation shape.
- **Future `warmup_pool_by_id` ergonomic helper (security-lead amendment SL-2 § follow-up; §5.2 line 1203).** A convenience that resolves the credential scheme via `CredentialAccessor::resolve(credential_id)` internally so consumers do not call the resolver themselves before `warmup_pool`. Gate: the resolution path MUST go through `CredentialAccessor`; bypassing it (e.g., reading scheme bytes from `RegisterOptions` directly) would resurrect the B-3 attack surface the §5.2 split closes. CP3 §11 ergonomics review verifies the helper composes the existing `CredentialAccessor` rather than introducing a parallel resolution path.

These are the only two CP3 deferrals from CP2 mechanism work. SL-3 (resource-side budget guidance) is not deferred; it lands in §5.1 with the CP3 §11 forward-ref.

## §6 — Operational (observability concrete)

Per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md) + [tech-lead amendment 2 / Phase 3 CP1 E1 invariant](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md). Every observability identifier below is **CP-review-gateable** — the names locked in §6.1-§6.4 are what land in the migration PR; review pushback that proposes alternative names lands as an amendment, not a CP3 rename.

### §6.1 Trace span names

Six spans, locked. Each name is the final string used in `tracing::info_span!` / `tracing::error_span!` invocations. No suffixes, no per-resource keys in the name itself (resource keys are *fields*, not name suffixes).

| Span name (literal)                       | Level | Where emitted                                                              | Fields                                                      |
|-------------------------------------------|-------|----------------------------------------------------------------------------|-------------------------------------------------------------|
| `resource.credential_refresh`             | INFO  | `Manager::on_credential_refreshed` outer scope (§3.2 line 793)             | `credential_id`, `resources_affected`                       |
| `resource.credential_refresh.dispatch`    | INFO  | Per-resource future inside `join_all`                                      | `credential_id`, `resource_key`, `timeout_budget_ms`        |
| `resource.credential_revoke`              | WARN  | `Manager::on_credential_revoked` outer scope                               | `credential_id`, `resources_affected`                       |
| `resource.credential_revoke.dispatch`     | WARN  | Per-resource future inside `join_all` (revocation)                         | `credential_id`, `resource_key`, `timeout_budget_ms`        |
| `resource.acquire.{topology}`             | DEBUG | `Manager::acquire_*` (§4.2 step 4)                                         | `resource_key`, `topology`, `scope`                         |
| `resource.shutdown`                       | INFO  | `Manager::graceful_shutdown` outer scope                                   | `drain_timeout_ms`, `policy`, `outstanding_at_drain_start`  |

The `.dispatch` per-resource child span is what satisfies tech-lead Phase 2 amendment 2's "per-resource child span" ask. **Span fields are redacted** per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md): `credential_id` is the typed ID (no scheme content); `resource_key` is the canonical key; no scheme bytes, no token material in any field.

**Span lifecycle.** Outer span enters at function entry; per-resource child spans enter at the start of each per-resource future (inside `async move { ... }` of the `join_all` map closure). The outer span captures `resources_affected` after the dispatcher list is loaded (§3.2 line 786). Both close on function return / future completion.

`{topology}` for the acquire span is one of `pooled`, `resident`, `service`, `transport`, `exclusive` — no enumeration of the `TopologyTag::Daemon` / `EventSource` per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md) extraction.

### §6.2 Counter metrics

Five metrics, locked. Three are NEW in this redesign; two are existing `ResourceOpsMetrics` preserved.

| Metric (literal)                                                  | Type      | Labels                                              | Where emitted                              |
|-------------------------------------------------------------------|-----------|-----------------------------------------------------|--------------------------------------------|
| `nebula_resource.credential_rotation_attempts`                    | Counter   | `outcome` ∈ {`success`, `failed`, `timed_out`}      | Per dispatch in §3.2 trampoline (one inc per resource per refresh event) |
| `nebula_resource.credential_revoke_attempts`                      | Counter   | `outcome` ∈ {`success`, `failed`, `timed_out`}      | Per dispatch in §3.2 trampoline (revocation path) |
| `nebula_resource.credential_rotation_dispatch_latency_seconds`    | Histogram | `outcome` (same set)                                | Per dispatch — wraps `tokio::time::timeout` body (covers Box::pin allocation cost per Q3) |
| `nebula_resource.acquire_total` (existing)                        | Counter   | none currently; CP3 may add `topology`              | `Manager::acquire_*` success path           |
| `nebula_resource.acquire_error_total` (existing)                  | Counter   | none currently                                      | `Manager::acquire_*` failure path           |

Plus existing `release_total`, `create_total`, `destroy_total` from `ResourceOpsMetrics` ([`metrics.rs:36-43`](../../../crates/resource/src/metrics.rs)) — preserved unchanged. Naming constants live in `nebula_metrics::naming::*` ([`metrics.rs:8-11`](../../../crates/resource/src/metrics.rs)) so the migration touches three sites: the `naming.rs` constants, the `ResourceOpsMetrics` struct field list, and the `record_*` methods.

**Histogram bucket discipline** (deferred to CP3 §11). The histogram bucketing is a config-level decision; CP2 commits to the metric *name* and *labels* but not the bucket boundaries. A reasonable default is `[0.001, 0.01, 0.1, 1.0, 10.0, 60.0]` seconds matching common rotation latency distributions.

**Cardinality.** `outcome` label has 3 values; no per-resource_key label on rotation metrics (cardinality bounded — see §6.4). Existing `acquire_total` / `acquire_error_total` are unlabeled today; if CP3 adds `topology` label, that's 5 values × 1 metric = 5 series.

### §6.3 `ResourceEvent` variant additions

Two new variants added to [`crates/resource/src/events.rs`](../../../crates/resource/src/events.rs); existing `HealthChanged` reused per B-2 (security amendment) with revocation-failure semantics — total event-shape delta is two new + one reused, not three new.

```rust
// crates/resource/src/events.rs — additions.

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    // ... existing variants preserved ...

    /// Aggregate event for one credential refresh dispatch cycle.
    /// Per-resource detail lives in `resource.credential_refresh.dispatch`
    /// tracing spans, not on the event payload (cardinality discipline).
    /// Outcome is the aggregated `RotationOutcome` from §3.5.
    CredentialRefreshed {
        credential_id: CredentialId,
        resources_affected: usize,
        outcome: RotationOutcome,
    },

    /// Aggregate event for one credential revocation dispatch cycle.
    /// Symmetric to `CredentialRefreshed`.
    CredentialRevoked {
        credential_id: CredentialId,
        resources_affected: usize,
        outcome: RotationOutcome,
    },
}
```

**Per-resource `HealthChanged` on revoke failure** (security amendment B-2). The existing `HealthChanged { key, healthy: false }` variant is *reused* — no new variant needed. Per §3.5 contract: aggregate `CredentialRevoked` always fires; per-resource `HealthChanged { healthy: false }` fires *only* for resources where `RevokeOutcome != Ok` (Failed or TimedOut). For successful revocations, the aggregate is sufficient — no per-resource event.

**Revocation health asymmetry.** Refresh emits 1 aggregate `CredentialRefreshed` + per-resource tracing spans (no per-resource events). Revocation emits 1 aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` events on the failure path. The asymmetry is the contract: refresh failure is recoverable (impl returns to healthy on next refresh attempt); revocation failure means the impl could not enforce the no-further-authenticated-traffic invariant — operators MUST see this per resource.

**`event()::key()` impl.** New variants need to extend `ResourceEvent::key()` at [`events.rs:91-106`](../../../crates/resource/src/events.rs). For `CredentialRefreshed` / `CredentialRevoked` the event has no resource key (it's credential-scoped, not resource-scoped) — the current pattern of `&'static fn key(&self) -> &ResourceKey` is no longer total. CP3 §12 picks: (a) `RegisterOptions::register_with_event_filter` per-consumer filter knob; or (b) crate-level filter trait that consumers implement. CP2 commits the *variants*; CP3 §12 must close the (a)/(b) trade-off **in the same wave that lands the new `ResourceEvent` variants** — they cannot land separately without breaking either the per-consumer filter contract (variants land first, no filter shape exists, every subscriber sees credential-scoped events whether or not they want them) or the broadcast cardinality budget (filter lands first, no variants exist, the filter API has no variants to filter against and ships dead).

### §6.4 Cardinality bounds

**Broadcast queue size: 256** (existing, [`manager.rs:275`](../../../crates/resource/src/manager.rs)). CP2 preserves. Slow subscribers receive `RecvError::Lagged` after the buffer wraps; this is observable and acceptable per Strategy §4.9.

**Tag cardinality.** `AcquireOptions::tags` is reserved-but-unused per [Phase 1 finding 🟠-8](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) and Strategy §5.2. CP2 introduces NO new tag-driven cardinality sources. Existing `outcome` label on rotation metrics caps at 3 values per metric; combined cardinality is small.

**Per-resource event cardinality bound.** For revocation, the per-resource `HealthChanged` events fire on the failure path. Theoretical upper bound: N resources sharing a credential where ALL fail revocation simultaneously emits N events on the broadcast channel. The channel capacity (256) bounds this — at N=256 simultaneous revoke failures, the aggregate `CredentialRevoked` + 256 `HealthChanged` exhausts the channel and the next event is `Lagged` for any subscriber that hasn't drained. **Operational guidance** (CP3 §11 finalises the runbook): if your deployment registers >256 resources against one credential, increase `event_tx` capacity via `ManagerConfig` (currently hardcoded; CP3 may surface as config). For 5 in-tree consumers × ~3 resources per consumer × 1 credential per resource group = ~15 expected, well under 256.

**No new cardinality from this redesign.** Strategy §5.2 defers `AcquireOptions::tags` wiring (#391); CP2 inherits the deferral. Three `outcome` enum values per metric × 5 metrics = 15 series; per-resource_key labels NOT added (would be unbounded with arbitrary registration). Future cardinality additions go through CP3 review.

### §6.5 DoD gate

CP4 ratification requires this checklist to be **demonstrably met** in the migration PR:

- [ ] Six trace spans (§6.1) emitted in the right places, with the right field names. Verifiable via `tracing-subscriber` test harness in CP3 §12 integration tests.
- [ ] Five counter metrics (§6.2) present in the `nebula_metrics::naming` constants list. `ResourceOpsMetrics` struct compiles with new fields. Increments fire in the right paths (verified via `MetricsRegistry` snapshot assertions in CP3 §12).
- [ ] Three `ResourceEvent` variants (§6.3) added; per-resource `HealthChanged { healthy: false }` fires on revocation failure path. Verifiable by subscribing to `event_tx` and asserting variant equality in integration tests.
- [ ] Cardinality bound documented in `crates/resource/docs/observability.md` (CP3 §13 deliverable — new file or extension to existing).
- [ ] No `ResourceEvent::CredentialRefreshed` / `CredentialRevoked` *without* the matching trace span + counter increment in the same code path. Cross-section consistency check at audit time.

Per `feedback_observability_as_completion.md`: **observability is DoD for a hot path, not a follow-up.** CP review hard-checks the gate; this is the section reviewers anchor to.

## §7 — Testing strategy

Per Strategy §6.3 + §4.9 observability gate. Compact subsection bullets; full per-test enumeration is CP3 §12.

### §7.1 Per-topology unit tests

**Coverage target: each of 5 topologies (Pool/Resident/Service/Transport/Exclusive) exercises the full lifecycle (§4) with at least one unit test.**

- **Pooled.** `register_pooled` → `acquire_pooled` (drain-tracker increments) → drop guard (recycle path) → `graceful_shutdown` graceful → registry empty. Existing test shape at [`tests/basic_integration.rs`](../../../crates/resource/tests/basic_integration.rs) covers; extend with the credential-bearing variant from §5.2.
- **Resident.** Single shared `Arc<R::Lease>` across acquires; verify `Released` event fires on every drop with `held: Duration > 0`; phase transitions through `Initialising → Ready → Draining → ShuttingDown` on shutdown.
- **Service.** `acquire_token` returns lease; `release_token` runs on drop; tainted tokens trigger `destroy` not `recycle` (token-lifecycle distinct from connection-lifecycle).
- **Transport.** `open_session` / `close_session` symmetry; verify `keepalive` doesn't fire on tainted handles. Transport currently has 0 Manager-level integration tests (Phase 1 🟠-13); CP3 §12 lands the first.
- **Exclusive.** Owned guard semantics; `reset` runs in `release_queue` on drop (not synchronously); guard's `tainted()` does NOT change `reset` behaviour (Exclusive has no recycle path).

Each test exercises trait reshape (§2.1) end-to-end: `type Credential = NoCredential` for Pool/Resident/Service/Transport/Exclusive *or* a `MockCredential` impl for one credential-bearing variant per topology. Spike validated this composition in `parallel_dispatch_crosses_topology_variants` ([`spike/.../resource-shape-test/src/lib.rs:583-607`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)).

### §7.2 Integration tests

**Extend [`crates/resource/tests/basic_integration.rs`](../../../crates/resource/tests/basic_integration.rs) (4114 lines current) with credential-rotation flows.** New tests:

- **`credential_refresh_drives_per_resource_swap`.** Register 3 resources of mixed topology against one `CredentialId`. Trigger `Manager::on_credential_refreshed(id, scheme)`. Verify each resource's `on_credential_refresh` was called once with the new scheme; verify `ResourceEvent::CredentialRefreshed { resources_affected: 3, outcome: RotationOutcome { ok: 3, .. } }` was broadcast.
- **`credential_revoke_drives_per_resource_taint_default`.** Register 2 resources with default `on_credential_revoke`. Trigger `Manager::on_credential_revoked(id)`. Verify subsequent `acquire_*` returns `Err(Error::credential_revoked(_))` (default-tainting per §5.3 option (b)).
- **`credential_revoke_failure_emits_health_changed`.** One resource overrides `on_credential_revoke` to return `Err`. Verify `ResourceEvent::HealthChanged { key, healthy: false }` is broadcast for that resource (security amendment B-2).
- **`drain_abort_records_failed_phase_not_ready`.** Per §5.5 wire test — verify `DrainTimeoutPolicy::Abort` lands `ResourcePhase::Failed`, not `Ready` (Phase 1 🔴-4 fix).

### §7.3 Property tests

- **Rotation idempotency.** Property: `Manager::on_credential_refreshed(id, scheme); Manager::on_credential_refreshed(id, scheme)` returns the same per-resource outcomes (modulo timing). Generated input: random N (1..32) of registered resources, all default `on_credential_refresh`. Tool: `proptest`.
- **Per-resource isolation invariant.** Property: for any subset S ⊆ registered resources where `dispatch_refresh` returns `Err`, the complement (R \ S) still returns `Ok` outcomes. Generated input: arbitrary `Result<(), MockError>` per resource. Validates security amendment B-1 at scale.
- **Drain-counter monotonicity.** Property: across any sequence of `acquire_pooled` / drop pairs, `drain_tracker.0` returns to 0 at the end. Generated input: arbitrary acquire/drop interleaving across N tasks. Already partially covered by existing tests.

### §7.4 Rotation-dispatch concurrency tests

Spike already validated the dispatch isolation with two integration tests:

- `parallel_dispatch_isolates_per_resource_latency` ([`spike/.../resource-shape-test/src/lib.rs:483-531`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) — 3 resources, one with 3s sleep, 250ms budget. Wall clock ~270ms (the budget) not 3s. Production carries forward as-is.
- `parallel_dispatch_isolates_per_resource_errors` ([`spike/.../lib.rs:537-578`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) — 3 resources, one returns `Err`, the rest return `Ok`. Outcomes reflect per-resource truth; sibling dispatches not poisoned.

CP2 commits both spike tests carry forward to production (re-implemented against the production `Manager` shape, not the spike `Manager` shape). New tests:

- **`parallel_dispatch_at_concurrency_cap_default`.** Register 32 resources (the default cap from §3.4); verify `join_all` does not OOM, all 32 outcomes return within ~budget × 1.1 wall-clock.
- **`parallel_dispatch_above_concurrency_cap_does_not_break`.** Register 64 resources (2× cap). The current shape uses unbounded `join_all`; verify it still completes (no `FuturesUnordered` cap yet — Strategy §4.3 deferral). Latency may be higher; correctness preserved.

Security-axis concurrency tests (per security-lead amendment SL-2 — empirical validation of the §5.3 / §3.5 / §3.2 contracts):

- **`revoke_during_inflight_acquire`.** Drive N in-flight `acquire_*` calls (mid-await on the runtime acquire) and call `Manager::on_credential_revoked` against the same `credential_id` while they are still pending. Verify all N in-flight acquires complete normally against the pre-revocation runtime (Strategy §4.2 + credential Tech Spec §4.3 lines 1062-1068 soft-revoke grace), but the next `acquire_*` after revoke completes returns `Err(Error::credential_revoked(_))` per §8.2 line 1505 wire.
- **`concurrent_refresh_plus_revoke`.** From two separate `tokio::spawn` tasks, fire `Manager::on_credential_refreshed(id, scheme)` and `Manager::on_credential_revoked(id)` against the same `credential_id`. Verify revocation is the final observable state regardless of dispatch ordering: `credential_revoked: AtomicBool == true` and subsequent `acquire_*` fails. The refresh outcome is observable via `RotationOutcome` but does NOT clear the revoke taint (revoke is final per Strategy §4.2 invariant).
- **`revoke_during_refresh`.** Fire `on_credential_revoked` while a refresh's blue-green pool swap (per §5.1) is in progress against the same resource. Verify the resource's pool is destroyed (post-revoke `acquire_*` fails) rather than refreshed (`AtomicBool` flip wins over the in-flight rebuild — the revoke's post-dispatch taint flip happens regardless of whether the parallel refresh override completes successfully).

### §7.5 Compile-fail probes

Four probes — three from spike + one new per CP1 Q5 resolution.

- (carried forward) `_wrong_refresh_signature_must_fail` — wrong-signature `on_credential_refresh` override rejected ([`spike/.../resource-shape-test/src/compile_fail.rs:11-95`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
- (carried forward) `_no_credential_scheme_is_inert_must_fail` — `NoScheme` cannot pretend to be `SecretToken` ([`spike/.../compile_fail.rs:96-115`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
- (carried forward) `_credential_bound_enforced_must_fail` — non-`Credential` type rejected ([`spike/.../compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
- **NEW** `_wrong_revoke_signature_must_fail` — wrong-signature `on_credential_revoke` override rejected. Symmetric to probe 1; required per CP1 §2.5 Q5 (the production-relevant gap not in the spike's three probes).

Probe location in production: `crates/resource/tests/compile_fail/` directory (`trybuild` integration). CP3 §12 specifies the harness wiring.

### §7.6 Coverage target

**Per-topology + per-Manager method.**

- **Per-topology**: each of 5 topologies has at least one full-lifecycle integration test (§7.1). Branch coverage on the topology's `Resource` impl methods: ≥ 80%.
- **Per-Manager method** (§3 surface): `register_inner`, `on_credential_refreshed`, `on_credential_revoked`, `graceful_shutdown` (all four `DrainTimeoutPolicy` × `ShutdownConfig` combinations), `warmup_pool`, `warmup_pool_no_credential`, `reload_config`. Branch coverage ≥ 80% per method; 100% for safety-critical paths (revocation default-tainting, abort-policy phase-failed).
- **Aggregate workspace target**: 80% line coverage on `crates/resource/src/` after migration. Tooling: `cargo tarpaulin` or `cargo llvm-cov` (CP3 §13 picks).

CP4 ratification gate: coverage report shows ≥ 80% on `crates/resource/src/manager/`, ≥ 80% on `crates/resource/src/topology/`, 100% on the security-critical paths enumerated in §6.5 DoD checklist.

## §8 — Storage / state

What Manager persists vs runtime-only. Compact subsections.

### §8.1 Persistent state

**None.** Manager is in-process only. No disk persistence, no database. All state is reconstructed at `Manager::new()` / `Manager::with_config()` time. The credential reverse-index, the registry, the drain tracker, the release queue, the cancel token — all in-memory.

This is not new for this redesign. Current Manager has no persistent state ([`manager.rs:247-265`](../../../crates/resource/src/manager.rs) struct fields are all `RwLock` / `DashMap` / `AtomicU64` / `broadcast::Sender` — none persist). CP2 preserves.

**Cross-cascade implication.** Credential persistence lives in `nebula-credential` (`credentials` table per credential Tech Spec §4); resource persistence does not exist. If a resource needs to persist *its* state (e.g., a long-lived workflow execution counter), that's the *consumer's* responsibility, not Manager's.

### §8.2 Runtime state ownership

`Registry` owns `Arc<dyn AnyManagedResource>` ([`crates/resource/src/registry.rs`](../../../crates/resource/src/registry.rs)). Each `ManagedResource<R>` owns:

- `R` (the user's `Resource` impl value).
- `arc_swap::ArcSwap<R::Config>` for hot-reload.
- `TopologyRuntime<R>` (an enum of pool/resident/service/transport/exclusive runtime state).
- `Arc<ReleaseQueue>` (clone of Manager's queue handle).
- `AtomicU64` generation counter.
- `arc_swap::ArcSwap<ResourceStatus>` for lock-free phase reads.
- Optional `AcquireResilience`, `Arc<RecoveryGate>`.

The reverse-index in CP2 owns `Arc<dyn ResourceDispatcher>` (§3.1 field type change), which internally holds `Arc<ManagedResource<R>>` — same ownership graph as today, just with an additional indirection through the dispatcher trampoline.

**Per-resource taint state** (§5.3). New atomic on `ManagedResource<R>`: `credential_revoked: AtomicBool`. Initialised `false`; flipped `true` by `Manager::set_revoked_for_dispatcher` after any `on_credential_revoke` dispatch (regardless of override or default). Read by `acquire_*` paths to early-fail with `Err(Error::credential_revoked(_))`.

### §8.3 Reverse-index lifetime

Lives with Manager. Created in `Manager::with_config` ([`manager.rs:283-296`](../../../crates/resource/src/manager.rs)), populated by `register_inner` (§3.1), cleared on `remove`/`shutdown`.

**`Manager::remove(key)` interaction.** Removing a registered resource MUST also remove its entry from the credential reverse-index. CP2 specifies: `remove` looks up the dispatcher entries that reference the about-to-be-removed `Arc<ManagedResource<R>>` and drops them from the `credential_resources` DashMap. Subsequent `on_credential_refreshed` calls do not see the removed resource. Implementation detail: dispatcher's `Arc<ManagedResource<R>>` weak-ptr would let the dispatcher self-remove on drop, but CP3 §11 picks the simpler eager-remove path (the registry knows the resource key; direct DashMap mutation).

**On `graceful_shutdown`.** Phase 3 CLEAR (§4.5 step 5) drops the registry; the reverse-index DashMap is also cleared at this step. After shutdown, no rotations dispatch to anything.

### §8.4 Generation counter discipline

`AtomicU64::new(0)` per `ManagedResource<R>` at register time ([`manager.rs:366`](../../../crates/resource/src/manager.rs)). Monotonic — only `fetch_add(1, Release)` per `reload_config` ([`manager.rs:1326-1328`](../../../crates/resource/src/manager.rs)). Read at `acquire_*` and guard-construction sites for ABA detection.

**`reload_config` bumps generation, then transitions phase to `Ready`** per existing pattern at [`manager.rs:1325-1335`](../../../crates/resource/src/manager.rs). CP2 preserves. **`on_credential_refresh` does NOT bump generation** — it's an orthogonal concern (config vs credential). Operators distinguishing config-reload from credential-refresh look at events (`ConfigReloaded` vs `CredentialRefreshed`), not generation.

**Memory ordering.** `Release` on increment, `Acquire` on read. Acquired guards capture generation at construction; if the manager subsequently `reload_config`s, the stale-generation guard is operationally identifiable but does NOT auto-invalidate. CP3 §11 specifies the guard-staleness escalation (today's pattern at [`guard.rs`](../../../crates/resource/src/guard.rs) `GuardInner::Guarded { generation }` records but does not enforce; CP2 preserves this conservative shape).

### §8.5 Cell + ArcSwap usage

**Lock-free phase reads.** `arc_swap::ArcSwap<ResourceStatus>` on every `ManagedResource<R>` ([`manager.rs:367`](../../../crates/resource/src/manager.rs)). Readers (health probes, `is_accepting`, drain tracking) load with `arc_swap::ArcSwap::load()` — no locks. Writers (phase transitions) `store(Arc::new(new_status))`.

**ArcSwap vs RwLock decision** (per §5.1 cross-ref). ArcSwap is correct for `ResourceStatus` because:
- Reads are FAR more frequent than writes (every `acquire_*` reads phase; phase changes only on lifecycle boundaries).
- Writers don't need to hold the lock across `await` (phase transition is a single `store`).
- Read-side does not need exclusion (reading "the latest" is sufficient).

**Pool swap uses RwLock instead** (§5.1) because writers DO need to hold across `await` (`build_pool_from_scheme(...).await?` inside the write guard).

**`config: ArcSwap<R::Config>`.** Same rationale as `ResourceStatus` — readers see the most recent config; writers `store(Arc::new(new))` synchronously. Hot-reload uses this shape per [`manager.rs:1318`](../../../crates/resource/src/manager.rs).

**No `RwLock<Pool>` on `ManagedResource<R>`.** The pool swap pattern (§5.1) is INSIDE the user's `Resource` impl (e.g., `PostgresPool::inner: Arc<RwLock<Pool>>`), not on `ManagedResource<R>`. Manager's owning structures stay lock-free; only the user-side runtime owns the swap-coordination lock.

## §9 — Manager file split (function-level cuts)

CP2 §5.4 locked the seven-submodule structure. CP3 §9 enumerates the per-`fn` cuts: which `impl Manager { fn ... }` block lands in which submodule, with current-line citations and post-split visibility. Public surface is preserved verbatim; submodule files re-export through `manager/mod.rs`.

**Method audit baseline.** `crates/resource/src/manager.rs` exposes 35 public methods on `Manager` plus 5 private helpers, totalling 2101 lines. The cuts trace existing internal seams (registration / dispatch / rotation / shutdown / gate / execute) per [Strategy §4.5](2026-04-24-nebula-resource-redesign-strategy.md) "split-by-state-shape, not split-by-line-count" discipline. `pub` visibility is preserved on every method whose current visibility is `pub`; `pub(crate)` is reserved for newly-extracted internal seams (the dispatcher trampoline, the registration funnel, the file-private helpers).

### §9.1 `manager/mod.rs` — type definition + constructors + lifecycle wiring

The hub. Holds `struct Manager`, the four constructor / lifecycle methods, and event subscription. All other submodules add `impl Manager` blocks; `mod.rs` is the *only* site that declares fields.

| Method                          | Current line       | Visibility post-split | Notes                                                     |
|---------------------------------|--------------------|-----------------------|-----------------------------------------------------------|
| `pub fn new`                    | [`manager.rs:269`](../../../crates/resource/src/manager.rs)   | `pub`                 | Wraps `with_config(ManagerConfig::default())` — unchanged |
| `pub fn with_config`            | [`manager.rs:274`](../../../crates/resource/src/manager.rs)   | `pub`                 | Field initialiser — sole writer of `Manager` fields       |
| `pub fn with_lifecycle`         | [`manager.rs:303`](../../../crates/resource/src/manager.rs)   | `pub`                 | Builder method — chaining preserved                       |
| `pub fn lifecycle`              | [`manager.rs:309`](../../../crates/resource/src/manager.rs)   | `pub`                 | Read-only accessor                                        |
| `pub fn subscribe_events`       | [`manager.rs:320`](../../../crates/resource/src/manager.rs)   | `pub`                 | Broadcast channel exposure                                |
| `pub fn cancel_token`           | [`manager.rs:1659`](../../../crates/resource/src/manager.rs)  | `pub`                 | Cancellation wiring                                       |
| `pub fn is_shutdown`            | [`manager.rs:1664`](../../../crates/resource/src/manager.rs)  | `pub`                 | Shutdown probe                                            |
| `pub fn contains`               | [`manager.rs:1636`](../../../crates/resource/src/manager.rs)  | `pub`                 | Registry membership probe                                 |
| `pub fn keys`                   | [`manager.rs:1641`](../../../crates/resource/src/manager.rs)  | `pub`                 | Registry key enumeration                                  |
| `pub fn recovery_groups`        | [`manager.rs:1646`](../../../crates/resource/src/manager.rs)  | `pub`                 | Recovery-group accessor                                   |
| `pub fn metrics`                | [`manager.rs:1652`](../../../crates/resource/src/manager.rs)  | `pub`                 | Metrics accessor                                          |
| `pub fn get_any`                | [`manager.rs:1692`](../../../crates/resource/src/manager.rs)  | `pub`                 | Type-erased lookup                                        |

**Field declarations** — verbatim from [`manager.rs:247-265`](../../../crates/resource/src/manager.rs); reverse-index field type changes from `Vec<ResourceKey>` to `Vec<Arc<dyn ResourceDispatcher>>` per §3.1. New CP3 field: `config: ManagerConfig` (currently inlined into individual fields; CP3 §9.1 reifies the struct so §9.5 timeout resolution can read `self.config.credential_rotation_timeout` directly per §3.3 line 899).

**No method bodies move into `mod.rs` from elsewhere.** Every `impl Manager { fn ... }` block in §9.2-§9.7 lives in its target submodule and re-exports through `pub use` if `pub`, or stays as `pub(crate)` if internal.

### §9.2 `manager/options.rs` — config + options + shutdown types

Public types only — no `Manager` methods. Lifts the type definitions out of `manager.rs:23-237` to a dedicated file. Re-exported through `manager/mod.rs` `pub use options::*` so `nebula_resource::ManagerConfig` etc. resolves unchanged.

| Type                       | Current line       | Visibility post-split | Notes                                                                |
|----------------------------|--------------------|-----------------------|----------------------------------------------------------------------|
| `ManagerConfig`            | [`manager.rs:193`](../../../crates/resource/src/manager.rs)   | `pub`                 | Gains `credential_rotation_timeout: Duration` + `credential_rotation_concurrency: usize` per §3.3 lines 850-854 |
| `RegisterOptions`          | [`manager.rs:220`](../../../crates/resource/src/manager.rs)   | `pub`                 | Gains `credential_rotation_timeout: Option<Duration>` per §3.3 line 879 + `credential_id: Option<CredentialId>` per §10.4 |
| `ShutdownConfig`           | [`manager.rs:85`](../../../crates/resource/src/manager.rs)    | `pub`                 | Unchanged                                                            |
| `DrainTimeoutPolicy`       | [`manager.rs:75`](../../../crates/resource/src/manager.rs)    | `pub`                 | Unchanged                                                            |
| `ShutdownError`            | [`manager.rs:158`](../../../crates/resource/src/manager.rs)   | `pub`                 | Unchanged                                                            |
| `ShutdownReport`           | [`manager.rs:138`](../../../crates/resource/src/manager.rs)   | `pub`                 | Unchanged                                                            |
| `ResourceHealthSnapshot`   | (currently `crate::manager::ResourceHealthSnapshot` re-export from a sibling file)   | `pub`                 | Stays in `state.rs`; `options.rs` does NOT take this type   |
| `DrainTimeoutError`        | [`manager.rs:187`](../../../crates/resource/src/manager.rs)   | `pub(crate)`          | Internal — used by `wait_for_drain` (§9.7)                          |

**Builder methods** on `ShutdownConfig` (`with_drain_timeout`, `with_drain_timeout_policy`, `with_release_queue_timeout` at [`manager.rs:115-132`](../../../crates/resource/src/manager.rs)) and matching builders to be added to `RegisterOptions` and `ManagerConfig` per CP3 §10.4 (the `RegisterOptions::with_credential_rotation_timeout` etc. builders) — all stay in `options.rs`. Builder pattern is maintained.

**`Default` impls** on each type (currently scattered across [`manager.rs:99-105, 207-213, 230-236`](../../../crates/resource/src/manager.rs)) move with the types.

**No `impl Manager` blocks in `options.rs`.** Pure type-definitions file; this is the cleanest cut in the seven-submodule split.

### §9.3 `manager/registration.rs` — register surface + reverse-index write

Houses every `register*` public method plus the §3.1 internal funnel `register_inner`. The reverse-index write path (Phase 1 🔴-1 fix) lives here exclusively — `mod.rs` does not write the DashMap.

| Method                            | Current line       | Visibility post-split | Notes                                                                |
|-----------------------------------|--------------------|-----------------------|----------------------------------------------------------------------|
| `pub fn register`                 | [`manager.rs:347`](../../../crates/resource/src/manager.rs)   | `pub`                 | Public funnel — calls `register_inner` (§3.1) + registry write       |
| `pub fn register_pooled`          | [`manager.rs:404`](../../../crates/resource/src/manager.rs)   | `pub`                 | `NoCredential` shortcut per §10.2 — bound `R::Credential = NoCredential`         |
| `pub fn register_resident`        | [`manager.rs:439`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape as `register_pooled`                                      |
| `pub fn register_service`         | [`manager.rs:468`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `pub fn register_exclusive`       | [`manager.rs:499`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `pub fn register_transport`       | [`manager.rs:530`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `pub fn register_pooled_with`     | [`manager.rs:561`](../../../crates/resource/src/manager.rs)   | `pub`                 | Credential-bearing path per §10.2 — accepts `RegisterOptions` (carries credential_id) |
| `pub fn register_resident_with`   | [`manager.rs:597`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `pub fn register_service_with`    | [`manager.rs:627`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `pub fn register_transport_with`  | [`manager.rs:659`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `pub fn register_exclusive_with`  | [`manager.rs:691`](../../../crates/resource/src/manager.rs)   | `pub`                 | Same shape                                                           |
| `fn register_inner<R>`            | NEW (§3.1)         | `pub(crate)`          | Reverse-index write funnel — file-private to `registration.rs` if no test access needed; `pub(crate)` if §9.5 rotation needs to call back |
| `pub fn lookup<R>`                | [`manager.rs:726`](../../../crates/resource/src/manager.rs)   | `pub`                 | Registry read — co-located with `register_inner` because both touch `registry` field directly |
| `pub fn remove`                   | [`manager.rs:1409`](../../../crates/resource/src/manager.rs)  | `pub`                 | Symmetric to `register` — same DashMap surface; reverse-index entry purge per §8.3 lives here |

**Why `lookup` lives here**: `lookup` is used by every `acquire_*` (§9.4) and every rotation dispatcher (§9.5) — the natural co-location is "the file that owns registry read/write." Moving `lookup` into `dispatch.rs` would require `dispatch.rs` to gain `pub(crate)` access to `registry`, which violates the §5.4 discipline.

**Atomic invariant restated.** Per §3.1 + §4.1 step 3-4, `register_inner` writes the reverse-index *before* `registry.register` writes the registry. Both lines live in `registration.rs`; no other submodule touches either DashMap.

### §9.4 `manager/dispatch.rs` — acquire surface (5 topologies × 4 variants)

20 acquire methods total: 5 topologies × {base credential-bearing, `_default` no-credential, `try_` non-blocking, `try__default` no-credential non-blocking}. Plus auxiliary `pool_stats` and `warmup_pool` / `warmup_pool_no_credential`.

| Method group                           | Current lines                  | Visibility | Notes                                                              |
|----------------------------------------|--------------------------------|------------|--------------------------------------------------------------------|
| `pub async fn acquire_pooled`          | [`manager.rs:752`](../../../crates/resource/src/manager.rs)              | `pub`      | Renamed parameter `auth: &R::Auth` → `credential: &<R::Credential as Credential>::Scheme` per §3.1 |
| `pub async fn acquire_pooled_default`  | [`manager.rs:811`](../../../crates/resource/src/manager.rs)              | `pub`      | `R::Credential = NoCredential` bound; passes `&NoScheme`           |
| `pub async fn acquire_resident*`       | [`manager.rs:833, 882`](../../../crates/resource/src/manager.rs)         | `pub`      | Same dual-shape per §10.3 unified vs dual decision                 |
| `pub async fn acquire_service*`        | [`manager.rs:904, 958`](../../../crates/resource/src/manager.rs)         | `pub`      | Same                                                               |
| `pub async fn acquire_transport*`      | [`manager.rs:980, 1034`](../../../crates/resource/src/manager.rs)        | `pub`      | Same                                                               |
| `pub async fn acquire_exclusive*`      | [`manager.rs:1056, 1109`](../../../crates/resource/src/manager.rs)       | `pub`      | Same                                                               |
| `pub async fn try_acquire_pooled*`     | [`manager.rs:1138, 1190`](../../../crates/resource/src/manager.rs)       | `pub`      | Non-blocking variant — Pool topology only                          |
| `pub async fn pool_stats`              | [`manager.rs:1217`](../../../crates/resource/src/manager.rs)             | `pub`      | Pool inspection                                                    |
| `pub async fn warmup_pool`             | [`manager.rs:1259`](../../../crates/resource/src/manager.rs)             | `pub`      | New signature per §5.2 — takes `credential: &<R::Credential as Credential>::Scheme` |
| `pub async fn warmup_pool_no_credential` | NEW (§5.2 line 1187)         | `pub`      | NoCredential bound; new method                                     |
| `pub fn reload_config`                 | [`manager.rs:1295`](../../../crates/resource/src/manager.rs)             | `pub`      | Hot-reload — touches both registry (read) and event_tx; co-locates with `acquire` because both consume `lookup` then `event_tx.send` |
| `fn record_acquire_result`             | [`manager.rs:1702`](../../../crates/resource/src/manager.rs)             | `pub(crate)` | Private metrics helper; called by every `acquire*` variant      |
| `pub fn health_check`                  | [`manager.rs:1674`](../../../crates/resource/src/manager.rs)             | `pub`      | Co-located with `acquire` — same lookup pattern                    |

**Per-method credential parameter rename.** Every credential-bearing `acquire_*` variant changes parameter `auth: &R::Auth` (current) to `credential: &<R::Credential as Credential>::Scheme` (post-redesign). The `Auth = ()` `where`-clause on the `_default` shorthands at [`manager.rs:813, 884, 960, 1036, 1111`](../../../crates/resource/src/manager.rs) becomes `R::Credential = NoCredential` per §10.2-§10.3 unified-vs-dual decision.

### §9.5 `manager/rotation.rs` — dispatcher trampoline + rotation handlers

The rotation core. Houses the §3.2 `ResourceDispatcher` trait + `TypedDispatcher<R>` impl, plus the `on_credential_refreshed` / `on_credential_revoked` `impl Manager` methods. Per-resource isolation, per-resource timeout enforcement, observability scaffolding (trace span, counter emission, event broadcast) all live here.

| Item                                  | Source                  | Visibility | Notes                                                              |
|---------------------------------------|-------------------------|------------|--------------------------------------------------------------------|
| `trait ResourceDispatcher`            | NEW (§3.2 line 697)     | `pub(crate)` | Internal trait; production test access via `#[doc(hidden)] pub use manager::ResourceDispatcher as __internal_ResourceDispatcher;` in `lib.rs` (§10.1 line 1783). Not part of the public API surface. |
| `struct TypedDispatcher<R>`           | NEW (§3.2 line 725)     | `pub(crate)` | Concrete dispatcher — never named outside the crate              |
| `impl<R: Resource> ResourceDispatcher for TypedDispatcher<R>` | NEW (§3.2 line 730) | (inherits) | Trait impl — Box::pin per dispatch (per Q3) |
| `pub async fn on_credential_refreshed` | [`manager.rs:1360`](../../../crates/resource/src/manager.rs) | `pub`      | Replaces `todo!()` body; signature gains `scheme: &(dyn Any + Send + Sync)` parameter per §3.6 line 978 |
| `pub async fn on_credential_revoked`   | [`manager.rs:1386`](../../../crates/resource/src/manager.rs) | `pub`      | Replaces `todo!()` body; symmetric to refreshed                    |
| `fn timeout_for`                      | NEW (§3.3 line 896)     | `pub(crate)` | Per-dispatcher timeout resolution                                |
| `fn dispatch_revoke_with_tainting`    | NEW (§5.3 line 1233)    | `pub(crate)` | Wraps `dispatch_revoke` + unconditional `set_revoked_for_dispatcher` |
| `fn set_revoked_for_dispatcher`       | NEW (§5.3 line 1243)    | `pub(crate)` | Atomic flip on `ManagedResource<R>::credential_revoked`            |
| `fn emit_refresh_event`               | NEW (§3.2 line 819)     | `pub(crate)` | Per-resource child-span emission + counter increment               |
| `fn emit_revoke_event`                | NEW (symmetric)         | `pub(crate)` | Per-resource child-span emission + B-2 HealthChanged on failure    |

**Trampoline trait visibility.** `ResourceDispatcher` is `pub(crate)` — *semantically* internal, declared with crate-only visibility. Production test access is provided exclusively through `#[doc(hidden)] pub use manager::ResourceDispatcher as __internal_ResourceDispatcher;` in `lib.rs` (§10.1 line 1783). CP2 §5.4 constrained the visibility to `pub(crate)`; CP3 §9.5 preserves that constraint and adds the `#[doc(hidden)]` re-export so the production test harness in CP3 §11.7 can name the trait via `nebula_resource::__internal_ResourceDispatcher` without bypassing module boundaries. The `__internal_` prefix and `#[doc(hidden)]` ensure the trait is excluded from rustdoc output and Cargo semver-checks; it is not part of the public API surface.

**Observability scaffolding co-location.** `emit_refresh_event` + `emit_revoke_event` + the trace span entry at §3.2 line 793 all live in `rotation.rs`. The counter increment (`record_rotation_attempts` per §3.2 line 825) lives on `ResourceOpsMetrics` (in [`crates/resource/src/metrics.rs`](../../../crates/resource/src/metrics.rs)) but is *called* from `rotation.rs`.

### §9.6 `manager/shutdown.rs` — graceful + force shutdown + drain-abort fix

Houses the four shutdown paths (`shutdown`, `graceful_shutdown`, `set_phase_all`, `wait_for_drain`) plus the §5.5 fix wiring `set_phase_all_failed` into the `DrainTimeoutPolicy::Abort` branch (Phase 1 🔴-4).

| Method                          | Current line       | Visibility post-split | Notes                                                              |
|---------------------------------|--------------------|-----------------------|--------------------------------------------------------------------|
| `pub fn shutdown`               | [`manager.rs:1430`](../../../crates/resource/src/manager.rs)  | `pub`                 | Force-shutdown — signals cancel token, drops registry              |
| `pub async fn graceful_shutdown` | [`manager.rs:1458`](../../../crates/resource/src/manager.rs) | `pub`                 | Phased drain with `set_phase_all_failed` fix on `Abort` branch     |
| `fn set_phase_all`              | [`manager.rs:1567`](../../../crates/resource/src/manager.rs)  | `pub(crate)`          | Phase transition helper — preserved unchanged                      |
| `fn set_phase_all_failed`       | NEW (§5.5 line 1289) | `pub(crate)`          | Per-resource `set_failed(err)` on Abort path; replaces `set_phase_all(Ready)` corruption |
| `async fn wait_for_drain`       | [`manager.rs:1592`](../../../crates/resource/src/manager.rs)  | `pub(crate)`          | Drain-tracker wait helper — co-locates with `graceful_shutdown` because both consume `drain_tracker` |

**`set_phase_all` vs `set_phase_all_failed` split.** The current single helper assumes "all transitions are uniform." The §5.5 fix introduces a per-resource error-carrying transition (`set_failed(err.clone())` per resource) that cannot be expressed as a uniform `set_phase_all(Phase)` call. The new helper iterates `registry.all_managed()` and calls `managed.set_failed(err)` per resource, emitting `HealthChanged { healthy: false }` per resource per [`events.rs:54-60`](../../../crates/resource/src/events.rs). The original `set_phase_all` is preserved for the `Force` policy (which still uses uniform transitions).

**Drain-tracker access.** `wait_for_drain` reads `self.drain_tracker.0.load(SeqCst)` and awaits `drain_tracker.1.notified()`. No other submodule touches `drain_tracker` — `acquire*` (§9.4) increments via `ResourceGuard::Drop`-driven RAII, but the increment itself is on `drain_tracker.0` directly through the guard. `shutdown.rs` is the only `impl Manager` site that reads it.

**`wait_for_drain` placement note (cross-ref).** CP2 §5.4 originally listed `wait_for_drain` in `manager/execute.rs`; CP3 §9.6 moves it here to `shutdown.rs` because it is consumed exclusively by `graceful_shutdown` and shares the `drain_tracker` field with no other site — see §9.7 below for the gate-vs-shutdown rationale.

### §9.7 `manager/gate.rs` — recovery gate + execute helpers

The recovery-gate helpers, the resilience-execution wrapper, and pool-config validation. Used by `dispatch.rs` (every `acquire*`) but lives in its own file because the helpers are a coherent internal seam ([`Strategy §4.5`](2026-04-24-nebula-resource-redesign-strategy.md): "trace existing internal seams").

| Item                          | Current line       | Visibility post-split | Notes                                                              |
|-------------------------------|--------------------|-----------------------|--------------------------------------------------------------------|
| `enum GateAdmission`          | (currently inline in [`manager.rs`](../../../crates/resource/src/manager.rs); line ~varies — check current source for exact range)  | `pub(crate)`          | Three-state admission enum (Open / Closed / Admitted)              |
| `fn admit_through_gate`       | (currently inline)  | `pub(crate)`          | Recovery-gate consultation — early-`Err` if gate closed            |
| `fn settle_gate_admission`    | (currently inline)  | `pub(crate)`          | Resolves ticket per result: `resolve` / `fail_transient` / `fail_permanent` |
| `fn execute_with_resilience`  | (currently inline)  | `pub(crate)`          | Wraps `acquire` body with timeout/retry/circuit-breaker             |
| `fn validate_pool_config`     | (currently inline)  | `pub(crate)`          | Pool-config validation — used by `register_pooled*`                 |
| `fn wait_for_drain`           | already in `shutdown.rs` (§9.6) | -      | NOT in `gate.rs` — moved per §9.6 because semantically it is a shutdown helper, not a gate helper |

**`wait_for_drain` placement.** CP2 §5.4 listed `wait_for_drain` in `manager/execute.rs`. CP3 §9 reconsiders: `wait_for_drain` consumes `drain_tracker` and is called only by `graceful_shutdown` — the natural co-location is `shutdown.rs`, not `execute.rs`. CP3 moves it to `shutdown.rs` (§9.6). This is a CP3 refinement of the CP2 cut, permissible per [§0.3 freeze policy](../specs/2026-04-24-nebula-resource-tech-spec.md) (function-arrangement is CP3 territory; CP2 locked file structure, not function placement).

**File rename.** CP2 §5.4 named the file `execute.rs`; CP3 §9.7 renames to `gate.rs` per the contents. The dominant shape of the file is the gate-admission state machine plus the execute-wrapper; `gate.rs` reads more honestly. `execute_with_resilience` lives alongside the gate helpers because the wrapper consumes `gate_admission` directly.

## §10 — Public API surface

The crate-level export list. Resolves Strategy §5.4 (NoCredential convenience symmetry) — CP3 §10.2 commits to **dual helpers** (no-cred shortcut + credential-bearing variant) over a single unified `RegisterOptions`-only path.

### §10.1 `lib.rs` re-exports

The full re-export tree post-redesign. Compare to current [`crates/resource/src/lib.rs:58-111`](../../../crates/resource/src/lib.rs). Departures from current shape:

- **`NoCredential` + `NoScheme` re-export added** (per §2.2 line 417). Imports from `nebula_credential`.
- **`Resource::Auth` removed**; `Resource::Credential` and `Credential` trait re-exported from `nebula_credential` for ergonomic adapter authoring.
- **`DaemonRuntime` / `EventSourceRuntime` re-exports REMOVED** per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md) (extraction).
- **`DaemonConfig` / `EventSourceConfig` REMOVED** (sibling extraction).
- **`Daemon` / `RestartPolicy` / `EventSource` topology trait + types REMOVED** (sibling extraction).
- **`RotationOutcome`, `RefreshOutcome`, `RevokeOutcome` ADDED** (§3.5).

```rust
// crates/resource/src/lib.rs (post-split, post-extraction).

// Existing re-exports preserved verbatim:
pub use cell::Cell;
pub use context::ResourceContext;
pub use error::{Error, ErrorKind, ErrorScope};
pub use events::ResourceEvent;
pub use ext::HasResourcesExt;
pub use guard::ResourceGuard;
pub use integration::{AcquireResilience, AcquireRetryConfig};
pub use manager::{
    DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions, ResourceHealthSnapshot,
    ShutdownConfig, ShutdownError, ShutdownReport,
    // NEW per CP3 §9.5:
    RotationOutcome, RefreshOutcome, RevokeOutcome,
};
pub use metrics::{ResourceOpsMetrics, ResourceOpsSnapshot};
pub use nebula_core::{ExecutionId, ResourceKey, ScopeLevel, WorkflowId, resource_key};
pub use nebula_resource_macros::{ClassifyError, Resource};
pub use options::{AcquireIntent, AcquireOptions};
pub use recovery::{
    GateState, RecoveryGate, RecoveryGateConfig, RecoveryGroupKey, RecoveryGroupRegistry,
    RecoveryTicket, RecoveryWaiter, WatchdogConfig, WatchdogHandle,
};
pub use registry::{AnyManagedResource, Registry};
pub use release_queue::ReleaseQueue;
pub use reload::ReloadOutcome;
pub use resource::{
    AnyResource, MetadataCompatibilityError, Resource, ResourceConfig, ResourceMetadata,
};

// NEW: NoCredential surface re-exported from nebula-credential
// per CP1 §2.2 + Q1.
pub use nebula_credential::{Credential, CredentialId, NoCredential, NoScheme};

// Topology runtime — Daemon and EventSource removed per ADR-0037.
pub use runtime::TopologyRuntime;
pub use runtime::{
    exclusive::ExclusiveRuntime,
    managed::ManagedResource,
    pool::{PoolRuntime, PoolStats},
    resident::ResidentRuntime,
    service::ServiceRuntime,
    transport::TransportRuntime,
};
pub use state::{ResourcePhase, ResourceStatus};

// Topology configurations — Daemon and EventSource removed per ADR-0037.
pub use topology::{
    exclusive::{Exclusive, config::Config as ExclusiveConfig},
    pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, config::Config as PoolConfig},
    resident::{Resident, config::Config as ResidentConfig},
    service::{Service, TokenMode, config::Config as ServiceConfig},
    transport::{Transport, config::Config as TransportConfig},
};
pub use topology_tag::TopologyTag;

// Internal seam exposed for production test access — NOT for adapter consumers.
#[doc(hidden)]
pub use manager::ResourceDispatcher as __internal_ResourceDispatcher;
```

**Module list change.** [`lib.rs:38-56`](../../../crates/resource/src/lib.rs) `pub mod` declarations: `manager` continues to be a single module declaration (file becomes a directory `manager/` per §9), no top-level changes. `runtime::daemon` and `runtime::event_source` modules deleted; `topology::daemon` and `topology::event_source` modules deleted. Engine-side modules (per §12) house the moved code.

### §10.2 `register_*` convenience methods — DUAL helpers (no-cred + credential-bearing)

**Decision: dual helpers — `register_pooled` (NoCredential shortcut) + `register_pooled_with` (credential-bearing path via `RegisterOptions`).** Same pattern across all five topologies: 5 × 2 = 10 helpers.

Strategy §5.4 framed the question as: "keep `Credential = NoCredential` shortcut, or require explicit `register_pooled::<R>(...)` with credential bound?" CP3 §10.2 commits to **keep the shortcut + add credential-bearing variant**, mirroring current `register_*` / `register_*_with` shape but with the no-cred bound made explicit.

**Rationale (three reasons):**

- **Migration parity.** Current 5 in-tree consumers all use `register_pooled` / `register_resident` etc. with `R::Auth = ()`. Atomic migration per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md) means changing the `Auth = ()` bound to `Credential = NoCredential` and renaming nothing else. A unified `register_pooled_with(RegisterOptions)`-only API would force every consumer to construct a `RegisterOptions` explicitly even for the no-credential trivial case — that's added boilerplate for the 60% of registrations that are unauthenticated caches / connections to local services.
- **DX symmetry with `acquire_*` / `acquire_*_default`.** §10.3 keeps the same pattern. Calling `register_pooled` without options means the matching `acquire_pooled_default` is also boilerplate-free. Forcing `RegisterOptions` everywhere desynchronises the acquire-side ergonomics.
- **Type-bound compile-time enforcement.** `register_pooled<R: Pooled<Credential = NoCredential>>` enforces no-cred at compile time. A `RegisterOptions::credential_id == None` runtime check would not catch credential-bearing R registered with no id at compile time — only at the next `register_inner` call (which now `Err`s per §3.1, but the failure surfaces at registration-call site, not import-time).

**Trade-off accepted.** 10 public methods on `Manager` instead of 5. Mitigated by: (a) the methods are mechanical thin wrappers — each is < 30 lines (current `register_pooled` at [`manager.rs:404-429`](../../../crates/resource/src/manager.rs) is 26 lines, post-rename stays at the same shape); (b) all 10 funnel through `register` + `register_inner` (§3.1) so the dispatch logic is single-sourced; (c) the doc comments form a clear pattern after the first one, so reading or maintaining the family is low-effort.

**The 10 methods** (5 topologies × {no-cred, cred-bearing}):

| Helper                            | Bound                                                              | Notes                                                              |
|-----------------------------------|--------------------------------------------------------------------|--------------------------------------------------------------------|
| `register_pooled<R>`              | `R: Pooled<Credential = NoCredential>`                             | Unchanged signature shape; bound updated from `Auth = ()` to `Credential = NoCredential` |
| `register_pooled_with<R>`         | `R: Pooled` (any credential)                                       | Accepts `RegisterOptions` carrying `credential_id: Option<CredentialId>` |
| `register_resident<R>`            | `R: Resident<Credential = NoCredential>`                           | Same                                                               |
| `register_resident_with<R>`       | `R: Resident`                                                      | Same                                                               |
| `register_service<R>`             | `R: Service<Credential = NoCredential>`                            | Same                                                               |
| `register_service_with<R>`        | `R: Service`                                                       | Same                                                               |
| `register_transport<R>`           | `R: Transport<Credential = NoCredential>`                          | Same                                                               |
| `register_transport_with<R>`      | `R: Transport`                                                     | Same                                                               |
| `register_exclusive<R>`           | `R: Exclusive<Credential = NoCredential>`                          | Same                                                               |
| `register_exclusive_with<R>`      | `R: Exclusive`                                                     | Same                                                               |

`register<R: Resource>` (the type-erased low-level method at [`manager.rs:347`](../../../crates/resource/src/manager.rs)) is preserved for callers that need to register a non-topology-specialised resource — it accepts `TopologyRuntime<R>` directly. Both no-cred and credential-bearing R can use it. The 10 dual helpers above are convenience over `register`.

**Total `register*` public surface = 11 methods** on `Manager`: the 10 topology-specialised helpers in the table above, plus the 1 low-level type-erased `register<R: Resource>`. The trade-off accounting earlier in this section ("10 public methods on `Manager` instead of 5") frames the dual-helper *delta* against a hypothetical unified-`RegisterOptions` baseline; the absolute count including the type-erased `register` is 11.

### §10.3 `acquire_*` paths — same dual pattern

Mirroring §10.2: 5 topologies × 4 variants = 20 acquire methods (with `try_*` only on Pool per current shape). Bound update mirrors §10.2:

| Pattern                                        | Bound after redesign                                | Replaces                                                       |
|------------------------------------------------|-----------------------------------------------------|----------------------------------------------------------------|
| `acquire_{topology}<R>(credential, ctx, opt)`  | `R: {Topology}` (any credential)                    | `auth: &R::Auth` → `credential: &<R::Credential as Credential>::Scheme` |
| `acquire_{topology}_default<R>(ctx, opt)`      | `R: {Topology}<Credential = NoCredential>`          | `where R::Auth = ()` → `where R::Credential = NoCredential`    |
| `try_acquire_pooled<R>(...)`                   | `R: Pooled` (any credential)                        | Same parameter rename                                          |
| `try_acquire_pooled_default<R>(...)`           | `R: Pooled<Credential = NoCredential>`              | Same                                                           |

`acquire_pooled_default` etc. delegate to the credential-bearing variant by passing `&NoScheme` literally:

```rust
pub async fn acquire_pooled_default<R>(
    &self,
    ctx: &ResourceContext,
    options: &AcquireOptions,
) -> Result<ResourceGuard<R>, Error>
where
    R: Pooled<Credential = NoCredential>,
    // ... other existing bounds ...
{
    self.acquire_pooled::<R>(&NoScheme, ctx, options).await
}
```

`NoScheme` is a zero-sized type per §2.2 line 351; passing `&NoScheme` is a single-byte reference with no runtime cost.

### §10.4 `RegisterOptions` final shape

Per CP1 §2.5 Q4 + CP3 §10.2 commit. Adds `credential_id: Option<CredentialId>` (read by `register_inner` per §3.1) and `credential_rotation_timeout: Option<Duration>` (per-resource override per §3.3).

```rust
// crates/resource/src/manager/options.rs (post-§9.2 split).

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RegisterOptions {
    pub scope: ScopeLevel,
    pub resilience: Option<AcquireResilience>,
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    /// Credential binding for this registration.
    ///
    /// REQUIRED when `R::Credential != NoCredential`. When `Some(id)`,
    /// `Manager` populates the credential reverse-index per §3.1 and the
    /// resource will receive `on_credential_refresh` / `on_credential_revoke`
    /// dispatches when the credential rotates or is revoked.
    ///
    /// Ignored (with `tracing::warn!`) when `R::Credential = NoCredential`.
    pub credential_id: Option<CredentialId>,
    /// Per-resource rotation-dispatch budget.
    ///
    /// `None` falls back to `ManagerConfig::credential_rotation_timeout`
    /// (default 30s). Override only when this resource has non-uniform
    /// rotation latency (e.g., remote pool with high handshake cost).
    pub credential_rotation_timeout: Option<Duration>,
}

impl Default for RegisterOptions { /* fields default to None / Global / etc. */ }

impl RegisterOptions {
    #[must_use]
    pub fn with_credential_id(mut self, id: CredentialId) -> Self {
        self.credential_id = Some(id);
        self
    }

    #[must_use]
    pub fn with_credential_rotation_timeout(mut self, timeout: Duration) -> Self {
        self.credential_rotation_timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn with_scope(mut self, scope: ScopeLevel) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub fn with_resilience(mut self, resilience: AcquireResilience) -> Self {
        self.resilience = Some(resilience);
        self
    }

    #[must_use]
    pub fn with_recovery_gate(mut self, gate: Arc<RecoveryGate>) -> Self {
        self.recovery_gate = Some(gate);
        self
    }
}
```

**`#[non_exhaustive]` discipline.** Prevents external callers from struct-literal construction; forces builder-pattern ergonomics. Future additions (e.g., `tainting_policy` if the §5.6 SL-1 gates clear) land as new builder methods, not breaking struct-literal patterns.

**No `tainting_policy` field in CP3 — see §10.5.**

### §10.5 `RegisterOptions::tainting_policy` — DEFERRED to post-CP3 (SL-1 gates not cleared)

**Decision: defer. `tainting_policy` does NOT enter the CP3 `RegisterOptions` surface.**

CP2 §5.6 (line 1320) recorded two gates required before `tainting_policy` knob ships: (1) a real in-tree consumer surfaces the multi-tenant exception (synthetic tests do not qualify); (2) a security-review hook in the surface review wave introducing it. CP3 §10 surface review confirms **neither gate has cleared**:

- **Gate 1 — real consumer.** The 5 in-tree consumers (`nebula-action`, `nebula-sdk`, `nebula-engine`, `nebula-plugin`, `nebula-sandbox`) all bind one credential per resource registration. None has a multi-tenant pool sharing one resource across many credentials. Phase 1 enumeration found no such pattern; Phase 4 spike did not surface one; CP1-CP2 review surfaced no consumer either. The §5.3 line 1252 trade-off — "zero in-tree consumers fit the multi-tenant exception today" — is still accurate at CP3.
- **Gate 2 — security-review hook.** No security-review hook is currently scheduled for the CP3 surface review wave. Adding `tainting_policy` would necessitate threading a security-review pass into the CP3 → migration PR landing path; CP3 ratification cadence does not include a fresh security-review touch (security-lead's work on this redesign closed at CP2 ratification per the [convergent-review pattern](../../../.claude/agent-memory-local/architect/feedback_convergent_review_edits.md)).

**Trade-off accepted.** Unconditional tainting per §5.3 option (b) remains the secure default. A future consumer that surfaces the multi-tenant exception triggers gate 1; a follow-up cascade re-engages security-review (gate 2) and lands `tainting_policy` as an additive `RegisterOptions` field via the `#[non_exhaustive]` builder pattern. Current consumers do not pay the cost of the un-needed knob; future consumers gain it without re-litigating §5.3.

**No premature knob.** Per `feedback_no_shims.md` discipline + §5.6 deferral commitment, CP3 ships unconditional tainting only. The `RegisterOptions` shape in §10.4 has no `tainting_policy` field.

## §11 — Adapter authoring contract

This section is the **content spec** for `crates/resource/docs/adapters.md` rewrite. The current doc ([`crates/resource/docs/adapters.md`](../../../crates/resource/docs/adapters.md)) fabricates `Resource::Auth = ()` (line 204) under a trait that no longer has `Auth`, references nonexistent adapter crates `nebula-resource-postgres` / `nebula-resource-redis`, and ships compile-failing examples (per [Phase 1 finding 🔴-3](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md), 50% fabrication rate). CP3 §11 specifies the rewrite content; the actual file rewrite is a Phase 8 deliverable (or implementation-PR scope).

### §11.1 Required imports

The minimum import set for an adapter crate. Listed by layer; comments explain *why* each is needed.

```rust
// Core trait + associated types.
use nebula_resource::{
    Resource, ResourceConfig, ResourceMetadata,    // trait + supporting types
    Error, ErrorKind,                              // unified error
    ResourceContext,                               // execution context (cancel, scope, accessor)
    ResourceKey, resource_key,                     // canonical key declaration
    AcquireOptions, RegisterOptions,               // call-site options (acquire / register)
    PoolConfig,                                    // Pool-topology config (used in §11.7 tests)
};

// Credential surface — REQUIRED. Either bind a real credential or opt out.
use nebula_credential::{Credential, NoCredential, NoScheme};
//                       ^^^^^^^^^^  ^^^^^^^^^^^^  ^^^^^^^^
//                       trait         opt-out      zero-sized scheme

// Schema declaration — required by `ResourceConfig`.
use nebula_schema::HasSchema;

// Topology trait — pick ONE per adapter.
use nebula_resource::Pooled;       // most database adapters
// OR Resident, Service, Transport, Exclusive (mutually exclusive).
```

**Imports referenced downstream.** `RegisterOptions` is consumed by `register_*_with` helpers (§10.2) when binding a credential at registration; `PoolConfig` is the Pool-topology configuration type (§11.7 integration tests use both). `NoScheme` is explicitly named in `&Self::Credential as Credential>::Scheme` projections (§11.2 line 2015, §11.4 line 2068) — keep imported even though Manager constructs it. `AcquireOptions` is used by every `acquire_*` call site (§11.7 line 2203).

**`NoCredential` location.** Imported from `nebula_credential` (per CP1 §2.5 Q1). `nebula_resource` re-exports for ergonomics (`pub use nebula_credential::{NoCredential, NoScheme};` per §10.1) — `use nebula_resource::NoCredential` also works. Authors writing adapter crates should import from `nebula_credential` directly (the canonical home); reasons: future adapter crates may not depend transitively on `nebula_resource` (e.g., a `nebula-credential-action` for engine-side credential injection), and importing from the type's home crate is clearer.

**No fabricated imports.** Current doc imports nonexistent items like `nebula_resource_postgres` (line 320) — DELETED in rewrite. The doc-rewrite acceptance gate is `cargo doc --all --no-deps` clean and `cargo test --doc` green for any non-`ignore` blocks.

### §11.2 Minimum `Resource` impl shape

Illustrative walkthrough using a `MockPostgresPool` adapter (an in-tree mock; no third-party driver dependency). The shape below is the structural skeleton — types, signatures, and the five "things to note" anchors. Some method bodies are abbreviated as `/* ... */` for narrative density (full bodies for `MockKvStore` and `MockHttpClient` mocks live in [`spike/.../resource-shape-test/src/lib.rs:125-200`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs) — those lines compile against trunk after the redesign and serve as the canonical compile-checked reference for adapter authors). The doc-rewrite acceptance gate (per §11.1) is `cargo test --doc` green for any non-`ignore` blocks; this block is annotated `rust,ignore` if rendered as a doc-test, since the placeholder bodies and the omitted `Pooled` impl are not directly compilable.

```rust,ignore
use nebula_credential::{Credential, NoCredential};
use nebula_resource::{
    Error, Resource, ResourceConfig, ResourceContext, ResourceKey,
    ResourceMetadata, resource_key,
};
use nebula_schema::HasSchema;

#[derive(Debug, Clone)]
pub struct MockPostgresConfig {
    pub host: String,
    pub max_size: u32,
}

impl HasSchema for MockPostgresConfig { /* schema derivation — see §11.1 */ }
impl ResourceConfig for MockPostgresConfig { /* validate / defaults */ }

#[derive(Debug, Clone)]
pub struct MockPostgresConnection { /* internal connection state */ }

#[derive(Debug, thiserror::Error)]
pub enum MockPostgresError {
    #[error("connection failed: {0}")]
    Connect(String),
}

impl From<MockPostgresError> for Error {
    fn from(e: MockPostgresError) -> Self { Error::transient(e.to_string()) }
}

pub struct MockPostgresPool;

impl Resource for MockPostgresPool {
    type Config = MockPostgresConfig;
    type Runtime = MockPostgresConnection;
    type Lease = MockPostgresConnection;
    type Error = MockPostgresError;
    // OPT-OUT — this adapter does not bind a credential.
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("mock-postgres")
    }

    async fn create(
        &self,
        config: &Self::Config,
        _scheme: &<Self::Credential as Credential>::Scheme, // = &NoScheme
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(MockPostgresConnection { /* construct from `config.host`, `config.max_size` */ })
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

// REQUIRED for Pool topology — adapter must also implement `Pooled` per §11.3.
// Not shown here; see [`spike/.../resource-shape-test/src/lib.rs:200+`](spike).
impl Pooled for MockPostgresPool { /* recycle / broken_check / ... */ }
```

**Five things to note.**

- **`type Credential` instead of `type Auth`.** The redesign replaces `Auth: AuthScheme` with `Credential: Credential` per [ADR-0036](../adr/0036-resource-credential-adoption-auth-retirement.md).
- **`scheme: &<Self::Credential as Credential>::Scheme` parameter.** The projected scheme — for `NoCredential`, it's `&NoScheme`. Always passed by reference; never owned (per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) hot-path borrow invariant).
- **No `R::Auth::default()` calls anywhere.** The `Auth: Default` bound is gone (per §5.2 line 1204). `NoScheme` IS `Default` (because `NoScheme` derives `Default` — see [`nebula_credential/src/no_credential.rs`](../../../crates/credential/src/no_credential.rs)), but adapters never construct it manually; Manager passes `&NoScheme` automatically through `acquire_pooled_default`.
- **`async fn` on impl side.** Per CP1 §2.1.1: trait declaration uses `impl Future<…> + Send` (RPITIT), but impl sites use `async fn` per clippy `manual_async_fn` lint default at 1.95+.
- **Default no-op rotation hooks.** `on_credential_refresh` + `on_credential_revoke` not overridden. Default body returns `Ok(())` per §2.1 line 197-198. For `NoCredential` resources, these dispatchers never fire (Manager skips reverse-index write per §3.1 line 638).

### §11.3 Topology selection guide

Five topologies remain post-extraction. Daemon and EventSource are no longer in `nebula-resource` per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md).

| Topology    | Shape                          | When to use                                                                 |
|-------------|--------------------------------|-----------------------------------------------------------------------------|
| `Pooled`    | N instances, checkout-on-acquire | Stateful connections (Postgres, Redis, gRPC channels). Most common choice. |
| `Resident`  | 1 instance, cloned on acquire  | Stateless / internally-pooled clients (`reqwest::Client`, AWS SDK, `tonic` channel). |
| `Service`   | 1 instance, token-mediated     | Token-bucket / rate-limited services where each acquire mints a fresh token. |
| `Transport` | 1 instance, session-mediated   | Transport-layer protocols (gRPC streaming, WebSocket, MQTT) where `open_session` returns a new logical session per acquire. |
| `Exclusive` | 1 instance, owned guard        | Resources with mutex semantics (file handles, single-writer DB connections). |

**If you previously used Daemon or EventSource**, see §12 — those topologies migrate to engine-side primitives (`DaemonRegistry` and `TriggerAction`-via-adapter).

**Common selection mistakes.**

- **Picking `Resident` for a single-connection stateful client.** If the client is genuinely single-connection and stateful (not internally pooled), `Exclusive` provides better mutual-exclusion semantics than `Resident`.
- **Picking `Pooled` for stateless HTTP.** `reqwest::Client` is internally pooled — wrapping it in `Pooled` adds redundant pool layers. Use `Resident`.
- **Picking `Service` for connection pools.** `Service` is for token-vending services (rate-limit semaphores), not connection-vending pools. `Pooled` is the right shape for "give me a connection, take it back when I'm done."

### §11.4 `type Credential = NoCredential;` opt-out walkthrough

For unauthenticated resources (caches, local services, in-memory stores):

```rust
use nebula_credential::{Credential, NoCredential, NoScheme};

impl Resource for InMemoryCache {
    type Credential = NoCredential;

    async fn create(
        &self,
        config: &Self::Config,
        _scheme: &NoScheme,                    // <Self::Credential as Credential>::Scheme = NoScheme
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(InMemoryCacheRuntime::new(config))
    }
}
```

**Three guarantees of `NoCredential` opt-out.**

- **Manager skips reverse-index write at register.** Per §3.1 line 627-636: `register_inner` checks `TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>()` and short-circuits. The resource will never receive `on_credential_refresh` / `on_credential_revoke` dispatches.
- **Compile-time enforcement.** `register_pooled<R: Pooled<Credential = NoCredential>>` rejects credential-bearing R at compile time; `register_pooled_with` accepts any credential. The wrong-method choice is caught by the bound, not by a runtime check.
- **Zero overhead.** `NoScheme` is a zero-sized type ([`nebula_credential/src/no_credential.rs:351`](../../../crates/credential/src/no_credential.rs)). Passing `&NoScheme` is a single-byte reference. The default `on_credential_refresh` / `on_credential_revoke` bodies are no-op and never reached.

**Common mistake.** Some authors write `type Credential = ();` (the unit type) by analogy with the old `type Auth = ();`. This does NOT compile — `()` does not implement `nebula_credential::Credential`. The compiler error names the missing trait bound; the §11.5 walkthrough makes this explicit. Compile-fail probe `_no_credential_scheme_is_inert_must_fail` (§7.5) carries this constraint forward.

### §11.5 Credential-bearing adapter walkthrough

A `RealPostgresPool` adapter that binds a real `Credential` (e.g., `PostgresCredential` from a hypothetical `nebula-credential-postgres` crate). Demonstrates the `<Self::Credential as Credential>::Scheme` projection.

```rust,ignore
// `nebula_credential_postgres`, `PostgresCredential`, `PostgresConnectionScheme`,
// `deadpool_postgres`, and `build_deadpool_from_dsn` are HYPOTHETICAL — no such
// adapter crate exists in the workspace today. The block below is a structural
// walkthrough of how the credential-bearing trait shape composes; it does not
// compile against trunk and is annotated `ignore` for that reason.

use nebula_credential::Credential;
use nebula_credential_postgres::{PostgresCredential, PostgresConnectionScheme};
use nebula_resource::{Error, Resource, ResourceConfig, ResourceContext, ResourceKey,
                       resource_key};

pub struct RealPostgresPool {
    inner: Arc<tokio::sync::RwLock<deadpool_postgres::Pool>>,
}

impl Resource for RealPostgresPool {
    type Config = PostgresConfig;
    type Runtime = deadpool_postgres::Object;
    type Lease = deadpool_postgres::Object;
    type Error = PostgresError;
    // CREDENTIAL-BOUND.
    type Credential = PostgresCredential;

    fn key() -> ResourceKey {
        resource_key!("postgres-real")
    }

    async fn create(
        &self,
        config: &Self::Config,
        scheme: &PostgresConnectionScheme,    // <PostgresCredential as Credential>::Scheme
        _ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        // Pull what is needed from the scheme. Do NOT clone the scheme onto self.
        let dsn = format!("postgresql://{}:{}@{}/{}",
            scheme.username(), scheme.password_redacted_str(), config.host, config.database);
        let pool = build_deadpool_from_dsn(&dsn).await?;
        let conn = pool.get().await?;
        Ok(conn)
    }
}
```

**Four invariants surfaced by this walkthrough** (per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) + [security review constraint #2](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)):

- **Borrow, do not clone.** `scheme: &PostgresConnectionScheme` is borrowed from the credential resolver. The impl pulls what it needs (DSN components) inside the await window; no `scheme.clone()` ever runs. Each clone is a zeroize obligation per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md).
- **No `Scheme::default()` in `create`.** Manager always supplies the resolved scheme; if the dispatcher fires, the scheme is real. Adapter never falls back to a default — there is no path where `create` runs with a stub scheme.
- **Manager NEVER holds the scheme.** The scheme reference lives on the dispatcher's stack only during `dispatch_refresh` / `create` execution. After the future resolves, the scheme reference is dropped; only the resource-side runtime (e.g., `RealPostgresPool::inner`) retains state, and that state is built from the scheme's contents (DSN, derived secrets) — not the scheme itself.
- **Pool swap on rotation.** §11.6 walkthrough.

### §11.6 `on_credential_refresh` / `on_credential_revoked` overrides — blue-green swap example

The canonical override pattern from [credential Tech Spec §3.6 lines 981-993](2026-04-24-credential-tech-spec.md), reproduced for the `RealPostgresPool` from §11.5.

```rust,ignore
// Continues the hypothetical `RealPostgresPool` from §11.5; same `ignore`
// rationale (no `nebula_credential_postgres` crate in trunk).

impl Resource for RealPostgresPool {
    // ... (associated types + create as in §11.5) ...

    async fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext<'a>,
    ) -> Result<(), Self::Error> {
        // Build the new pool OUTSIDE the lock — `build_pool_from_scheme` is async
        // and may take seconds (handshake, DNS, SSL). The Manager dispatches with
        // a budget (default 30s; per-resource override via RegisterOptions per §3.3).
        // SL-3 budget guidance: target ≤ 60% of dispatch timeout, leaving headroom
        // for swap installation + old-pool drop.
        //
        // `new_scheme` derefs to `&PostgresConnectionScheme` per credential Tech
        // Spec §15.7 line 3394-3429 `Deref<Target = Scheme>` impl. Pull what is
        // needed via `&*new_scheme` inside the await window; do NOT clone the
        // scheme onto the new pool (clone would be another zeroize obligation
        // per PRODUCT_CANON §12.5 + credential Tech Spec §15.7 `!Clone` discipline).
        let new_pool = build_pool_from_scheme(&*new_scheme).await?;

        // Acquire write lock, swap, release.
        let mut guard = self.inner.write().await;
        *guard = new_pool;
        // Old pool's RAII guards drain naturally as outstanding queries finish.
        // `new_scheme` zeroizes on Drop at end of scope (per credential Tech
        // Spec §15.7 line 3412 Drop ordering); `ctx` is borrowed.
        Ok(())
    }

    async fn on_credential_revoke(
        &self,
        _credential_id: &nebula_credential::CredentialId,
    ) -> Result<(), Self::Error> {
        // Override to taint outstanding handles + destroy pool synchronously.
        // Manager will ALSO flip the per-resource `credential_revoked` atomic
        // post-dispatch (per §5.3 option (b)) — your override augments, not replaces.
        let mut guard = self.inner.write().await;
        // Drop all idle connections; outstanding handles complete on their RAII drop.
        guard.close_idle().await;
        Ok(())
    }
}
```

**Three things to note about override semantics.**

- **Override augments, never replaces, Manager's enforcement.** Per §5.3 line 1227: "override is additive, not corrective." Your override does resource-specific cleanup (drain idle connections, log, etc.); Manager handles the invariant-enforcement layer (atomic taint flip). You do NOT need to flip `credential_revoked` yourself.
- **Idempotency.** Per §2.3 line 432: "Manager MAY retry under specific recovery flows." Pool-swap is naturally idempotent — re-publishing the same pool is a no-op.
- **Budget.** Per §5.1 SL-3: target your `build_pool_from_scheme` to complete in ≤ 60% of the dispatch budget (default 18s under the 30s default). `nebula_resource.credential_rotation_dispatch_latency_seconds` histogram (§6.2) monitors per-resource latency.

### §11.7 Testing your adapter

Three test layers. Compile-fail probes are crate-side (you do not write them); integration tests are adapter-side.

**1. Compile-fail probes (covered by `nebula-resource` itself).** §7.5 enumerates four trait-shape probes that prevent: wrong-signature `on_credential_refresh`, wrong-signature `on_credential_revoke`, `NoScheme` masquerading as a real scheme, non-`Credential` types in the bound. You inherit these — your adapter does not need its own.

**2. Integration tests (`tests/integration.rs`).** Standard pattern:

```rust
use nebula_core::ExecutionId;
use nebula_resource::{Manager, ResourceContext, AcquireOptions, PoolConfig};

#[tokio::test]
async fn register_and_acquire() {
    let manager = Manager::new();
    manager.register_pooled(
        MockPostgresPool,                  // R: Pooled<Credential = NoCredential>
        MockPostgresConfig::default(),
        PoolConfig::default(),
    ).expect("valid config registers");

    let ctx = ResourceContext::new(ExecutionId::new());
    let handle = manager
        .acquire_pooled_default::<MockPostgresPool>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire succeeds after register");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Pool);
}
```

For credential-bearing adapters, mock the credential resolver via `ResourceContext::with_credential_accessor(...)`; integration tests do NOT require a live credential store. CP3 §12 (in `tests/basic_integration.rs`) demonstrates the pattern.

**3. Rotation tests (credential-bearing adapters only).** Per §7.4: register your adapter with `Manager::register_pooled_with(R, config, opts.with_credential_id(cred_id))`, fire `manager.on_credential_refreshed(&cred_id, &new_scheme)`, assert your `on_credential_refresh` ran. Existing pattern in spike `parallel_dispatch_isolates_per_resource_errors` ([`spike/.../resource-shape-test/src/lib.rs:537-578`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) — carries forward to production tests.

### §11.8 Common pitfalls

- **Calling `Scheme::default()` inside `create`.** Don't. Manager always passes the resolved scheme; falling back to a default re-introduces the silent-empty-credential bug (Phase 1 🟡-17). The `Auth: Default` bound is gone; the type system already prevents this.
- **Cloning the scheme onto `self` or `Runtime`.** Don't. The scheme reference is borrowed for the lifetime of `create` / `on_credential_refresh`. Pull derived data (DSN string, token bytes) and let the borrow end.
- **Sharing pool state across credentials in multi-tenant scenarios.** If you implement `on_credential_revoke`, your default override semantics (Manager's unconditional taint flip per §5.3) terminate ALL traffic on this resource. If your adapter wants per-credential tenant isolation, see §10.5 — `RegisterOptions::tainting_policy` is deferred and not currently available.
- **Implementing `Daemon` or `EventSource` in your adapter.** These topologies have moved to the engine layer per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md). See §12 for the engine-side landing.
- **Forgetting `#[derive(Clone)]` on the resource struct.** `Manager::register*` requires `R: Clone + Send + Sync + 'static` for the topology variants that store the resource value (Pool, Service, Transport, Exclusive). `Resident` does not require `Clone` on R itself but does on `R::Lease`.
- **Returning `Err(Self::Error)` with the wrong `ErrorKind`.** Map driver errors to `Transient` (will retry), `Permanent` (give up), or `Exhausted` (rate limit). Use `ClassifyError` derive for ergonomic mapping.

## §12 — Daemon / EventSource extraction landing site

[ADR-0037 amended-in-place gate text](../adr/0037-daemon-eventsource-engine-fold.md) commits to engine-fold; CP3 §12 specifies the landing site. The engine-side implementation is a separate work item (engine team coordination); CP3 §12 is the *contract* the implementation honours.

### §12.1 Engine module path

**Decision: `crates/engine/src/daemon/`** — a new top-level engine module dedicated to long-running worker primitives. EventSource lands as `crates/engine/src/daemon/event_source.rs` (the module that adapts EventSource → existing TriggerAction substrate).

Two alternatives considered:

- **`crates/engine/src/runtime/daemon/`.** Sub-module of an existing `runtime` directory. **Rejected** — engine's existing `runtime` directory ([`crates/engine/src/runtime/`](../../../crates/engine/src/runtime/)) already houses execution-runtime shapes (handlers, dispatchers); Daemon is *long-running* state, not *execution-runtime* state. The conceptual seam is different. Co-locating would dilute the runtime module's purpose.
- **`crates/engine/src/scheduler/`.** Anticipating the future-cascade `nebula-scheduler` extraction (per [Strategy §6.5](2026-04-24-nebula-resource-redesign-strategy.md) future-cascade trigger). **Rejected** — pre-naming a module after a future crate that may never extract is speculative. `daemon/` is honest about today's contents; if the future cascade fires, the rename to `scheduler` is a single-PR mechanical rename per `feedback_no_shims.md` (no shim, no compat alias).

`crates/engine/src/daemon/` is the chosen path.

### §12.2 `DaemonRegistry` primitive

The engine-side equivalent of `nebula-resource::Manager`, scoped to long-running worker lifecycles. Manages `Daemon` impl lifecycle (start, stop, restart per `RestartPolicy`).

```rust
// crates/engine/src/daemon/registry.rs (NEW).

pub struct DaemonRegistry {
    daemons: dashmap::DashMap<DaemonKey, Arc<dyn AnyDaemonHandle>>,
    cancel: CancellationToken,
    event_tx: broadcast::Sender<DaemonEvent>,
}

impl DaemonRegistry {
    pub fn new() -> Self { /* ... */ }

    pub fn register<D: Daemon>(
        &self,
        daemon: D,
        config: D::Config,
        restart_policy: RestartPolicy,
    ) -> Result<(), DaemonError> { /* ... */ }

    pub async fn start_all(&self) -> Result<(), DaemonError> { /* ... */ }

    pub async fn stop_all(&self) -> Result<(), DaemonError> { /* ... */ }
}
```

**Source migration.** [`crates/resource/src/runtime/daemon.rs`](../../../crates/resource/src/runtime/daemon.rs) (493 LOC) and [`crates/resource/src/topology/daemon/`](../../../crates/resource/src/topology/daemon/) (`Daemon` trait + `RestartPolicy` enum + `DaemonConfig`) move to `crates/engine/src/daemon/`. Type-name preservation: `Daemon` trait stays `Daemon`; `RestartPolicy` stays `RestartPolicy`; `DaemonConfig` stays `DaemonConfig`. **Re-import path changes**: every consumer that wrote `use nebula_resource::Daemon;` rewrites to `use nebula_engine::Daemon;`.

`DaemonRegistry` is engine-internal — engine bootstrap (or applications using engine) constructs and consults it. Action and resource code does not touch `DaemonRegistry` directly.

### §12.3 EventSource → TriggerAction adapter signature

EventSource lands as a thin adapter onto engine's existing `TriggerAction` substrate ([`PRODUCT_CANON.md §3.5 line 82`](../PRODUCT_CANON.md), [`INTEGRATION_MODEL.md:99`](../INTEGRATION_MODEL.md)).

```rust
// crates/engine/src/daemon/event_source.rs (NEW).

/// Adapter that wraps an EventSource impl as a TriggerAction.
pub struct EventSourceAdapter<E: EventSource> {
    source: E,
    config: E::Config,
}

impl<E: EventSource> TriggerAction for EventSourceAdapter<E> {
    type TriggerEvent = E::Event;

    async fn subscribe(&self, ctx: &ActionContext) -> Result<EventStream<Self::TriggerEvent>, Error> {
        // Delegates to E::subscribe, converts E::Event into TriggerAction event stream.
        self.source.subscribe(ctx).await.map(|s| s.into())
    }
}
```

**Source migration.** [`crates/resource/src/runtime/event_source.rs`](../../../crates/resource/src/runtime/event_source.rs) (75 LOC) + [`crates/resource/src/topology/event_source/`](../../../crates/resource/src/topology/event_source/) (`EventSource` trait + `EventSourceConfig`) move to `crates/engine/src/daemon/event_source.rs`. The adapter shape (above) is new — engine's `TriggerAction` substrate already handles event streaming, so EventSource becomes an *adapter*, not a re-implementation.

**Re-import paths.** `use nebula_resource::EventSource;` → `use nebula_engine::EventSource;`. Same for `EventSourceConfig`.

### §12.4 Per-consumer migration steps

Five in-tree consumers; each touches Daemon / EventSource differently. Migration is mechanical per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md) atomic-PR-wave.

| Consumer            | Daemon usage                          | EventSource usage                        | Migration steps                                         |
|---------------------|----------------------------------------|------------------------------------------|---------------------------------------------------------|
| `crates/action/`    | None (verified — no `Daemon` import)   | None                                     | No-op. `nebula_resource` import for `Resource` only.    |
| `crates/sdk/`       | None (verified)                        | None                                     | No-op.                                                  |
| `crates/engine/`    | Self — engine becomes the *home* of `Daemon` + `EventSource` (§12.1-§12.3) | Same | Implementation work — see §12.1-§12.3 above. NEW module. |
| `crates/plugin/`    | None (verified)                        | None                                     | No-op.                                                  |
| `crates/sandbox/`   | None (verified)                        | None                                     | No-op.                                                  |

**Verification methodology.** `rg "Daemon|EventSource" crates/{action,sdk,plugin,sandbox}/src/` returns zero hits at CP3 draft time. Phase 1 evidence already established this (zero `Manager`-level Daemon/EventSource tests across consumers per [ADR-0037 Status section](../adr/0037-daemon-eventsource-engine-fold.md)).

**Implementation-time verification.** Migration PR re-runs the rg query before merge; if any consumer surfaces a Daemon/EventSource use during the migration wave, that consumer is added to the migration steps with the corresponding rewrite pattern.

### §12.5 `TopologyRuntime<R>` enum shrink (7 → 5)

Mechanical change. [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) currently:

```rust
pub(crate) enum TopologyRuntime<R: Resource> {
    Pool(PoolRuntime<R>),
    Resident(ResidentRuntime<R>),
    Service(ServiceRuntime<R>),
    Transport(TransportRuntime<R>),
    Exclusive(ExclusiveRuntime<R>),
    Daemon(DaemonRuntime<R>),         // REMOVED
    EventSource(EventSourceRuntime<R>), // REMOVED
}
```

Post-extraction:

```rust
pub(crate) enum TopologyRuntime<R: Resource> {
    Pool(PoolRuntime<R>),
    Resident(ResidentRuntime<R>),
    Service(ServiceRuntime<R>),
    Transport(TransportRuntime<R>),
    Exclusive(ExclusiveRuntime<R>),
}
```

**Match-arm sweep.** Every `match topology { ... }` site in `nebula_resource` removes the two arms. Current sites (rg `match.*topology` baseline): registration, dispatch, shutdown, `acquire_*` paths. The removal is mechanical — `cargo check` after the sibling enum-variant deletion surfaces every site.

**`reload_config` daemon special-case** at [`manager.rs:1346`](../../../crates/resource/src/manager.rs) (`TopologyRuntime::Daemon(_) => ReloadOutcome::Restarting`) is removed alongside the variant. `ReloadOutcome::Restarting` enum variant remains (engine-side daemons may still surface it via their own reload path); `nebula-resource`'s `reload_config` no longer emits it.

## §13 — Evolution policy

How the redesigned crate handles future change. Compact subsections; this is policy, not specification.

### §13.1 Versioning posture

**Pre-redesign**: `nebula-resource = frontier` per [`docs/MATURITY.md:36`](../../MATURITY.md). Design-stable, interfaces-stable, behavior-stable, observability-partial.

**Post-redesign target**: `core` (or `stable` per [`docs/MATURITY.md`](../../MATURITY.md) legend). Maturity bump conditional on §6.4 transition criteria from [Strategy §6.4](2026-04-24-nebula-resource-redesign-strategy.md):

- Zero 🔴 findings in new `nebula_resource.credential_rotation_attempts` counter `errors` label over the [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md) soak window (1-2 weeks).
- Phase 7 register shows zero unresolved `concerns: open` rows.
- Per-consumer tests pass against new shape ([Strategy §6.2 validation gate](2026-04-24-nebula-resource-redesign-strategy.md) closed).
- Documentation surface rebuilt per Strategy §4.7; dx-tester re-evaluation reports zero compile-fail walkthroughs.

Maturity bump proposed by architect at cascade-completion summary; ratified by tech-lead in dedicated PR per [`docs/MATURITY.md`](../../MATURITY.md) review cadence. CP4 §16 records the migration-PR completion handoff that triggers the proposal.

### §13.2 Breaking-change discipline

**No shims** (per `feedback_no_shims.md`). Future trait or method changes that break the public surface land as breaking changes — replace the wrong thing directly; do not add an adapter, bridge, alias, or feature-flag-gated old-shape compatibility layer. `nebula-resource` has zero external adopters per [Strategy §2.5](2026-04-24-nebula-resource-redesign-strategy.md); breaking changes are absorbed by the 5 in-tree consumers in atomic PR waves.

**No deprecated re-exports.** When a public type or method is removed, the `pub use` in `lib.rs` is removed in the same PR. No `#[deprecated(note = "use X instead")]` six-month transition periods — `frontier` and `core` maturity tiers both permit hard breaking changes per [`docs/MATURITY.md`](../../MATURITY.md).

The current `_with` builder cleanup (Phase 1 🔴-3 / Strategy §5.4 — replaced by §10.2 dual-helper decision in this CP3) demonstrates the discipline: rather than deprecate `register_pooled` and add `register_pooled_v2`, the redesign updates `register_pooled`'s bound from `Auth = ()` to `Credential = NoCredential`. One method, one shape, one PR.

### §13.3 Deprecation policy (post-`core`)

When `nebula-resource` reaches `core` maturity (§13.1), deprecation cadence tightens — `core` carries an "interfaces-stable" connotation that `frontier` does not.

- **Deprecation precedes removal.** `#[deprecated(note = "use X instead", since = "M.N.0")]` for one minor version cycle before removal. The `note` MUST name the replacement — bare `#[deprecated]` is rejected.
- **Removal in next minor.** After one minor cycle (e.g., deprecated in 1.5.0, removed in 1.6.0), the type or method is deleted. `lib.rs` re-export is removed atomically.
- **CHANGELOG entry required.** Every deprecation lands a CHANGELOG entry under `### Deprecated` for the deprecating release; every removal lands under `### Removed` for the removing release.
- **Pre-`core` exception.** While `nebula-resource` is `frontier` (current state), deprecation is OPTIONAL — hard breaking changes are permitted per [`docs/MATURITY.md`](../../MATURITY.md). The redesign is using this affordance.

### §13.4 Cross-crate boundary stability

`nebula-resource` consumes `nebula-credential` primitives (per CP1 §2.1 trait declaration). The dependency direction is **one-way**: `nebula-resource → nebula-credential`. Future evolution preserves this:

- **No reverse dependency.** `nebula-credential` MUST NOT depend on `nebula-resource`. Past tension on this boundary surfaced when adding `NoCredential` (Q1 resolution per CP1 §2.2 — `NoCredential` lives in `nebula_credential`, *not* `nebula_resource`).
- **Re-exports for ergonomic adapter authoring.** `nebula-resource::lib.rs` re-exports `Credential`, `CredentialId`, `NoCredential`, `NoScheme` (per §10.1) so adapter authors can `use nebula_resource::*` for the trait surface. The re-exports are convenience; they do NOT introduce a hidden dependency direction.
- **Engine-side dependency**. `nebula-engine` depends on `nebula-resource` per current `Cargo.toml` topology. Post-extraction (§12), engine also owns `Daemon` and `EventSource` — but does NOT depend on `nebula-resource` for them; those types live entirely engine-side.

**Future cascade considerations.** If a future cascade changes the credential surface (new associated types on `Credential` trait, new methods), `nebula-resource` absorbs the change atomically — see CP4 §15 forward-references for known credential-side cascades.

### §13.5 Public-surface freeze schedule

**During redesign cascade** (current state, through Phase 8 cascade-completion summary): `nebula-resource` is in design churn. Public-surface changes via Tech Spec amendment cycle ([§0.3](#03-freeze-policy-per-cp)).

**Post-cascade soak** (Strategy §6.3, 1-2 weeks): public surface FROZEN for evaluation. Any surface change during soak is a regression unless it closes a 🔴 finding — even then, fix lands in `main` and re-starts the soak window.

**Post-soak, post-`core`-bump**: public surface stable per §13.3 deprecation policy. Additions (new methods, new fields under `#[non_exhaustive]`) land via additive minor releases. Removals via the deprecation cycle.

**Cadence recap.**

| Phase                     | Cadence                                                               |
|---------------------------|-----------------------------------------------------------------------|
| In redesign cascade       | Tech Spec checkpoint amendments per §0.3                              |
| Post-cascade soak         | FROZEN (1-2 weeks); only 🔴 fixes permitted                          |
| Post-soak, `frontier`     | Hard breaking changes permitted; 5 in-tree consumers absorb atomically |
| Post-`core`-bump          | Deprecation cycle (one minor) + CHANGELOG discipline                  |

Future cascade triggers (per [Strategy §6.5](2026-04-24-nebula-resource-redesign-strategy.md)) — `Runtime`/`Lease` collapse, `AcquireOptions::intent/.tags` wiring, Service/Transport merge, Daemon sibling-crate spinout, AuthScheme: Clone revisit — each opens a new cascade and its own freeze schedule.

## §14 — Cross-references

Compact cross-reference subsections — every external claim in CP1-CP3 traces back to one of these tables. CP4 §14 is the audit surface for spec-auditor's claim-vs-source pass; if an §X.Y citation in CP1-CP3 cannot be resolved here, that is an §14 omission to fix, not a CP1-CP3 amendment.

### §14.1 Strategy refs (Strategy §4 → Tech Spec sections)

Strategy §4 carries the binding decisions ratified at Strategy CP3 freeze (2026-04-24). Each Tech Spec section that elaborates a Strategy §4 decision into compile-able shape:

| Strategy §            | Tech Spec section(s)                                                                  | What Tech Spec adds beyond Strategy            |
|-----------------------|---------------------------------------------------------------------------------------|------------------------------------------------|
| §4.1 trait reshape    | §2.1 (trait declaration), §11.2 (minimum impl walkthrough)                            | Compile-able Rust signatures, RPITIT lifetime  |
| §4.2 revocation       | §2.1 (default body), §5.3 (Manager-enforced taint), §11.6 (override walkthrough)      | Option (b) tainting + atomic flip mechanism    |
| §4.3 rotation dispatch| §3.2-§3.5 (dispatcher trampoline), §5.1 (pool swap), §7.4 (concurrency tests)         | `ResourceDispatcher` trait + per-resource timeout |
| §4.4 Daemon/EventSource extraction | §10.1 (re-export removal), §12 (engine landing site)                       | `crates/engine/src/daemon/` path + `DaemonRegistry` |
| §4.5 manager file-split | §5.4 (file structure), §9 (function-level cuts)                                     | Seven submodules + per-`fn` placement table   |
| §4.6 drain-abort fix  | §5.5 (set_failed wiring), §7.2 (assertion test)                                       | `set_phase_all_failed` helper + B-2 events     |
| §4.7 doc rewrite      | §11 (adapter authoring contract — content spec for `crates/resource/docs/adapters.md`) | 8 subsections covering imports → pitfalls    |
| §4.8 atomic migration | §16.1 (PR wave plan), §16.2 (per-consumer sequence)                                   | Concrete consumer order + DoD checklist        |
| §4.9 observability DoD| §6.1 (trace spans), §6.2 (counters), §6.3 (events), §6.5 (DoD gate)                  | Names + labels + bucket boundaries             |

Strategy §5 open items map to CP4 §15 resolutions (§15.1-§15.5). Strategy §6 post-validation roadmap maps to CP4 §16 implementation handoff (§16.1-§16.5).

### §14.2 ADR refs (ADR-0036 + ADR-0037 sections + amendment record)

| ADR section                                        | Tech Spec section that ratifies / elaborates                                       |
|----------------------------------------------------|------------------------------------------------------------------------------------|
| [ADR-0036 §Decision](../adr/0036-resource-credential-adoption-auth-retirement.md) (trait + hooks) | §2.1 (compile-able signatures) + §3.1 (reverse-index write path) + §3.2 (dispatcher trampoline) |
| [ADR-0036 §Consequences positive](../adr/0036-resource-credential-adoption-auth-retirement.md)    | §11.6 (blue-green swap walkthrough demonstrates safety claim)                      |
| [ADR-0036 §Alternatives 2 (sub-trait)](../adr/0036-resource-credential-adoption-auth-retirement.md) | §10.2 (dual-helper pattern keeps single trait — sub-trait still rejected)           |
| [ADR-0037 §Decision](../adr/0037-daemon-eventsource-engine-fold.md) (engine fold)                 | §12.1 (engine module path) + §12.2 (`DaemonRegistry`) + §12.3 (EventSource adapter) |
| [ADR-0037 amended-in-place 2026-04-25](../adr/0037-daemon-eventsource-engine-fold.md) (CP1 gate calibration) | §12.5 (`TopologyRuntime<R>` enum shrink) — the *decision* axis CP1 ratifies |

**ADR ratification path.** Both ADRs flipped from `proposed` to `accepted` at CP1 ratification per ADR-0036 §Status acceptance gate and ADR-0037 §Review amended gate. CP2-CP4 do not gate further ADR transitions; ADR amendments inside this cascade record any CP-discovered deltas via the "Amended in place on" pattern (ADR-0037 already records the CP1 calibration amendment; no further amendments required by CP4).

### §14.3 credential Tech Spec refs (§3.6 + §Credential::revoke + §4.3)

`nebula-resource` consumes `nebula-credential` primitives one-way. Cross-cascade coordination closed at Strategy §4.2 footnote — no credential-side spec extension required.

| credential Tech Spec section                                                                  | Tech Spec consumer site(s)                                              |
|-----------------------------------------------------------------------------------------------|--------------------------------------------------------------------------|
| [§3.6 lines 928-996](2026-04-24-credential-tech-spec.md) (`on_credential_refresh` shape)     | §2.1 (trait declaration adopts §3.6 verbatim), §11.6 (override pattern)  |
| [§3.6 lines 935-955](2026-04-24-credential-tech-spec.md) (associated-type shape)             | §2.1 (`type Credential: Credential` + projection)                        |
| [§3.6 lines 961-993](2026-04-24-credential-tech-spec.md) (blue-green pool swap)              | §5.1 (pool-swap mechanism) + §11.6 (walkthrough)                         |
| [§Credential::revoke line 228](2026-04-24-credential-tech-spec.md) (`async fn revoke`)       | §2.1 (revocation default-body invariant — credential side fires the call) |
| [§4.3 lines 1062-1068](2026-04-24-credential-tech-spec.md) (revocation lifecycle modes)      | §5.3 (Manager-enforced taint matches credential's hard-revocation mode)  |

`Credential` and `NoCredential` types are imported, not extended — the `Credential` trait surface used by `Resource::Credential` bound is the one declared in [`crates/credential/src/credential.rs`](../../../crates/credential/src/credential.rs); no new trait method or associated type proposed by this Tech Spec.

### §14.4 Phase 1 finding map (each 🔴/🟠 → Tech Spec section that resolves it)

[Phase 1 §4 severity matrix](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) enumerates 28 findings; each in-scope finding has a Tech Spec section pointer below. Out-of-scope (deferred-with-pointer / future-cascade / accepted-as-is) per Strategy §0 are listed separately.

| Phase 1 ID | Severity | Finding                                                                | Resolved by                                                |
|------------|----------|------------------------------------------------------------------------|------------------------------------------------------------|
| 🔴-1       | 🔴       | Credential×Resource seam: silent revocation drop + latent panic         | §3.1 reverse-index write + §3.2 dispatcher + §5.3 tainting |
| 🔴-2       | 🔴       | Daemon orphan-surface (no public start path)                            | §12 engine extraction (Daemon migrates out of resource)    |
| 🔴-3       | 🔴       | `docs/api-reference.md` ~50% fabrication + `adapters.md` compile-fails  | §11 adapter contract content spec (rewrite payload)        |
| 🔴-4       | 🔴       | Drain-abort phase corruption (`Abort` flips to Ready)                   | §5.5 `set_phase_all_failed` + §7.2 assertion test          |
| 🔴-5       | 🔴       | `Resource::Auth` dead bound (100% `()` usage)                           | §2.1 `type Auth` removed + §10.2 `Credential = NoCredential` |
| 🔴-6       | 🔴       | EventSource same orphan-surface as Daemon                                | §12 engine extraction (EventSource→TriggerAction adapter)  |
| 🟠-7       | 🟠       | `register_*_with` builder + 2101-line file                               | §9 file-split + §10.2 dual-helper pattern                  |
| 🟠-8       | 🟠       | `AcquireOptions::intent/.tags` reserved-but-unused                       | §15.2 (deferred per Strategy §5.2)                         |
| 🟠-9       | 🟠       | Daemon + EventSource out-of-canon §3.5                                   | §12 engine extraction (canon §3.5 alignment)               |
| 🟠-10      | 🟠       | No `deny.toml` wrappers rule (SF-1)                                     | Standalone-fix devops PR per Strategy §0                   |
| 🟠-11      | 🟠       | 5-assoc-type friction; 9/9 tests prove `Runtime == Lease` unused        | §15.3 (future-cascade per Strategy §5.3)                   |
| 🟠-12      | 🟠       | `register_pooled` silently requires `Auth = ()`                          | §2.1 + §10.2 `register_pooled<R: Pooled<Credential = NoCredential>>` |
| 🟠-13      | 🟠       | Transport topology — 0 Manager-level integration tests                  | Post-cascade test debt (Strategy §6.3 follow-up issue)     |
| 🟠-14      | 🟠       | Missing observability on credential rotation path                        | §6 observability (trace + counter + event) + §6.5 DoD gate |
| 🟠-15      | 🟠       | `Credential` vs `Auth` 3-way doc contradiction                          | §11 adapter contract (single-source `Credential` naming)   |

🟡-grade and 🟢-grade findings: §14.4-cont. coverage in [Phase 7 register](../tracking/nebula-resource-concerns-register.md) per §14.5.

### §14.5 Concerns register link

[`docs/tracking/nebula-resource-concerns-register.md`](../../tracking/nebula-resource-concerns-register.md) is the canonical lifecycle tracker for all 35 concerns (28 Phase 1 findings + Strategy decisions + Phase 0 infrastructure rows). CP4 §15.6 enumerates the status flips for the 22 `tech-spec-material` rows.

Register lifecycle (per register §"Lifecycle rules"): `tech-spec-material` rows must transition out of `open` status before CP4 freeze — this happens via §15.6. Closure of the register itself ties to MATURITY transition `frontier` → `core` per Strategy §6.4 (referenced in §13.1 + §16.5).

### §14.6 Spike artefact link

[`docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/`](../drafts/2026-04-24-nebula-resource-redesign/spike/) carries the Phase 4 iter-1 PASS artefacts that ground every CP1-CP3 trait-shape claim. Key files referenced inline:

- [`spike/.../resource-shape-test/src/lib.rs:125-156`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs) — `MockKvStore` (NoCredential, no topology) — referenced by §11.2 fall-back.
- [`spike/.../resource-shape-test/src/lib.rs:158-200`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs) — `MockHttpClient` (Resident, NoCredential) — second compile-checked baseline.
- [`spike/.../resource-shape-test/src/lib.rs:483-531`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs) — `parallel_dispatch_isolates_per_resource_latency` — carries forward to §7.4 production tests.
- [`spike/.../resource-shape-test/src/lib.rs:537-578`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs) — `parallel_dispatch_isolates_per_resource_errors` — security B-1 isolation invariant validation.
- [`spike/.../resource-shape-test/src/compile_fail.rs`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs) — three of four production compile-fail probes (§7.5) carry forward verbatim.
- [`spike/NOTES.md`](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md) — exit-criteria record (7/7 PASS, iter-2 deferred per Phase 4 closure).

Spike worktree status: artefacts archival post-cascade; no further compilation expected against trunk after Tech Spec ratification (consumer migration happens against production code, not spike code).

## §15 — Open items resolution

Each Strategy §5.x open item gets a CP4 §15.x resolution with a section pointer to where it closes. Strategy §0 freeze policy permits Tech Spec to close §5 items with explicit "extends Strategy §X.Y" annotation; closure does not require Strategy amendment when the closure resolves an open question rather than reversing a §4 decision.

### §15.1 Strategy §5.1 — Daemon revisit triggers (concretized)

**Status: closed-with-trigger.** Per [ADR-0037 amended-in-place 2026-04-25 gate text](../adr/0037-daemon-eventsource-engine-fold.md), the engine-fold *decision* is locked at CP1; the engine-side *implementation* (module path, `DaemonRegistry` shape, adapter signature) is CP3 §13 deliverable, all of which §12 specifies. Strategy §5.1's "trigger to revisit sibling-crate" framing concretizes as:

- **Trigger 1 (LOC threshold):** Daemon-specific engine code in `crates/engine/src/daemon/` grows beyond ~500 LOC (current sum: 493 daemon + 75 EventSource = 568 LOC; threshold conservative against current size).
- **Trigger 2 (proliferation):** ≥2 non-trigger long-running workers materialize that don't fit the existing `DaemonRegistry` + `EventSourceAdapter` shape (e.g., a scheduler-shaped primitive distinct from both).
- **Trigger fire path:** opens a new cascade per Strategy §0 amendment cycle; the new cascade considers `nebula-scheduler` extraction from engine-side. Does NOT re-route through `nebula-resource` per ADR-0037 §Consequences neutral.

This Tech Spec does NOT propose immediate sibling-crate extraction. §12.1 records the rejection of `crates/engine/src/scheduler/` pre-naming for the same reason.

### §15.2 Strategy §5.2 — `AcquireOptions::intent/.tags` (final treatment: deprecate)

**Status: decided — `#[deprecated]` (not remove, not retain).**

Strategy §5.2 framed three options: (a) `#[doc(hidden)]`, (b) `#[deprecated(note = "#391 not wired")]`, (c) retain unchanged. Per Strategy §5.2 cross-ref to `PRODUCT_CANON.md §4.5` + `feedback_incomplete_work.md`, **option (b) is more honest** than the alternatives:

- **(c) retain** — false-capability per canon §4.5; reserved-but-unused public API is what the redesign closes.
- **(a) `#[doc(hidden)]`** — hides the field from docs but leaves it construct-able; doesn't surface to callers that the field is non-functional.
- **(b) `#[deprecated]`** — actively warns callers; pairs with engine-side ticket #391's eventual resolution. If #391 lands, deprecation is removed; if #391 dies, `AcquireOptions::intent` and `AcquireOptions::tags` are removed in a follow-up minor (per §13.3 deprecation policy once the crate hits `core`).

**Implementation in Tech Spec:** [`crates/resource/src/options.rs:17-64`](../../../crates/resource/src/options.rs) `AcquireOptions::intent` and `AcquireOptions::tags` fields gain `#[deprecated(note = "engine integration ticket #391 not yet wired; field is reserved but does not affect acquire dispatch")]`. Lands in the migration PR wave (per §16.1). Future cascade closes (per Strategy §6.5 trigger: ticket #391 either wires or formally dies).

### §15.3 Strategy §5.3 — `Runtime`/`Lease` collapse (future-cascade trigger confirmed)

**Status: deferred — future cascade trigger confirmed.**

Strategy §5.3's trigger framing held through Phase 4-6: any consumer that sets `Runtime != Lease` during spike, Tech Spec drafting, or post-cascade implementation triggers a future cascade. CP1-CP3 evidence:

- **Spike (Phase 4)** — all spike test resources set `type Lease = Runtime;` (verified across `MockKvStore`, `MockHttpClient`, `MockPgPool`, etc. in [`spike/.../resource-shape-test/src/lib.rs`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)). No spike resource fired the trigger.
- **Tech Spec (CP1-CP3)** — every walkthrough (§11.2, §11.5) sets `Lease = Runtime` (or analogue). No Tech Spec example fires the trigger.
- **Production trunk** — Phase 1 §2.3 verified 9/9 tests + 5/5 production resources set `Lease = Runtime`. CP4 verification: no §9 method or §11 example departs from this.

**Trigger fire path (unchanged from Strategy §5.3):** any future consumer that genuinely needs `Lease != Runtime` (i.e., the lease-side type carries reduced privileges or wraps the runtime) opens a future cascade per Strategy §6.5; ADR candidate at trigger fire. Open in [register row R-050](../../tracking/nebula-resource-concerns-register.md).

### §15.4 Strategy §5.4 — `NoCredential` convenience symmetry (closed via §10.2 dual-helper)

**Status: closed — resolved by §10.2 dual-helper pattern.**

Strategy §5.4 framed the question as: "keep `Credential = NoCredential` shortcut, or require explicit `register_pooled::<R>(...)` with credential bound?" CP3 §10.2 commits to the dual-helper pattern: 5 topologies × {`register_pooled<R: Pooled<Credential = NoCredential>>`, `register_pooled_with<R: Pooled>`} = 10 dual helpers, plus the type-erased `register<R: Resource>` (per CP3 §10.2 line 1817 amended for E3 = 11 total). The trade-off (10 public methods vs unified `RegisterOptions`-only path) is recorded with three reasons (migration parity, DX symmetry with `acquire_*_default`, compile-time enforcement) and one accepted cost (10 thin wrappers; mitigated by single-sourcing through `register` + `register_inner`).

CP3 §10.2 closes the open item; no further resolution required. dx-tester ratification in CP3 review confirms (or surfaces alternative; see CP3 review handoff in §16.4).

### §15.5 Strategy §5.5 — revoke spec extension (closed)

**Status: closed at Strategy §4.2 footnote (CP2 tech-lead E4) + reaffirmed via CP1 Q5 + CP2 §5.3.**

Strategy §5.5 originally asked whether `nebula-credential` Tech Spec needs an extension to support `Resource::on_credential_revoke`. Tech-lead ratification at Strategy CP2 (E4) closed the question: credential Tech Spec already provides `Credential::revoke` ([line 228](2026-04-24-credential-tech-spec.md): `async fn revoke(ctx, state) -> Result<(), RevokeError>`) and revocation lifecycle modes ([§4.3 lines 1062-1068](2026-04-24-credential-tech-spec.md): soft/hard/cascade revocation, `state_kind = 'revoked'` semantics). The resource-side `on_credential_revoke` hook is a *consumer* of those existing primitives.

- **CP1 Q5** confirmed the revoke-side trait method shape (`on_credential_revoke(&self, credential_id: &CredentialId)` — symmetric to `on_credential_refresh` per §2.1 line 197-198) without requiring credential-side changes.
- **CP2 §5.3** locked the Manager-side default-tainting mechanism (option (b) unconditional taint flip on revocation) — operates against the credential's existing revocation primitives.
- **Tech Spec §14.3** (this CP4) records the cross-spec one-way-dependency: `nebula-resource → nebula-credential`; reverse direction explicitly forbidden per §13.4.

No spec extension required, no cross-cascade coordination round needed. Strategy §5.5 is not re-opened by CP4.

### §15.6 Concerns register status flips (22 tech-spec-material rows)

Per [register §"Lifecycle rules"](../../tracking/nebula-resource-concerns-register.md) rule 2: "tech-spec-material items must be addressed (status not 'open') before Phase 6 Tech Spec CP4 freeze." Status flips below for all 22 `tech-spec-material` rows; flip to `decided` with section pointer:

| ID    | Concern                                                              | Status flip                                  | Section pointer              |
|-------|----------------------------------------------------------------------|----------------------------------------------|------------------------------|
| R-001 | `Resource::Auth` dead bound; `Resource::Credential` adoption          | open → decided                               | §2.1 + §11.2 + ADR-0036      |
| R-002 | Reverse-index never populated; latent `todo!()` panic                 | open → decided                               | §3.1 + §5.3 + §14.4 row 🔴-1 |
| R-003 | `on_credential_revoked` semantics                                     | open → decided                               | §2.1 + §5.3 + §11.6          |
| R-004 | Rotation dispatch mechanics — parallel + per-resource timeout         | open → decided                               | §3.2-§3.5 + §7.4             |
| R-005 | `warmup_pool` must not call `Scheme::default()`                       | open → decided                               | §5.2 (warmup signature)      |
| R-010 | Daemon topology no public start path                                  | open → decided                               | §12 (engine extraction)      |
| R-011 | EventSource orphan-surface pattern                                    | open → decided                               | §12.3 (EventSourceAdapter)   |
| R-012 | Daemon + EventSource out-of-canon §3.5                                | open → decided                               | §12 + §10.1 (re-export removal) |
| R-020 | `manager.rs` 2101 L grab-bag                                          | open → decided                               | §5.4 + §9 (file-split)       |
| R-021 | `register_*_with` builder anti-pattern                                | open → decided                               | §9.3 + §10.2 (dual-helper)   |
| R-022 | `register_pooled` silently requires `Auth = ()`                       | open → decided                               | §2.1 + §10.2                 |
| R-023 | Drain-abort phase corruption                                          | open → decided                               | §5.5 + §7.2 + §9.6           |
| R-030 | `docs/api-reference.md` ~50% fabrication                              | open → decided (content-spec)                | §11 adapter contract         |
| R-031 | `docs/adapters.md` compile-fails                                      | open → decided (content-spec)                | §11 adapter contract         |
| R-032 | `docs/Architecture.md` describes vanished v1                          | open → decided (rewrite-or-delete deferred to migration PR per Strategy §4.7) | §11 + §16.1 |
| R-033 | `docs/README.md` case-drift broken intra-doc links                    | open → decided (migration PR scope)          | §16.1                        |
| R-034 | `docs/dx-eval-real-world.rs` references nonexistent type              | open → decided (CI-gate deferred to §13)     | §13 + §16.1                  |
| R-035 | `docs/events.md` variant count 7 vs actual 10                         | open → decided (migration PR scope)          | §16.1                        |
| R-043 | Macros emit `DeclaresDependencies` for trait not in runtime           | open → decided (CP1 §1 wiring confirmed)     | §1 (CP1 trait declaration)   |
| R-051 | `AcquireOptions::intent/.tags` reserved-but-unused                    | open → decided                               | §15.2 (`#[deprecated]`)      |
| R-053 | `integration/` module name collision                                  | open → decided                               | §5.4 (file-split absorbs)    |
| R-060 | Rotation path ships without observability                             | open → decided                               | §6 + §6.5 DoD gate           |

**Total flipped:** 22 `tech-spec-material` rows now status `decided` with section pointers. Register §"Lifecycle rules" rule 2 satisfied for CP4 freeze.

Remaining `open`-status rows in the register (post-flip): zero `tech-spec-material`; non-`tech-spec-material` rows (post-cascade, future-cascade, standalone-fix, invariant-preservation) keep their respective statuses per register lifecycle.

### §15.7 Cross-cascade amendment R2 — `on_credential_refresh` signature re-pin to credential CP5 §15.7 SchemeGuard shape — ENACTED

**Trigger.** Cross-cascade consolidated review 2026-04-26 ([`docs/superpowers/drafts/2026-04-24-cross-cascade-consolidated-review.md`](../drafts/2026-04-24-cross-cascade-consolidated-review.md)) §3.2.1 surfaced 🔴 STRUCTURAL `on_credential_refresh` parameter shape divergence: this Tech Spec §2.1 + ADR-0036 §Decision adopted the **pre-supersession** credential Tech Spec §3.6 borrowed-`&Scheme` shape; credential CP5 §15.7 supersedes to owned `SchemeGuard<'a, _>` shape (per [credential Tech Spec §15.7 line 3394-3429](2026-04-24-credential-tech-spec.md), iter-3 lifetime-pin refinement at line 3503-3516). ADR-0036 was accepted 2026-04-24 — same date credential CP5 supersession landed — supersession-propagation gap (cross-cascade review §6.3 Pattern A). Resource impls per pre-amendment §2.1 would violate the credential CP5 SchemeGuard zeroize / no-Clone / no-retention contract (compile-fail probe coverage gap per credential Tech Spec §16.1.1 probes #6, #7).

**Status invariant.** Per ADR-0035 amended-in-place precedent + cross-cascade review §7.1 path (a) routing: R2 is signature reconciliation between cascade Tech Specs that ratified at adjacent dates without cross-pin verification. R2 amendment-in-place applies to:

1. Resource Tech Spec §2.1 trait method signature (this section enacts).
2. Resource Tech Spec §2.1.1 idiomatic impl form example (this section enacts).
3. Resource Tech Spec §2.3 invariants documentation (this section enacts).
4. Resource Tech Spec §11.6 blue-green swap walkthrough example (this section enacts).
5. ADR-0036 §Decision conceptual signature + §Status frontmatter qualifier (separate Edit on ADR file per cross-cascade enactment per §15.7.5 below).

#### §15.7.1 Enactment

This CP **enacts** R2 in-place per ADR-0035 amended-in-place precedent. Tech Spec edits are inline at §2.1 / §2.1.1 / §2.3 / §11.6; ADR-0036 file edit lands as a separate enactment per §15.7.5 below (paralleling action Tech Spec §15.5.1 ADR-0039 amendment-in-place precedent).

**Per ADR composition analysis:**

- **ADR-0028** (cross-crate credential invariants) — unaffected. R2 amendment realigns this Tech Spec with the credential CP5 SchemeGuard contract; ADR-0028 §Decision items remain authoritative.
- **ADR-0030** (engine owns credential orchestration) — unaffected.
- **ADR-0033** (integration credentials Plane B) — unaffected.
- **ADR-0035** (phantom-shim capability pattern) — unaffected. R2 is at the rotation-hook layer, not at the phantom-shim/sealed-trait pattern layer.
- **ADR-0036** (resource Credential adoption) — amended-in-place per §15.7.5 below. §Decision conceptual signature re-pinned from `&Scheme` to `SchemeGuard<'a, _>`; §Status frontmatter gains `accepted (amended-in-place 2026-04-26 — cross-cascade R2)` qualifier; §"Amended in place on" gains entry.
- **ADR-0037** (Daemon + EventSource engine fold) — unaffected. R2 is at the Resource trait method shape layer, not at the engine-fold layer.

**Sections amended (R2 — multi-section enactment in this Tech Spec + ADR-0036):**

| Cross-cascade amendment | Document section | Class | Spike risk |
|---|---|---|---|
| **R2 trait signature** — `on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> impl Future + Send + 'a` per credential CP5 §15.7 canonical form | §2.1 trait declaration | AMEND | None — credential CP5 §15.7 spike iter-3 PASSED at the `SchemeGuard<'a, _>` shape; spike validation is upstream, not in this cascade |
| **R2 idiomatic impl** — `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> Result<...>` walkthrough; `&*new_scheme` Deref pattern; zeroize-on-Drop comment | §2.1.1 impl form example | AMEND | None — example follows trait signature |
| **R2 invariants** — borrow invariant replaced with owned-guard invariant; cite credential CP5 §15.7 line 3394-3429 + iter-3 lifetime-pin (line 3503-3516); cite probes #6, #7 | §2.3 first bullet | AMEND | None — invariant documentation only |
| **R2 walkthrough** — RealPostgresPool blue-green swap example; `SchemeGuard` parameter; `&*new_scheme` Deref pattern; zeroize-on-Drop comment | §11.6 example block | AMEND | None — walkthrough follows trait signature |
| **R2 ADR cross-pin** — ADR-0036 §Decision conceptual signature + §Status frontmatter qualifier + §"Amended in place on" entry | ADR-0036 §Decision + §Status + §"Amended in place on" | AMEND | None — ADR signature reconciliation |

**Picked: amendment-in-place per ADR-0035 precedent on all 5 sections.** Rationale per cross-cascade consolidated review §7.1 + `feedback_adr_revisable.md` ("ADRs are point-in-time; if following one forces workarounds, supersede it") + credential CP5 spike iter-3 validation:

1. **Path (a) per consolidated review §7.1** — re-pin to credential Tech Spec §15.7 CP5 SchemeGuard shape is the principled option (credential CP5 has spike-validated `SchemeGuard` shape per credential Tech Spec §15.7 line 3503-3516; reversal of credential CP5 — path (b) — would invalidate the upstream spike).
2. **Cross-source authority alignment.** Credential Tech Spec §3.6 line 970 explicitly states: "**Superseded by §15.7 (CP5 2026-04-24).**" Cited resource Tech Spec frontmatter line 14 references credential Tech Spec §3.6 lines 928-996 — but the cited range is the **superseded** shape per the credential Tech Spec's own §3.6 supersession header. R2 closes the cross-source authority drift.
3. **Compile-fail probe coverage restored.** Credential Tech Spec §16.1.1 probes #6 (`tests/compile_fail_scheme_guard_retention.rs` — Resource impl that stores `SchemeGuard` in struct field outlasting call) and #7 (`SchemeGuard` Clone attempt) test against the `SchemeGuard<'a, _>` shape. Pre-amendment §2.1 had no `SchemeGuard` parameter — probe fixtures could not be written. Post-amendment §2.1 enables the probe coverage.
4. **5 in-tree consumer migration.** All 5 in-tree `impl Resource for *` sites (per ADR-0036 §Negative line 121-122) migrate from `&Scheme` parameter shape to `SchemeGuard<'a, _>` parameter shape in the same PR wave per Strategy §4.8 atomicity. Migration is mechanical (parameter rename + `&*` Deref accessor pattern); zero community plugin migration impact (no community plugin yet implements the new Resource trait — frontier maturity per `docs/MATURITY.md:36`).

#### §15.7.2 Why amend-in-place vs supersede

Per ADR-0035 §Status block: amendments are valid for "canonical-form corrections" (cross-source-authoritative shape preservation under inconsistency); supersession is reserved for paradigm shifts. R2 is canonical-form correction — pre-amendment §2.1 cited the pre-supersession credential §3.6 shape; post-amendment §2.1 cites the post-supersession credential §15.7 shape. The Resource trait paradigm (5 assoc types + 9 lifecycle methods) is preserved; only the rotation-hook parameter type is re-pinned.

R2 lands as standalone post-freeze amendment (vs bundling with action-side R1 into a single CP) because R1 affects the action Tech Spec (different document, different authoring authority); this Tech Spec records R2 + ADR-0036 amendment, while R1 is recorded in action Tech Spec §15.13 with parallel cross-cascade-review citation.

#### §15.7.3 Cross-cascade and downstream impact

**ADR composition.** ADR-0028 + ADR-0030 + ADR-0033 + ADR-0035 + ADR-0037 — all preserved per §15.7.1 per-ADR composition analysis. ADR-0036 is amended-in-place per §15.7.5 below (separate ADR file edit).

**Production code impact.** R2 amendment requires production code change at implementation time (atomic per Strategy §4.8 PR wave):

- `crates/resource/src/resource.rs:233` — trait method signature changes from `fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> impl Future<Output = Result<(), Self::Error>> + Send` to `fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a`. Default body remains `async { Ok(()) }`.
- `crates/resource/src/manager.rs` — dispatcher path (per §3 of this Tech Spec) must hand `SchemeGuard<'a, _>` instead of `&Scheme` to each per-resource future. Spec-side dispatcher narrative in §3.2 line 731-857 references `&(dyn Any + Send + Sync)` parameter shape; post-implementation, Manager constructs `SchemeGuard` via `SchemeFactory<C>` per credential Tech Spec §15.7 line 3438-3447 and hands the owned guard to each per-resource future. Spec narrative in §3.2 + §3.6 retains the `&(dyn Any + Send + Sync)` shape at the type-erased dispatcher boundary; the typed-dispatcher (`TypedDispatcher<R>`) downcasts to typed `SchemeGuard<'a, R::Credential>` before calling `R::on_credential_refresh`. (Spec narrative re-pin is implementation-time, not amendment-time — the type-erased boundary signature is unchanged because erasure happens before the typed dispatch.)
- 5 in-tree consumer resources (`runtime/{daemon,pool,resident,service}.rs` + 9 test resources per ADR-0036 §Negative) — atomically migrate from `&Scheme` parameter shape to `SchemeGuard<'a, _>` parameter shape; `&*new_scheme` Deref pattern at `build_pool_from_scheme(...)` call sites.

**Reverse-dep impact.** Per existing §16.2 per-consumer migration sequence:

- `nebula-action` — zero sites this cascade (action is not a Resource implementer; cross-cascade R1 closed action's parallel-shape stub).
- `nebula-sdk` / `nebula-plugin` / `nebula-sandbox` — each crate's `impl Resource for *` sites migrate to `SchemeGuard<'a, _>` parameter pattern (atomically with this Tech Spec's parent PR wave per Strategy §4.8).
- `nebula-engine` — Manager dispatcher path constructs `SchemeGuard` from `SchemeFactory<C>` per credential Tech Spec §15.7; engine bootstrap unchanged at the type-erased boundary.

**Aggregate cross-cascade R2 footprint:** ~5 Resource impl sites + Manager dispatcher path narrative re-pin at implementation time. Atomic with the existing Strategy §4.8 PR wave; zero additional review rounds (R2 is included in the existing CP4 freeze ratification scope as cross-cascade alignment).

#### §15.7.4 §16.4 + §16.5 cascade-final precondition update

Tech Spec ratification (CP4 freeze) is unaffected — this amendment-in-place is post-freeze (per ADR-0035 amended-in-place precedent). §16.4 DoD checklist gains one new invariant:

- [ ] **Cross-cascade R2 absorbed.** Trait `on_credential_refresh` signature uses `SchemeGuard<'a, Self::Credential>` parameter (verifier: `rg "fn on_credential_refresh" crates/resource/src/resource.rs` returns the post-amendment signature shape; zero matches against pre-amendment `&<Self::Credential as Credential>::Scheme` parameter shape). Compile-fail probes per credential Tech Spec §16.1.1 probes #6, #7 fire as expected against fixtures that retain `SchemeGuard` in struct fields or attempt `Clone`.

#### §15.7.5 ADR-0036 amendment-in-place enactment

Per cross-cascade review §7.1 R2 path (a) routing, ADR-0036 §Decision conceptual signature requires re-pin to credential CP5 §15.7 `SchemeGuard<'a, _>` shape. ADR-0036 amendment lands as separate Edit on the ADR file (paralleling action Tech Spec §15.5.1 ADR-0039 amendment-in-place precedent):

1. **§Status frontmatter** — `status: accepted` → `status: accepted (amended-in-place 2026-04-26 — cross-cascade R2)`.
2. **§Decision** — conceptual signature re-pinned from `async fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> Result<(), Self::Error> { Ok(()) }` to `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> Result<(), Self::Error> { Ok(()) }`. Cross-ref credential Tech Spec §15.7 line 3394-3429 (canonical CP5 form) + iter-3 lifetime-pin refinement (line 3503-3516).
3. **§"Amended in place on"** — entry added: `2026-04-26 — cross-cascade R2: §Decision conceptual signature re-pinned from pre-supersession credential Tech Spec §3.6 borrowed-&Scheme shape to post-supersession credential Tech Spec §15.7 SchemeGuard<'a, _> shape per cross-cascade consolidated review §7.1 path (a) routing. Resource Tech Spec §15.7 records the corresponding multi-section amendment-in-place; Tech Spec §2.1 + §2.1.1 + §2.3 + §11.6 re-pinned. Counterpart action-side R1 (action §2.2.4 stub Resource trait removal) recorded in action Tech Spec §15.13.`

Status invariant: ADR-0036 retains `accepted` status (canonical-form correction per ADR-0035 §Status block precedent); amendment qualifier in frontmatter is the cross-cascade marker, not a status transition.

#### §15.7.6 §15.x closure entries

§15.5 Strategy §5.5 — revoke spec extension closure preserved verbatim. R2 affects the refresh-side trait signature only; the revoke-side hook signature (`on_credential_revoke(&self, credential_id: &CredentialId)`) is unchanged because revocation has no scheme to swap (per §2.3 line 462 + credential Tech Spec §3.6 line 990-991).

§15.6 concerns register status flips preserved verbatim. R-001 / R-002 / R-003 / R-004 rows remain `decided` post-amendment; the amendment refines the §2.1 trait shape but does not re-open the underlying concerns.

**Counterpart enactment for R1.** R1 (action Tech Spec §2.2.4 stub Resource trait removal — replace with `use nebula_resource::Resource;` import-only) is enacted in [action Tech Spec §15.13](2026-04-24-nebula-action-tech-spec.md). Both R1 and R2 originate from the same cross-cascade consolidated review §7.1 amendment routing.

## §16 — Implementation handoff

How CP1-CP3's design lands as code. Compact subsections; this is sequencing + DoD, not specification (the specification is §1-§13).

### §16.1 PR wave plan (atomic single-PR per Strategy §4.8)

**Decision: single atomic PR per Strategy §4.8 + ADR-0036 §Consequences positive.**

Strategy §4.8 commits to atomic 5-consumer migration in one PR wave; CP4 §16.1 reaffirms. Single-PR rationale: the trait reshape, reverse-index write, dispatcher implementation, observability scaffolding, file-split, drain-abort fix, Daemon/EventSource extraction, doc rewrite, and 5 consumer migrations all couple structurally — splitting the wave forces a "half-migrated state" in trunk that contradicts Strategy §0 freeze policy and `feedback_no_shims.md`.

**PR contents (non-exhaustive):**

- `crates/resource/src/` — trait reshape (§2.1), file-split (§9), reverse-index + dispatcher (§3.1-§3.2), Daemon/EventSource removal (§10.1, §12.5), observability wiring (§6).
- `crates/engine/src/daemon/` — NEW module (per §12.1); `DaemonRegistry` + `EventSourceAdapter` per §12.2-§12.3.
- `crates/{action,sdk,plugin,sandbox}/` — consumer migrations per §16.2.
- `crates/resource/docs/` — adapter contract rewrite per §11 content spec; api-reference.md, README.md, events.md, Architecture.md per Strategy §4.7.
- `crates/resource/tests/compile_fail/` — three carry-forward probes from spike + one new (§7.5).
- `docs/tracking/nebula-resource-concerns-register.md` — status flips per §15.6.

**Alternative considered: phased 2-3 PR sequence.** Rejected — violates atomicity invariant (security-lead BLOCK on Option A precedent). The phased alternative was Strategy §6.2 Phase C "default unless review surfaces a separable concern"; CP4 surfaces no separable concern.

**PR review reviewers:** tech-lead (overall ratification), security-lead (B-1/B-2/B-3/SL-1/SL-2/SL-3 invariant verification), rust-senior (trait surface + interface), dx-tester (adapter contract rewrite + DX), spec-auditor (cross-section + claim-vs-source).

### §16.2 Per-consumer migration sequence (action / sdk / engine / plugin / sandbox)

Per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md) consumer list. Each consumer's per-file change sequence:

| Consumer        | Daemon usage | EventSource usage | Migration steps                                                     |
|-----------------|--------------|-------------------|---------------------------------------------------------------------|
| `nebula-action` | None         | None              | Rewrite every `type Auth = ();` → `type Credential = NoCredential;` in `impl Resource for *`. Replace `register_pooled<R>` call sites — bound update only (signature shape unchanged). |
| `nebula-sdk`    | None         | None              | Same pattern as action; mostly mechanical. Re-export tree update if any sdk-side re-exports name `Resource::Auth`. |
| `nebula-engine` | Self (becomes home) | Self (TriggerAction adapter) | Substantive — see §12.1-§12.3. New `crates/engine/src/daemon/` module. Migrates 493 LOC daemon + 75 LOC EventSource code into engine-side. Adds `DaemonRegistry` construction in engine bootstrap. |
| `nebula-plugin` | None         | None              | Same as action / sdk.                                               |
| `nebula-sandbox`| None         | None              | Same as action / sdk.                                               |

**Mechanical verification.** Pre-PR `rg "type Auth = " crates/{action,sdk,engine,plugin,sandbox}/src/` returns the union of sites needing rewrite. Post-PR same query returns zero. `rg "Daemon|EventSource" crates/{action,sdk,plugin,sandbox}/src/` returns zero hits both pre- and post-PR per CP3 §12.4 verification.

**Order within PR (mechanical sub-ordering):** action / sdk first (smallest delta, no engine coupling), then plugin / sandbox (similar pattern), then engine (carries the substantive new module). All five land in one commit-set; the ordering is for review-narrative purposes, not git-history requirement.

### §16.3 Rollback strategy (if soak period reveals issues)

Per [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md) post-merge soak (1-2 weeks observability-driven). Rollback paths:

- **Targeted fix (preferred).** A 🔴-class issue surfaces during soak (e.g., dispatcher leaks `&Scheme` across an `await` window). Fix lands in `main` per §13.5 "post-cascade soak" rule ("only 🔴 fixes permitted; FROZEN otherwise") with a fresh soak-window restart. Atomic: no partial revert.
- **Feature-flag escape (NOT used).** Per `feedback_no_shims.md` and Strategy §4.8 atomicity, no feature flag pre-installed for rollback. The crate is `frontier`; hard breaking changes are the discipline.
- **Full revert (last resort).** If a structural problem (e.g., `Resource::Credential` shape breaks unexpectedly under real-load) escapes soak, revert the entire PR wave and re-open Strategy via Strategy §0 amendment cycle. The revert is mechanical (`git revert <merge-commit>`); the redesign cascade restarts at Phase 2 (scope round 2) with new evidence. This path is contemplated but not anticipated — Phase 4 spike PASSED iter-1 with all 7 exit criteria met, and CP1-CP3 review cycles surfaced no shape-breaking concerns.

**Soak failure thresholds.** Fire targeted-fix path: counter `errors` label increments at non-trivial rate (>0.1% of attempts); structural panic in any rotation path; consumer test regression that didn't surface pre-merge. Fire full-revert path: shape-level invariant violation (e.g., a credential is observably retained beyond `on_credential_revoke` post-dispatch).

### §16.4 Definition of done checklist (CI green + 7 invariants)

CP4 freeze gates merge-readiness; merge gates DoD. Both checks below must hold for the redesign to be declared "complete":

**CI gate (must be green pre-merge):**

- `cargo +nightly fmt --all -- --check` clean (per `.github/workflows/ci.yml` lines 60-66).
- `cargo clippy --workspace -- -D warnings` clean (per `.github/workflows/ci.yml` lines 87-88).
- `cargo nextest run -p nebula-{resource,action,sdk,engine,plugin,sandbox} --profile ci` green (per `.github/workflows/test-matrix.yml` lines 160-164).
- `cargo doc --all --no-deps` clean (per §11.1 doc-rewrite acceptance gate).
- `cargo test --doc` green for non-`ignore` blocks in `crates/resource/docs/`.

**Seven invariants (must be verified post-merge in soak window):**

1. **No `todo!()` in `Manager` rotation paths.** Verifier: `rg "todo!" crates/resource/src/manager/` returns zero. Resolves §3.1 + §3.2 (CP1).
2. **Reverse-index populated atomically with registry write.** Verifier: §7.2 wire test `register_pop_atomic` per §3.1.
3. **Per-resource isolation invariant.** Verifier: §7.4 `parallel_dispatch_isolates_per_resource_errors` carry-forward from spike. Security B-1.
4. **Manager NEVER holds `&Scheme` across dispatch boundary.** Verifier: §11.6 walkthrough lint + spec-auditor manual trace; security constraint #2.
5. **`Scheme::default()` NOT called at warmup.** Verifier: `rg "::default\(\)" crates/resource/src/manager/` returns zero matches against `R::Auth` / `R::Credential::Scheme`. Security B-3.
6. **Rotation dispatch emits trace + counter + event.** Verifier: §6.5 DoD gate; trace span in collected traces; counter non-zero on first rotation; `ResourceEvent::CredentialRefreshed` broadcast end-to-end. Strategy §4.9.
7. **Drain-abort lands phase = `Failed`, not `Ready`.** Verifier: §7.2 wire test `drain_abort_records_failed_phase_not_ready`. Phase 1 🔴-4 fix.

**Verification cadence:** invariants 1-2 verified at PR merge (CI integration tests); 3-7 verified during soak window (1-2 weeks per Strategy §6.3); MATURITY bump (§16.5) gates on all seven holding zero-defect through soak.

### §16.5 MATURITY transition trigger (Strategy §6.4)

`nebula-resource = frontier` today ([`docs/MATURITY.md:36`](../../MATURITY.md)). Post-cascade target: `core` (or `stable` per the `MATURITY.md` legend). Trigger conditions (from Strategy §6.4):

- **Soak window clean.** Counter `nebula_resource.credential_rotation_attempts` `errors` label shows zero 🔴-class signal across the 1-2 week post-merge soak.
- **Register zeroed.** Phase 7 register's `tech-spec-material` rows all `decided` (§15.6 already satisfies this); no `open` rows remain post-soak.
- **Consumer tests pass.** All 5 in-tree consumers green per §16.4 CI gate, sustained through soak.
- **Doc surface clean.** `cargo doc --all --no-deps` + `cargo test --doc` consistently green; dx-tester re-evaluation reports zero compile-fail walkthroughs (vs current ~50% baseline per Phase 1 🔴-3).

**Bump proposal path.** Architect proposes `frontier → core` bump in Phase 8 cascade-completion summary (per Strategy §6.1 milestone Phase 8); tech-lead ratifies in dedicated PR per [`docs/MATURITY.md`](../../MATURITY.md) review cadence. The bump PR is *separate* from the migration PR (per §16.1) — migration ships at `frontier`; the maturity bump is a post-soak observability-validated decision.

**Register closure ties to maturity transition.** Register §"Close condition" = `MATURITY.md` transition `frontier` → `core` (verified at register §"Owner during cascade" → "Owner post-cascade" handoff). After bump, register transitions to `completion-frozen` status; further concerns become new register entries in subsequent cascades, not retroactive amendments.

CP4 freeze unblocks the migration PR; migration merge unblocks soak; soak completion + invariant verification unblocks maturity bump proposal. The cascade arc closes when `MATURITY.md` records `nebula-resource = core`.

### Open items raised this checkpoint

**CP1 open items (unchanged from CP1 ratification):**

- **§2.4 — TopologyRuntime engine-side landing.** ADR-0037 commits to engine-fold; engine-side module layout, primitive naming (`DaemonRegistry` / `WorkerRuntime` / etc.), `EventSource → TriggerAction` adapter signature is **CP3 §13 deliverable**. Out of CP1 scope.
- **§3.4 — `FuturesUnordered` fan-out at N>32.** [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) defers; CP1 records the soft cap (32) as configuration; production landing of FuturesUnordered fan-out conditional on operational signal post-soak per [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **§3.5 — Per-resource event broadcast cardinality bound.** CP1 LOCKED aggregate-only `ResourceEvent::CredentialRefreshed { resources_affected, outcome: RotationOutcome }` for the refresh path and aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` (only on `RevokeOutcome != Ok`) for the revocation path per security B-2. **Resolved in CP2 §6.4**: per-resource `HealthChanged` events are bounded by the existing `event_tx` capacity of 256; deployments registering >256 resources against one credential need to surface broadcast capacity via `ManagerConfig` (CP3 §11 ergonomics).
- **§2.3 — Idempotency expectation on `on_credential_refresh`.** CP2 §6 was scheduled to finalize whether Manager retries on transient failures; **CP2 leaves Manager retry policy out of scope** (consumers decide; default is no retry) and preserves the implementer-side idempotency expectation from CP1 §2.3.

**CP2 open items (raised this checkpoint):**

- **§5.3 — Multi-tenant pool taint exception.** CP2 commits unconditional taint-flip on revocation (option (b)). A future consumer with multi-tenant pool sharing one resource across many credentials may need a per-`RegisterOptions::tainting_policy` knob to opt out. Trigger to revisit: a real consumer (not synthetic test) that needs the exception. CP3 §11 candidate.
- **§5.4 — Function-level boundaries within submodules.** CP2 locks file structure (which submodule each public method lands in) but defers the per-`impl Manager { fn ... }` block placement to CP3 §9. Reviewer note: cut points within `mod.rs` may surface ergonomics that warrant restructuring; CP3 captures.
- **§6.3 — `ResourceEvent::key()` return type.** New `CredentialRefreshed` / `CredentialRevoked` variants are credential-scoped, not resource-scoped — current `key() -> &ResourceKey` is no longer total. CP3 §12 picks: `Option<&ResourceKey>` (breaking subscriber change) or orthogonal `credential_id() -> Option<&CredentialId>` accessor (additive). CP2 commits the *variants*; defers return-type to CP3.
- **§6.5 — Histogram bucket boundaries.** CP2 commits the histogram metric *name* and *labels* but defers bucket configuration to CP3 §11 (default `[0.001, 0.01, 0.1, 1.0, 10.0, 60.0]` seconds is a candidate; production tuning post-soak per Strategy §6.3).
- **§7.6 — Coverage tooling pick.** CP2 sets the 80% target; CP3 §13 picks `cargo tarpaulin` vs `cargo llvm-cov` based on Windows + macOS coverage support. **Resolved at CP3** — addressed in evolution-policy §13.5 implicitly (post-`core` posture); concrete tool pick deferred to migration PR (Strategy §6.2 implementation wave). Either tool meets the 80% target; `cargo llvm-cov` preferred for Windows + macOS parity if maintainer-tested by migration time, falling back to `cargo tarpaulin` otherwise.

**CP3 open items (raised this checkpoint):**

- **§9.7 — `gate.rs` vs `execute.rs` filename.** CP2 §5.4 used `execute.rs`; CP3 §9.7 renames to `gate.rs` per the dominant content shape. Reviewer pushback option — keep `execute.rs` per CP2 lock — would require restoring the filename without changing contents. Architect recommends `gate.rs` (the gate-admission state machine is more representative of the file's purpose than the resilience-execution wrapper). Tech-lead ratifies in CP3 review.
- **§10.2 — register_* method count (10 helpers).** CP3 §10.2 commits to dual helpers per topology = 10 public methods on `Manager`. Reviewer concern surfaced in CP2 ratification ("does this fit the public-surface budget?") — CP3 records the trade-off (migration parity + DX symmetry + compile-time enforcement) but the count of 10 is itself a load-bearing decision. If DX feedback during the surface-review wave (or implementation PR review) flags 10 as over-budget, the alternative is to fold the dual into a single `register_pooled_with(RegisterOptions)`-only API at the cost of mandatory boilerplate for unauthenticated registrations (60% of current consumers). Architect prefers dual; dx-tester reviews specifically.
- **§11 — adapters.md rewrite scope.** CP3 §11 is the *content spec* for the rewrite. The actual rewrite is a Phase 8 deliverable per [Strategy §4.7](2026-04-24-nebula-resource-redesign-strategy.md), bundled in the implementation PR wave. CP3 commits to §11 as the content; the `crates/resource/docs/adapters.md` file is touched at PR-implementation time, not now.
- **§12.1 — `crates/engine/src/daemon/` module path.** CP3 commits to this path. Engine team has not weighed in on the path choice (their work happens at PR-implementation time per Strategy §4.4). If engine team prefers a different path during implementation, CP3 §12.1 records the rationale for `daemon/` (rejection of `runtime/` and `scheduler/` alternatives); engine team override is permitted via amendment cycle but not expected.
- **§12.4 — engine self-migration scope.** §12.4 lists `crates/engine/` as the migration-target consumer (engine becomes the *home* of Daemon/EventSource). Implementation work is engine-side and substantial (493+75 LOC moved + new `DaemonRegistry` struct + `EventSourceAdapter`). CP3 §12 specifies the contract; the implementation work is engine-team's responsibility within the migration PR wave.
- **§13.1 — MATURITY.md row update timing.** CP3 §13.1 commits to "post-soak `core` bump" but does not currently propose the bump. The bump is a separate PR per [`docs/MATURITY.md`](../../MATURITY.md) review cadence; CP4 §16 records the migration-PR-completion handoff that triggers the bump proposal.

### Handoffs requested

**CP1 handoffs (closed at CP1 ratification — preserved as historical record):**

- spec-auditor (CP1): structural audit. CLOSED 2026-04-25 with PASS_WITH_MINOR per CP1 changelog.
- rust-senior (CP1): trait-shape ratification. CLOSED 2026-04-25 with RATIFY_WITH_EDITS per CP1 changelog.
- tech-lead (CP1): ratification → flipped ADR-0036 + ADR-0037 to `accepted`. CLOSED 2026-04-25 per CP1 changelog.

**CP2 handoffs requested (parallel co-decision review, no architect iteration between):**

- **tech-lead**: CP2 ratification of §4 lifecycle + §5 mechanism + §6 observability + §7 testing + §8 storage. Specifically scrutinize: (a) §5.3 revocation default-tainting decision (option (b)) — security implication central, lock vs question; (b) §6.1-§6.3 observability identifier locks (every name is CP-review-gateable per §6 prelude; reviewer pushback on names lands as amendment, not CP3 rename); (c) §5.4 file-split cut points — submodule list locks CP3 §9 function-level work.
- **security-lead**: CP2 ratification of all security-axis content. Specifically scrutinize: (a) §5.3 revocation default-tainting (option (a)/(b)/(c) trade-off — confirm option (b) honours [B-1 / constraint #2 invariant](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)); (b) §5.2 `warmup_pool` credential-bearing signature (B-3 amendment honoured? `Scheme::default()` removed?); (c) §6.3 per-resource `HealthChanged { healthy: false }` on revocation failure (B-2 amendment honoured? cardinality acceptable?); (d) §5.1 `Arc<RwLock<Pool>>` vs `ArcSwap` — Manager NEVER holds scheme longer than dispatch call (constraint #2 invariant)?
- **spec-auditor** (after tech-lead + security-lead converge): CP2 structural audit. Verify cross-section consistency (every § forward ref to CP3/CP4 is real, every code-block-cited-line is in the cited file), forward-reference integrity (no §6.5 DoD claim about a §7 test that doesn't exist in §7), claim-vs-source (every "per Strategy §X" is in Strategy §X; every spike `lib.rs:N-M` line range is in the spike file). Pay specific attention to §5.3 — three options enumerated; verify each rejection rationale derives from the cited source (Strategy §4.2, security review).

**CP3 handoffs requested (parallel review; co-decision body when tech-lead + dx-tester converge):**

- **dx-tester**: CP3 ratification of §11 adapter authoring contract specifically. Specifically scrutinize: (a) §11.2 minimum `Resource` impl shape — is this the minimum a newcomer needs to write a working adapter, or does the walkthrough miss a step (e.g., `HasSchema` derivation, `ResourceConfig::validate` shape)? (b) §11.3 topology selection guide — are the "common selection mistakes" the right traps to surface? (c) §11.5 credential-bearing walkthrough — does the `RealPostgresPool` shape compile against a hypothetical `nebula-credential-postgres` crate, or does it surface API gaps in the `Credential` trait? (d) §11.8 common pitfalls — are these the actual newcomer traps, or are there others surfaced by Phase 1 dx-tester input? (e) §10.2 dual-helper decision (10 public `register_*` methods) — does this fit the DX budget, or is the unified-`RegisterOptions`-only path more honest about the credential-bearing requirement?
- **tech-lead**: CP3 ratification of §9 + §10 + §12 + §13. Specifically scrutinize: (a) §9 function-level cuts — are the per-method submodule assignments correct given current `manager.rs` line ranges? (b) §10.2 register_* dual-helper decision — accept the trade-off (10 methods over 5 + RegisterOptions); (c) §10.5 SL-1 deferral — confirm `tainting_policy` stays out of CP3 surface (gate 1 + gate 2 not cleared); (d) §12.1 engine module path (`crates/engine/src/daemon/`) — engine team coordination; (e) §13 evolution policy — versioning posture, breaking-change discipline, freeze schedule.
- **spec-auditor** (after tech-lead + dx-tester converge): CP3 structural audit. Verify cross-section consistency (every §9 method citation is in the actual `manager.rs` line range; every §10.1 re-export line is in the proposed `lib.rs`; every §11 walkthrough type is consistent with §2 trait shape; every §12 ADR-0037 cross-ref is in the amended ADR text), forward-reference integrity (every CP4 forward-ref in §13.1 (MATURITY bump) and §13.5 (post-soak schedule) is consistent), claim-vs-source (every "per Strategy §X" lands in Strategy §X; every spike file path resolves). Pay specific attention to §9.7 — the CP2 → CP3 file rename (`execute.rs` → `gate.rs`) is a deliberate refinement; verify the freeze-policy permits this kind of CP2-to-CP3 cut adjustment per §0.3.

## Changelog

- 2026-04-25 CP1 draft — §0 + §1 + §2 + §3 (architect)
- 2026-04-25 CP1 ratified (architect; tech-lead RATIFY_WITH_EDITS + rust-senior RATIFY_WITH_EDITS + spec-auditor PASS_WITH_MINOR). Edits applied: §3.2 `Result<…, Error>` wrapper restored on `on_credential_refreshed`; §3.5 `RotationOutcome` aggregate type defined; §3.5 event broadcast contract LOCKED to aggregate-only refresh + aggregate revoke + per-resource `HealthChanged` on revoke failure (B-2); §3.2 dispatcher lifetime SAFETY comment added; §3.1 NEW error constructors (`Error::missing_credential_id`, `Error::scheme_type_mismatch::<R>()`) called out as required additions. ADR-0037 acceptance gate amended in place to gate on the engine-fold *decision*, not the engine-side *implementation* (which is CP3 §13).
- 2026-04-26 CP2 draft — §4 lifecycle + §5 implementation specifics + §6 operational + §7 testing + §8 storage (architect). Status flipped to `CP2 draft — awaiting tech-lead + security-lead review` per cadence. Key locks: §5.3 revocation default-hook = option (b) Manager-enforced taint flip; §5.1 pool swap = `Arc<RwLock<Pool>>` (NOT `ArcSwap`); §5.2 `warmup_pool` takes credential scheme explicitly + dedicated `warmup_pool_no_credential` for opt-out; §6.1 six trace span names locked; §6.2 five counter metrics locked; §6.3 two new `ResourceEvent` variants + per-resource `HealthChanged` on revoke failure (B-2 honoured); §5.4 seven-submodule file-split cut points locked; §5.5 `ManagedResource::set_failed` wired into `DrainTimeoutPolicy::Abort` path (Phase 1 🔴-4 closed).
- 2026-04-24 CP2 ratified (architect; tech-lead RATIFY_WITH_EDITS + security-lead ENDORSE_WITH_AMENDMENTS). Status flipped to `CP2 ratified — pending CP3 dispatch`. Six bounded edits applied: **E1** §6.3 line 1357 — "Three variants added" → "Two new variants added; existing `HealthChanged` reused per B-2"; **E2** §1.2 — 5-submodule list reconciled to §5.4's 7-submodule list with explicit "extending Strategy §4.5" rationale (registration.rs holds 🔴-1 fix; shutdown.rs holds 🔴-4 fix); **E3** §6.3 line 1391 — `event()::key()` CP3 §12 forward-ref tightened to pin (a) `RegisterOptions::register_with_event_filter` vs (b) crate-level filter trait trade-off and require it close in the same CP3 wave that lands the new variants; **SL-2** §7.4 — three security-axis concurrency tests added (`revoke_during_inflight_acquire`, `concurrent_refresh_plus_revoke`, `revoke_during_refresh`); **SL-3** §5.1 — resource-side `build_pool_from_scheme` budget guidance added (≤ 60% of dispatch timeout) with CP3 §11 forward-ref; **SL-1** new §5.6 — CP3 deferrals subsection capturing future `RegisterOptions::tainting_policy` knob + future `warmup_pool_by_id` helper with their gating constraints. §5 prelude updated five-items → six-items. ADR-0036 + ADR-0037 already at `accepted` per CP1 ratification — CP2 lock is a Tech Spec internal milestone, not an ADR gate. Awaiting orchestrator dispatch of CP3 (§9-§13 interface + ergonomics).
- 2026-04-24 CP3 draft — §9 manager file split (function-level cuts) + §10 public API surface + §11 adapter authoring contract + §12 Daemon/EventSource engine landing + §13 evolution policy (architect). Status flipped to `CP3 draft — awaiting spec-auditor + dx-tester + tech-lead review` per cadence. Key locks: §9 seven-submodule function-level assignments enumerated against current `manager.rs` line ranges; §9.7 CP2 file rename `execute.rs` → `gate.rs` (refinement, freeze-policy permitted per §0.3); §10.2 dual-helper register_* decision (5 topologies × {NoCredential shortcut, credential-bearing} = 10 public methods; rejected unified-`RegisterOptions`-only alternative for migration parity + DX symmetry); §10.4 `RegisterOptions` final shape with `credential_id` + `credential_rotation_timeout` fields (`#[non_exhaustive]` builder pattern); §10.5 SL-1 `tainting_policy` confirmed deferred (gate 1 + gate 2 not cleared); §11 adapter contract content spec for `crates/resource/docs/adapters.md` rewrite (8 subsections covering imports, minimum impl, topology guide, NoCredential opt-out, credential-bearing walkthrough, override pattern, testing, common pitfalls); §12.1 engine module path = `crates/engine/src/daemon/` (rejected `runtime/` and `scheduler/` alternatives); §12.5 `TopologyRuntime<R>` enum shrink 7 → 5 enumerated; §13 evolution policy commits to no-shims discipline + post-`core` deprecation cycle + freeze schedule (in-cascade, post-cascade soak FROZEN, post-soak `frontier`, post-`core`-bump). CP3 deferrals: §7.6 coverage tool pick deferred to migration PR; CP4 §16 records `core` maturity bump trigger. ADR-0036 + ADR-0037 unchanged at `accepted`. Awaiting parallel review (spec-auditor + dx-tester + tech-lead).
- 2026-04-24 CP3 review edits applied + CP4 draft (architect; final architect pass per orchestrator dispatch). Status flipped to `CP4 draft — awaiting spec-auditor + tech-lead review (final review round before FROZEN)`. **Six CP3 review edits applied inline:** **E1** (tech-lead) §9.5 dispatcher visibility — `ResourceDispatcher` declared `pub(crate)` (not `pub`) + `#[doc(hidden)] pub use ... as __internal_ResourceDispatcher` re-export from `lib.rs` (§10.1 line 1783) for production test access; clean shape replaces "two-faced" framing. **E2** (tech-lead) §9.6 cross-pointer added noting `wait_for_drain` moved from CP2 §5.4 `execute.rs` to CP3 §9.6 `shutdown.rs`. **E3** (tech-lead) §10.2 line 1817 — total `register*` public surface clarified as 11 (10 dual helpers + 1 type-erased `register<R: Resource>`). **DX-1** (dx-tester) §11.1 imports — added `RegisterOptions`, `PoolConfig`, `AcquireOptions` to import list with downstream-reference notes (`NoScheme` retained for projection sites). **DX-2** (dx-tester) §11.2 compile-claim — over-claim "every line compiles against trunk" replaced with honest illustrative-shape framing; `rust,ignore` annotation; `Pooled` impl line added; spike `MockKvStore`/`MockHttpClient` lines 125-200 cited as canonical compile-checked baseline. **DX-3** (dx-tester) §11.5 + §11.6 `rust,ignore` annotation on hypothetical `PostgresCredential`/`build_pool_from_scheme` blocks with explicit "no such crate exists in trunk" framing. **CP4 §14-§16 drafted:** §14 cross-references (six tables: Strategy §4 → Tech Spec, ADR-0036/0037 + amendment record, credential Tech Spec §3.6/§Credential::revoke/§4.3, Phase 1 🔴/🟠 finding map, register link, spike artefact link); §15 open items resolution (§15.1-§15.5 close all five Strategy §5 items: §5.1 daemon revisit triggers concretized, §5.2 `AcquireOptions::intent/.tags` final treatment = `#[deprecated]`, §5.3 `Runtime`/`Lease` collapse future-cascade trigger confirmed, §5.4 `NoCredential` symmetry closed via §10.2 dual-helper, §5.5 revoke spec extension closed via Strategy §4.2 footnote; §15.6 register status flips for all 22 `tech-spec-material` rows to `decided`); §16 implementation handoff (§16.1 atomic single-PR per Strategy §4.8, §16.2 per-consumer migration sequence action/sdk/engine/plugin/sandbox, §16.3 rollback strategy, §16.4 DoD checklist with CI gate + 7 invariants, §16.5 MATURITY transition trigger). Awaiting parallel review (spec-auditor + tech-lead) before FROZEN.

---

**Tech Spec CP1 + CP2 + CP3 ratified.** ADR-0036 and ADR-0037 (per its 2026-04-25 amended-in-place gate text) both at `accepted` on CP1 ratification. CP3 review edits applied; CP4 (§14-§16 meta + open items + handoff) drafted. Awaiting spec-auditor + tech-lead final review before Tech Spec FROZEN.

- 2026-04-26 amended-in-place — cross-cascade R2 per §15.7 (architect; cross-cascade consolidated review 2026-04-26 §7.1 path (a) routing). `on_credential_refresh` signature re-pinned from pre-supersession credential Tech Spec §3.6 borrowed-`&Scheme` shape to post-supersession credential Tech Spec §15.7 `SchemeGuard<'a, _>` shape (canonical CP5 form). Sections amended: §2.1 trait declaration (parameter shape + lifetime form + default body), §2.1.1 idiomatic impl form example (PostgresPool walkthrough with `&*new_scheme` Deref pattern + zeroize-on-Drop comment), §2.3 invariants documentation (borrow invariant replaced with owned-guard invariant; cite credential CP5 §15.7 line 3394-3429 + iter-3 lifetime-pin line 3503-3516 + probes #6, #7), §11.6 RealPostgresPool blue-green swap example (parameter shape + Deref pattern + zeroize-on-Drop comment), §15.7 NEW enactment record (per-ADR composition analysis; amendment table; impact analysis; ADR-0036 §15.7.5 counterpart enactment). Status frontmatter qualifier appended `+ amended-in-place 2026-04-26 — cross-cascade R2`. Counterpart action-side R1 (action Tech Spec §2.2.4 stub Resource trait removal) recorded in [action Tech Spec §15.13](2026-04-24-nebula-action-tech-spec.md). ADR-0036 §Decision conceptual signature + §Status frontmatter + §"Amended in place on" updated as separate Edit per §15.7.5.
