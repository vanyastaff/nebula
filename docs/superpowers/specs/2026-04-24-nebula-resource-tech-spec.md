---
name: nebula-resource tech spec (implementation-ready design)
status: CP1 ratified — pending CP2 dispatch
date: 2026-04-25
authors: [architect (subagent dispatch)]
scope: nebula-resource — single-crate redesign; 5 in-tree consumers migrate atomically
cascade_phase: Phase 6 CP1
strategy: docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md (FROZEN CP3)
adrs:
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md (proposed-pending-CP1)
  - docs/adr/0037-daemon-eventsource-engine-fold.md (proposed-pending-CP1)
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

§0 → §1 (goals + non-goals) → §2 (trait contract) → §3 (runtime model) → §4 (lifecycle, CP2) → §5 (storage schema, CP2) → §6 (security, CP2) → §7 (operational, CP2) → §8 (testing, CP2) → §9-§13 (interface, CP3) → §14-§16 (meta + open items + handoff, CP4).

CP1 readers: load Strategy §4, ADR-0036 §Decision, ADR-0037 §Decision, [credential Tech Spec §3.6 lines 928-996](2026-04-24-credential-tech-spec.md), and [`spike/NOTES.md`](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md) before reading §2-§3 of this document. Citations are dense; load order matters.

## §1 — Goals and non-goals

### §1.1 Primary goals

- **Replace `Resource::Auth` with `Resource::Credential`.** Adopt [credential Tech Spec §3.6 lines 935-955](2026-04-24-credential-tech-spec.md) shape verbatim per [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md). `type Credential: Credential` becomes the trait-level credential binding; `<Self::Credential as Credential>::Scheme` flows into `create` and the new rotation hooks. `type Auth: AuthScheme` is removed (no shim, no alias) per `feedback_no_shims.md` discipline encoded in [Strategy §2.4](2026-04-24-nebula-resource-redesign-strategy.md).
- **Eliminate the silent revocation drop.** Resolve [Phase 1 finding 🔴-1](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md): `Manager::on_credential_refreshed` at [`crates/resource/src/manager.rs:1360`](../../../crates/resource/src/manager.rs) and `on_credential_revoked` at [`:1386`](../../../crates/resource/src/manager.rs) terminate in `todo!()` over an empty reverse-index. The reverse-index write path lands atomically with the dispatchers in this spec.
- **Per-resource rotation hooks with isolation.** New trait methods `on_credential_refresh` + `on_credential_revoke` with default no-op bodies. Manager dispatches via parallel `join_all` with per-resource timeout enforcement per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) + [security amendment B-1](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). One slow / failing resource cannot block siblings.
- **Atomic landing.** Trait reshape, reverse-index write path, dispatcher implementation, observability (trace + counter + event), 5-consumer migration, doc rewrite, Daemon/EventSource extraction land in one PR wave per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md).

### §1.2 Secondary goals

- **Daemon and EventSource extraction.** Per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md), Daemon and EventSource topologies leave `nebula-resource` and fold into the engine layer. The `TopologyRuntime<R>` enum at [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) shrinks 7 → 5 variants. Pool / Resident / Service / Transport / Exclusive remain. Engine-side landing site (module layout, primitive naming, `EventSource → TriggerAction` adapter signature) is CP3 §13 deliverable.
- **`manager.rs` file-split.** [Strategy §4.5](2026-04-24-nebula-resource-redesign-strategy.md) keeps the `Manager` type monolithic and splits the 2101-line file into submodules (`mod.rs`, `options.rs`, `gate.rs`, `execute.rs`, `rotation.rs`). Public API does not change. Cut points finalized in CP2 §5.
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
    fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = new_scheme;
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

    async fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> Result<(), Self::Error> {
        let new_pool = build_pool_from_scheme(new_scheme).await?;
        let mut guard = self.inner.write().await;
        *guard = new_pool;
        Ok(())
    }
}
```

The blue-green swap example mirrors [credential Tech Spec §3.6 lines 981-993](2026-04-24-credential-tech-spec.md). Old connections drain naturally as their RAII guards drop; new queries use the new pool (read lock acquires against the new inner after swap).

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

**`on_credential_refresh(new_scheme)`:**

- **Borrow invariant.** `new_scheme: &<Self::Credential as Credential>::Scheme` — borrowed, not owned. Per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) hot-path invariant, no `Scheme::clone()` on the dispatcher path; each clone is another zeroize obligation per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md). Resource impls pull what they need from the borrowed scheme inside the await window.
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

### Open items raised this checkpoint

- **§2.4 — TopologyRuntime engine-side landing.** ADR-0037 commits to engine-fold; engine-side module layout, primitive naming (`DaemonRegistry` / `WorkerRuntime` / etc.), `EventSource → TriggerAction` adapter signature is **CP3 §13 deliverable**. Out of CP1 scope.
- **§3.4 — `FuturesUnordered` fan-out at N>32.** [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) defers; CP1 records the soft cap (32) as configuration; production landing of FuturesUnordered fan-out conditional on operational signal post-soak per [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **§3.5 — Per-resource event broadcast cardinality bound.** CP1 LOCKS aggregate-only `ResourceEvent::CredentialRefreshed { resources_affected, outcome: RotationOutcome }` for the refresh path and aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` (only on `RevokeOutcome != Ok`) for the revocation path per security B-2. CP2 §7 deliverable: upper-bound cardinality strategy for the revocation `HealthChanged` events when N is large (broadcast channel capacity is 256 at [`manager.rs:275`](../../../crates/resource/src/manager.rs)); the *shape* of the per-resource events is decided.
- **§2.3 — Idempotency expectation on `on_credential_refresh`.** CP2 §6 finalizes whether Manager retries on transient failures; CP1 commits to the implementer-side idempotency expectation (resources SHOULD treat repeated invocations as no-op).

### Handoffs requested

- **spec-auditor**: CP1 structural audit. Verify cross-section consistency (every § forward ref resolves), claim-vs-source (every "per Strategy §X" is in Strategy §X; every spike file:line is in the spike), forward-reference integrity (CP2-CP4 markers don't leak content). Pay specific attention to §2.5 Q1-Q5 — each resolution claims rationale; verify the rationale derives from the cited source.
- **rust-senior**: CP1 trait-shape ratification. Verify §2.1 Resource trait compiles against current `nebula_credential::Credential` shape ([`crates/credential/src/contract/credential.rs:100-127`](../../../crates/credential/src/contract/credential.rs)); §2.4 topology sub-traits compose against the parent reshape; §3.2 dispatcher trampoline `Send + Sync` invariants are honored. Specifically scrutinize §2.5 Q2 (`TypeId` over sealed-trait) and Q4 (per-Manager + per-resource timeout) — these are trade-off calls where rust-senior judgment is decisive.
- **tech-lead**: CP1 ratification → flips ADR-0036 + ADR-0037 from `proposed` to `accepted` per their respective acceptance gates. Specifically scrutinize §2.5 Q3 (Box::pin acceptance + 32-concurrency default — tech-lead Phase 2 mentioned the number; CP1 commits it as a default) and §3.6 Phase 1 finding resolutions (every claim that the redesign "resolves" a 🔴 is a load-bearing commitment).

## Changelog

- 2026-04-25 CP1 draft — §0 + §1 + §2 + §3 (architect)
- 2026-04-25 CP1 ratified (architect; tech-lead RATIFY_WITH_EDITS + rust-senior RATIFY_WITH_EDITS + spec-auditor PASS_WITH_MINOR). Edits applied: §3.2 `Result<…, Error>` wrapper restored on `on_credential_refreshed`; §3.5 `RotationOutcome` aggregate type defined; §3.5 event broadcast contract LOCKED to aggregate-only refresh + aggregate revoke + per-resource `HealthChanged` on revoke failure (B-2); §3.2 dispatcher lifetime SAFETY comment added; §3.1 NEW error constructors (`Error::missing_credential_id`, `Error::scheme_type_mismatch::<R>()`) called out as required additions. ADR-0037 acceptance gate amended in place to gate on the engine-fold *decision*, not the engine-side *implementation* (which is CP3 §13).

---

**Tech Spec CP1 ratified.** ADR-0036 and ADR-0037 (per its 2026-04-25 amended-in-place gate text) both clear-able to `accepted` on this ratification. CP2 (§4-§8) follows.
